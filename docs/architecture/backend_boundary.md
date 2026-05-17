# Backend Boundary

この文書は, `mrtklib-egui` と MRTKLIB backend の境界を定義する.

## Boundary definition

MRTKLIB backend は bundled または user-selected の `mrtk` executable である.

GUI は `mrtk` executable を child process として起動する.
GUI は MRTKLIB を library として link しない.

## Process model

```text
egui App
  <-> in-process channel
ProcessSupervisor
  <-> std::process / files
mrtk executable
```

`egui App` は rendering and UI state を担当する.
`ProcessSupervisor` は process lifecycle, stdout/stderr stream reading, stop/kill, and event delivery を担当する.
`mrtk executable` は MRTKLIB processing を担当する.

## Input contract

`ProcessSupervisor` から `mrtk` へ渡す入力は次である.

- executable path
- subcommand
- argv
- config file path
- input file path
- output file path
- environment variables
- working directory

Command preview と実行 argv は同じ source から生成する.
表示用 shell string を source of truth にしない.

## Output contract

`ProcessSupervisor` が `mrtk` から受け取る出力は次である.

- stdout stream
- stderr stream
- process exit status
- output files
- trace/log files

stdout/stderr は UI thread で直接読まない.
worker thread が chunk read, line split, truncation, credential masking を行い, event channel に送る.

## Event model

Native app 内部では WebSocket を使わない.
`mrtklib-docker-ui` の WebSocket log/status/progress message は in-process event に写像する.

Conceptual event:

```rust
pub enum MrtkEvent {
    Started { process_id: String, pid: u32 },
    StdoutLine { process_id: String, line: String },
    StderrLine { process_id: String, line: String },
    Exited { process_id: String, code: Option<i32> },
    Error { process_id: String, message: String },
}
```

この型は現時点の conceptual model であり, exact implementation API は未確定である.

## Stream safety rules

stdout/stderr handling は long line and missing newline に耐える必要がある.

Current constants:

```text
READ_CHUNK_BYTES = 8192
MAX_UI_LINE_BYTES = 8 * 1024
MAX_PARTIAL_LINE_BYTES = 1024 * 1024
```

Processing order:

```text
stdout/stderr
  -> chunk read
  -> line split
  -> lossy UTF-8 decode
  -> truncation
  -> credential masking
  -> MrtkEvent
  -> channel
  -> egui AppState
```

## Credential masking

credential masking は UI 表示前の single point で行う.

Masking target includes:

- environment-derived credentials
- downloader credentials
- URL userinfo
- credentials loaded from local credential file

Exact masking patterns are implementation details and must be tested.

## Explicit exclusions

この boundary は次を含まない.

- MRTKLIB C API
- MRTKLIB C ABI
- Rust FFI to MRTKLIB
- protocol decoder owned by GUI
- direct access to MRTKLIB internal data structure
