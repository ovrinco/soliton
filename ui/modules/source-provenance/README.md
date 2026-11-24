The selected helpers are pinned to DATA-026 r04 (Lorengine), DATA-028 r03
(Z-ETL), CORE-026 (Phod), and CORE-016 r03 (QREAM, built with `core016` and
the declared crypto feature). QROC is a Soliton-owned CORE-020 r03 adapter,
not a bundled QROC executable: the inspected TypeScript source is a web/CLI
cascade without a safe standalone writer. The adapter therefore emits only a
separate local candidate, digest, diff, and gate evidence; QREAM and the
project-owned writer remain required for promotion. The QREAM operation
materializes a local graph
in a fresh database and records provider warnings; it does not fabricate a
provider or claim live external evidence.
