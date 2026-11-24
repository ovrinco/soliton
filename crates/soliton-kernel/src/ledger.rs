//! The data plane: one SQLite ledger of dated observations beside the planner's
//! object files (design S2a, `DATA_PLANE.md`). Rows are appended and corrected
//! by supersession, never edited. Planner items are joined by id at query time
//! and never copied in. The planner works without this file: it is only
//! written when it exists, and removing it is the undo.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;

use crate::date::CivilDate;
use crate::error::{KernelError, Result};
use crate::id::ObjectId;
use crate::model::DataClass;

pub mod finance;
pub mod habits;
pub mod kitchen;

pub const LEDGER_FILE: &str = "ledger.sqlite";
pub const SCHEMA_VERSION: i64 = 2;
const SCHEMA: &str = include_str!("ledger.sql");
/// Version 1 to 2, additive (S4b): the kitchen and health kinds, the finance and
/// health module marks, the statement mapping table. Its undo is in the file.
const MIGRATION_2: &str = include_str!("ledger_v2.sql");

impl From<rusqlite::Error> for KernelError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Ledger(error.to_string())
    }
}

/// One kind of observation and the module that owns it. Adding a kind is one
/// row; no column, table or other module changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KindDef {
    pub kind: String,
    pub writer: String,
    pub default_class: DataClass,
    pub unit: Option<String>,
    pub description: String,
}

/// An observation to append. `data_class` defaults to the kind's class and may
/// only raise it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewObservation {
    pub kind: String,
    pub observed_on: String,
    pub observed_at: Option<String>,
    pub item_id: Option<String>,
    pub category: Option<String>,
    pub subject: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub data_class: Option<DataClass>,
    pub writer: String,
    pub source_ref: Option<String>,
    pub private: bool,
    pub extra: Option<Value>,
}

/// Scopes of the hide-all marks above row level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkScope {
    Item,
    Category,
    Goal,
    Module,
}

impl MarkScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Category => "category",
            Self::Goal => "goal",
            Self::Module => "module",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "item" => Ok(Self::Item),
            "category" => Ok(Self::Category),
            "goal" => Ok(Self::Goal),
            "module" => Ok(Self::Module),
            _ => Err(KernelError::Ledger(format!("unknown mark scope {value:?}"))),
        }
    }
}

/// The named cross-module questions (`queries.sql` of the design).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Question {
    /// Weeks the morning bundle was kept vs not, in tasks finished and food spending.
    MorningBundleWeeks,
    /// Sleep against its baseline, day by day, beside the morning bundle and home meals.
    SleepBaselineDays,
    /// Food spending by category per week beside meals at home and out.
    FoodSpendWeeks,
    /// Spending by category by ISO week (S4a Q6).
    SpendByCategoryWeeks,
}

impl Question {
    pub const ALL: [Self; 4] = [
        Self::MorningBundleWeeks,
        Self::SleepBaselineDays,
        Self::FoodSpendWeeks,
        Self::SpendByCategoryWeeks,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MorningBundleWeeks => "morning-bundle-weeks",
            Self::SleepBaselineDays => "sleep-baseline-days",
            Self::FoodSpendWeeks => "food-spend-weeks",
            Self::SpendByCategoryWeeks => "spend-by-category-weeks",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|question| question.name() == value)
            .ok_or_else(|| KernelError::Ledger(format!("unknown question {value:?}")))
    }

    const fn sql(self) -> &'static str {
        match self {
            Self::MorningBundleWeeks => Q1,
            Self::SleepBaselineDays => Q2,
            Self::FoodSpendWeeks => Q3,
            Self::SpendByCategoryWeeks => Q6,
        }
    }
}

