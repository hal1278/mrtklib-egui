# mrtklib-egui design

This document is the reference for the design of `mrtklib-egui`.
Each rule states its rationale where the reason is not obvious. History is in the Git log, not here.

## Purpose and scope

`mrtklib-egui` is an RTKLIB-style GUI suite for MRTKLIB, written in Rust and egui.
The target users are users of classic RTKLIB, so Windows is a primary target.

It is also the first consumer of the MRTKLIB machine interface ([h-shiono/MRTKLIB#326](https://github.com/h-shiono/MRTKLIB/issues/326), [#327](https://github.com/h-shiono/MRTKLIB/issues/327)).
Building the GUI checks that the interface is sufficient in practice, so that gaps are found before the interface schema is frozen.

This repository does not reimplement MRTKLIB and is not a library binding for it.
The toolkit is egui for now and may change.

## Applications

| Executable | Equivalent | MRTKLIB feature | Boundary |
|---|---|---|---|
| `mrtklib-egui-navi` | RTKNAVI | `mrtk run` | RPC |
| `mrtklib-egui-post` | RTKPOST | `mrtk post` (finite process) | argv, files, progress output, cancel through stdin |
| `mrtklib-egui-conv` | RTKCONV | `mrtk convert` (finite process) | same as above |
| `mrtklib-egui-plot` | RTKPLOT | solution files, the `mrtk run` monitor port | reads defined formats only |

- Each application is a separate executable. Shared parts (RPC client, process supervisor, configuration handling) live in shared crates of one cargo workspace. There is no launcher.
  Rationale: this matches the structure of the RTKLIB suite, and egui support for multiple windows is not verified yet.
- STRSVR, RTKGET and NTRIP browser equivalents are out of the first milestone.
  Rationale: the four applications above are enough to exercise the new interfaces (RPC, progress output, configuration handling); more applications would widen the interface work (for example relay statistics).

## User interface

- Functions that RTKLIB also has follow the RTKLIB GUI, at least in the first implementation.
  Rationale: the target users know RTKLIB.
- Functions that MRTKLIB adds (for example CLAS and MADOCA-PPP) follow the same style. The MADOCALIB GUI may serve as a reference.
- The look is the classic light one in every OS theme, including a dark OS setting.
  Rationale: RTKLIB has no dark mode.
- On Windows the applications use the fonts that RTKLIB shows on Japanese Windows: Tahoma for Latin text and MS UI Gothic (in `Msgothic.ttc`) for Japanese text, the font that Windows links to Tahoma first there. A Japanese Windows environment is assumed to have both; no other fonts are tried in their place, at least in the first implementation. egui's built-in fonts stay after them in the list and draw only what Tahoma and MS UI Gothic lack. The fonts are looked up in the system font directory, then in the current user's font directory.
  Rationale: RTKLIB's forms use Tahoma and leave Japanese glyphs to Windows' font linking. egui draws text itself and has no OS font fallback, so the same fonts are chosen explicitly. Reading the font-link chain from the registry would also follow customized systems, but needs considerably more code and cannot be checked on the Linux development machine. The link was checked on Windows 11 with the Japanese system locale: the font-link entry for Tahoma starts with MS UI Gothic in `MSGOTHIC.TTC`.
- On Linux and macOS, egui's built-in fonts draw Latin text and a system font draws Japanese text: Noto Sans CJK JP on Linux, Hiragino on macOS (not verified). No font is bundled.
  Rationale: bundling a CJK font adds tens of megabytes.
- Monospace text uses egui's built-in monospace font, with the Japanese font after it.
- A font inside a collection (`.ttc`) is chosen by family name, because one file holds several faces (MS Gothic, MS PGothic and MS UI Gothic).
- The environment variable `MRTKLIB_EGUI_CJK_FONT` names a file that replaces the Japanese font (on Windows, MS UI Gothic); if that file cannot be read, the application stops. It does not change the Latin font.
- If a font is not found, the applications still start and print a warning on stderr: without Tahoma, Latin text uses the built-in font; without a Japanese font, Japanese text cannot be shown.

## Relationship to MRTKLIB

### Coupling

- The GUI starts the `mrtk` executable as a child process and never links MRTKLIB.
  Rationale: the GUI then does not depend on MRTKLIB internals or a C ABI, control logic is not duplicated in each frontend, and a crash in positioning does not take the GUI down.
- Keep the way the backend is launched replaceable. A future option is a single executable that links MRTKLIB and starts itself with backend arguments, keeping the process boundary.
- Each GUI process that needs a backend starts its own `mrtk` and ties it to its own lifetime. With several receivers there are several `mrtk` processes. Such a backend is used only by the GUI that started it; the GUI does not show its endpoint or token, at least in the first implementation.
  Rationale: this is how each RTKNAVI window owns its server, and it avoids designing discovery, ownership and authentication for a shared backend. A system-wide shared backend is out of scope.
- The GUI can also connect to an `mrtk run` that it did not start, when the user enters its host, port and token. At least in the first implementation, the GUI does not detect running backends and never attaches to one on its own.
  Rationale: the token is set when the backend starts (by the parent process, a setting, or shown on the terminal), so only the user can supply it.

### Interfaces

- `mrtk run` is controlled only through the RPC (JSON-RPC 2.0 over WebSocket). The GUI never uses the telnet console; the machine-interface design (#326) keeps it only as a frozen compatibility interface.
- `mrtk post` and `mrtk convert` stay finite processes. The GUI passes argv and files, reads a machine-readable progress mode (one JSON object per line, not on stdout), and cancels by writing a line to stdin or closing it. Results are the exit status and the output files.
  Rationale: these jobs need no server. A signal is hard to deliver to a child process on Windows, and a forced kill leaves partial output files.
- MRTKLIB provides configuration handling: schema, read, write and validate, through the RPC for `mrtk run` and through `mrtk` subcommands otherwise. The GUI has no knowledge of TOML. Which options the GUI shows, where and under which labels, is the GUI's choice.
  Rationale: value tables, defaults and validation are not reimplemented per frontend; a hand-written implementation drifts and drops unknown keys.
- The GUI never parses human-readable MRTKLIB output (console text, stderr progress lines, log messages) to obtain state. Showing such text as-is is fine. Information that exists only as human-readable text is a candidate for the MRTKLIB machine interface and is raised there instead of being parsed.
  Rationale: removing this kind of parsing from frontends is the purpose of the machine interface.
- The protocol is versioned. From 1.0 the GUI checks the major version it supports; while the protocol is 0.x, where any change may be incompatible, it accepts only the exact version it was built for.
- Exact method names, option names, environment variable names and line formats are defined by MRTKLIB `docs/rpc/` (protocol 0.1 is drafted in the fork that implements the RPC; the RPC itself is not implemented yet). RPC method, field and event names keep the spelling of `docs/rpc/`; Rust code applies only mechanical case conversion, so that code and schema can be compared directly.

### Allowed in the GUI

The GUI does not implement:

- a positioning engine or GNSS algorithms
- reading or writing of GNSS data formats (receiver raw data, RTCM, RINEX, correction data, products and any other format MRTKLIB handles), with the single exception of MRTKLIB solution files below
- a GNSS time or ephemeris computation engine
- MRTKLIB option parsing and validation

These are part of the GUI's job:

- decoding the RPC and the machine-readable output of `mrtk`
- reading MRTKLIB solution files for plotting
- conversions for display, such as GPST/UTC to local time or GPS week and seconds, and an ENU baseline from a solution and the base position

### The `mrtk` executable

- `mrtk` is bundled next to the GUI executables, and the GUI uses that copy by default. A user setting can select another `mrtk`. `mrtk` on PATH is not used by default.
  Rationale: users start the GUI like RTKLIB, without a separate install; the user setting allows trying development builds.
- The GUI shows the path and version of the `mrtk` in use. If that `mrtk` speaks a protocol version the GUI does not support, the GUI shows an error message and does not use it.
- During the prototype, the bundled `mrtk` is built from the fork that implements the RPC (`hal1278/MRTKLIB`). After upstream merges it, it is built from upstream.

## `mrtk run` session

1. The GUI creates a random token and passes it to `mrtk` in an environment variable.
   Rationale: the token never appears on stdout, so it cannot leak through a frontend that logs child output.
2. The GUI starts `mrtk run` in the mode that opens only the RPC, with port 0.
3. When `mrtk` is listening, it prints the endpoint as one machine-readable line. That line is the readiness signal.
   Rationale: port 0 avoids port conflicts, and the line replaces a fixed wait that cannot tell "still starting" from "failed".
4. The GUI connects over WebSocket and presents the token.
5. The GUI pulls status and events. Solutions come as a sequence with sequence numbers.
6. Configuration changes are accepted only while the server is stopped, at least in the first implementation. Keys that take effect immediately (currently the log sink level) are the exception.
   Rationale: MRTKLIB writes option values into state that a running engine uses.
7. On exit the GUI calls `shutdown`. If `mrtk` does not respond, the process supervisor kills it.
   Rationale: `shutdown` saves navigation data and closes output files without OS-specific signals; the kill keeps the backend tied to the GUI lifetime even when it hangs.

Each connection starts with an initial synchronization. Events missed while disconnected cannot be recovered; to record solutions, use the `mrtk` output streams (for example a solution file), not the RPC.

## Mock server

- `mrtklib-egui-mock` imitates the `mrtk run` RPC endpoint (protocol 0.1). GUI code and its tests use it until `mrtk` implements the RPC, and later for situations that the real server rarely produces (a failed start, lost events, protocol errors).
  Rationale: GUI tests then run with `cargo test` alone, without MRTKLIB or another runtime.
- It is a library, which tests start in-process, and an executable with the RPC options, the token variable and the ready line of `mrtk run`.
- Its tests validate every message it sends against a pinned copy of MRTKLIB `docs/rpc/openrpc.json` (`crates/mrtklib-egui-mock/contract/`). The MRTKLIB contract test is planned to check the real server against the same file when the RPC is implemented; then the mock and the server cannot drift apart unnoticed.
  Rationale: a generic OpenRPC mock follows the schema but has no state (start, then running, then solution events; `busy` during a transition; sequence numbers).
- Positioning is simulated: a rover circling near a fixed base, going from single to float to fix, with a missing solution every 40 epochs. The values are test data, not positioning; the coordinate conversions that make them consistent stay in the mock and are not reused for display in the GUI crates.
- The mock is used by tests and during development only; the applications do not depend on it.
- Dependencies: `tungstenite` (a WebSocket implementation with a blocking API, so the mock needs no async runtime) and, for the tests only, `jsonschema` with its default features off (no fetching of external `$ref` targets over HTTP).
- Differences from `mrtk run` in the executable: it does not generate a token when none is given and does not print the `--rpc-no-auth` warning. Its behavior options (start failure, input end, timing) are fixed for the whole process.

## RPC client

- `mrtklib-egui-rpc` is the client of the protocol 0.1 RPC, used by the GUI applications and tested against the mock server. It depends on no GUI toolkit and starts no process (that is the process supervisor's job).
- `Client` is one blocking connection that sends one request at a time and returns typed results. A JSON-RPC error carries `reason` and `state`; callers branch on `reason`, never on `message`. `Session` runs a `Client` on a worker thread and keeps a model that the UI reads, as a copy, on every frame; the application passes a callback (a repaint request) that tells it when the model changed.
  Rationale: blocking `tungstenite` on a thread needs no async runtime, as in the mock, and keeps macOS possible. A model behind a mutex has no queue that could grow without bound when the UI is slow.
- The session pulls: `sync` once, then `getEvents` every poll interval, and again at once while more events are ready. When events are lost it calls `sync` again; the lifecycle is recovered, the missing solutions and log messages are not. The model follows the events in publish order and keeps the last run's solution and mode until the next run starts; the UI decides how to show them outside `running`. After a `running` event it asks `getStatus` and takes only the mode from the response, and discards a response older than the last status event.
  Rationale: this is the polling that the protocol is designed for (`docs/rpc/` §7), and the model never shows a state that later events undo.
- While the protocol is 0.x the client accepts only version 0.1 exactly; from 1.0 it checks the major version.
  Rationale: any 0.x change may be incompatible (`docs/rpc/` §4), so a client built for 0.1 cannot know that a later 0.x is safe.
- Provisional values: poll interval 100 ms (the protocol's example; a client polling every 0.1 to 1 s loses no events in normal operation). Timeout 5 s for connecting and for each request, from sending it to receiving its response: lifecycle requests are answered when they take effect, not when the transition ends, so a healthy server answers quickly; 5 s leaves room for a loaded machine and still reports a hung server; revisit after measuring the real `mrtk run`. 1000 log messages kept, so that memory stays bounded when level-2 messages flood; set when the log panel is designed.
- There is no automatic reconnection in the first implementation. When the connection ends, the model says why; a server that closes with status 1001 ("going away") is shown as exited, not as an error. A new connection starts with `sync`.
- The token is used for the handshake only. It is not kept and never appears in `Debug`, `Display` or an error. `tungstenite` logs the whole handshake request, token included, at the `log` trace level (`log` is a required dependency of `tungstenite`), so a logger added to the applications must keep `tungstenite` below trace.

## Process supervisor

`mrtklib-egui-supervisor` is a library without a GUI dependency that handles child processes.

- It starts, stops and kills child processes, collects exit status, and reads their output. UI code (windows, panels, widgets) never starts a process.
- The command preview and the executed argv are built from the same value. Display text is never executed.
- `CommandLine` holds the program and argv; its preview quotes ambiguous words for display, not shell execution. Secret environment variables are supplied separately and their values added to the masking list. Tokens are 32 OS-random bytes encoded as unpadded base64url; failure of the OS random source stops token generation.
- Each piped output has a blocking std reader thread; this works with Linux, Windows pipes and macOS without an async runtime. Stdin is null in this slice.
- An `Arc<Mutex<...>>` holds a bounded snapshot, copied by the UI; a notify callback runs after updates, outside the lock, as in the RPC session. UI scheduling never holds up pipe reads; the callback promptly requests a repaint.
- A monitor polls `try_wait` every 100 ms (provisional: exit is shown within 0.1 s). The child lock covers only `try_wait` or `kill`, so a blocking wait cannot delay a kill.
- `stop` waits for the caller's shutdown request during a grace period, then kills and reaps. Exit waits also drain output so final lines are available. Drop kills a running child, reaps it and joins the threads, tying its lifetime to the GUI.
- If the GUI itself crashes or is killed, this crate cannot kill the child: std provides no portable parent-death signal. A Windows job object is a later platform task.

## Child process output

- Output is not read on the UI thread.
- Pending lines are bounded to 64 KiB per pipe, far above normal output. An overlong display line is wholly replaced by one marker and discarded to the next newline: a credential split at a cut cannot be masked reliably. Split on newline and strip one trailing carriage return, including a final unterminated line.
- Display lines keep at most 1000 characters after masking, plus a visible `…` when truncated. Output and supervisor errors each retain 1000 entries; older entries are dropped and counted. These values are provisional: bounded memory as in the RPC session, and readable lines; revisit when the log panel is designed.
- Lines defined as machine-readable (the readiness line, progress JSON) are decoded whole in their format, without display processing. A line over the limit or not in the format is an error; it is never silently dropped or truncated. Which lines are machine-readable is defined by MRTKLIB.
- This slice recognizes only stderr lines starting with `MRTK-RPC-READY `: decode a JSON object with a string `url`, ignore unknown members and never display the line. Malformed UTF-8, overlong or malformed readiness lines fail startup; a second ready line records an error.
- Readiness is waiting, ready with a URL, or failed; callers copy the snapshot or block on a condition variable. Exit before ready records the exit status after stderr is drained, avoiding a race with the final ready line.
- The ready timeout is 10 s, provisionally: the server reports listening immediately, so healthy starts are fast; this allows a slow machine but reports a stuck start. Revisit with real `mrtk run`.
- Other lines are shown without parsing, processed in this order: split into lines, lossy UTF-8 decoding, credential masking, truncation.
  Rationale: masking after truncation can miss a pattern such as `user:pass@host` whose `@host` was cut off, and show part of a password.
- A pipe read error is displayed and the pending incomplete line is discarded: it may also cut a credential in the middle.

## Credentials

- Credentials are masked at a single point before anything is displayed or logged. Targets: URL userinfo, configuration values that the schema marks as secret, and the token created by the GUI.
- `mrtklib_egui_supervisor::mask::mask` replaces every non-empty supplied secret value with `***`. Within each whitespace-separated word it skips leading quotes, an assignment prefix and an optional `scheme://`; before the first path slash, userinfo ends at the last `@`. Userinfo containing a colon is replaced entirely by `***`; userinfo without a password stays visible. Schema-marked secrets will be supplied when configuration RPC exists.
- The backend masks secrets in runtime state (streams, log, status). `getConfig` can return plaintext to the authenticated GUI, so the GUI masks marked values on screen and in logs.
- Masking patterns are tested.

## Settings persistence

- The GUI's own state (windows, recent files, the last settings of each application) is stored in the directory of the executables, at least in the first implementation. The GUI runs standalone, without installation.
  Rationale: this is how RTKLIB is used. If the directory is not writable, saving fails with an error message; there is no fallback location.
- Positioning settings can be loaded from and saved to MRTKLIB TOML files. Whether to share them with the CLI is the user's choice.
- The last positioning settings of each application are also stored as TOML in that directory, read and written through MRTKLIB.
  Rationale: this follows RTKLIB, whose GUIs keep their own settings and import or export the CLI option format. Editing a CLI TOML file in place would require rewriting parts of it while keeping comments and unknown keys.

## Platforms

- Linux and Windows are targets. Development and first verification happen on Linux. `mrtk run` works on Windows only after the MRTKLIB Windows port reaches the stream layer.
- macOS is a future target. It is not verified or guaranteed, because there is no test environment. Libraries and threading choices must not exclude it.

## Distribution

- Releases are archives on GitHub Releases, without an installer.
- The license is `BSD-2-Clause OR MIT OR Apache-2.0`. The bundled `mrtk` carries the MRTKLIB BSD-2-Clause notice.
- Release artifacts record the MRTKLIB commit used for the build and whether its source tree had uncommitted changes.
