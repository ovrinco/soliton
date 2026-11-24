//! Finance capture (S4b, OC-12 result line 6; the intake design is
//! `design/finance_intake.md` of soliton-whole T11). Money comes in five ways:
//! a statement file, a receipt as a picture or file, a pay stub, an e-mailed
//! receipt saved as a file, and cash by hand. Every file enters through the
//! intake folder the tool knows (`<workspace>/intake/finance`), read only when
//! it is listed or registered, and becomes one `document` row (its kind, name,
//! hash and arrival day); every line read from it names that row as its source
//! (`document:<id>#<line>`), so each line stays attributable to its document.
//! Statement and fixed-header CSVs are read by plain code, a receipt's lines are
//! typed once against its document, a pay stub writes `income_line` rows and
//! cash writes one line whose source is `cash`. Every category word is accepted:
//! one the key list does not know is kept under `other` with its word. The
//! currency is read from each line or given with it, never assumed. No bank
//! credentials, no model, no network.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{utc_now, Ledger, NewObservation};
use crate::date::CivilDate;
use crate::error::{KernelError, Result};
use crate::id::ObjectId;

pub const WRITER: &str = "finance";
pub const PURCHASE_LINE: &str = "purchase_line";
/// One file that came in through the intake folder.
pub const DOCUMENT: &str = "document";
/// One line of money in (a pay stub's pay).
pub const INCOME_LINE: &str = "income_line";
/// The source of a line entered by hand for cash.
pub const CASH_SOURCE: &str = "cash";
/// Where a category word the key list does not know is kept, with its word.
pub const OTHER: &str = "other";
/// The spending categories by key and the word a person reads (the window's
/// `categoryWords` and the design table). Every other word lands under `other`.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("food:groceries", "Groceries"),
    ("food:eating_out", "Eating out"),
    ("food:home", "Home cooking"),
    ("household:utilities", "Utilities"),
    ("household:toiletries", "Toiletries"),
    ("household:property_tax", "Property tax"),
    ("household:other", "Household"),
    ("transport:fuel", "Fuel"),
    ("transport", "Transport"),
    ("health", "Health"),
    ("other", "Other"),
    ("uncategorised", "Not yet categorised"),
];
/// Other words for a category key, as receipts and statements write them.
const CATEGORY_ALIASES: &[(&str, &str)] = &[
    ("grocery", "food:groceries"),
    ("food", "food:groceries"),
    ("restaurant", "food:eating_out"),
    ("takeaway", "food:eating_out"),
    ("takeout", "food:eating_out"),
    ("dining", "food:eating_out"),
    ("toiletry", "household:toiletries"),
    ("personal care", "household:toiletries"),
    ("property taxes", "household:property_tax"),
    ("utility", "household:utilities"),
    ("household", "household:other"),
    ("gas", "transport:fuel"),
    ("gasoline", "transport:fuel"),
    ("petrol", "transport:fuel"),
    ("diesel", "transport:fuel"),
    ("pharmacy", "health"),
];
/// The category of a line whose bank category is not mapped: the operator picks.
pub const UNCATEGORISED: &str = "uncategorised";
/// The largest statement read at once.
pub const MAX_STATEMENT_BYTES: usize = 8 * 1024 * 1024;

/// How the dates of a bank's statement are written.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DateFormat {
    #[serde(rename = "YYYY-MM-DD")]
    Iso,
    #[serde(rename = "DD/MM/YYYY")]
    DayMonthYear,
    #[serde(rename = "MM/DD/YYYY")]
    MonthDayYear,
}

impl DateFormat {
    fn read(self, text: &str) -> Result<CivilDate> {
        let text = text.trim();
        let bad = || KernelError::Ledger(format!("date {text:?} is not {}", self.name()));
        match self {
            Self::Iso => text.parse().map_err(|_| bad()),
            Self::DayMonthYear | Self::MonthDayYear => {
                let parts: Vec<&str> = text.split('/').collect();
                let [a, b, year] = parts.as_slice() else {
                    return Err(bad());
                };
                let (day, month) = if self == Self::DayMonthYear {
                    (a, b)
                } else {
                    (b, a)
                };
                let year: i32 = year.parse().map_err(|_| bad())?;
                let month: u8 = month.parse().map_err(|_| bad())?;
                let day: u8 = day.parse().map_err(|_| bad())?;
                CivilDate::new(year, month, day).map_err(|_| bad())
            }
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Iso => "YYYY-MM-DD",
            Self::DayMonthYear => "DD/MM/YYYY",
            Self::MonthDayYear => "MM/DD/YYYY",
        }
    }
}

/// Which header names a bank's statement uses, set once per bank.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatementMapping {
    pub bank: String,
    pub date: String,
    pub date_format: DateFormat,
    pub description: String,
    pub amount: String,
    pub currency: String,
    #[serde(default)]
    pub category: Option<String>,
    /// True when money spent is written as a negative amount.
    pub debit_negative: bool,
    /// The bank's category names mapped to ledger categories (`food:groceries`).
    #[serde(default)]
    pub categories: BTreeMap<String, String>,
}

/// What one import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportOutcome {
    pub added: Vec<ObjectId>,
    /// Lines already in the ledger, refused by `source_ref`.
    pub duplicates: usize,
    /// Money in (credits, refunds): not a purchase, left out.
    pub credits: usize,
    pub period: String,
}

