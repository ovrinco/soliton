# OVRIN module pins

`manifest.json` records the exact source revision and SHA-256 digest used by
the native panel. The `bin/` directory contains only the five selected helper
executables, never Rust target directories or dependency caches. At runtime
the Rust store resolves these files beside the installed desktop executable,
verifies their digest, and records both the pinned and current digest in
`modules-list`; a missing or changed executable is reported as unavailable
rather than replaced with a stub.

Inputs are copied into a private per-job directory before an engine is run.
Generated configurations, output files, manifests, and bounded stdout/stderr
are retained as evidence below that directory. QROC has a local proposal-only
adapter: it captures a source digest, a separate revision candidate, a diff,
and exact user-supplied generation gate evidence. It never opens the target
project, contacts QREAM, or claims promotion authority; the CORE-020
project-owned writer and current QREAM warrant remain an explicit dependency.
