# Architecture Principles

この文書は, `mrtklib-egui` の architecture 全体に共通する責務境界と判断基準を定義する.

この文書は具体的な Rust type, module path, file layout, build script の詳細を定義しない.
それらは対応する architecture model 文書または implementation 文書で扱う.

## Scope

この repository は, MRTKLIB 操作用の native GUI application を Rust + egui で実装する.

この repository は MRTKLIB の再実装ではない.
この repository は MRTKLIB の ABI/FFI binding project でもない.

## Backend boundary

MRTKLIB backend は `mrtk` executable である.

GUI は MRTKLIB を library として link しない.
GUI は MRTKLIB の static library, shared library, C ABI, or Rust FFI を使用しない.

GUI と MRTKLIB backend の境界は process boundary とする.

```text
egui App
  <-> in-process channel
ProcessSupervisor
  -> command-line arguments / config files / environment / working directory
mrtk executable
  -> stdout / stderr / output files / trace files / exit status
ProcessSupervisor
  <-> in-process channel
egui App
```

`ProcessSupervisor` は GUI application 側の process orchestration layer である.
`ProcessSupervisor` は MRTKLIB backend ではない.

## Communication contract

GUI から `mrtk` へ渡すものは次に限定する.

- executable path
- subcommand
- command-line arguments
- TOML config file path
- input/output file paths
- environment variables
- working directory

`mrtk` から GUI が受け取るものは次に限定する.

- stdout
- stderr
- generated output files
- trace/log files
- exit status

初期設計では stdin を通信路に含めない.

## Unified `mrtk` CLI

新しい code, documentation, and UI surface では unified `mrtk` binary の subcommand を primary concept とする.

採用する主要 subcommand は次である.

- `mrtk post`
- `mrtk run`
- `mrtk relay`
- `mrtk convert`
- `mrtk cssr2rtcm3`

旧 command name は compatibility mapping として扱ってよいが, architecture-level concept にはしない.

## MRTKLIB source and bundle model

Development/user build の既定では, `h-shiono/MRTKLIB` の `main` branch の最新 commit を取得して `mrtk` を build する.

MRTKLIB に変更を加えたい場合, fork repository の commit または local repository を指定できるようにする.

Release artifact には `mrtk` executable と MRTKLIB metadata を含める.
GUI は使用中の MRTKLIB source, resolved commit, and executable path を表示する.

## Do not reimplement MRTKLIB

GUI 側に次を実装しない.

- positioning engine
- GNSS algorithm implementation
- RINEX parser library
- RTCM parser library
- UBX/SBF raw decoder library
- CSSR decoder
- GNSS time/ephemeris computation engine

必要な処理が既存の `mrtk` CLI output から得られない場合, 原則として GUI 側ではなく MRTKLIB 側の subcommand or machine-readable output mode として扱う.

## Configuration ownership

MRTKLIB configuration の primary format は TOML とする.

GUI は TOML の editing surface を提供するが, TOML document 全体の owner ではない.
GUI が理解していない key は import/export round-trip で保持する.

## Path roots

Docker-specific path である `/workspace`, `/data`, `/opt/mrtklib/corrections` は native app の architecture-level path として使わない.

Native app では logical root を使う.

- `WorkspaceRoot`: writable output root
- `DataRoot`: input data root
- `CorrectionsRoot`: bundled/system correction file root
- `LogsRoot`: writable log root
- `PresetsRoot`: writable preset root

Exact default path は platform-specific implementation document で扱う.

## Current non-goals

次は current scope 外である.

- MRTKLIB の Rust 再実装
- MRTKLIB ABI/FFI binding
- protocol parser library creation
- positioning engine implementation
- web server compatibility layer
- Docker compatibility layer
- Windows/macOS first-class implementation before Linux baseline
- plugin ABI stability