/// One current purchase line, as the lines read gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct PurchaseLine {
    pub id: ObjectId,
    pub observed_on: String,
    /// The ISO week's Monday, as Q6 groups it.
    pub week: String,
    pub merchant: String,
    pub amount_minor: i64,
    pub currency: String,
    pub category: String,
    /// `statement:<bank>:<period>#<line>` or what wrote the line.
    pub source: Option<String>,
    /// The item a receipt line names; empty for a statement line.
    pub item: String,
    /// The category word as the document wrote it; empty when it wrote none.
    pub category_word: String,
}

/// Which lines to read: one week (its Monday or any day in it), one category, or both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineFilter {
    pub week: Option<CivilDate>,
    pub category: Option<String>,
}

/// Reads CSV text (RFC 4180: commas, double-quoted fields, doubled quotes, LF or CRLF).
pub fn read_csv(text: &str) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                }
                row.clear();
            }
            _ => field.push(c),
        }
    }
    if quoted {
        return Err(KernelError::Ledger(
            "the statement ends inside a quoted field".into(),
        ));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

/// The longest header line read from a statement.
pub const MAX_HEADER_BYTES: u64 = 64 * 1024;

/// The column names of a statement: its first line only, at most
/// `MAX_HEADER_BYTES`, read by plain code. Nothing past the first line is read.
pub fn read_statement_header(path: &std::path::Path) -> Result<Vec<String>> {
    use std::io::{BufRead, BufReader, Read};
    let file = std::fs::File::open(path)
        .map_err(|error| KernelError::Ledger(format!("the statement cannot be opened: {error}")))?;
    let mut line = Vec::new();
    BufReader::new(file.take(MAX_HEADER_BYTES))
        .read_until(b'\n', &mut line)
        .map_err(|error| KernelError::Ledger(format!("the statement cannot be read: {error}")))?;
    if line.last() != Some(&b'\n') && line.len() as u64 == MAX_HEADER_BYTES {
        return Err(KernelError::Ledger(
            "the statement's first line is longer than 64 KiB".into(),
        ));
    }
    let text = String::from_utf8(line)
        .map_err(|_| KernelError::Ledger("the statement's first line is not UTF-8".into()))?;
    let header = read_csv(text.trim_end_matches(['\r', '\n']))?
        .into_iter()
        .next()
        .unwrap_or_default();
    if header.iter().all(|name| name.trim().is_empty()) {
        return Err(KernelError::Ledger(
            "the statement has no header line".into(),
        ));
    }
    Ok(header
        .into_iter()
        .map(|name| name.trim().to_owned())
        .collect())
}

/// A decimal amount in minor units (two decimal places at most), with its sign.
fn minor_units(text: &str) -> Result<i64> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | ' '))
        .collect();
    let bad = || {
        KernelError::Ledger(format!(
            "amount {text:?} is not a number with at most two decimals"
        ))
    };
    let (negative, digits) = match cleaned.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, cleaned.strip_prefix('+').unwrap_or(&cleaned)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fraction.is_empty()
        || fraction.len() > 2
        || !whole
            .chars()
            .chain(fraction.chars())
            .all(|c| c.is_ascii_digit())
    {
        return Err(bad());
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| bad())?
    };
    let fraction: i64 = format!("{fraction:0<2}").parse().map_err(|_| bad())?;
    let value = whole
        .checked_mul(100)
        .and_then(|w| w.checked_add(fraction))
        .ok_or_else(bad)?;
    Ok(if negative { -value } else { value })
}

fn column(header: &[String], name: &str) -> Result<usize> {
    header
        .iter()
        .position(|h| h.trim() == name.trim())
        .ok_or_else(|| KernelError::Ledger(format!("the statement has no column {name:?}")))
}

impl Ledger {
    /// Keeps a bank's column mapping, replacing the earlier one for that bank.
    pub fn set_statement_mapping(&self, mapping: &StatementMapping) -> Result<()> {
        if mapping.bank.trim().is_empty() {
            return Err(KernelError::Ledger(
                "a statement mapping names its bank".into(),
            ));
        }
        for name in [
            &mapping.date,
            &mapping.description,
            &mapping.amount,
            &mapping.currency,
        ] {
            if name.trim().is_empty() {
                return Err(KernelError::Ledger(
                    "a statement mapping names the date, description, amount and currency columns"
                        .into(),
                ));
            }
        }
        let text = serde_json::to_string(mapping)
            .map_err(|error| KernelError::Codec(error.to_string()))?;
        self.connection.execute(
            "INSERT INTO statement_mapping (bank, mapping, recorded_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(bank) DO UPDATE SET mapping = excluded.mapping, recorded_at = excluded.recorded_at",
            params![mapping.bank.trim(), text, utc_now()],
        )?;
        Ok(())
    }

    /// Every bank set up, by name, with its mapping.
    pub fn statement_mappings(&self) -> Result<Vec<StatementMapping>> {
        let mut statement = self
            .connection
            .prepare("SELECT mapping FROM statement_mapping ORDER BY bank")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut found = Vec::new();
        for text in rows {
            found.push(
                serde_json::from_str(&text?)
                    .map_err(|error| KernelError::Codec(error.to_string()))?,
            );
        }
        Ok(found)
    }

    pub fn statement_mapping(&self, bank: &str) -> Result<Option<StatementMapping>> {
        let text: Option<String> = self
            .connection
            .query_row(
                "SELECT mapping FROM statement_mapping WHERE bank = ?1",
                [bank.trim()],
                |row| row.get(0),
            )
            .optional()?;
        text.map(|text| {
            serde_json::from_str(&text).map_err(|error| KernelError::Codec(error.to_string()))
        })
        .transpose()
    }

