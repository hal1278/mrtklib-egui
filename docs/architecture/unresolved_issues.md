# Unresolved Architecture Issues

この文書は, 現時点で確定していない architecture decision を記録する.

ここに記録された事項は未確定であり, implementation や AGENTS.md で確定事項として扱ってはならない.

## Runtime `mrtk` selection priority

確定済み:

- release artifact には bundled `mrtk` を含める.
- GUI は使用中の MRTKLIB source and commit を表示する.
- user-selected external `mrtk` は許可する.

未確定:

- bundled `mrtk` を常に最優先にするか.
- user setting を最優先にするか.
- environment variable をどの順位に置くか.
- PATH 上の `mrtk` を fallback として許可するか.

Candidate A:

```text
1. user setting
2. bundled
3. environment variable
4. PATH
```

Candidate B:

```text
1. bundled
2. user setting
3. environment variable
4. PATH
```

Release reproducibility を重視する場合は Candidate B が有利である.
Development flexibility を重視する場合は Candidate A が有利である.

## Machine-readable output from MRTKLIB

確定済み:

- 初期設計では stdout/stderr/output files を読む.
- GUI 側で MRTKLIB internal protocol parser を作らない.

未確定:

- MRTKLIB 側に JSON Lines output mode を追加するか.
- CSV / text solution output を当面使うか.
- progress/status をどの subcommand がどの format で出すか.

## Monitor feature

確定済み:

- GUI 側に UBX/SBF/NMEA parser library を作らない.

未確定:

- MRTKLIB 側に monitor subcommand を追加するか.
- 既存の `mrtk run` or `mrtk relay` output で代替するか.
- Monitor tab を initial Linux version に含めるか.

## Platform default paths

確定済み:

- Docker 固定 path は native app の architecture-level path として使わない.
- Logical roots を使う.

未確定:

- Linux の default workspace path.
- Linux の config/state/cache directory policy.
- Windows/macOS の default directory.
- bundled correction files の exact path.

## Release package format

確定済み:

- release artifact には GUI executable, `mrtk` executable, and metadata を含める.

未確定:

- Linux package format: `tar.gz`, AppImage, distro package.
- Windows package format: zip, MSI.
- macOS package format: `.app`, dmg.
- signing and notarization policy.

Initial Linux release は `tar.gz` が最小候補である.

## CI matrix

確定済み:

- development OS and first supported OS は Linux.
- long-term target は Linux / Windows / macOS.

未確定:

- CI で MRTKLIB を毎回 build するか.
- GUI-only test job と bundle job を分離するか.
- Windows/macOS で MRTKLIB build をいつ有効化するか.

## GUI layout details

確定済み:

- Rust + egui + eframe を使う.
- `ProcessSupervisor` を通して `mrtk` process を扱う.
- Post, Convert, Run, Relay, CLAS Pipeline, Settings are known feature surfaces.

未確定:

- SidePanel navigation か top tab navigation か.
- Log pane を bottom panel に固定するか.
- Command preview の exact position.
- Realtime plot の implementation timing.
- Theme/font/density policy.
