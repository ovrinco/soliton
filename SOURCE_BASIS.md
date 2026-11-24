# Source basis

Candidate source is selected from Soliton commit dcac76903d84b913d8222d49076b065c5865ccb2, the recorded source of installed release 0.5.30-ev-20261030-dcac769.

Included: root Cargo workspace and lock, build script, active Rust source and crates, QML interface, required assets and helper source, retained portable tests and synthetic first-run example.

Excluded: archived organ implementations, executable bundles, unused Phod vendor source, workstation machine-log fixtures, historical acceptance scripts, installed binary checksums and records, and machine-specific tests. Original files are preserved in the private preparation record.

Public-copy changes: Cargo wrapper uses the caller's Cargo or SOLITON_CARGO; local model manager requires explicit configuration; default Zeno agent directory is home-relative; two provenance notes replace private absolute paths with descriptive provenance; README and example describe the actually tested subset.

The private selection manifest maps selected original files to hashes. The published file hash manifest identifies the candidate's final bytes.

Owner-directed release licensing assigns Apache-2.0 to first-party code and aligns all three Cargo package manifests.

Release quality repairs introduce a named habit observation row alias, a capture-job context object to reduce argument count, and remove one unnecessary borrow. Rustfmt normalizes the exported Rust workspace. These changes apply to the candidate only.

SQLite release hardening pins rusqlite 0.41.0 with fallible_uint and forwards an opt-in bundled-sqlite feature through both shared crates. The default remains system linkage. The tracker column-name iterator is adapted to the newer API, and a feature-gated test checks the bundled SQLite security baseline.