    /// Reads one statement of `bank` into purchase lines, all or none. Each line's
    /// `source_ref` is `statement:<bank>:<first day>..<last day>#<line>`; a line
    /// whose `source_ref` is already current is refused and counted. A statement
    /// that came in through the intake folder is read by `import_statement_document`.
    pub fn import_statement(&mut self, bank: &str, text: &str) -> Result<ImportOutcome> {
        self.import_statement_text(bank, text, None)
    }

    /// Reads a statement document of `bank` (a CSV registered from the intake
    /// folder) into purchase lines, all or none, as `import_statement` does; each
    /// line's `source_ref` is `document:<id>#<line>`.
    pub fn import_statement_document(
        &mut self,
        document: &ObjectId,
        bank: &str,
    ) -> Result<ImportOutcome> {
        let file = self.document_file(document)?;
        file.expect_kind(&[DocumentKind::Csv])?;
        let text = file.text(MAX_STATEMENT_BYTES)?;
        self.import_statement_text(bank, &text, Some(document))
    }

    fn import_statement_text(
        &mut self,
        bank: &str,
        text: &str,
        document: Option<&ObjectId>,
    ) -> Result<ImportOutcome> {
        if text.len() > MAX_STATEMENT_BYTES {
            return Err(KernelError::Ledger(
                "the statement is larger than 8 MiB".into(),
            ));
        }
        let mapping = self.statement_mapping(bank)?.ok_or_else(|| {
            KernelError::Ledger(format!(
                "no statement mapping for {bank:?}; set it once first"
            ))
        })?;
        let rows = read_csv(text)?;
        let Some((header, lines)) = rows.split_first() else {
            return Err(KernelError::Ledger("the statement is empty".into()));
        };
        let date = column(header, &mapping.date)?;
        let description = column(header, &mapping.description)?;
        let amount = column(header, &mapping.amount)?;
        let currency = column(header, &mapping.currency)?;
        let category = mapping
            .category
            .as_deref()
            .map(|c| column(header, c))
            .transpose()?;
        struct Line {
            number: usize,
            day: CivilDate,
            description: String,
            spent: i64,
            currency: String,
            category: String,
            word: String,
        }
        let mut parsed = Vec::new();
        let mut credits = 0;
        for (index, cells) in lines.iter().enumerate() {
            let number = index + 1;
            let cell = |i: usize| cells.get(i).map_or("", |c| c.trim());
            let at = |error: KernelError| {
                KernelError::Ledger(format!("statement line {number}: {error}"))
            };
            let day = mapping.date_format.read(cell(date)).map_err(at)?;
            let value = minor_units(cell(amount)).map_err(at)?;
            let spent = if mapping.debit_negative {
                -value
            } else {
                value
            };
            if spent <= 0 {
                credits += 1;
                continue;
            }
            let code = cell(currency).to_ascii_uppercase();
            if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(at(KernelError::Ledger(format!(
                    "currency {:?} is not a three-letter code",
                    cell(currency)
                ))));
            }
            let text = cell(description);
            if text.is_empty() {
                return Err(at(KernelError::Ledger(
                    "the line has no description".into(),
                )));
            }
            let bank_category = category.map(cell).unwrap_or_default();
            // The bank's mapping first, then the key list; a word neither knows
            // waits as uncategorised for his pick, its word kept.
            parsed.push(Line {
                number,
                day,
                description: text.to_owned(),
                spent,
                currency: code,
                category: mapping
                    .categories
                    .get(bank_category)
                    .cloned()
                    .or_else(|| known_category(bank_category).map(str::to_owned))
                    .unwrap_or_else(|| UNCATEGORISED.to_owned()),
                word: bank_category.to_owned(),
            });
        }
        let (Some(first), Some(last)) = (
            parsed.iter().map(|l| l.day).min(),
            parsed.iter().map(|l| l.day).max(),
        ) else {
            return Ok(ImportOutcome {
                credits,
                ..ImportOutcome::default()
            });
        };
        let period = format!("{first}..{last}");
        let transaction = self.connection.transaction()?;
        let mut outcome = ImportOutcome {
            credits,
            period: period.clone(),
            ..ImportOutcome::default()
        };
        for line in &parsed {
            let source_ref = match document {
                Some(document) => document_ref(document, line.number),
                None => format!("statement:{}:{period}#{}", mapping.bank, line.number),
            };
            let seen: i64 = transaction.query_row(
                "SELECT count(*) FROM current_observation WHERE kind = ?1 AND source_ref = ?2",
                params![PURCHASE_LINE, source_ref],
                |row| row.get(0),
            )?;
            if seen > 0 {
                outcome.duplicates += 1;
                continue;
            }
            outcome.added.push(Self::insert(
                &transaction,
                &NewObservation {
                    kind: PURCHASE_LINE.into(),
                    observed_on: line.day.to_string(),
                    category: Some(line.category.clone()),
                    subject: line.description.clone(),
                    quantity: Some(1.0),
                    unit: Some("item".into()),
                    amount_minor: Some(line.spent),
                    currency: Some(line.currency.clone()),
                    writer: WRITER.into(),
                    source_ref: Some(source_ref),
                    extra: Some(json!({
                        "merchant": line.description,
                        "line_no": line.number,
                        "capture": "statement",
                        "bank": mapping.bank,
                        "category_word": line.word,
                        "document": document.map(ObjectId::as_str),
                        "standing": OBSERVED,
                    })),
                    ..NewObservation::default()
                },
            )?);
        }
        transaction.commit()?;
        Ok(outcome)
    }
}

