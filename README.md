# Zembed

Embedded development for the [Zed](https://zed.dev) editor.

Zembed lets you debug microcontrollers from Zed's built-in debugger:
breakpoints, stepping, call stack, variables. It provides two debug adapters:

- **`gdb-openocd`**, for **C and C++** firmware (Arduino, ESP-IDF, ...),
  through GDB and OpenOCD. With **PlatformIO** and an **ESP32**, it needs no
  configuration at all.
- **`probe-rs`**, for **embedded Rust**, through [probe-rs](https://probe.rs).

> **Status: prototype.** Tested on one board so far (Seeed XIAO ESP32S3, Linux).
> See [Known limits](#known-limits).

## Quick start: ESP32 with PlatformIO

For ESP32-S3, C3, C5, C6, H2 and P4 boards, which debug over the same USB
cable you upload with.

1. **Install the tools once.** In the Arduino IDE, install the "esp32" board
   package (Boards Manager, version 3.x). It ships the GDB and OpenOCD Zembed
   needs. ESP-IDF or a recent PlatformIO `espressif32` platform work too.
2. **Prepare the project.** Add `build_type = debug` to your environment in
   `platformio.ini` so the firmware is not optimized, and copy
   [`templates/platformio/tasks.json`](templates/platformio/tasks.json) to
   `.zed/tasks.json` (build, upload and monitor tasks) if the project has no
   PlatformIO tasks yet.
3. **Debug.** Open a C or C++ file, run `debugger: start` (F4) and pick
   **Debug: PlatformIO: upload** (any `pio run` task works).

No `.zed/debug.json` is needed: Zembed turns every `pio run` task into an
"upload, then debug" session. A plain `pio run` gets `-t upload` added, the
`-e <env>` of the task picks the firmware, and tasks that never end
(`monitor`) or do not flash (`clean`, `test`, ...) are left alone. To attach
to the running firmware without uploading, use the **Launch** tab of
`debugger: start`, pick the `gdb-openocd` adapter and type `auto` as the
program (Zed requires a value; anything that is not a file path means "find
the firmware").

Zembed reads the board from `platformio.ini`, finds GDB and OpenOCD, checks
that the board answers, then halts the chip. Set breakpoints and continue.
Both cores are debugged, and FreeRTOS tasks show up as threads. When the
session ends, the firmware keeps running.

To name the sessions or change a setting, write them in `.zed/debug.json`
instead, starting from
[`templates/platformio/debug.json`](templates/platformio/debug.json):

```json
[
  {
    "label": "Upload and debug",
    "adapter": "gdb-openocd",
    "build": { "command": "pio", "args": ["run", "-t", "upload"] }
  },
  { "label": "Attach to running firmware", "adapter": "gdb-openocd" }
]
```

## Quick start: embedded Rust (probe-rs)

1. Copy [`templates/rust-embedded/debug.json`](templates/rust-embedded/debug.json)
   to `.zed/debug.json` in your firmware project.
2. Set `program` to your ELF (`target/<target triple>/debug/<crate name>`) and,
   if needed, `chip`.
3. Run `debugger: start` (F4) and pick **Zembed: build, flash and debug**.

For build / flash / RTT tasks, copy
[`templates/rust-embedded/tasks.json`](templates/rust-embedded/tasks.json) to
`.zed/tasks.json` and run them with `task: spawn`.

## Install

Zembed is not in the Zed extension registry yet, so install it as a dev
extension. You need Rust with the WebAssembly target:
`rustup target add wasm32-wasip2`.

1. Clone this repository.
2. In Zed, run `zed: install dev extension` and pick the cloned folder.

For the `probe-rs` adapter, also install
[probe-rs](https://probe.rs/docs/getting-started/installation/).

## Generate the configuration: `zembed init`

Zed does not let an extension write files in your project, so Zembed comes
with a small command that writes `.zed/debug.json`, `.zed/tasks.json` and
`.zed/settings.json` for you. It needs Python 3, which PlatformIO already
brings. Install it once:

```sh
ln -s "$PWD/cli/zembed" ~/.local/bin/zembed   # from the cloned repository
```

Then, in a firmware project (PlatformIO or embedded Rust):

```sh
zembed init            # asks a few questions
zembed init --yes      # takes the defaults, asks nothing
zembed doctor          # checks the project, the tools and the probe
```

`zembed init` reads `platformio.ini` (or `Cargo.toml`), picks the debug adapter
for the chip (`gdb-openocd` for ESP32, `probe-rs` otherwise), and asks only
what it cannot guess: the environment when there are several, the tasks to
add, the debug sessions to add, and whether to add `build_type = debug` to
`platformio.ini`. It never replaces an existing file without asking (or
without `--force`), and keeps the old one as `.bak`.

Useful options: `--lang fr` (labels and comments in French; the default
follows `$LANG`), `--env <name>`, `--tasks build,upload,monitor` (among
`build`, `upload`, `upload_monitor`, `monitor`, `clean`, `compiledb`, `test`,
`free_probe`; empty for no `tasks.json`), `--label "XIAO ESP32S3"`,
`--chip <probe-rs chip>`, `--no-debug`, `--no-attach`, `--no-clangd`,
`--debug-build`, `--dry-run` (show, write nothing), `--force`. See
`zembed init --help`.

## Troubleshooting

- **"OpenOCD could not reach the chip"**: the board is unplugged, or another
  program holds the probe: a debug session still open, a `probe-rs` or
  `openocd` process left behind, the Arduino IDE debugger. Stop it, for example
  with `pkill -f "probe-rs dap-server"; pkill openocd`, and try again.
- **"no usable ... gdb found"**: the message lists the GDBs found and why each
  was rejected. Zembed needs GDB 14 or newer with Python, which older
  PlatformIO packages do not have. Install the Arduino "esp32" package 3.x, or
  set `gdb` to a suitable GDB.
- **Tasks open an empty terminal, or `pio` is not found**: Zed runs tasks with
  `bash -i -c "<command>"`. If your `~/.bashrc` ends with `exec zsh` (or
  another shell), the command never runs. Make it conditional:
  `[ -z "$BASH_EXECUTION_STRING" ] && exec zsh`, then restart Zed.
- **A breakpoint is never hit**: build with `build_type = debug`, and make sure
  the firmware on the chip is the one you built (use **Upload and debug**).

## Configuration

### `gdb-openocd`

Every key is optional for an ESP32 with built-in USB-JTAG in a PlatformIO
project. Set them to override what Zembed detects, or to debug other chips.

| Key | Description |
| --- | --- |
| `chip` | Espressif chip, e.g. `esp32s3`. Detected from the PlatformIO `board`. |
| `program` | Firmware ELF, absolute or relative to `cwd`. Defaults to `.pio/build/<env>/firmware.elf`. |
| `pioEnv` | PlatformIO environment. Defaults to `default_envs`, otherwise the first `[env:...]`. |
| `gdb` | GDB 14+ with Python: a path (`~` is expanded) or a name on your `PATH`. |
| `openocd` | OpenOCD to start for the session: a path or a name on your `PATH`. |
| `openocdArgs` | OpenOCD arguments selecting the probe and chip. Defaults to `["-f", "board/<chip>-builtin.cfg"]`. |
| `openocdScripts` | OpenOCD scripts directory. Defaults to `share/openocd/scripts` next to OpenOCD's `bin`. |
| `target` | A GDB server you started yourself, e.g. `localhost:3333`, instead of `openocd`. |
| `checkProbe` | Check that the probe and chip answer before the session. Defaults to `true`. |
| `resumeOnDisconnect` | Resume the firmware when the session ends. Defaults to `true`. |
| `cwd` | Base for a relative `program`. Defaults to the project root. |

Where Zembed looks for the tools, newest first: the Arduino IDE
(`~/.arduino15/packages/esp32/tools`, `~/Library/Arduino15/...` on macOS),
ESP-IDF (`~/.espressif/tools`), PlatformIO (`~/.platformio/packages`), and your
`PATH`. The board is read from the project's `boards/` folder or PlatformIO's
board definitions, else guessed from its name.

For a chip without built-in USB-JTAG (ESP32, ESP32-S2, or any non-Espressif
chip), name the probe:

```json
{
  "label": "Debug (ESP-Prog)",
  "adapter": "gdb-openocd",
  "openocdArgs": ["-f", "board/esp32-wrover-kit-3.3v.cfg"]
}
```

How it works: Zembed starts `gdb -i=dap`, GDB's own Debug Adapter Protocol
mode, and GDB runs `target remote | openocd ... -c "gdb_port pipe"`, so
OpenOCD starts and stops with the session. Two workarounds are applied:

- GDB's DAP mode (seen in 17.1) marks the target as running whenever a new
  thread appears, which breaks every request once OpenOCD lists the RTOS
  tasks. Zembed patches that handler at startup.
- OpenOCD leaves the chip halted when GDB detaches. Zembed resumes every core.

### `probe-rs`

A minimal `.zed/debug.json` entry:

```json
[
  {
    "label": "Debug firmware",
    "adapter": "probe-rs",
    "request": "launch",
    "build": { "command": "cargo", "args": ["build"] },
    "chip": "STM32F411CEUx",
    "program": "target/thumbv7em-none-eabihf/debug/my-firmware"
  }
]
```

| Key | Description |
| --- | --- |
| `request` | `launch` flashes, resets and debugs. `attach` connects to the running chip without flashing or resetting. |
| `chip` | probe-rs chip name (`probe-rs chip list`). Optional when it can be detected from the project. |
| `program` | Firmware ELF, absolute or relative to `cwd`. Optional in a PlatformIO project. |
| `pioEnv` | PlatformIO environment to debug, when `program` is not set. Defaults to the project's default environment. |
| `svdFile` | CMSIS-SVD file, absolute or relative to `cwd`. |
| `rttEnabled`, `rttChannelFormats` | RTT settings (see the limits below). |
| `cwd` | Base for relative paths. Defaults to the worktree root. |
| `probe` | `VID:PID[:Serial]`, when several probes are plugged in. |
| `speed`, `wireProtocol`, `connectUnderReset` | Probe connection options. |
| `flashingConfig` | Flashing options for `launch`. Defaults to `{ "flashingEnabled": true }`. |
| `coreConfigs` | Native probe-rs per-core configuration (multi-core targets). Wins over the shortcuts above. |

`program`, `pioEnv`, `svdFile`, `rttEnabled` and `rttChannelFormats` are Zembed
shortcuts for `coreConfigs[0]`. Every other key is passed to probe-rs unchanged, so any
option from the
[probe-rs debug configuration](https://probe.rs/docs/tools/debugger/) works.

If `probe-rs` is not on your `PATH`, point Zed to it in `settings.json`:

```json
{ "dap": { "probe-rs": { "binary": "/path/to/probe-rs" } } }
```

Zembed starts `probe-rs dap-server --port <free port> --single-session` and Zed connects to it
over TCP. To pin the port, add `"tcp_connection": { "port": 50000 }` to the
entry.

## Known limits

- **Tested on one board.** A Seeed XIAO ESP32S3 over its built-in USB-JTAG,
  on Linux, with Espressif GDB 17.1, Espressif OpenOCD 0.12 and probe-rs 0.32.
  Please open an issue with your board and tool versions if something fails.
- **`gdb-openocd`: attach only, Linux and macOS only.** There is no
  reset-and-halt at startup yet, and the tool discovery and OpenOCD command
  use a POSIX shell, so Windows is not supported.
- **Tools are not downloaded.** GDB, OpenOCD and probe-rs must be installed.
- **`probe-rs`: no line breakpoints in C++ code** (probe-rs 0.32). probe-rs
  only accepts a source breakpoint after the function prologue, and it detects
  the prologue of GCC-compiled code for C only, so every line of a `.cpp` file
  is refused with "No valid breakpoint information found". Use `gdb-openocd`
  for Arduino and other C++ firmware.
- **`probe-rs`, dual-core ESP32: one core per session**, core 0 by default,
  while Arduino runs `loop()` on core 1. Add
  `"coreConfigs": [{ "coreIndex": 1 }]` to follow core 1.
- **No RTT output in the `probe-rs` session.** probe-rs sends RTT data through
  custom DAP events that Zed ignores. Use the RTT tasks of the Rust template.
- **No peripheral (SVD) view.** Zed extensions cannot add custom panels.
- **Tasks are templates.** Zed only lets an extension ship tasks for a language
  it defines, so they have to be copied into the project.
- **PlatformIO's own debugger (`pio debug`) is not used.** It drives GDB
  through GDB/MI and writes its own messages into that stream, so it cannot be
  plugged into Zed. `debug_tool` and other `debug_*` options of
  `platformio.ini` are ignored.

## Roadmap

- Publish to the Zed extension registry, so no Rust toolchain is needed.
- Download GDB and OpenOCD automatically when they are missing.
- More boards: ESP32-C3/C6, STM32, nRF52, RP2040.
- `gdb-openocd`: launch with reset and halt, Windows.
- RTT inside the debug console.

## Development

```sh
rustup target add wasm32-wasip2
cargo test                                      # configuration logic
cargo build --release --target wasm32-wasip2    # what Zed builds
python3 -m unittest discover -s cli             # the zembed command
```

Layout:

```
extension.toml               manifest: debug adapters, PlatformIO locator
src/lib.rs                   routes Zed's calls to an adapter by name
src/locator.rs               turns `pio run` tasks into debug sessions
src/adapters/probe_rs.rs     probe-rs adapter
src/adapters/gdb_openocd.rs  GDB (DAP mode) + OpenOCD adapter
src/esp.rs                   Espressif chips: tool discovery, probe check
src/paths.rs                 path helpers shared by the adapters
src/platformio.rs            platformio.ini helpers (environment, firmware path)
debug_adapter_schemas/       JSON schema per adapter (completion in debug.json)
templates/                   files to copy into a project's .zed/ folder
cli/zembed                   `zembed init` / `zembed doctor` (Python 3, no dependencies)
```

To add an adapter, create a module in `src/adapters/`, declare it in
`extension.toml`, add its schema, and route its name in `src/lib.rs`.

## License

[MIT](LICENSE)
