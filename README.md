# mrtklib-egui

An RTKLIB-style GUI suite for [MRTKLIB](https://github.com/h-shiono/MRTKLIB), written in Rust and egui. It controls the `mrtk` executable through its machine interface and does not link to MRTKLIB. This is a prototype; the design is in [docs/design.md](docs/design.md).

The steps below build the applications from source on Linux, for developers. They describe the current prototype and will change as it grows.

## What works now

| Application | State |
|---|---|
| `mrtklib-egui-navi` | Starts and stops `mrtk run` and shows its solution. Settings cannot be edited in the GUI yet. |
| `mrtklib-egui-post`, `-conv`, `-plot` | Empty windows. |

Windows is not supported yet: `mrtk run` needs the MRTKLIB Windows port first.

## Build `mrtk`

The RPC that the GUI uses exists only in the fork branch `prototype/machine-interface` for now.

```sh
git clone --branch prototype/machine-interface https://github.com/hal1278/MRTKLIB.git
```

Install the dependencies and build as described in that branch's [`docs/getting-started/install.md`](https://github.com/hal1278/MRTKLIB/blob/prototype/machine-interface/docs/getting-started/install.md). CivetWeb must be built with WebSocket support. The result is `MRTKLIB/build/mrtk`.

## Build the GUI

Requirements: a recent stable Rust toolchain (`cargo`). At run time the window system libraries (Wayland or X11, libxkbcommon) and an OpenGL or Vulkan driver are loaded; desktop installations normally have them. Japanese text needs Noto Sans CJK JP (for example the `fonts-noto-cjk` package on Debian and Ubuntu); without it the applications print a warning and still start.

```sh
git clone --branch develop https://github.com/hal1278/mrtklib-egui.git
cd mrtklib-egui
cargo build --release -p mrtklib-egui-navi
```

The executable is `target/release/mrtklib-egui-navi`. A first release build takes about 10 minutes on a 12-thread desktop CPU.

## Run navi

Navi uses the `mrtk` next to its own executable and keeps its settings in that directory. To keep the build tree clean, select both with environment variables instead:

```sh
mkdir -p "$HOME/mrtklib-navi"
MRTKLIB_EGUI_MRTK=/path/to/MRTKLIB/build/mrtk \
MRTKLIB_EGUI_SETTINGS_DIR="$HOME/mrtklib-navi" \
  target/release/mrtklib-egui-navi
```

- The status line at the bottom of the window shows the `mrtk` command, including the path of the MRTKLIB TOML configuration that it reads. Put your configuration at that path; without it MRTKLIB defaults are used.
- `mrtk` runs in the settings directory: relative paths in the configuration resolve there, and `rtkrcv.nav` is saved there.
- The configuration is read when navi starts. After editing it, restart navi.
- `mrtk` exits when navi exits, also when navi is killed.

## Nix (optional)

Nix users can use the flake instead of installing the Rust toolchain and run-time libraries themselves. It covers the GUI only; build `mrtk` as above.

```sh
nix develop                  # shell with cargo, rustfmt, clippy and rumdl
MRTKLIB_EGUI_MRTK=/path/to/MRTKLIB/build/mrtk \
MRTKLIB_EGUI_SETTINGS_DIR="$HOME/mrtklib-navi" \
  nix run .#navi
```

With `nix run` both variables are needed: the executable is in the read-only Nix store, so there is no `mrtk` next to it and settings cannot be saved there. The run-time library path also includes `/run/opengl-driver/lib`, where NixOS keeps the GPU driver; this has been tried on NixOS only.

## Checks

The checks for changes are listed in [`AGENTS.md`](AGENTS.md). Tests against a real `mrtk` are described in [docs/design.md](docs/design.md#real-server-tests).

## License

Licensed under any of [BSD-2-Clause](LICENSE-BSD), [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