/// The standing of a line plain code read or he typed (OD-2026-10-10-03-R04);
/// a line a model reads would be `inferred`.
pub const OBSERVED: &str = "observed";
/// The header the prototype's CSV export writes, in any order.
pub const FIXED_HEADER: [&str; 7] = [
    "item_name",
    "price",
    "store",
    "purchase_date",
    "quantity",
    "unit",
    "category",
];

/// The category key of a word: a key, a category's word or a known other word
/// for it, read without case. `None` when the key list does not know it.
#[must_use]
pub fn known_category(word: &str) -> Option<&'static str> {
    let word = word.trim().to_lowercase();
    if word.is_empty() {
        return None;
    }
    CATEGORIES
        .iter()
        .find(|(key, name)| *key == word || name.to_lowercase() == word)
        .map(|(key, _)| *key)
        .or_else(|| {
            CATEGORY_ALIASES
                .iter()
                .find(|(alias, _)| *alias == word)
                .map(|(_, key)| *key)
        })
}

/// The key a line is kept under: the known key, `other` for any other word,
/// `uncategorised` for no word. Never refused (T743-006).
#[must_use]
pub fn category_for(word: &str) -> &'static str {
    match known_category(word) {
        Some(key) => key,
        None if word.trim().is_empty() => UNCATEGORISED,
        None => OTHER,
    }
}

fn document_ref(document: &ObjectId, line: usize) -> String {
    format!("document:{document}#{line}")
}

/// The intake folder of a workspace; the tool knows it, nobody picks it.
#[must_use]
pub fn intake_dir(root: &Path) -> PathBuf {
    root.join("intake").join("finance")
}

/// What kind of file a document is, by its extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentKind {
    Csv,
    Pdf,
    Jpg,
    Png,
    Eml,
}

impl DocumentKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Pdf => "pdf",
            Self::Jpg => "jpg",
            Self::Png => "png",
            Self::Eml => "eml",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "csv" => Ok(Self::Csv),
            "pdf" => Ok(Self::Pdf),
            "jpg" | "jpeg" => Ok(Self::Jpg),
            "png" => Ok(Self::Png),
            "eml" => Ok(Self::Eml),
            other => Err(KernelError::Ledger(format!(
                "unknown document kind {other:?} (csv, pdf, jpg, png or eml)"
            ))),
        }
    }

    /// The kind a file name's extension says, if it is one the intake reads.
    #[must_use]
    pub fn of_name(name: &str) -> Option<Self> {
        let (_, extension) = name.rsplit_once('.')?;
        Self::parse(extension).ok()
    }
}

/// One file in the intake folder, as listing it finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntakeFile {
    pub name: String,
    pub bytes: u64,
    /// `None` for a file whose extension the intake does not read.
    pub kind: Option<DocumentKind>,
    pub sha256: String,
    /// The `document` row holding this file's hash: `Some` once it is in.
    pub document: Option<ObjectId>,
    /// The day it came in when registered, else the day the file was last written.
    pub day: String,
}

/// One typed line of a receipt (a picture, a file or an e-mail saved as a file).
#[derive(Clone, Debug, PartialEq)]
pub struct ReceiptLine {
    pub merchant: String,
    pub day: CivilDate,
    pub item: String,
    pub quantity: f64,
    pub unit: String,
    /// What the line cost, in minor units.
    pub amount_minor: i64,
    pub currency: String,
    /// His word for the category; any word is kept.
    pub category: String,
}

/// A pay stub's figures: one period's pay from one employer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayStub {
    pub employer: String,
    pub paid_on: CivilDate,
    pub period_from: CivilDate,
    pub period_to: CivilDate,
    pub gross_minor: i64,
    pub net_minor: i64,
    pub currency: String,
    /// Each deduction by its name, in minor units.
    pub deductions: Vec<(String, i64)>,
}

impl PayStub {
    /// Reads a pay stub written as two columns, `field,value`: employer,
    /// pay_date, period_start, period_end (ISO days), gross, net, currency, and
    /// one `deduction:<name>` row per deduction. Plain code; no model.
    pub fn from_csv(text: &str) -> Result<Self> {
        let rows = read_csv(text)?;
        let mut fields = BTreeMap::new();
        let mut deductions = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let [field, value] = row.as_slice() else {
                return Err(KernelError::Ledger(format!(
                    "pay stub line {}: two columns, field and value",
                    index + 1
                )));
            };
            let field = field.trim();
            if field.eq_ignore_ascii_case("field") && index == 0 {
                continue;
            }
            if let Some(name) = field.strip_prefix("deduction:") {
                deductions.push((name.trim().to_owned(), minor_units(value)?));
            } else {
                fields.insert(field.to_ascii_lowercase(), value.trim().to_owned());
            }
        }
        let get = |name: &str| {
            fields
                .get(name)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| KernelError::Ledger(format!("the pay stub has no {name}")))
        };
        Ok(Self {
            employer: get("employer")?.clone(),
            paid_on: get("pay_date")?.parse()?,
            period_from: get("period_start")?.parse()?,
            period_to: get("period_end")?.parse()?,
            gross_minor: minor_units(get("gross")?)?,
            net_minor: minor_units(get("net")?)?,
            currency: get("currency")?.clone(),
            deductions,
        })
    }
}

/// One current income line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomeLine {
    pub id: ObjectId,
    pub observed_on: String,
    pub employer: String,
    pub net_minor: i64,
    pub gross_minor: i64,
    pub currency: String,
    pub source: Option<String>,
    /// The pay period's first and last day (ISO), empty when the stub gave none.
    pub period_from: String,
    pub period_to: String,
}

