//! The grid's reads, added in this copy: the sheet's tabs in order and one tab's rows in the store's order.
use crate::types::*;
use crate::{Error, Result, Store};
use rusqlite::{params_from_iter, types::Value as SqlValue};
use serde::Serialize;
use serde_json::json;

/// One tab of the sheet, as the store orders them.
#[derive(Clone, Debug, Serialize)]
pub struct Tab {
    pub name: String,
    pub position: i64,
    pub kind: String,
    pub rows: usize,
}
/// One table of a tab: a requirement tab has one, a register tab one per block.
#[derive(Clone, Debug, Serialize)]
pub struct Section {
    pub table: String,
    pub columns: Vec<String>,
    pub headers: Vec<String>,
    pub total: usize,
}
#[derive(Debug, Serialize)]
pub struct TabPage {
    pub tab: String,
    pub sections: Vec<Section>,
    pub rows: Vec<Row>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}
/// An order column and its direction; the store's own order breaks ties.
#[derive(Clone, Copy, Debug)]
pub struct Order<'a> {
    pub column: &'a str,
    pub descending: bool,
}

const FAULT_COLS: &[&str] = &[
    "id", "seen", "area", "fault", "effect", "source", "status", "fix", "line",
];

struct Source {
    table: String,
    key: String,
    pos: String,
    columns: Vec<String>,
    headers: Vec<String>,
    requirement: bool,
}

