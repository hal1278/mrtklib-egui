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
| `mrtklib-egui-conv` | RTKCONV | `mrtk convert` (finite process) | argv, files, stderr display and exit status in the first slice |
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
- `mrtk post` and `mrtk convert` stay finite processes. The target interface passes argv and files, reads a machine-readable progress mode (one JSON object per line, not on stdout), and cancels by writing a line to stdin or closing it. Results are the exit status and the output files. The Conv first slice is an explicit exception: `convert` has no progress/cancel interface yet, so it displays stderr without parsing and has no Abort action.
  Rationale: these jobs need no server. A signal is hard to deliver to a child process on Windows, and a forced kill leaves partial output files.
- MRTKLIB provides configuration handling: schema, read, write and validate, through the RPC for `mrtk run` and through `mrtk` subcommands otherwise. The GUI has no knowledge of TOML. Which options the GUI shows, where and under which labels, is the GUI's choice.
  Rationale: value tables, defaults and validation are not reimplemented per frontend; a hand-written implementation drifts and drops unknown keys.
- The GUI never parses human-readable MRTKLIB output (console text, stderr progress lines, log messages) to obtain state. Showing such text as-is is fine. Information that exists only as human-readable text is a candidate for the MRTKLIB machine interface and is raised there instead of being parsed.
  Rationale: removing this kind of parsing from frontends is the purpose of the machine interface.
- The protocol is versioned. From 1.0 the GUI checks the major version it supports; while the protocol is 0.x, where any change may be incompatible, it accepts only the exact version it was built for.
- Exact method names, option names, environment variable names and line formats are defined by MRTKLIB `docs/rpc/` (protocol 0.1 is implemented in the fork). RPC method, field and event names keep the spelling of `docs/rpc/`; Rust code applies only mechanical case conversion, so that code and schema can be compared directly.

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
- In the navi and conv prototypes, `MRTKLIB_EGUI_MRTK` selects a development executable. Relative overrides resolve against the working directory, including bare filenames, to avoid an accidental PATH search; an empty override is an error. Otherwise `mrtk` (`mrtk.exe` on Windows) is resolved next to `std::env::current_exe()`, without searching PATH. A later user setting will take precedence over the development variable.
- The GUI shows the path and version of the `mrtk` in use. If that `mrtk` speaks a protocol version the GUI does not support, the GUI shows an error message and does not use it.
- During the prototype, the bundled `mrtk` is built from the fork that implements the RPC (`hal1278/MRTKLIB`). After upstream merges it, it is built from upstream.

## `mrtk run` session

1. The GUI creates a random token and passes it to `mrtk` in an environment variable.
   Rationale: the token never appears on stdout, so it cannot leak through a frontend that logs child output.
2. The GUI starts `mrtk run --rpc-port 0 --no-console --exit-on-stdin-eof --config <file>` in the settings directory, with a stdin pipe held open by the supervisor. The pipe is never written; EOF or a read error asks the backend for normal shutdown even before readiness or after an RPC connection is lost.
3. When `mrtk` is listening, it prints the endpoint as one machine-readable line. That line is the readiness signal.
   Rationale: port 0 avoids port conflicts, and the line replaces a fixed wait that cannot tell "still starting" from "failed".
4. The GUI connects over WebSocket and presents the token.
5. The GUI pulls status and events. Solutions come as a sequence with sequence numbers.
6. Configuration changes are accepted only while the server is stopped, at least in the first implementation. Keys that take effect immediately (currently the log sink level) are the exception.
   Rationale: MRTKLIB writes option values into state that a running engine uses.
7. On exit the GUI queues `shutdown`, then the process supervisor closes stdin and waits up to 5 s before killing a backend that has not exited. RPC shutdown and stdin EOF are one idempotent shutdown.
   Rationale: `shutdown` saves navigation data and closes output files without OS-specific signals; the kill keeps the backend tied to the GUI lifetime even when it hangs.

Each connection starts with an initial synchronization. Events missed while disconnected cannot be recovered; to record solutions, use the `mrtk` output streams (for example a solution file), not the RPC.

### Navi first vertical slice