fn currency_code(text: &str) -> Result<String> {
    let code = text.trim().to_ascii_uppercase();
    if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(KernelError::Ledger(format!(
            "currency {text:?} is not a three-letter code"
        )));
    }
    Ok(code)
}

/// The SHA-256 of a file, read in pieces.
fn file_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&hasher.finalize());
    Ok(crate::hash::ContentHash::from(bytes).to_string())
}

fn modified_day(meta: &fs::Metadata) -> String {
    let seconds = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_secs());
    CivilDate::new(1970, 1, 1)
        .and_then(|epoch| epoch.add_days(i64::try_from(seconds / 86_400).unwrap_or(0)))
        .map_or_else(|_| "1970-01-01".to_owned(), |day| day.to_string())
}

/// A registered document and its file in the intake folder, its hash checked.
struct DocumentFile {
    id: ObjectId,
    kind: DocumentKind,
    path: PathBuf,
}

impl DocumentFile {
    fn expect_kind(&self, kinds: &[DocumentKind]) -> Result<()> {
        if kinds.contains(&self.kind) {
            return Ok(());
        }
        let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
        Err(KernelError::Ledger(format!(
            "document {} is a {} file; this reads {}",
            self.id,
            self.kind.as_str(),
            names.join(" or ")
        )))
    }

    fn text(&self, cap: usize) -> Result<String> {
        let bytes = fs::read(&self.path)?;
        if bytes.len() > cap {
            return Err(KernelError::Ledger(format!(
                "document {} is larger than {} MiB",
                self.id,
                cap / (1024 * 1024)
            )));
        }
        String::from_utf8(bytes)
            .map_err(|_| KernelError::Ledger(format!("document {} is not UTF-8 text", self.id)))
    }
}

