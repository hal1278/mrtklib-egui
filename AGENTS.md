# AGENTS.md

Working rules for AI agent sessions in this repository.
Do not rely on chat history or unstated assumptions.
If a requirement is not written down, ask.
All documents in this repository are written in English.

## What this repository is

An RTKLIB-style GUI suite for MRTKLIB, written in Rust and egui.
It is the first consumer of the MRTKLIB machine interface (h-shiono/MRTKLIB#326, #327).

The design reference is `docs/design.md`: the decided design with its rationale.
It holds no open questions, plans or history.
This file summarizes rules owned by `docs/design.md`.
If they disagree, `docs/design.md` wins; fix this file.
The machine interface itself (method names, formats, versions) is defined by MRTKLIB `docs/rpc/`.

This repository does not reimplement MRTKLIB and does not link to it.

## Backend boundary

- The backend is the `mrtk` executable, started as a child process.
- `mrtk run` is controlled only through the RPC.
  Never use the telnet console.
- `mrtk post` and `mrtk convert` target argv, files, machine-readable progress output and cancel through stdin.
  The Conv first slice is an explicit exception: `convert` has no progress/cancel interface yet, so it displays stderr without parsing and offers no Abort action (see `docs/design.md`).
- MRTKLIB provides configuration handling (schema, read, write, validate).
  The GUI does not parse or write MRTKLIB TOML itself.
- Never parse human-readable MRTKLIB output (console text, stderr progress lines, log messages) to obtain state.
  Showing such text to the user as-is is fine.
  If the GUI needs information that exists only as human-readable text, report it to the user as a candidate MRTKLIB interface item instead of parsing.
- Do not depend on MRTKLIB internal structs, a C ABI or FFI.

## Do not implement in Rust

- a positioning engine or GNSS algorithms
- reading or writing of GNSS data formats (receiver raw data, RTCM, RINEX, correction data, products and any other format MRTKLIB handles), except MRTKLIB solution files for plotting
- a GNSS time or ephemeris computation engine (exception: GPST/UTC conversion with a built-in leap-second table for displaying solutions, see `docs/design.md`)
- parsing or validation of MRTKLIB configuration options

Allowed, and part of the GUI's job:

- decoding the RPC and the machine-readable output of `mrtk`
- reading MRTKLIB solution files (a defined file format) for plotting
- conversions for display, such as GPST/UTC to local time or GPS week and seconds, and an ENU baseline from a solution and the base position

## Process rules

- UI code (windows, panels, widgets) must not spawn processes.
  The process supervisor owns process lifecycle.
- The command preview and the executed argv come from the same value.
  Never execute display text.
- Read child output off the UI thread.
  Machine-readable lines are decoded whole, without display processing; display lines are masked before they are truncated.
  Details: `docs/design.md`.
- Mask credentials at a single point before anything is displayed or logged.
- Keep the way the backend is launched replaceable (bundled `mrtk` now, possibly the GUI executable itself later).

## Naming

- Use `Backend` only for the `mrtk` process, never for a GUI-internal layer.
- RPC method, field and event names follow MRTKLIB `docs/rpc/`.
  In Rust, apply only mechanical case conversion.
- Avoid names that look like parts of MRTKLIB itself (e.g. `mrtklib-core`, `MrtklibParser`).

## Development phase

This is the prototype phase.
A working vertical slice comes first; structure is cleaned up later in planned refactors.

- When a design decision is made, update `docs/design.md`, with its rationale where it is not obvious.
  Do not add dated history; Git keeps it.
- Do not settle undecided questions silently; ask the user.
- Do not add dependencies beyond what the current slice needs.
- Use safe Rust unless there is a documented reason.
- Keep public APIs minimal.
  Keep tests close to the module that owns the behavior.
- Do not add silent fallbacks that hide failures.

## Checks

After code changes, run:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

After Markdown changes, run `rumdl check .` (configuration: `rumdl.toml`).

Test code that uses the RPC against the mock server.
If a tool is unavailable, say so in the report.

In addition to mock coverage, tests needing the real `mrtk` are opt-in: set `MRTKLIB_EGUI_TEST_MRTK` and `MRTKLIB_EGUI_TEST_MRTKLIB_DIR`, then run `cargo test -p mrtklib-egui-navi -- --ignored real_backend_`.

For UI changes, optionally render conv's or plot's headless screenshots with a GPU adapter and the application font/style setup:

```sh
MRTKLIB_EGUI_SNAPSHOT_DIR=/path/to/snapshots \
  cargo test -p mrtklib-egui-conv -p mrtklib-egui-plot -- --ignored snapshot_
```

On NixOS, use the development shell's `LD_LIBRARY_PATH` for the GPU libraries.
These tests are ignored by default and prohibit child process launches.

## Edit scope

Allowed: `Cargo.toml`, `Cargo.lock`, `README.md`, `LICENSE-*`, `flake.nix`, `crates/**`, `src/**`, `xtask/**`, `tests/**`, `docs/**`, `AGENTS.md`, `rumdl.toml`, `.gitignore`, and CI files when working on CI.

Do not edit: Git metadata, generated caches, unrelated editor settings, or files outside this repository.
Changes to MRTKLIB are made in the MRTKLIB repository, not here.

## Git

Commit only when the user asks (a standing request from the user counts).
Push only when the user asks, every time.
