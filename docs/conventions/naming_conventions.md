# Naming Conventions

この文書は, `mrtklib-egui` の architecture-level symbol に対する命名規則を定義する.

この文書は個別 symbol の rename 表ではない. 新しい symbol を作るとき, または既存 symbol の妥当性を判断するときの基準を定義する.

## Scope

対象は次である.

- public type
- public enum
- public enum variant
- trait
- architecture-level internal type
- module name
- config/schema key
- document 内で使う architecture term

次は弱適用または対象外である.

- local variable
- private helper function
- test-only fixture name
- temporary prototype name

ただし, local symbol であっても architecture concept を表す場合はこの規則に従う.

## General style

Rust symbol は Rust の一般的な命名規則に従う.

| 対象 | 規則 |
| --- | --- |
| type / trait / enum | `UpperCamelCase` |
| function / method / field | `snake_case` |
| constant | `SCREAMING_SNAKE_CASE` |
| crate/package | `kebab-case` |
| module | `snake_case` |

文書本文は日本語で書く.
code identifier, command name, file name, type name は英語表記を維持する.
独自の日本語訳語は作らない.

## Avoid MRTKLIB core confusion

GUI app 側の Rust module/type は MRTKLIB 本体の core implementation と混同される名前を避ける.

Avoid:

- `mrtklib-core`
- `mrtklib-parser`
- `MrtklibCore`
- `MrtklibParser`

Reason:

この repository は MRTKLIB の再実装ではない.
`core` や `parser` は MRTKLIB 本体の implementation layer と誤認されやすい.

Prefer:

- `ProcessSupervisor`
- `MrtkCommand`
- `BundleMetadata`
- `PathRoots`
- `PresetStore`

## Config / Spec / State boundary

`Config`, `Spec`, and `State` は明確に分離する.

| suffix | 意味 |
| --- | --- |
| `Config` | user-facing file/schema shape |
| `Spec` | validation / normalization 後の declarative specification |
| `State` | runtime で変化する値 |

Rules:

- `Config` を process execution state として使わない.
- `Spec` を live mutable object として使わない.
- `State` を serialized config shape として使わない.

## Id / Index boundary

`Id` と `Index` を混同しない.

- `Id`: semantic identifier.
- `Index`: dense storage position or UI list position.

Example:

- `ProcessId` は process を識別する semantic id.
- `LogLineIndex` は log buffer 内の位置.

## Process terms

`Backend` は MRTKLIB 側の `mrtk executable` を指す場合に限定する.
GUI 内部の process orchestration layer には `Backend` を使わない.

Use:

- `ProcessSupervisor`
- `ProcessInfo`
- `ProcessState`
- `MrtkEvent`

Avoid:

- `RustBackend`
- `GuiBackend`
- `MrtklibBackend` for GUI-owned code

## Command terms

`mrtk` subcommand は `MrtkSubcommand` として表す.

Primary names are:

- `Post`
- `Run`
- `Relay`
- `Convert`
- `Cssr2Rtcm3`

Legacy command names such as `rnx2rtkp`, `rtkrcv`, `str2str`, and `convbin` are compatibility inputs, not architecture-level primary names.

## Path terms

Native app では Docker path を primary name として使わない.

Use logical root names:

- `WorkspaceRoot`
- `DataRoot`
- `CorrectionsRoot`
- `LogsRoot`
- `PresetsRoot`

Avoid using `/workspace`, `/data`, `/opt/mrtklib/corrections` as architecture-level concepts.