- The backend starts when the app starts and lives until the app closes, like RTKNAVI's server initialized at window creation. Start and Stop send only the RPC lifecycle requests. The executed command, working directory and displayed preview share one `CommandLine`: `run --rpc-port 0 --no-console --exit-on-stdin-eof --config <settings directory>/<GUI executable file stem>.toml`. The configuration filename is provisional until the configuration screens. At launch, a missing file is reported in status text as using MRTKLIB defaults, based on a file existence check, never by parsing backend output. `generate_token()` supplies the secret environment variable `MRTK_RPC_TOKEN`.
- A non-egui controller owns `Supervisor` and `Session`. Each logic pass reads the supervisor snapshot and starts one session on `Ready { url }`, without `wait_ready` or automatic reconnection. Both workers notify through `Context::request_repaint`; the shared window constructor can supply that context. Pure modules resolve the program and map state to display text, colors and enabled buttons; egui only draws the view and forwards actions.
- Exit and window close start a shutdown worker: queue RPC `shutdown`, then call `Supervisor::stop` with a 5 s grace. Shutdown stops positioning, saves navigation data and closes output files; measured Linux replay shutdown to exit status 0 takes 1.8–2.0 s, so 5 s allows a slow machine, and the kill afterwards keeps the backend tied to the GUI lifetime. The window remains responsive and defers closing until process exit and the supervisor's bounded drain complete. Completion is signalled before joining the session, so its request/close timeouts do not extend the window's closing delay.
- Time is two lines: `GPST <week> <tow>` and ISO 8601 UTC with milliseconds. Round to the nearest millisecond before splitting fields, carrying into the next second, day, year or GPS week. UTC is the GPS epoch (1980-01-06) plus GPST minus the epoch's `leapSeconds`; Gregorian calendar conversion needs no new dependency or leap-second table. Time of week must be finite and in `[0, 604800)` before rounding; invalid input or checked arithmetic failure shows `GPST invalid time` and `UTC invalid time` instead of a date.
- Solution labels and RGB colors follow RTKNAVI: `----` silver `#C0C0C0`, FIX green `#008000`, FLOAT orange `#FFAA00`, SBAS fuchsia `#FF00FF`, DGPS blue `#0000FF`, SINGLE red `#FF0000`, PPP teal `#008080`. DR uses RTKPLOT's gray `#808080`. While running, status text and the square use that color (the square is white for none). Outside running, or after disconnect, status and time text are gray and the square is white.
- Position is `Lat:` and `Lon:` in signed decimal degrees with 8 decimals and `°`, as in mrtklib-docker-ui; RTKNAVI's N/S and E/W labels are not used because the σ line uses `E:` and `N:` for ENU components. `He:` is ellipsoidal height with 3 decimals and `m`. The σ line is `E:%6.3f N:%6.3f U:%6.3f m`, from the square roots of ENU diagonal covariance, clamping negative variances to zero. Quality is RTKNAVI's `Age:%4.1f s Ratio:%4.1f #Sat:%2d`. Like RTKNAVI, `none` keeps the last valid solution's time, position, σ and quality; only the server's solution status changes to `----` (silver, white indicator while running), and NONE's own time is not shown. Before any valid solution these fields have placeholders; the last display remains after Stop and clears on the next Starting event.
- Start is shown when stopped, Stop otherwise; the action is enabled only when connected and the lifecycle is stopped or running. It is disabled during closing. Options stays disabled with a configuration-unavailable hover explanation. Protocol 0.1 supplies no stream state, so there are no Rover/Base/Corr indicators.
- A GUI Start operation begins with no overwrite consents. The session worker settles each accepted Start once, from the first `running` or `stopped` status event with a sequence greater than the response boundary for that request. The server serializes lifecycle transitions (`busy` during starting or stopping), so that first terminal event is the result of this Start. The settled lifecycle is retained with the request outcome, even when other clients complete later transitions in the same event page or before the next UI tick. The controller opens one modal dialog only when that settled lifecycle is `stopped` with `outputExists`, using its original paths; `running` or any other stopped result ends the operation. Failures from other clients remain in status text. If a synchronization replaces the model before settlement, the Start outcome becomes unknown and the operation ends without a dialog; later events cannot settle it, and Start can be tried again. A synchronization after settlement preserves the known outcome. The dialog lists the refused paths in server order, with the working directory and an `Overwrite` checkbox for each path. It stays within the compact window: long paths wrap, the working directory and the paths scroll, and the buttons stay visible. Start is enabled only when every checkbox is checked, and retries with every consent accumulated during this operation, unchanged for the server's byte comparison. A later refusal lists its new paths and retains earlier consents; Cancel ends the operation. Each row owns its per-path choice so other choices can be added later. Append needs MRTKLIB support; editing a path needs configuration RPC. Neither control is shown. Raw paths are retained for consent; all display text still passes through credential masking.
- The bottom status text shows the backend command preview (including path), MRTKLIB and protocol versions when connected, startup or connection failures, or server exit. It also shows lifecycle, `stopReason`, `lastError`, the last request's error reason and message, known mode and nonzero lost-events count. Startup failure includes the supervisor's last masked output line. All view text, including labels and hover text, passes through the supervisor's common mask with the token before egui draws it. Long status text scrolls inside the compact window.
- Switching position formats and time systems, orthometric height, solution scroll-back, PPP-AR labels, input stream indicators and a log panel are outside this slice. Tests cover pure display logic and the controller's readiness/session mapping with an in-process mock. Live session tests check persistent lifecycle states; transient display states are tested with constructed models.

