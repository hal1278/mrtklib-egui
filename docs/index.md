# docs

この文書群は, `mrtklib-egui` の architecture, implementation guide, and repository conventions を定義する canonical documentation である.

## Authoritative language

この文書群の authoritative language は日本語である.

code identifier, command name, file name, crate name, type name, and project-specific technical term は英語表記を維持する.
独自の日本語訳語は作らない.

## Project purpose

この repository は, MRTKLIB 操作用の native GUI application を Rust + egui で開発する.

この repository は MRTKLIB の再実装ではない.
MRTKLIB backend は bundled または user-selected の `mrtk` executable である.
GUI は `mrtk` process の起動, 停止, 設定, log 表示, output file 表示, and release bundle metadata 表示を担当する.

## Document groups

- `architecture/`: architecture specification.
- `conventions/`: architecture と implementation の両方に適用する横断規則.
- `implementation/`: architecture を repository に実装するための guide.

## Authority order

文書間で矛盾がある場合, 次の順に優先する.

1. `docs/architecture/`
2. `docs/conventions/`
3. `docs/implementation/`

ただし, 文書分割, file naming, and document ownership に関する事項は `docs/conventions/document_conventions.md` を優先する.
命名規則に関する事項は `docs/conventions/naming_conventions.md` を優先する.

## Current document set

現時点で確定した範囲だけを文書化する.
未確定事項は `docs/architecture/unresolved_issues.md` に分離する.
