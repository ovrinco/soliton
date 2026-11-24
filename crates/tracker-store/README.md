# Tracker store (read side)

Origin: the read side of `crates/tracker-store` in the QREAM rewrite (`QREAM rewrite source tree`), commit
`cd7b5fbcbe7979383b17347d82ad0aca76abf623` (branch `qtg-T01`), the source of the installed `tracker` command
(`~/.local/lib/tracker/releases/qtg-20261030-cd7b5fb`). Its read side is unchanged from the first carried copy in
`QREAM reference source tree/crates/tracker-store` (commit `ff16c1f`). One store library and one colour convention
serve three fronts: the QREAM rewrite, Zavent Sheets and Soliton.

Carried as written: `Store::open_read_only`, `revision`, `query`, `exists`, `get`, `group`, `members`, `links`,
`list`, `events_since`, the error enum (read variants), the column lists and the value helpers of `types.rs`.

Not carried (no reader needs them): every write (`open`, `set`, `status`, `add`, `evidence`, `assign`, `usage`,
`duration`, `fault`, `vocab`), the outbox and history of `record.rs`, `export.rs`, `protocol.rs`, `accounting.rs`
and the binary. Soliton writes the record only through the installed `tracker` command.

Added in this copy: `sheet.rs`, the grid's reads (`tabs`, `tab_page`: one tab's sections, columns and rows in the
store's order, with an optional column filter and an optional order column).

Pins changed to the workspace's locked versions, offline: `serde =1.0.228` (as at the origin), `serde_json
=1.0.145` (origin `=1.0.151`, without `preserve_order`: a row is a map whose key order is not the store's; the
store's column order travels beside the rows as each section's `columns`), `thiserror =2.0.17` (origin `=2.0.18`),
`rusqlite =0.32.1` without default features (the system SQLite, as `soliton-kernel`). Dropped with the write side:
`chrono`, `sha2`, `uuid`, `rust_xlsxwriter`, `csv`, `clap`, `parquet`.

A store is only ever opened read-only here: no pragma, no table, no write reaches the file.