## Conv first slice

- The main window follows RTKCONV: GPST start/end date and clock fields, editable interval choices, input and format, output directory, nine output rows, and Convert, Abort, Options, Plot, Process and Exit. Input formats are Auto plus the fourteen CLI mappings. Auto omits `-r`; it does not infer format by inspecting GNSS files. Browsing uses `rfd` 0.17.2 OS dialogs: default xdg-portal and Wayland features on Linux and native dialogs on Windows. A worker thread opens the dialog, sends the selection or cancellation through a channel, then requests a repaint; the UI only polls and remains responsive. Input uses an open-file dialog, output directory a folder dialog, and output paths a save-file dialog. The form stays locked while a file dialog is pending. No in-window chooser is retained. The form and the log areas scroll inside the window. Conv installs a small `log` logger that writes masked library warnings and errors to stderr and retains the last rfd warning/error for each dialog. If the dialog returns no path with a diagnostic, the message area shows `File dialog unavailable: …`; cancellation without a diagnostic stays silent. Diagnostics are cleared before each dialog, and a successful fallback selection is accepted despite earlier warnings.
- A controller without egui owns the supervisor and the overwrite/close decisions. Pure modules own automatic naming, argv construction, JSON settings and output display. Every Convert builds one `CommandLine`; its preview is also the executed argv. One pure resolver requires an absolute input path (including after home expansion); a relative input is refused with a message asking for an absolute path or `~/...`. Relative output directories and user-edited output paths resolve against the resolved input file's directory, independently of the selected output directory. The GUI's current directory is never used for conversion paths, because it depends on how the application was launched. A leading `~` alone or followed by `/` expands using `std::env::home_dir()` for input, directory and output fields; an unavailable home directory is an error when expansion is requested. Other `~user` forms remain literal. Paths are normalized lexically, removing `.` and `..` before automatic naming or collision checks. Parent components above the root stay at the root; paths that cannot become absolute using these rules are rejected instead of consulting drive-specific current directories. File-dialog directory hints use these same bases and are absolute; an empty hint uses the OS dialog's default, and browsing can replace an invalid relative input without resolving it. The executable path is also absolute and lexically normalized. Resolution does not access the filesystem or resolve symbolic links. The prepared conversion carries the command and selected output paths together; preview, overwrite checks and argv use those same resolved values without a second resolution pass. The child's working directory is not changed. No `-d` is passed, because that option relocates explicit paths too. Conversion fields and Options are locked while running or awaiting overwrite consent, so the displayed command and pending consent stay tied to the job.
- All nine output switches are always supplied: `-o -n -g -h -q -l -b -i -s`. An unchecked or unavailable slot gets an empty argv string, including with `-c`, to prevent implicit outputs. OBS, NAV and SBS are initially selected, as are the stored choices for the other six rows; rows GNAV through INAV are available only for RINEX 2.x, and SBS is unavailable for explicitly selected RINEX input. An enabled empty output path is rejected before launch.
- Automatic paths use the input directory, or the selected output directory. The input's last extension becomes `.obs`, `.nav`, `.gnav`, `.hnav`, `.qnav`, `.lnav`, `.cnav`, `.inav` or `.sbs`; `*` and `?` in the basename become `0`. A generated path equal to the resolved input gets `_` appended, so the overwrite check covers the actual suffixed output. An enabled user-edited path that resolves to the input is rejected with a message, without renaming it. Input and edited path text remain as entered; automatic path fields show the resolved names, including when settings are loaded. Aliases through symbolic links or hard links are not detected: resolution is lexical, and MRTKLIB's own collision check also compares path strings. With RINEX2 Name, use `%r%n0.%yO`, NAV `%r%n0.%yN` for RINEX 2 or GPS-only and `%r%n0.%yP` otherwise, separate NAV suffixes `G H Q L C I`, and `%r%n0_%y.sbs`, in that same directory. MRTKLIB resolves keywords. Directory/naming option changes regenerate unedited paths; edits survive until the input changes. This deliberately preserves explicit user paths when changing options.
- Options are a modal draft: Cancel discards it; OK validates and applies it. RINEX defaults to 3.04, frequencies to 5, time tolerance to 0.005, Station ID to `0000`, and all seven systems to selected. Other defaults follow RTKCONV's empty headers, zero XYZ/delta and unchecked correction flags. The GUI always passes `-tt`, regardless of interval selection, because the CLI's actual zero initialization differs from its advertised default. `-c` is passed only with RINEX2 Name. Unchecked systems generate separate `-y` arguments; excluded satellites generate one `-x` each, and receiver options remain one argv element. Empty optional header/comment fields are omitted; XYZ and antenna delta default to explicit `0/0/0`.
- Slash-joined observer, receiver, antenna, XYZ and delta fields reject literal `/`. They also reject an empty field preceding a nonempty one, because the CLI's tokenization would shift its position; trailing empty fields are representable. This is validation of the argv interface, not MRTKLIB configuration parsing.
- Frequencies is a choice from 1 through 5, mapped to `-f N`. Its hover lists the first-N slot meanings: GPS L1 L2 L5; GLONASS G1 G2; Galileo E1 E5a E5b E6 E5ab; QZSS L1 L5 L2 L6; SBAS L1 L5; BeiDou B1I B3 B2I/B2b B1C B2a; NavIC L5 S. Independent frequency checkboxes would misrepresent the CLI.
- Unreachable controls remain visible and disabled with reasons: Sep NAV, Phase Shift, the fixed-position checkbox beside Approx Pos XYZ, RunBy, FCN, Unit, Observation Types C/L/D/S (always on), Debug (no trace file) and Abort (needs a progress mode). XYZ edits remain reachable through `-hp`, but decoded metadata may replace them. Plot and Process are disabled for a later slice. Output view buttons are outside this slice.
- Signal Mask is reachable from Options. Its 108 checkboxes follow RTKCONV's groups and observation-code indices, with actual strings from MRTKLIB's observation-code table, validated against the CLI `setmask`/`obs2code` parser. Galileo's `8I` caption is corrected to `8L`; no other caption is changed or dropped. All choices remain selectable independently of systems and frequencies: MRTKLIB combines those filters with RINEX-version eligibility and available observations. The mask has its own draft; OK applies to the Options draft, Cancel discards it, and cancelling Options also discards an accepted mask draft. Set All and Unset All are separate buttons, affecting every checkbox.
- Every mask checkbox is initially selected, including for older JSON settings without a mask field. If all 108 are selected, omit `-mask`, preserving MRTKLIB's default of all signals. Otherwise pass exactly one comma-separated allow-list such as `GL1C,EL8L`; `-mask` clears all systems first, so unchecked and undisplayed codes are excluded. An empty selection is refused with a message. The GUI persists the selection in its JSON options; invalid checkbox counts reject the settings file rather than changing its meaning.
- Output rows fill the available width with browse buttons at the right. Option labels and edits use aligned columns and left-aligned labels. Every conversion combo allows enough popup height for all its choices at the initial window size. Frequencies remains the 1 through 5 choice described above.
- Before launch, all enabled output paths without `%` are checked for existence, with errors reported. One modal lists existing absolute paths, deduplicated, and offers Overwrite or Cancel. Consent launches the frozen command; it does not expand keywords or parse output files. Paths containing keywords are left to MRTKLIB because their resolved names require GNSS metadata.
- The one-line message shows the last retained stderr line and, after exit, `exit N`, `exit 255 (-1)`, or the OS signal description. A scrollable log shows retained stderr lines and supervisor errors, with dropped-line counts. These are display text subject to common credential masking and bounds, never parsed as progress or completion metadata. Exit zero alone does not establish that every requested output was retained by MRTKLIB.
- Closing during a conversion opens a modal with Wait focused by default and Kill and close. Wait keeps the window and job open. Kill and close warns that output files may be incomplete, explicitly kills the child, then waits asynchronously for exit and bounded output draining before closing. Exit and the window close control use the same path; normal completion never kills the child. The supervisor's existing Drop cleanup still protects against an unexpected application teardown.
- GUI settings include input, format, time fields, output directory, all output choices/paths and their edited flags, and conversion options. They are JSON in `<settings directory>/<GUI executable file stem>.json`, independent of MRTKLIB TOML. They load at startup and save only on close. Missing files use defaults; unreadable or invalid files print a masked warning on stderr and use defaults without touching the file until close. Settings-path resolution errors prevent conversion; save failures are reported on stderr without a fallback directory. Unknown JSON fields are ignored and omitted fields use defaults; invalid format/version/frequency selections reject the file. Editable argv fields are validated when applying Options or converting, so unfinished edits can round-trip.

