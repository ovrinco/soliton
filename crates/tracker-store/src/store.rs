//! The store's reads, carried from the origin's `store.rs` (README.md); every write is left at the origin.
use crate::types::*;
use crate::{Error, Result};
use rusqlite::{
    Connection, OpenFlags, params_from_iter,
    types::{Value as SqlValue, ValueRef},
};
use serde_json::{Value, json};
use std::path::Path;

pub struct Store {
    pub(crate) db: Connection,
}
type Source = (String, String, String, Vec<String>, Option<String>);
pub(crate) type Query = (Vec<String>, Vec<Vec<Value>>);
impl Store {
    /// Open for reading only: no pragma, no table, no write reaches the file.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::Invalid(format!(
                "no tracker database at {}",
                path.display()
            )));
        }
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self { db })
    }
    /// SQLite's count that moves whenever another connection commits to the file since this one opened: a change
    /// is seen without reading a table, and a commit that lands mid-read is still seen on the next look.
    pub fn data_version(&self) -> Result<u64> {
        Ok(self.db.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }
    pub fn revision(&self) -> Result<u64> {
        Ok(self
            .db
            .query_row("SELECT coalesce(max(seq),0) FROM event", [], |r| r.get(0))?)
    }
    pub(crate) fn query_params(&self, sql: &str, args: &[SqlValue]) -> Result<Query> {
        let mut stmt = self.db.prepare(sql)?;
        // CLI query is a read-only lens, never a bypass around logged writes.
        if !stmt.readonly() {
            return Err(Error::Invalid(
                "query must be read-only; use tracker mutation verbs".into(),
            ));
        }
        let cols: Vec<String> = stmt.column_names().map(|s| (*s).into()).collect::<Vec<_>>();
        let n = cols.len();
        let rows = stmt
            .query_map(params_from_iter(args), |r| {
                (0..n)
                    .map(|i| {
                        Ok(match r.get_ref(i)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(i) => json!(i),
                            ValueRef::Real(f) => json!(f),
                            ValueRef::Text(s) => Value::String(
                                std::str::from_utf8(s)
                                    .map_err(|e| {
                                        rusqlite::Error::FromSqlConversionFailure(
                                            i,
                                            rusqlite::types::Type::Text,
                                            Box::new(e),
                                        )
                                    })?
                                    .into(),
                            ),
                            ValueRef::Blob(_) => {
                                return Err(rusqlite::Error::InvalidColumnType(
                                    i,
                                    cols[i].clone(),
                                    rusqlite::types::Type::Blob,
                                ));
                            }
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((cols, rows))
    }
    pub fn query(&self, sql: &str) -> Result<Query> {
        self.query_params(sql, &[])
    }
    pub(crate) fn objects(&self, sql: &str, args: &[SqlValue]) -> Result<Vec<Row>> {
        let (cols, rows) = self.query_params(sql, args)?;
        Ok(rows
            .into_iter()
            .map(|r| cols.iter().cloned().zip(r).collect())
            .collect())
    }
    pub(crate) fn has(&self, table: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            [table],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn locate(&self, id: &str) -> Result<(String, String, Vec<String>)> {
        self.find(id)?
            .ok_or_else(|| Error::Invalid(format!("no row with ID {id:?}")))
    }
    /// Whether a requirement, register row or fault with this ID exists.
    pub fn exists(&self, id: &str) -> Result<bool> {
        Ok(self.find(id)?.is_some())
    }
    fn find(&self, id: &str) -> Result<Option<(String, String, Vec<String>)>> {
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM requirement WHERE id=?)",
            [id],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(Some((
                "requirement".into(),
                "id".into(),
                REQ_COLS.iter().map(|s| (*s).into()).collect(),
            )));
        }
        for b in self.objects("SELECT tbl,cols FROM block", &[])? {
            let tbl = text(&b["tbl"]);
            let cols: Vec<String> = serde_json::from_str(&text(&b["cols"]))?;
            let key = cols
                .first()
                .ok_or_else(|| Error::Invalid(format!("empty register block {tbl}")))?;
            let sql = format!(
                "SELECT EXISTS(SELECT 1 FROM {} WHERE {}=?)",
                quote(&tbl),
                quote(key)
            );
            if self.db.query_row(&sql, [id], |r| r.get::<_, bool>(0))? {
                return Ok(Some((tbl, key.clone(), cols)));
            }
        }
        if self
            .db
            .query_row("SELECT EXISTS(SELECT 1 FROM fault WHERE id=?)", [id], |r| {
                r.get::<_, bool>(0)
            })?
        {
            return Ok(Some((
                "fault".into(),
                "id".into(),
                vec![
                    "id", "seen", "area", "fault", "effect", "source", "status", "fix", "line",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            )));
        }
        Ok(None)
    }
    fn row_at(&self, table: &str, key: &str, id: &str) -> Result<Row> {
        self.objects(
            &format!("SELECT * FROM {} WHERE {}=?", quote(table), quote(key)),
            &[SqlValue::Text(id.into())],
        )?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Invalid(format!("no row with ID {id:?}")))
    }
    pub(crate) fn campaign_cells(&self, row: &mut Row, id: &str) -> Result<()> {
        if let Some(c) = self
            .objects(
                "SELECT * FROM campaign WHERE id=?",
                &[SqlValue::Text(id.into())],
            )?
            .first()
        {
            for k in CAMP_COLS {
                row.insert((*k).into(), c[*k].clone());
            }
        }
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Row> {
        let (table, key, _) = self.locate(id)?;
        let mut row = self.row_at(&table, &key, id)?;
        if table == "requirement" {
            self.campaign_cells(&mut row, id)?;
            row.insert(
                "evidence".into(),
                json!(self.objects(
                    "SELECT path,sha256,note,author,at FROM evidence WHERE id=? ORDER BY seq",
                    &[SqlValue::Text(id.into())]
                )?),
            );
        }
        row.insert("assignments".into(), serde_json::to_value(self.links(id)?)?);
        row.insert("_table".into(), json!(table));
        Ok(row)
    }
    pub fn group(&self, id: &str) -> Result<Row> {
        let row = self.get(id)?;
        if row.get("_table").and_then(Value::as_str) != Some("t_outcomes_2") {
            return Err(Error::Invalid(format!("{id} is not an outcome group")));
        }
        Ok(row)
    }
    pub fn members(&self, id: &str) -> Result<Vec<String>> {
        Ok(self
            .objects(
                "SELECT id FROM requirement WHERE outcome_id=? ORDER BY sheet,position,id",
                &[SqlValue::Text(id.into())],
            )?
            .iter()
            .map(|r| text(&r["id"]))
            .collect())
    }
    pub fn list(
        &self,
        tab: Option<&str>,
        status: Option<&str>,
        search: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Page> {
        let mut sources: Vec<Source> = vec![(
            "requirement".into(),
            "id".into(),
            "position".into(),
            REQ_COLS
                .iter()
                .map(|s| (*s).into())
                .collect::<Vec<String>>(),
            None,
        )];
        for b in self.objects(
            "SELECT tbl,sheet,cols FROM block ORDER BY sheet,header_row",
            &[],
        )? {
            let cols: Vec<String> = serde_json::from_str(&text(&b["cols"]))?;
            let key = cols
                .first()
                .ok_or_else(|| Error::Invalid("empty block".into()))?
                .clone();
            sources.push((
                text(&b["tbl"]),
                key,
                "_pos".into(),
                cols,
                Some(text(&b["sheet"])),
            ));
        }
        if tab == Some("Faults") {
            sources.push((
                "fault".into(),
                "id".into(),
                "id".into(),
                vec![
                    "id", "seen", "area", "fault", "effect", "source", "status", "fix", "line",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                Some("Faults".into()),
            ));
        }
        let mut rows = Vec::new();
        let mut total = 0;
        for (table, key, pos, cols, sheet) in sources {
            if sheet
                .as_deref()
                .is_some_and(|s| tab.is_some_and(|t| s != t))
            {
                continue;
            }
            let mut conditions = Vec::new();
            let mut args = Vec::new();
            if table == "requirement"
                && let Some(t) = tab
            {
                conditions.push("sheet=?".into());
                args.push(SqlValue::Text(t.into()));
            }
            if let Some(s) = status {
                if !cols.iter().any(|c| c == "status") {
                    continue;
                }
                conditions.push("status=?".into());
                args.push(SqlValue::Text(s.into()));
            }
            if let Some(s) = search.filter(|s| !s.is_empty()) {
                conditions.push(format!(
                    "({})",
                    cols.iter()
                        .map(|c| format!(
                            "instr(lower(coalesce(CAST({} AS TEXT),'')),lower(?))>0",
                            quote(c)
                        ))
                        .collect::<Vec<_>>()
                        .join(" OR ")
                ));
                args.extend(cols.iter().map(|_| SqlValue::Text(s.into())));
            }
            let clause = if conditions.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", conditions.join(" AND "))
            };
            let count: usize = self.db.query_row(
                &format!("SELECT count(*) FROM {}{clause}", quote(&table)),
                params_from_iter(&args),
                |r| r.get(0),
            )?;
            let skip = offset.saturating_sub(total);
            if rows.len() < limit && skip < count {
                let mut a = args.clone();
                a.push(SqlValue::Integer((limit - rows.len()) as i64));
                a.push(SqlValue::Integer(skip as i64));
                let sql = format!(
                    "SELECT * FROM {}{clause} ORDER BY {}{},{} LIMIT ? OFFSET ?",
                    quote(&table),
                    if table == "requirement" { "sheet," } else { "" },
                    quote(&pos),
                    quote(&key)
                );
                for mut row in self.objects(&sql, &a)? {
                    row.insert("_table".into(), json!(table));
                    if let Some(s) = &sheet {
                        row.insert("sheet".into(), json!(s));
                    }
                    if table == "requirement" {
                        let id = text(&row["id"]);
                        self.campaign_cells(&mut row, &id)?;
                    }
                    rows.push(row);
                }
            }
            total += count;
        }
        Ok(Page {
            rows,
            total,
            limit,
            offset,
        })
    }
    pub fn links(&self, id: &str) -> Result<Vec<Assignment>> {
        self.locate(id)?;
        if !self.has("assignment")? {
            return Ok(Vec::new());
        }
        self.objects(
            "SELECT * FROM assignment WHERE row_id=? ORDER BY seq DESC",
            &[SqlValue::Text(id.into())],
        )?
        .into_iter()
        .map(|r| Ok(serde_json::from_value(json!(r))?))
        .collect()
    }
    pub fn events_since(&self, seq: u64) -> Result<Vec<Event>> {
        let mut events = self.objects(
            "SELECT * FROM event WHERE seq>? ORDER BY seq",
            &[SqlValue::Integer(i64::try_from(seq).map_err(|_| {
                Error::Invalid("event cursor out of range".into())
            })?)],
        )?;
        for e in &mut events {
            for k in ["old", "new"] {
                if let Some(s) = e[k].as_str() {
                    let value = serde_json::from_str(s)?;
                    e.insert(k.into(), value);
                }
            }
        }
        Ok(events)
    }
}