impl Ledger {
    /// The workspace this ledger belongs to.
    fn root(&self) -> PathBuf {
        self.path()
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf)
    }

    /// Registers the `document` and `income_line` kinds once; a ledger made
    /// before the intake lacks them. No schema change (the habit kind's way).
    pub fn ensure_finance_kinds(&self) -> Result<()> {
        for (kind, description) in [
            (DOCUMENT, "One file that came in through the finance intake: its kind, name, hash and the day it arrived"),
            (INCOME_LINE, "One line of money in, from a pay stub"),
        ] {
            self.connection.execute(
                "INSERT OR IGNORE INTO kind (kind, writer, default_class, unit, description) \
                 VALUES (?1, ?2, 'personal_sensitive', NULL, ?3)",
                params![kind, WRITER, description],
            )?;
        }
        Ok(())
    }

    /// The files in the intake folder by name, each with its size, kind and,
    /// once in, its document. Creates the folder private (0700) the first time.
    /// The folder is read only here and in `register_document`.
    pub fn list_intake(&self) -> Result<Vec<IntakeFile>> {
        self.ensure_finance_kinds()?;
        let dir = intake_dir(&self.root());
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                return Err(KernelError::PathDenied(
                    "the finance intake is not a folder".into(),
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&dir)?;
            }
            Err(error) => return Err(error.into()),
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let meta = fs::symlink_metadata(entry.path())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !meta.file_type().is_file() || name.starts_with('.') {
                continue;
            }
            let sha256 = file_sha256(&entry.path())?;
            let found = self.document_by_hash(&sha256)?;
            files.push(IntakeFile {
                kind: DocumentKind::of_name(&name),
                bytes: meta.len(),
                day: found
                    .as_ref()
                    .map_or_else(|| modified_day(&meta), |(_, day)| day.clone()),
                document: found.map(|(id, _)| id),
                sha256,
                name,
            });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(files)
    }

    fn document_by_hash(&self, sha256: &str) -> Result<Option<(ObjectId, String)>> {
        let found: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT id, observed_on FROM current_observation \
                 WHERE kind = ?1 AND json_extract(extra, '$.sha256') = ?2 ORDER BY recorded_at, id LIMIT 1",
                params![DOCUMENT, sha256],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        found.map(|(id, day)| Ok((id.parse()?, day))).transpose()
    }

    /// Writes one `document` row for a file in the intake folder: its name as
    /// the subject, its hash, kind and size in extra, `day` the day it arrived.
    /// Refuses a file outside the folder, a kind its name does not carry, and a
    /// hash already registered (naming the first document).
    pub fn register_document(
        &self,
        path: &Path,
        kind: DocumentKind,
        day: CivilDate,
    ) -> Result<ObjectId> {
        self.ensure_finance_kinds()?;
        let dir = intake_dir(&self.root());
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| KernelError::PathDenied("a document needs a file name".into()))?;
        let inside =
            path.parent().and_then(|parent| parent.canonicalize().ok()) == dir.canonicalize().ok();
        if !inside {
            return Err(KernelError::PathDenied(format!(
                "{name} is not in the finance intake"
            )));
        }
        let meta = fs::symlink_metadata(path)?;
        if !meta.file_type().is_file() {
            return Err(KernelError::PathDenied(format!(
                "{name} is not a regular file"
            )));
        }
        if DocumentKind::of_name(name) != Some(kind) {
            return Err(KernelError::Ledger(format!(
                "{name} is not a {} file",
                kind.as_str()
            )));
        }
        let sha256 = file_sha256(path)?;
        if let Some((first, _)) = self.document_by_hash(&sha256)? {
            return Err(KernelError::Conflict(format!(
                "{name} is already in as document {first}"
            )));
        }
        self.append_observation(&NewObservation {
            kind: DOCUMENT.into(),
            observed_on: day.to_string(),
            category: Some(kind.as_str().into()),
            subject: name.to_owned(),
            writer: WRITER.into(),
            source_ref: Some(format!("intake:{name}")),
            extra: Some(json!({"sha256": sha256, "kind": kind.as_str(), "bytes": meta.len()})),
            ..NewObservation::default()
        })
    }

    /// A current document and its file, whose hash must still match.
    fn document_file(&self, document: &ObjectId) -> Result<DocumentFile> {
        let found: Option<(String, Option<String>, Option<String>)> = self
            .connection
            .query_row(
                "SELECT subject, json_extract(extra, '$.kind'), json_extract(extra, '$.sha256') \
                 FROM current_observation WHERE id = ?1 AND kind = ?2",
                params![document.as_str(), DOCUMENT],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((name, Some(kind), Some(sha256))) = found else {
            return Err(KernelError::Ledger(format!(
                "no current document {document}"
            )));
        };
        let path = intake_dir(&self.root()).join(&name);
        if file_sha256(&path)? != sha256 {
            return Err(KernelError::Conflict(format!(
                "{name} changed since it came in as document {document}"
            )));
        }
        Ok(DocumentFile {
            id: document.clone(),
            kind: DocumentKind::parse(&kind)?,
            path,
        })
    }

    /// How many current lines of `kind` name this document as their source.
    fn lines_of(&self, kind: &str, document: &ObjectId) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT count(*) FROM current_observation WHERE kind = ?1 AND source_ref LIKE ?2",
            params![kind, format!("document:{document}#%")],
            |row| row.get(0),
        )?)
    }

    fn refuse_read_twice(&self, kind: &str, document: &ObjectId) -> Result<()> {
        let lines = self.lines_of(kind, document)?;
        if lines > 0 {
            return Err(KernelError::Conflict(format!(
                "document {document} already has {lines} lines"
            )));
        }
        Ok(())
    }

    /// Reads a CSV in the prototype's fixed header (item_name, price, store,
    /// purchase_date, quantity, unit, category; any order, other columns left)
    /// into purchase lines, all or none. `price` is the line's total, the
    /// purchase date ISO, the currency given for the whole file.
    pub fn import_fixed_csv(
        &mut self,
        document: &ObjectId,
        currency: &str,
    ) -> Result<ImportOutcome> {
        let file = self.document_file(document)?;
        file.expect_kind(&[DocumentKind::Csv])?;
        self.refuse_read_twice(PURCHASE_LINE, document)?;
        let currency = currency_code(currency)?;
        let rows = read_csv(&file.text(MAX_STATEMENT_BYTES)?)?;
        let Some((header, lines)) = rows.split_first() else {
            return Err(KernelError::Ledger("the file is empty".into()));
        };
        let mut at = BTreeMap::new();
        for name in FIXED_HEADER {
            at.insert(name, column(header, name)?);
        }
        let mut parsed = Vec::new();
        for (index, cells) in lines.iter().enumerate() {
            let number = index + 1;
            let cell = |name: &str| cells.get(at[name]).map_or("", |c| c.trim());
            let line_error =
                |error: KernelError| KernelError::Ledger(format!("line {number}: {error}"));
            for name in FIXED_HEADER {
                if cell(name).is_empty() {
                    return Err(KernelError::Ledger(format!(
                        "line {number}: {name} is empty"
                    )));
                }
            }
            let quantity: f64 = cell("quantity")
                .parse()
                .ok()
                .filter(|q: &f64| q.is_finite() && *q > 0.0)
                .ok_or_else(|| {
                    KernelError::Ledger(format!(
                        "line {number}: quantity {:?} is not a positive number",
                        cell("quantity")
                    ))
                })?;
            parsed.push(ReceiptLine {
                merchant: cell("store").to_owned(),
                day: cell("purchase_date").parse().map_err(line_error)?,
                item: cell("item_name").to_owned(),
                quantity,
                unit: cell("unit").to_owned(),
                amount_minor: minor_units(cell("price")).map_err(line_error)?,
                currency: currency.clone(),
                category: cell("category").to_owned(),
            });
        }
        let added = self.write_lines(document, &parsed, "fixed_csv")?;
        let period = match (
            parsed.iter().map(|l| l.day).min(),
            parsed.iter().map(|l| l.day).max(),
        ) {
            (Some(first), Some(last)) => format!("{first}..{last}"),
            _ => String::new(),
        };
        Ok(ImportOutcome {
            added,
            period,
            ..ImportOutcome::default()
        })
    }

    /// Writes a receipt's lines, typed once, against its document (a picture,
    /// a PDF or an e-mail saved as a file), all or none; the document stays the
    /// lines' source. A document whose lines are already in is refused.
    pub fn record_receipt_lines(
        &mut self,
        document: &ObjectId,
        lines: &[ReceiptLine],
    ) -> Result<Vec<ObjectId>> {
        let file = self.document_file(document)?;
        file.expect_kind(&[
            DocumentKind::Jpg,
            DocumentKind::Png,
            DocumentKind::Pdf,
            DocumentKind::Eml,
        ])?;
        self.refuse_read_twice(PURCHASE_LINE, document)?;
        let capture = if file.kind == DocumentKind::Eml {
            "email_receipt"
        } else {
            "receipt"
        };
        self.write_lines(document, lines, capture)
    }

    fn write_lines(
        &mut self,
        document: &ObjectId,
        lines: &[ReceiptLine],
        capture: &str,
    ) -> Result<Vec<ObjectId>> {
        if lines.is_empty() {
            return Err(KernelError::Ledger(format!(
                "document {document} gives no lines"
            )));
        }
        let mut rows = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let number = index + 1;
            let refuse = |what: &str| Err(KernelError::Ledger(format!("line {number}: {what}")));
            if line.merchant.trim().is_empty() || line.item.trim().is_empty() {
                return refuse("a line names its merchant and item");
            }
            if line.amount_minor <= 0 {
                return refuse("a purchase line costs more than nothing");
            }
            if !(line.quantity.is_finite() && line.quantity > 0.0) || line.unit.trim().is_empty() {
                return refuse("a line has a positive quantity and a unit");
            }
            let currency = currency_code(&line.currency)
                .map_err(|e| KernelError::Ledger(format!("line {number}: {e}")))?;
            rows.push(NewObservation {
                kind: PURCHASE_LINE.into(),
                observed_on: line.day.to_string(),
                category: Some(category_for(&line.category).to_owned()),
                subject: line.item.trim().to_owned(),
                quantity: Some(line.quantity),
                unit: Some(line.unit.trim().to_owned()),
                amount_minor: Some(line.amount_minor),
                currency: Some(currency),
                writer: WRITER.into(),
                source_ref: Some(document_ref(document, number)),
                extra: Some(json!({
                    "merchant": line.merchant.trim(),
                    "item": line.item.trim(),
                    "line_no": number,
                    "capture": capture,
                    "category_word": line.category.trim(),
                    "document": document.as_str(),
                    "standing": OBSERVED,
                })),
                ..NewObservation::default()
            });
        }
        let transaction = self.connection.transaction()?;
        let mut added = Vec::new();
        for row in &rows {
            added.push(Self::insert(&transaction, row)?);
        }
        transaction.commit()?;
        Ok(added)
    }

    /// Writes a pay stub's pay as one `income_line` (the net paid, on the pay
    /// day; gross, period and deductions in extra) against its document.
    pub fn import_pay_stub(&mut self, document: &ObjectId, stub: &PayStub) -> Result<ObjectId> {
        self.ensure_finance_kinds()?;
        self.document_file(document)?;
        self.refuse_read_twice(INCOME_LINE, document)?;
        if stub.employer.trim().is_empty() {
            return Err(KernelError::Ledger("a pay stub names its employer".into()));
        }
        if stub.period_to < stub.period_from {
            return Err(KernelError::Ledger(
                "the pay period ends before it starts".into(),
            ));
        }
        if stub.net_minor < 0 || stub.gross_minor < stub.net_minor {
            return Err(KernelError::Ledger(
                "a pay stub's net is between nothing and its gross".into(),
            ));
        }
        let deductions: Vec<_> = stub
            .deductions
            .iter()
            .map(|(name, amount)| json!({"name": name, "amount_minor": amount}))
            .collect();
        self.append_observation(&NewObservation {
            kind: INCOME_LINE.into(),
            observed_on: stub.paid_on.to_string(),
            category: Some("income:pay".into()),
            subject: stub.employer.trim().to_owned(),
            quantity: Some(1.0),
            amount_minor: Some(stub.net_minor),
            currency: Some(currency_code(&stub.currency)?),
            writer: WRITER.into(),
            source_ref: Some(document_ref(document, 1)),
            extra: Some(json!({
                "employer": stub.employer.trim(),
                "gross_minor": stub.gross_minor,
                "net_minor": stub.net_minor,
                "period_from": stub.period_from.to_string(),
                "period_to": stub.period_to.to_string(),
                "deductions": deductions,
                "capture": "pay_stub",
                "document": document.as_str(),
                "standing": OBSERVED,
            })),
            ..NewObservation::default()
        })
    }

    /// Reads a pay stub document written as a two-column CSV (`PayStub::from_csv`)
    /// and writes its income line.
    pub fn import_pay_stub_csv(&mut self, document: &ObjectId) -> Result<ObjectId> {
        let file = self.document_file(document)?;
        file.expect_kind(&[DocumentKind::Csv])?;
        let stub = PayStub::from_csv(&file.text(MAX_STATEMENT_BYTES)?)?;
        self.import_pay_stub(document, &stub)
    }

    /// Cash spent, entered by hand: one purchase line whose source is `cash`.
    pub fn record_cash(
        &mut self,
        day: CivilDate,
        merchant: &str,
        amount_minor: i64,
        currency: &str,
        category: &str,
    ) -> Result<ObjectId> {
        if merchant.trim().is_empty() {
            return Err(KernelError::Ledger(
                "a cash line names where the money went".into(),
            ));
        }
        if amount_minor <= 0 {
            return Err(KernelError::Ledger(
                "a cash line costs more than nothing".into(),
            ));
        }
        self.append_observation(&NewObservation {
            kind: PURCHASE_LINE.into(),
            observed_on: day.to_string(),
            category: Some(category_for(category).to_owned()),
            subject: merchant.trim().to_owned(),
            quantity: Some(1.0),
            unit: Some("item".into()),
            amount_minor: Some(amount_minor),
            currency: Some(currency_code(currency)?),
            writer: WRITER.into(),
            source_ref: Some(CASH_SOURCE.into()),
            extra: Some(json!({
                "merchant": merchant.trim(),
                "capture": "cash",
                "category_word": category.trim(),
                "standing": OBSERVED,
            })),
            ..NewObservation::default()
        })
    }

    /// The current income lines by day, read through the visibility view while
    /// hide-all is on.
    pub fn income_lines(&self, hide_all: bool) -> Result<Vec<IncomeLine>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, observed_on, subject, coalesce(amount_minor, 0), \
             coalesce(json_extract(extra, '$.gross_minor'), amount_minor, 0), coalesce(currency, ''), source_ref, \
             coalesce(json_extract(extra, '$.period_from'), ''), coalesce(json_extract(extra, '$.period_to'), '') \
             FROM {} WHERE kind = ?1 ORDER BY observed_on, id",
            view(hide_all)
        ))?;
        let rows = statement.query_map([INCOME_LINE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;
        let mut lines = Vec::new();
        for row in rows {
            let (
                id,
                observed_on,
                employer,
                net_minor,
                gross_minor,
                currency,
                source,
                period_from,
                period_to,
            ) = row?;
            lines.push(IncomeLine {
                id: id.parse()?,
                observed_on,
                employer,
                net_minor,
                gross_minor,
                currency,
                source,
                period_from,
                period_to,
            });
        }
        Ok(lines)
    }
}