impl fmt::Display for Question {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The answer to one question: column names and rows of plain values.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub question: Question,
    pub hide_all: bool,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Column names and rows of plain values, for a query that takes parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

pub struct Ledger {
    path: PathBuf,
    connection: Connection,
    version: i64,
}

impl Ledger {
    /// Connection-local screen marks supplement persistent ledger marks without
    /// changing owner data or the schema. Main visibility rules still apply.
    pub fn set_screen_marks(&self, ids: &[&str], writers: &[&str]) -> Result<()> {
        self.connection.execute_batch("CREATE TEMP TABLE IF NOT EXISTS screen_mark (scope TEXT, key TEXT);
            DELETE FROM screen_mark;
            CREATE TEMP VIEW IF NOT EXISTS visible_observation AS
            SELECT o.* FROM main.visible_observation o WHERE NOT EXISTS
            (SELECT 1 FROM screen_mark m WHERE
                (m.scope='id' AND (m.key=o.item_id OR m.key=o.category OR m.key=json_extract(o.extra,'$.goal_id')))
                OR (m.scope='module' AND m.key=o.writer));")?;
        for id in ids {
            self.connection
                .execute("INSERT INTO screen_mark VALUES ('id', ?1)", [id])?;
        }
        for writer in writers {
            self.connection
                .execute("INSERT INTO screen_mark VALUES ('module', ?1)", [writer])?;
        }
        Ok(())
    }

    #[must_use]
    pub fn path_in(root: &Path) -> PathBuf {
        root.join(LEDGER_FILE)
    }

    /// Creates `ledger.sqlite` in the workspace root at the current schema
    /// version (1, then the additive 2). Reads or rewrites no planner file.
    /// Refuses an existing file.
    pub fn migrate(root: &Path) -> Result<Self> {
        let path = Self::path_in(root);
        if fs::symlink_metadata(&path).is_ok() {
            return Err(KernelError::Conflict(format!(
                "{} already exists",
                path.display()
            )));
        }
        // Create the file 0600 before SQLite opens it, like the rest of the workspace.
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let connection = Self::connect(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.execute_batch(&format!("BEGIN;\n{SCHEMA}\n{MIGRATION_2}\nCOMMIT;"))?;
        Ok(Self {
            path,
            connection,
            version: SCHEMA_VERSION,
        })
    }

    /// Migrates a version 1 ledger to version 2 in place, in one transaction.
    /// Refuses a ledger already at the current version.
    pub fn upgrade(&mut self) -> Result<i64> {
        let from = self.version;
        if from != 1 {
            return Err(KernelError::Conflict(format!(
                "ledger schema version {from}; only version 1 is upgraded"
            )));
        }
        self.connection
            .execute_batch(&format!("BEGIN;\n{MIGRATION_2}\nCOMMIT;"))?;
        self.version = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        Ok(from)
    }

    #[must_use]
    pub const fn version(&self) -> i64 {
        self.version
    }

    /// Opens the ledger if the workspace has one. `None` means the data plane is
    /// off for this workspace and nothing is written.
    pub fn open_existing(root: &Path) -> Result<Option<Self>> {
        let path = Self::path_in(root);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(KernelError::PathDenied(format!(
                    "{} is not a regular file",
                    path.display()
                )))
            }
            Ok(_) => {}
        }
        let connection = Self::connect(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        // Version 1 opens as it is; `upgrade` takes it to the current version.
        if !(1..=SCHEMA_VERSION).contains(&version) {
            return Err(KernelError::Ledger(format!(
                "ledger schema version {version}, expected 1 to {SCHEMA_VERSION}"
            )));
        }
        Ok(Some(Self {
            path,
            connection,
            version,
        }))
    }

    fn connect(path: &Path, flags: OpenFlags) -> Result<Connection> {
        let connection =
            Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        Ok(connection)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn register_kind(&self, kind: &KindDef) -> Result<()> {
        if kind.kind.trim().is_empty() || kind.writer.trim().is_empty() {
            return Err(KernelError::Ledger(
                "a kind needs a name and a writer".into(),
            ));
        }
        self.connection.execute(
            "INSERT INTO kind (kind, writer, default_class, unit, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                kind.kind,
                kind.writer,
                kind.default_class.as_str(),
                kind.unit,
                kind.description
            ],
        )?;
        Ok(())
    }

    pub fn kinds(&self) -> Result<Vec<KindDef>> {
        let mut statement = self.connection.prepare(
            "SELECT kind, writer, default_class, unit, description FROM kind ORDER BY kind",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut kinds = Vec::new();
        for row in rows {
            let (kind, writer, class, unit, description) = row?;
            kinds.push(KindDef {
                kind,
                writer,
                default_class: DataClass::parse(&class)?,
                unit,
                description,
            });
        }
        Ok(kinds)
    }

    fn default_class(connection: &Connection, kind: &str) -> Result<DataClass> {
        let class: Option<String> = connection
            .query_row(
                "SELECT default_class FROM kind WHERE kind = ?1",
                [kind],
                |row| row.get(0),
            )
            .optional()?;
        let class = class.ok_or_else(|| KernelError::Ledger(format!("unknown kind {kind:?}")))?;
        DataClass::parse(&class)
    }

    fn insert(connection: &Connection, row: &NewObservation) -> Result<ObjectId> {
        row.observed_on.parse::<CivilDate>()?;
        if row.subject.trim().is_empty() || row.writer.trim().is_empty() {
            return Err(KernelError::Ledger(
                "an observation needs a subject and a writer".into(),
            ));
        }
        let floor = Self::default_class(connection, &row.kind)?;
        let class = row.data_class.unwrap_or(floor);
        if class < floor {
            return Err(KernelError::Ledger(format!(
                "data class {} is below the {} default {} for kind {}",
                class.as_str(),
                row.kind,
                floor.as_str(),
                row.kind
            )));
        }
        let id = ObjectId::mint("obs")?;
        let extra = row.extra.as_ref().map(Value::to_string);
        connection.execute(
            "INSERT INTO observation (id, kind, observed_on, observed_at, item_id, category, subject, \
             quantity, unit, amount_minor, currency, data_class, writer, source_ref, recorded_at, \
             superseded_by, private, extra) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, ?16, ?17)",
            params![
                id.as_str(),
                row.kind,
                row.observed_on,
                row.observed_at,
                row.item_id,
                row.category,
                row.subject,
                row.quantity,
                row.unit,
                row.amount_minor,
                row.currency,
                class.as_str(),
                row.writer,
                row.source_ref,
                utc_now(),
                row.private.then_some(1_i64),
                extra,
            ],
        )?;
        Ok(id)
    }

    /// The values of one observation as a row to append, so a value taken
    /// back can return as the next row of its chain.
    pub(crate) fn copy_of(&self, id: &ObjectId) -> Result<NewObservation> {
        let found = self
            .connection
            .query_row(
                "SELECT kind, observed_on, observed_at, item_id, category, subject, quantity, unit, \
                 amount_minor, currency, data_class, writer, source_ref, coalesce(private, 0), extra \
                 FROM observation WHERE id = ?1",
                [id.as_str()],
                |r| {
                    Ok((
                        NewObservation {
                            kind: r.get(0)?,
                            observed_on: r.get(1)?,
                            observed_at: r.get(2)?,
                            item_id: r.get(3)?,
                            category: r.get(4)?,
                            subject: r.get(5)?,
                            quantity: r.get(6)?,
                            unit: r.get(7)?,
                            amount_minor: r.get(8)?,
                            currency: r.get(9)?,
                            data_class: None,
                            writer: r.get(11)?,
                            source_ref: r.get(12)?,
                            private: r.get::<_, i64>(13)? != 0,
                            extra: None,
                        },
                        r.get::<_, String>(10)?,
                        r.get::<_, Option<String>>(14)?,
                    ))
                },
            )
            .optional()?;
        let (mut row, class, extra) =
            found.ok_or_else(|| KernelError::Ledger(format!("no observation {id}")))?;
        row.data_class = Some(DataClass::parse(&class)?);
        row.extra = extra
            .map(|text| serde_json::from_str(&text))
            .transpose()
            .map_err(|e| KernelError::Ledger(format!("the extra of {id} cannot be read: {e}")))?;
        Ok(row)
    }

    /// Appends one observation and returns its id.
    pub fn append_observation(&self, row: &NewObservation) -> Result<ObjectId> {
        Self::insert(&self.connection, row)
    }

    /// Writes `row` as the correction of `old`, which must be current. The old
    /// row keeps its values; only its `superseded_by` link is set.
    pub fn supersede(&mut self, old: &ObjectId, row: &NewObservation) -> Result<ObjectId> {
        let transaction = self.connection.transaction()?;
        let current: Option<Option<String>> = transaction
            .query_row(
                "SELECT superseded_by FROM observation WHERE id = ?1",
                [old.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        match current {
            None => return Err(KernelError::Ledger(format!("no observation {old}"))),
            Some(Some(by)) => {
                return Err(KernelError::Conflict(format!(
                    "{old} is already superseded by {by}"
                )))
            }
            Some(None) => {}
        }
        let id = Self::insert(&transaction, row)?;
        transaction.execute(
            "UPDATE observation SET superseded_by = ?1 WHERE id = ?2 AND superseded_by IS NULL",
            params![id.as_str(), old.as_str()],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    /// The current check-in rows written for a planner item, newest first.
    pub fn current_for_item(
        &self,
        item_id: &str,
        subject: &str,
    ) -> Result<Vec<(ObjectId, f64, Option<Value>)>> {
        let mut statement = self.connection.prepare(
            "SELECT id, coalesce(quantity, 0), extra FROM current_observation \
             WHERE item_id = ?1 AND subject = ?2 ORDER BY recorded_at DESC, id DESC",
        )?;
        let rows = statement.query_map(params![item_id, subject], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut found = Vec::new();
        for row in rows {
            let (id, quantity, extra) = row?;
            let extra = extra
                .map(|text| serde_json::from_str(&text))
                .transpose()
                .map_err(|error| KernelError::Codec(error.to_string()))?;
            found.push((id.parse()?, quantity, extra));
        }
        Ok(found)
    }

    pub fn mark_private(&self, scope: MarkScope, key: &str) -> Result<()> {
        if key.trim().is_empty() {
            return Err(KernelError::Ledger("a private mark needs a key".into()));
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO private_mark (scope, key) VALUES (?1, ?2)",
            params![scope.as_str(), key],
        )?;
        Ok(())
    }

    pub fn unmark_private(&self, scope: MarkScope, key: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM private_mark WHERE scope = ?1 AND key = ?2",
            params![scope.as_str(), key],
        )?;
        Ok(())
    }

    /// Whether a persistent mark sits on this key in this scope.
    pub fn is_marked_private(&self, scope: MarkScope, key: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM private_mark WHERE scope = ?1 AND key = ?2)",
            params![scope.as_str(), key],
            |row| row.get(0),
        )?)
    }

    /// The keys of the persistent module marks, by name (the writers hidden
    /// while the mode is on, whatever the interface's own marks say).
    pub fn private_modules(&self) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT key FROM private_mark WHERE scope = 'module' ORDER BY key")?;
        let keys = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(keys)
    }

    /// Marks a goal and each of its items (the planner supplies the items).
    pub fn mark_goal_private(&mut self, goal_id: &ObjectId, items: &[ObjectId]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO private_mark (scope, key) VALUES ('goal', ?1)",
            [goal_id.as_str()],
        )?;
        for item in items {
            transaction.execute(
                "INSERT OR IGNORE INTO private_mark (scope, key) VALUES ('item', ?1)",
                [item.as_str()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Runs one named question over `current_observation`, or over
    /// `visible_observation` while the hide-all control is on.
    pub fn ask(&self, question: Question, hide_all: bool) -> Result<Answer> {
        let sql = if hide_all {
            question
                .sql()
                .replace("current_observation", "visible_observation")
        } else {
            question.sql().to_owned()
        };
        let table = self.table(&sql, [])?;
        Ok(Answer {
            question,
            hide_all,
            columns: table.columns,
            rows: table.rows,
        })
    }

    /// Runs one read query and returns its columns and rows as plain values.
    fn table(&self, sql: &str, parameters: impl rusqlite::Params) -> Result<Table> {
        let mut statement = self.connection.prepare(sql)?;
        let columns: Vec<String> = statement.column_names().map(str::to_owned).collect();
        let width = columns.len();
        let mut rows = Vec::new();
        let mut cursor = statement.query(parameters)?;
        while let Some(row) = cursor.next()? {
            let mut values = Vec::with_capacity(width);
            for index in 0..width {
                values.push(match row.get_ref(index)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => Value::from(n),
                    rusqlite::types::ValueRef::Real(n) => Value::from(n),
                    rusqlite::types::ValueRef::Text(text) => {
                        Value::from(String::from_utf8_lossy(text).into_owned())
                    }
                    rusqlite::types::ValueRef::Blob(_) => Value::Null,
                });
            }
            rows.push(values);
        }
        Ok(Table { columns, rows })
    }
}

fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    let date = CivilDate::new(1970, 1, 1)
        .and_then(|epoch| epoch.add_days(days))
        .map_or_else(|_| "1970-01-01".to_owned(), |date| date.to_string());
    format!(
        "{date}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

// The questions of the designs' queries.sql (T001 Q1-Q3, T007 Q6). Q1 (S3) reads the morning
// bundle from its check-ins: a day is kept when every morning check-in that day
// is kept, and a week is kept when at least half its recorded days are.
const Q1: &str = "WITH w AS (SELECT DISTINCT week FROM current_observation WHERE kind <> 'habit'),
mday AS (SELECT week, observed_on, min(quantity) AS kept FROM current_observation
         WHERE kind='check_in' AND quantity IS NOT NULL
           AND (subject='bundle:morning' OR json_extract(extra,'$.bundle')='morning')
         GROUP BY week, observed_on),
bundle AS (SELECT week, avg(kept) AS kept_rate, sum(kept >= 1) AS days_kept, count(*) AS days FROM mday GROUP BY week),
tasks AS (SELECT week, count(*) AS n FROM current_observation
          WHERE kind='check_in' AND subject='task.completed' AND quantity=1 GROUP BY week),
food AS (SELECT week, sum(amount_minor) AS pence FROM current_observation
         WHERE kind='purchase_line' AND category LIKE 'food:%' GROUP BY week)
SELECT w.week,
       CASE WHEN bundle.kept_rate IS NULL THEN 'no record' WHEN bundle.kept_rate >= 0.5 THEN 'kept' ELSE 'not kept' END AS morning_bundle,
       coalesce(tasks.n,0) AS tasks_finished,
       printf('%.2f', coalesce(food.pence,0)/100.0) AS food_spend_gbp,
       coalesce(bundle.days_kept,0) AS bundle_days_kept,
       coalesce(bundle.days,0) AS bundle_days_recorded
FROM w LEFT JOIN bundle USING(week) LEFT JOIN tasks USING(week) LEFT JOIN food USING(week)
ORDER BY w.week";

const Q2: &str = "WITH base AS (SELECT avg(quantity) AS b FROM current_observation WHERE kind='measurement' AND subject='sleep_hours')
SELECT m.observed_on AS day, m.quantity AS sleep_h,
       CASE WHEN m.quantity >= base.b THEN 'above' ELSE 'below' END AS vs_baseline,
       (SELECT max(quantity) FROM current_observation c WHERE c.kind='check_in' AND c.subject='bundle:morning' AND c.observed_on=m.observed_on) AS bundle_kept,
       (SELECT count(*) FROM current_observation f WHERE f.kind='meal' AND f.category='food:home' AND f.observed_on=m.observed_on) AS home_meals
FROM current_observation m, base
WHERE m.kind='measurement' AND m.subject='sleep_hours' ORDER BY day";

const Q3: &str = "SELECT p.week, p.category, printf('%.2f', sum(p.amount_minor)/100.0) AS spend_gbp,
       (SELECT count(*) FROM current_observation m WHERE m.kind='meal' AND m.week=p.week AND m.category='food:home') AS meals_home,
       (SELECT count(*) FROM current_observation m WHERE m.kind='meal' AND m.week=p.week AND m.category='food:out')  AS meals_out
FROM current_observation p WHERE p.kind='purchase_line' AND p.category LIKE 'food:%'
GROUP BY p.week, p.category ORDER BY p.week, p.category";

// S4a Q6 (T007 queries.sql), unchanged: the currency is the rows' own.
const Q6: &str = "SELECT week, category, printf('%.2f', sum(amount_minor) / 100.0) AS spent, currency, count(*) AS lines
FROM current_observation WHERE kind = 'purchase_line'
GROUP BY week, category, currency ORDER BY week, category";
