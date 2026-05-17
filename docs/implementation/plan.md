# Implementation Plan

この文書は, 確定済み architecture に基づく初期 implementation order を定義する.

この文書は working slice の順序を定義するが, exact UI layout や全機能の詳細仕様は定義しない.

## Principle

Speculative abstraction より working, tested baseline を優先する.

MRTKLIB の再実装や parser library 作成に進まない.
`mrtk` executable との process boundary を保つ.

## Initial implementation order

1. Repository skeleton.
2. egui application skeleton.
3. Settings surface for `mrtk` path and bundle metadata display.
4. `xtask` skeleton for MRTKLIB source selection and metadata generation.
5. `ProcessSupervisor`.
6. `mrtk post` command builder and execution.
7. stdout/stderr log pane.
8. `mrtk convert` command builder and execution.
9. `mrtk run` long-running process handling.
10. `mrtk relay` process handling.
11. CLAS Pipeline orchestration.

## Slice criteria

Each slice should satisfy:

- compiles with `cargo build`
- formatted with `cargo fmt`
- relevant unit tests are added where behavior is non-trivial
- architecture boundaries in `docs/architecture/` are not contradicted
- unresolved design questions are recorded in `docs/architecture/unresolved_issues.md`

## First functional target

The first functional target is `mrtk post`.

Minimum capability:

- select or configure `mrtk` executable
- show MRTKLIB bundle/source metadata when available
- build argv for `mrtk post`
- show command preview
- start process
- stop process if still running
- stream stdout/stderr to log pane
- show exit status
- show output file path

## Initial non-targets

The initial implementation does not include:

- Monitor tab protocol parsing
- UBX/SBF/NMEA parser library
- RINEX parser library
- RTCM/CSSR parser library
- realtime plot
- Windows/macOS packaging
- plugin system
- ABI/FFI link to MRTKLIB