fn view(hide_all: bool) -> &'static str {
    if hide_all {
        "visible_observation"
    } else {
        "current_observation"
    }
}

impl Ledger {
    /// The current purchase lines, by week then day, filtered by week and
    /// category; read through the visibility view while hide-all is on.
    pub fn purchase_lines(&self, filter: &LineFilter, hide_all: bool) -> Result<Vec<PurchaseLine>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id, observed_on, week, coalesce(json_extract(extra,'$.merchant'), subject), \
             amount_minor, currency, coalesce(category, ?2), source_ref, \
             coalesce(json_extract(extra,'$.item'), ''), coalesce(json_extract(extra,'$.category_word'), '') FROM {} \
             WHERE kind = ?1 \
               AND (?3 IS NULL OR week = date(?3, '-6 days', 'weekday 1')) \
               AND (?4 IS NULL OR coalesce(category, ?2) = ?4) \
             ORDER BY week, observed_on, source_ref, id",
            view(hide_all)
        ))?;
        let rows = statement.query_map(
            params![
                PURCHASE_LINE,
                UNCATEGORISED,
                filter.week.map(|w| w.to_string()),
                filter.category.as_deref().map(str::trim)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )?;
        let mut lines = Vec::new();
        for row in rows {
            let (
                id,
                observed_on,
                week,
                merchant,
                amount,
                currency,
                category,
                source,
                item,
                category_word,
            ) = row?;
            lines.push(PurchaseLine {
                id: id.parse()?,
                observed_on,
                week,
                merchant,
                amount_minor: amount.unwrap_or(0),
                currency: currency.unwrap_or_default(),
                category,
                source,
                item,
                category_word,
            });
        }
        Ok(lines)
    }

    /// Moves a current purchase line to `category` by superseding it; every
    /// other value is carried over, so Q6 follows. Returns the new row's id.
    pub fn recategorise_line(&mut self, line: &ObjectId, category: &str) -> Result<ObjectId> {
        let category = category.trim();
        if category.is_empty() {
            return Err(KernelError::Ledger(
                "a purchase line's category needs a name".into(),
            ));
        }
        type Row = (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            Option<f64>,
            Option<String>,
            Option<i64>,
            Option<String>,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
        );
        let found: Option<Row> = self
            .connection
            .query_row(
                "SELECT kind, observed_on, observed_at, item_id, category, subject, quantity, unit, \
                 amount_minor, currency, data_class, writer, source_ref, private, extra \
                 FROM current_observation WHERE id = ?1",
                [line.as_str()],
                |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                        r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?,
                        r.get(14)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            kind,
            observed_on,
            observed_at,
            item_id,
            old,
            subject,
            quantity,
            unit,
            amount_minor,
            currency,
            class,
            writer,
            source_ref,
            private,
            extra,
        )) = found
        else {
            return Err(KernelError::Ledger(format!(
                "no current purchase line {line}"
            )));
        };
        if kind != PURCHASE_LINE {
            return Err(KernelError::Ledger(format!(
                "{line} is not a purchase line"
            )));
        }
        if old.as_deref() == Some(category) {
            return Err(KernelError::Ledger(format!(
                "{line} is already in {category}"
            )));
        }
        let extra = extra
            .map(|text| serde_json::from_str(&text))
            .transpose()
            .map_err(|error| KernelError::Codec(error.to_string()))?;
        self.supersede(
            line,
            &NewObservation {
                kind,
                observed_on,
                observed_at,
                item_id,
                category: Some(category.to_owned()),
                subject,
                quantity,
                unit,
                amount_minor,
                currency,
                data_class: Some(crate::model::DataClass::parse(&class)?),
                writer,
                source_ref,
                private: private.unwrap_or(0) != 0,
                extra,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_reads_quotes_commas_and_crlf() {
        let rows = read_csv("a,b\r\n\"x, y\",\"say \"\"hi\"\"\"\r\n\n1,2").unwrap();
        assert_eq!(
            rows,
            [vec!["a", "b"], vec!["x, y", "say \"hi\""], vec!["1", "2"]]
        );
        assert!(read_csv("a,\"b").is_err());
    }

    #[test]
    fn amounts_read_to_minor_units() {
        assert_eq!(minor_units("-12.3").unwrap(), -1230);
        assert_eq!(
            minor_units("\"1,234.56\"".trim_matches('"')).unwrap(),
            123_456
        );
        assert_eq!(minor_units("7").unwrap(), 700);
        assert!(minor_units("1.234").is_err());
        assert!(minor_units("abc").is_err());
    }
}