impl Store {
    /// Every tab in the sheet's order, with its row count.
    pub fn tabs(&self) -> Result<Vec<Tab>> {
        let mut tabs = Vec::new();
        for t in self.objects(
            "SELECT name,position,kind FROM sheet ORDER BY position,name",
            &[],
        )? {
            let name = text(&t["name"]);
            let mut rows = 0;
            for s in self.sources(&name)? {
                rows += if s.requirement {
                    self.count(&s, " WHERE sheet=?", &[SqlValue::Text(name.clone())])?
                } else {
                    self.count(&s, "", &[])?
                };
            }
            tabs.push(Tab {
                position: t["position"].as_i64().unwrap_or(0),
                kind: text(&t["kind"]),
                rows,
                name,
            });
        }
        Ok(tabs)
    }
    /// One tab's rows as the store holds them (`list`'s rows: requirement rows carry their campaign cells), across
    /// its sections in order, filtered on a column's text (case-folded containment) and ordered by a column when asked.
    pub fn tab_page(
        &self,
        tab: &str,
        filter: Option<(&str, &str)>,
        order: Option<Order<'_>>,
        limit: usize,
        offset: usize,
    ) -> Result<TabPage> {
        let sources = self.sources(tab)?;
        if sources.is_empty() {
            return Err(Error::Invalid(format!("no tab named {tab:?}")));
        }
        if let Some((column, _)) = filter {
            known(&sources, column)?;
        }
        if let Some(o) = order {
            known(&sources, o.column)?;
        }
        let mut sections = Vec::new();
        let mut rows = Vec::new();
        let mut total = 0;
        for s in &sources {
            let mut conditions = Vec::new();
            let mut args = Vec::new();
            if s.requirement {
                conditions.push("sheet=?".to_string());
                args.push(SqlValue::Text(tab.into()));
            }
            if let Some((column, value)) = filter {
                if !s.columns.iter().any(|c| c == column) {
                    sections.push(section(s, 0));
                    continue;
                }
                conditions.push(format!(
                    "instr(lower(coalesce(CAST({} AS TEXT),'')),lower(?))>0",
                    quote(column)
                ));
                args.push(SqlValue::Text(value.into()));
            }
            let clause = if conditions.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", conditions.join(" AND "))
            };
            let count = self.count(s, &clause, &args)?;
            sections.push(section(s, count));
            let skip = offset.saturating_sub(total);
            if rows.len() < limit && skip < count {
                let by = match order.filter(|o| s.columns.iter().any(|c| c == o.column)) {
                    Some(o) => format!(
                        "{} {},",
                        quote(o.column),
                        if o.descending { "DESC" } else { "ASC" }
                    ),
                    None => String::new(),
                };
                let mut a = args.clone();
                a.push(SqlValue::Integer((limit - rows.len()) as i64));
                a.push(SqlValue::Integer(skip as i64));
                let sql = format!(
                    "SELECT * FROM {}{clause} ORDER BY {by}{},{} LIMIT ? OFFSET ?",
                    quote(&s.table),
                    quote(&s.pos),
                    quote(&s.key)
                );
                for mut row in self.objects(&sql, &a)? {
                    row.insert("_table".into(), json!(s.table));
                    if s.requirement {
                        let id = text(&row["id"]);
                        self.campaign_cells(&mut row, &id)?;
                    } else {
                        row.insert("sheet".into(), json!(tab));
                    }
                    rows.push(row);
                }
            }
            total += count;
        }
        Ok(TabPage {
            tab: tab.into(),
            sections,
            rows,
            total,
            limit,
            offset,
        })
    }
    /// The tables that make up a tab, as `list` reads them: the requirement table for a requirement tab, each
    /// register block of the tab in header order, and the fault table under Faults.
    fn sources(&self, tab: &str) -> Result<Vec<Source>> {
        let kind = self
            .objects(
                "SELECT kind FROM sheet WHERE name=?",
                &[SqlValue::Text(tab.into())],
            )?
            .first()
            .map(|r| text(&r["kind"]));
        let mut out = Vec::new();
        if kind.as_deref() == Some("requirement") {
            let names = |a: &[&str], b: &[&str]| -> Vec<String> {
                a.iter().chain(b).map(|s| (*s).to_string()).collect()
            };
            out.push(Source {
                table: "requirement".into(),
                key: "id".into(),
                pos: "position".into(),
                columns: names(REQ_COLS, CAMP_COLS),
                headers: names(REQ_HEADERS, CAMP_HEADERS),
                requirement: true,
            });
            return Ok(out);
        }
        for b in self.objects(
            "SELECT tbl,cols,headers FROM block WHERE sheet=? ORDER BY header_row",
            &[SqlValue::Text(tab.into())],
        )? {
            let columns: Vec<String> = serde_json::from_str(&text(&b["cols"]))?;
            let headers: Vec<String> = serde_json::from_str(&text(&b["headers"]))?;
            let key = columns
                .first()
                .ok_or_else(|| Error::Invalid("empty block".into()))?
                .clone();
            out.push(Source {
                table: text(&b["tbl"]),
                key,
                pos: "_pos".into(),
                columns,
                headers,
                requirement: false,
            });
        }
        if tab == "Faults" && self.has("fault")? {
            out.push(Source {
                table: "fault".into(),
                key: "id".into(),
                pos: "id".into(),
                columns: FAULT_COLS.iter().map(|s| (*s).to_string()).collect(),
                headers: FAULT_COLS.iter().map(|s| s.to_uppercase()).collect(),
                requirement: false,
            });
        }
        Ok(out)
    }
    fn count(&self, s: &Source, clause: &str, args: &[SqlValue]) -> Result<usize> {
        Ok(self.db.query_row(
            &format!("SELECT count(*) FROM {}{clause}", quote(&s.table)),
            params_from_iter(args),
            |r| r.get(0),
        )?)
    }
}

fn section(s: &Source, total: usize) -> Section {
    Section {
        table: s.table.clone(),
        columns: s.columns.clone(),
        headers: s.headers.clone(),
        total,
    }
}
/// Filter and order columns are the store's own: a requirement tab's campaign cells are joined after the read.
fn known(sources: &[Source], column: &str) -> Result<()> {
    let own = |s: &Source| {
        s.columns
            .iter()
            .any(|c| c == column && !(s.requirement && CAMP_COLS.contains(&c.as_str())))
    };
    if sources.iter().any(own) {
        Ok(())
    } else {
        Err(Error::Invalid(format!("no column {column:?} in this tab")))
    }
}