## Mock server

- `mrtklib-egui-mock` imitates the `mrtk run` RPC endpoint (protocol 0.1). GUI code and its tests use it for situations that the real server rarely produces (a failed start, lost events, protocol errors).
  Rationale: GUI tests then run with `cargo test` alone, without MRTKLIB or another runtime.
- It is a library, which tests start in-process, and an executable with the RPC options, the token variable and the ready line of `mrtk run`.
- Its tests validate every message it sends against a pinned copy of MRTKLIB `docs/rpc/openrpc.json` (`crates/mrtklib-egui-mock/contract/`). Opt-in real-server tests also exercise the controller with the Linux backend; the default suite needs no MRTKLIB installation.
  Rationale: a generic OpenRPC mock follows the schema but has no state (start, then running, then solution events; `busy` during a transition; sequence numbers).
- Positioning is simulated: a rover circling near a fixed base, going from single to float to fix, with a missing solution every 40 epochs. The values are test data, not positioning; the coordinate conversions that make them consistent stay in the mock and are not reused for display in the GUI crates.
- The mock is used by tests and during development only; the applications do not depend on it.
- Dependencies: `tungstenite` (a WebSocket implementation with a blocking API, so the mock needs no async runtime) and, for the tests only, `jsonschema` with its default features off (no fetching of external `$ref` targets over HTTP).
- Differences from `mrtk run` in the executable: optional leading `run` and `--no-console` are accepted (`--no-console` has no effect: there is no console), and `--config <file>` is accepted but ignored (no configuration is read). It does not generate a token when none is given, remove the token variable from its environment, or print the `--rpc-no-auth` warning. `--exit-on-stdin-eof` requires `--no-console` and requests the same normal shutdown as RPC on EOF or a read error. Its behavior options (start failure, simulated existing outputs through repeatable `--existing-output PATH`, input end, timing) are fixed for the whole process. Existing outputs are simulated strings, without filesystem access or filename expansion. `overwrite` uses exact comparison and rejects NUL strings; refusals list distinct paths in configured order. Unknown parameters and event limits below 1 are `invalidParams`; larger limits are cut to the server maximum (1000 by default).

