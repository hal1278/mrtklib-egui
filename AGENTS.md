# AGENTS.md

This file defines working rules for AI agent sessions launched from this repository root.

## Scope

The current working directory is the repository root.

Use only files available under this repository unless the user explicitly provides additional context.
Do not rely on prior chat history, external assumptions, or unstated migration context.

## Primary task

Implement a Rust + egui native GUI application for operating MRTKLIB through the `mrtk` executable.

This repository is not a reimplementation of MRTKLIB.
This repository is not an ABI/FFI binding project for MRTKLIB.

## Documentation authority

When implementation requirements are unclear, read `docs/` first.

Authority order:

1. `docs/architecture/`
2. `docs/conventions/`
3. `docs/implementation/`

Exceptions:

- Document splitting, file naming, and ownership follow `docs/conventions/document_conventions.md`.
- Naming follows `docs/conventions/naming_conventions.md`.
- Unresolved decisions are recorded in `docs/architecture/unresolved_issues.md` and must not be treated as settled.

## Hard rules

Do not implement MRTKLIB functionality in Rust.

Specifically, do not create:

- a positioning engine
- a GNSS algorithm implementation
- a RINEX parser library
- an RTCM parser library
- a UBX/SBF raw decoder library
- a CSSR decoder
- a GNSS time/ephemeris computation engine
- an ABI/FFI binding to MRTKLIB

The backend boundary is the `mrtk` process.
GUI code communicates with MRTKLIB through command-line arguments, config files, environment variables, working directory, stdout, stderr, output files, trace/log files, and exit status.

## Process rules

Tabs and UI widgets must not directly spawn `std::process::Command`.
Process lifecycle is owned by `ProcessSupervisor` or the equivalent owner defined in the architecture docs.

Command preview and executed argv must be generated from the same source.
Do not use display shell text as the source of truth for execution.

stdout/stderr must be read off the UI thread.
Long lines and missing newlines must not be allowed to grow an unbounded UI buffer.
Credentials must be masked before log lines are displayed.

## Configuration rules

TOML configuration round-trip must preserve keys that the GUI does not understand.
Known form fields may override known keys, but unknown keys must not be silently dropped.

## MRTKLIB source rules

Default development/user build source is `h-shiono/MRTKLIB` latest `main` commit unless the user specifies a fork commit or local repository.

Release artifacts must record the resolved MRTKLIB commit and whether the source tree was dirty.

Do not edit vendored or local MRTKLIB source from this repository unless the user explicitly asks for changes in that MRTKLIB repository.

## Edit scope

Allowed to edit:

- `Cargo.toml`
- `Cargo.lock`
- `src/**`
- `xtask/**`
- `tests/**`
- `docs/**`
- `AGENTS.md`
- `.gitignore`
- CI files when explicitly working on CI

Do not edit:

- Git metadata
- generated cache directories
- unrelated editor settings
- files outside this repository
- vendored MRTKLIB source, unless explicitly requested

Do not commit or push unless the user explicitly asks.

## Rust implementation rules

Use safe Rust unless there is a documented reason not to.
Keep public APIs minimal.
Do not add dependencies unless they are needed for the current implementation slice.
Prefer explicit validation errors over silent normalization.
Do not hide invariant violations by defaulting values.
Keep tests close to the module that owns the behavior.

## Validation

After implementation edits, run available checks.

Preferred checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

If documentation files were edited, run the available Markdown lint tool if configured.
If a tool is unavailable, record that explicitly in the final report.

## Done criteria

A task is done only when:

- the current implementation slice compiles, or the compile failure is explicitly recorded
- relevant behavior has tests where practical
- architecture boundaries remain intact
- names follow `docs/conventions/naming_conventions.md`
- unresolved decisions are not silently resolved without documentation
