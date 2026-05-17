# Document Conventions

この文書は `docs/` 配下の文書分割, file naming, and document ownership を定義する.

## Directory roles

- `architecture/`: 既存実装に依存しない architecture specification.
- `conventions/`: architecture と implementation の両方に適用する横断規則.
- `implementation/`: architecture を repository に実装するための guide.

## File naming

- `index.md`: directory entry point.
- `principles.md`: 最上位原則.
- `plan.md`: 作業順序.
- `*_model.md`: model 単位の文書.
- `*_boundary.md`: 外部 interface or process boundary.
- `*_mapping.md`: 抽象概念間の対応.
- `*_layout.md`: file/module/crate の物理配置.
- `*_conventions.md`: 横断規則.
- `*_issues.md`: 未確定事項.
- `*_criteria.md`: 判定条件.

## Ownership rule

各概念は 1 つの owner document で定義する.
他文書では再定義せず, 参照または適用のみを書く.

Examples:

- MRTKLIB backend boundary: `docs/architecture/backend_boundary.md`
- Top-level architecture principles: `docs/architecture/principles.md`
- Document ownership and file naming: this document
- Architecture naming rules: `docs/conventions/naming_conventions.md`
- Unresolved architecture decisions: `docs/architecture/unresolved_issues.md`

## Language

文書本文は日本語で書く.
code identifier, command name, file name, crate name, type name は英語表記を維持する.
独自の日本語訳語は作らない.