## RPC client

- `mrtklib-egui-rpc` is the client of the protocol 0.1 RPC, used by the GUI applications and tested against the mock server. It depends on no GUI toolkit and starts no process (that is the process supervisor's job).
- `Client` is one blocking connection that sends one request at a time and returns typed results. A JSON-RPC error carries `reason` and `state`; callers branch on `reason`, never on `message`. `Session` runs a `Client` on a worker thread and keeps a model that the UI reads, as a copy, on every frame; the application passes a callback (a repaint request) that tells it when the model changed.
  Rationale: blocking `tungstenite` on a thread needs no async runtime, as in the mock, and keeps macOS possible. A model behind a mutex has no queue that could grow without bound when the UI is slow.
- `Client::start` and `restart` accept overwrite paths; empty lists omit the member, using the contract default. `LastError.paths` defaults to empty when absent. `Session::request_start` queues consents while keeping the comparable `Command` enum; request outcomes retain the accepted sequence boundary and the Start outcome (`pending`, the first terminal lifecycle, or `unknown` after synchronization before settlement). The worker records settlement while applying every event in order, before publishing the model; reading only the latest lifecycle cannot attribute a result to a request. Restart retains its acceptance result without Start outcome tracking because it includes an intermediate stopped event.
- The session pulls: `sync` once, then `getEvents` every poll interval, and again at once while more events are ready. When events are lost it calls `sync` again; the lifecycle is recovered, the missing solutions and log messages are not. The model follows events in publish order and keeps the latest solution (including `none`), the last valid (non-`none`) solution per event and the mode until the next run starts; the UI decides how to show them outside `running`. A `sync` replaces the last valid solution with the snapshot's non-`none` solution or `None`, since earlier missing solutions are not recovered. Keeping the last valid solution is this client's choice for an RTKNAVI-like display; the protocol does not ask clients for it (a `none` solution carries only its time and status). After a `running` event it asks `getStatus` and takes only the mode from the response, and discards a response older than the last status event.
  Rationale: this is the polling that the protocol is designed for (`docs/rpc/` §7), and the model never shows a state that later events undo.
- While the protocol is 0.x the client accepts only version 0.1 exactly; from 1.0 it checks the major version.
  Rationale: any 0.x change may be incompatible (`docs/rpc/` §4), so a client built for 0.1 cannot know that a later 0.x is safe.
- Poll interval 100 ms (the protocol's example; a client polling every 0.1 to 1 s loses no events in normal operation). Timeout 5 s for connecting and for each request, from sending it to receiving its response: lifecycle requests are answered when they take effect, not when the transition ends, so a healthy server answers quickly; 5 s leaves room for a loaded machine and still reports a hung server; measured Linux replay requests take 0.1–1.9 ms, so the timeout retains ample margin. Provisional: 1000 log messages kept, so that memory stays bounded when level-2 messages flood; set when the log panel is designed.
- There is no automatic reconnection in the first implementation. When the connection ends, the model says why; a server that closes with status 1001 ("going away") is shown as exited, not as an error. A new connection starts with `sync`.
- The token is used for the handshake only. It is not kept and never appears in `Debug`, `Display` or an error. `tungstenite` logs the whole handshake request, token included, at the `log` trace level (`log` is a required dependency of `tungstenite`), so a logger added to the applications must keep `tungstenite` below trace.

## Process supervisor

`mrtklib-egui-supervisor` is a library without a GUI dependency that handles child processes.

- It starts, stops and kills child processes, collects exit status, and reads their output. UI code (windows, panels, widgets) never starts a process.
- The command preview and the executed argv are built from the same value. Display text is never executed.
- `CommandLine` holds the program, argv and optional working directory; execution applies that directory with `Command::current_dir`. With a directory, the preview is `Working directory: "…" | Program: …`; without one, it is the argv alone. This is a labelled display, not a shell command. Its preview quotes ambiguous words for display, not shell execution, and its debug output contains only that masked preview. Secret environment variables are supplied separately and their values added to the masking list. Tokens are 32 OS-random bytes encoded as unpadded base64url; failure of the OS random source stops token generation.
- `Options.readiness` selects `ReadyLine` (the default for navi) or `None` for finite jobs. `None` starts at `Readiness::NotRequired`, treats all output as display text, and enforces neither the ready timeout nor an exit-before-ready failure; `wait_ready` returns an explicit no-readiness error. Spawn/read/monitor failures remain errors.
- Each piped output has a blocking std reader thread; this works with Linux, Windows pipes and macOS without an async runtime. Stdin defaults to null; a held-open pipe option lets the supervisor retain the write end separately from the child lock and never write to it. Navi uses that pipe with `--exit-on-stdin-eof`.
- An `Arc<Mutex<...>>` holds a bounded snapshot, copied by the UI; a notify callback runs after updates, outside the lock, as in the RPC session. UI scheduling never holds up pipe reads; the callback promptly requests a repaint.
- A monitor polls `try_wait` every 100 ms (provisional: exit is shown within 0.1 s). The child lock covers only `try_wait` or `kill`, so a blocking wait cannot delay a kill.
- `stop` first closes the held stdin pipe without waiting behind the child lock, waits during the grace period, then kills and reaps. Drop also closes that pipe before killing and reaping a running child. Both await readers for at most the remaining drain timeout, detach them and join only the monitor on drop; detached readers keep updating the shared state until the pipes close.
- The drain timeout is 1 s after reaping: the exited child's output is already in the pipe and is read within milliseconds. The bound matters when a descendant holds a pipe open and keeps GUI shutdown responsive. Expiry records an error that output is no longer awaited; exit waits finish after draining or expiry. The monitor and shutdown calls share one deadline, so repeated waits do not extend it.
- If the GUI itself crashes or is killed, the OS closes the held stdin pipe, allowing `mrtk --exit-on-stdin-eof` to shut down normally. This crate cannot enforce termination of a hung child after the GUI dies: std provides no portable parent-death signal. A Windows job object is a later platform task. Descendants (programs that `mrtk` started) are not killed by this crate; `mrtk`'s own shutdown is responsible for them.

## Child process output

- Output is not read on the UI thread.
- Pending lines are bounded to 64 KiB per pipe, far above normal output. An overlong display line is wholly replaced by one marker and discarded to the next newline: a credential split at a cut cannot be masked reliably. Display lines split on LF or CR, treating CRLF as one delimiter, including a final unterminated line. Machine-readable lines retain LF framing and strip only one trailing CR; embedded CR bytes are not display delimiters.
- Display lines keep at most 1000 characters after masking, plus a visible `…` when truncated. Output and supervisor errors each retain 1000 entries; older entries are dropped and counted. These values are provisional: bounded memory as in the RPC session, and readable lines; revisit when the log panel is designed.
- Lines defined as machine-readable (the readiness line, progress JSON) are decoded whole in their format, without display processing. A line over the limit or not in the format is an error; it is never silently dropped or truncated. Which lines are machine-readable is defined by MRTKLIB.
- In ready-line mode, this slice recognizes only stderr lines starting with `MRTK-RPC-READY `: decode a JSON object with a string `url`, ignore unknown members and never display the line. Malformed UTF-8, overlong or malformed readiness lines fail startup; a second ready line records an error.
- Readiness is waiting, ready with a URL, or failed; callers copy the snapshot or block on a condition variable. Exit before ready records the exit status after stderr is drained or the drain timeout passes, avoiding a race with the final ready line without waiting forever for inherited pipes. A ready line already read keeps winning.
- The ready timeout is 10 s, and remains enforced after exit while draining: the server reports listening immediately, so healthy starts are fast; this allows a slow machine but reports a stuck start. Measured Linux replay readiness is 23–33 ms; start to running is 0.1–0.3 s, the first valid PPP solution is 0.65–0.86 s after Start (mode `ppp-kine`), and stop to stopped is about 50 ms. These measurements support retaining the ready, request and exit timeouts.
- Other lines are shown without parsing, processed in this order: split into lines, lossy UTF-8 decoding, credential masking, truncation.
  Rationale: masking after truncation can miss a pattern such as `user:pass@host` whose `@host` was cut off, and show part of a password.
- A pipe read error is displayed and the pending incomplete line is discarded: it may also cut a credential in the middle.

## Credentials

- Credentials are masked at a single point before anything is displayed or logged. Targets: URL userinfo, configuration values that the schema marks as secret, and the token created by the GUI.
- `mrtklib_egui_supervisor::mask::mask` first replaces every non-empty supplied secret value with `***`, so a secret spanning userinfo and host is removed whole. Within each whitespace-separated word it then skips leading quotes, an assignment prefix and an optional `scheme://`; before the first path slash, userinfo ends at the last `@`. Userinfo containing a colon is replaced entirely by `***`; userinfo without a password stays visible. Schema-marked secrets will be supplied when configuration RPC exists.
- The backend masks secrets in runtime state (streams, log, status). `getConfig` can return plaintext to the authenticated GUI, so the GUI masks marked values on screen and in logs.
- Masking patterns are tested.

## Settings persistence

- The GUI's own state (windows, recent files, the last settings of each application) is stored in the directory of the executables, at least in the first implementation. The GUI runs standalone, without installation.
  Rationale: this is how RTKLIB is used. If the directory is not writable, saving fails with an error message; there is no fallback location.
- `MRTKLIB_EGUI_SETTINGS_DIR` selects a development settings directory. A relative override resolves against the current directory, an empty override is an error, and there is no fallback. Otherwise the directory is the GUI executable's directory. Navi runs its child there and always supplies the explicit `.toml` configuration described above: RTKNAVI keeps settings and navigation data next to its executable; TOML is MRTKLIB's canonical format and the future `saveConfig` accepts only `.toml`. An explicit extension avoids the implicit `./rtkrcv.conf`, which MRTKLIB reads as the legacy format.
- Relative configuration paths resolve against the settings directory, and `rtkrcv.nav` is saved there. The backend reads configuration at process startup; editing it requires an application restart until configuration RPC exists. An unwritable settings directory makes navigation data saving and relative outputs fail; there is no fallback location.
- Positioning settings can be loaded from and saved to MRTKLIB TOML files. Whether to share them with the CLI is the user's choice.
- The last positioning settings of each application are also stored as TOML in that directory, read and written through MRTKLIB.
  Rationale: this follows RTKLIB, whose GUIs keep their own settings and import or export the CLI option format. Editing a CLI TOML file in place would require rewriting parts of it while keeping comments and unknown keys.
- Conv's controls are `mrtk convert` arguments, not an MRTKLIB configuration, so conv keeps them in the JSON settings file described in the Conv section.

## Real-server tests

Mock tests remain the default (`cargo test`). Linux navi tests prefixed `real_backend_` are ignored unless explicitly requested. They require `MRTKLIB_EGUI_TEST_MRTK` (the local executable) and `MRTKLIB_EGUI_TEST_MRTKLIB_DIR` (the local source tree); missing variables fail clearly rather than skipping. The tests neither fetch nor build MRTKLIB.

```sh
MRTKLIB_EGUI_TEST_MRTK=/path/to/MRTKLIB/build/mrtk \
MRTKLIB_EGUI_TEST_MRTKLIB_DIR=/path/to/MRTKLIB \
  cargo test -p mrtklib-egui-navi -- --ignored real_backend_
```

The tests use system `tar` to extract the tree's MALIB archive into temporary directories. Literal substitutions in `conf/malib/rtkrcv.toml` replace the data prefix and stream1 output path; they do not parse TOML. They launch through the controller's supervisor/session path with an explicit settings directory, observe running and a valid solution, stop, verify `outputExists` paths on the next Start, consent through the dialog and start again, then verify RPC shutdown exits with status 0. A separate test closes held stdin through `Supervisor::stop` without an RPC connection and checks exit status 0 and navigation data saved in the settings directory. Every wait is bounded; no window is opened.

Conv also has an ignored `real_backend_convert_ubx` test using the same two environment variables. It converts `tests/data/rtklib/rcvraw/ubx_20080526.ubx` from the supplied tree through the controller, with only OBS and NAV selected in a temporary output directory. It waits at most 30 s, checks exit status 0 and a nonempty OBS file, and needs no window. Run it with `cargo test -p mrtklib-egui-conv -- --ignored real_backend_`.

Conv's ignored `snapshot_` tests use `egui_kittest` 0.36.2 with only its `wgpu` feature and `image` PNG support. The harness calls the same drawing and logic methods as the native app, with the application's font/style setup and the actual initial 760 by 700 window size. A test-only controller explicitly prohibits process launches. The tests open Options, Signal Mask and overwrite dialogs by their labelled buttons, and render those views plus the main form to PNGs without a visible window. They require a GPU adapter (a software adapter is sufficient) and a nonempty output directory variable; missing configuration or rendering failures fail clearly.

```sh
MRTKLIB_EGUI_SNAPSHOT_DIR=/path/to/snapshots \
  cargo test -p mrtklib-egui-conv -- --ignored snapshot_
```

The files are `snapshot_main.png`, `snapshot_options.png`, `snapshot_mask.png` and `snapshot_overwrite.png`. On NixOS, run in the development shell (`nix develop`); its `LD_LIBRARY_PATH` supplies the GPU runtime libraries. These are review artifacts, not pixel-comparison baselines, so platform font differences do not fail normal tests.

## Platforms

- On Nix/NixOS, rfd needs `libdbus-1.so.3` (or `libdbus-1.so`) at run time for xdg-portal dialogs. The shared Nix runtime library list includes `pkgs.dbus.lib`, both in the executable run paths for `nix run` and in the development shell's `LD_LIBRARY_PATH`, so this dependency is available without a separate launcher setting.
- Linux and Windows are targets. Development and first verification happen on Linux. `mrtk run` works on Windows only after the MRTKLIB Windows port reaches the stream layer.
- macOS is a future target. It is not verified or guaranteed, because there is no test environment. Libraries and threading choices must not exclude it.

## Distribution

- Releases are archives on GitHub Releases, without an installer.
- The license is `BSD-2-Clause OR MIT OR Apache-2.0`. The bundled `mrtk` carries the MRTKLIB BSD-2-Clause notice.
- Dependencies compiled into the applications must have permissive licenses compatible with distributing the applications under `BSD-2-Clause OR MIT OR Apache-2.0`. When a crate offers alternatives, such as `Apache-2.0 OR GPL-2.0-only`, the permissive alternative applies. Copyleft crates are avoided in dev-dependencies too: `egui_kittest` uses only `wgpu`, leaving out `snapshot` because it pulls an MPL-2.0 crate.
- Binary releases include the license texts and copyright notices of all Rust dependencies compiled into the executables, generated from Cargo metadata at release time, in addition to the MRTKLIB notice above.
- Release artifacts record the MRTKLIB commit used for the build and whether its source tree had uncommitted changes.
