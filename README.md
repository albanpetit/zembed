# Zembed

Embedded development for the [Zed](https://zed.dev) editor.

Zembed is a Zed extension that lets you debug microcontrollers from Zed's
built-in debugger. It provides two debug adapters:

- **`probe-rs`**, for **embedded Rust**, through [probe-rs](https://probe.rs).
- **`gdb-openocd`**, for **C and C++** firmware (Arduino, ESP-IDF, ...),
  through GDB and OpenOCD.

Both understand **PlatformIO** projects.

> **Status: prototype.** The extension compiles, its configuration logic is
> unit-tested, and it has been tried on a single board so far. See
> [Known limits](#known-limits).

## What you get

- A `probe-rs` debug adapter for Zed: breakpoints, stepping, call stack,
  variables, flashing before the session, attaching to a running target.
- A short configuration format for `.zed/debug.json`, with completion and
  validation, on top of the native probe-rs options.
- Chip auto-detection from your project (`Embed.toml`, or the
  `probe-rs run --chip <CHIP>` runner in `.cargo/config.toml`).
- A `gdb-openocd` debug adapter that runs GDB in its DAP mode and lets it
  start OpenOCD for the duration of the session.
- PlatformIO projects: the firmware to debug is found from `platformio.ini`.
- Ready-to-copy [templates](templates) for debug sessions and tasks, for
  [embedded Rust](templates/rust-embedded) (build / flash / RTT) and
  [PlatformIO](templates/platformio) (build / upload / monitor / test).

## Requirements

- A recent Zed, with the built-in debugger and debug adapter extensions.
- For the `probe-rs` adapter: `probe-rs` on your `PATH`
  ([installation](https://probe.rs/docs/getting-started/installation/)) and a
  probe it supports (ST-Link, J-Link, CMSIS-DAP, ...).
- For the `gdb-openocd` adapter: GDB 14 or newer with Python support, built
  for your chip, and an OpenOCD that knows your chip and probe.
- Rust with the `wasm32-wasip2` target, needed by Zed to build the extension
  while it is installed as a dev extension: `rustup target add wasm32-wasip2`.

## Install (dev extension)

Zembed is not published in the Zed extension registry yet.

1. Clone this repository.
2. In Zed, run `zed: install dev extension` from the command palette and pick
   the cloned folder.

## Quick start: embedded Rust

1. Copy [`templates/rust-embedded/debug.json`](templates/rust-embedded/debug.json)
   to `.zed/debug.json` in your firmware project.
2. Set `program` to your ELF (`target/<target triple>/debug/<crate name>`) and,
   if needed, `chip`.
3. Run `debugger: start` (F4) and pick **Zembed: build, flash and debug**.

For build / flash / RTT tasks, copy
[`templates/rust-embedded/tasks.json`](templates/rust-embedded/tasks.json) to
`.zed/tasks.json` and run them with `task: spawn`.

## Quick start: PlatformIO

Tasks work with any board PlatformIO supports:

1. Copy [`templates/platformio/tasks.json`](templates/platformio/tasks.json) to
   `.zed/tasks.json` in your PlatformIO project.
2. Run `task: spawn` and pick **PlatformIO: build**, **upload**, **monitor**...

Run **PlatformIO: generate compile_commands.json** once to give clangd the
project's include paths and defines.

### Debugging C and C++ firmware (Arduino, ESP-IDF)

Use the `gdb-openocd` adapter. It needs:

- A **GDB 14 or newer**, built for your chip and with Python support.
- An **OpenOCD** that knows your chip and probe.

The tools PlatformIO installs are not always suitable: on the machine used for
testing, its ESP32 GDB was version 12 (no DAP mode) and its OpenOCD was a
generic build without ESP32 support. The "esp32" board package of the Arduino
IDE ships both in recent versions, under
`~/.arduino15/packages/esp32/tools/` (`~/Library/Arduino15/...` on macOS).

1. Copy
   [`templates/platformio/debug.esp32-gdb.json`](templates/platformio/debug.esp32-gdb.json)
   to `.zed/debug.json`.
2. Adapt the `gdb` and `openocd` paths to the versions you have, and
   `openocdArgs` to your chip.
3. Add `build_type = debug` to your environment in `platformio.ini`, so the
   firmware is not optimized.
4. Run `debugger: start` (F4) and pick **ESP32-S3: upload and debug (GDB)**.

The session uploads with PlatformIO, attaches, and halts the chip. Set your
breakpoints, then continue. On a dual-core ESP32 both cores are debugged, and
FreeRTOS tasks appear as threads.

If the session starts but shows no thread and never stops, OpenOCD failed to
reach the chip (board unplugged, probe used by another program): GDB's DAP
mode does not report that error. Run the OpenOCD command by hand to see it.

### Debugging through probe-rs

The `probe-rs` adapter also works on PlatformIO firmware, but **it cannot set
line breakpoints in C++ files** (see [Known limits](#known-limits)), so it is
mostly useful for C or Rust code. It needs `probe-rs` installed and a probe it
supports (ST-Link, J-Link, CMSIS-DAP, or the built-in USB-JTAG of
ESP32-C3/C6/S3).

1. Copy [`templates/platformio/debug.json`](templates/platformio/debug.json) to
   `.zed/debug.json`.
2. Set `chip` to your microcontroller as probe-rs names it. PlatformIO board
   names differ: `nucleo_f411re` is `STM32F411RETx`. Search with
   `probe-rs chip list | grep -i f411`.
3. Run `debugger: start` (F4) and pick **PlatformIO: build, flash and debug**.

Both adapters debug `.pio/build/<environment>/firmware.elf`. The environment
is `pioEnv` if you set it, otherwise `default_envs`, otherwise the first
`[env:...]` of `platformio.ini`.

## Configuration: `probe-rs`

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

## Configuration: `gdb-openocd`

```json
[
  {
    "label": "Debug firmware (GDB)",
    "adapter": "gdb-openocd",
    "request": "attach",
    "gdb": "~/.arduino15/packages/esp32/tools/xtensa-esp-elf-gdb/17.1_20260402/bin/xtensa-esp32s3-elf-gdb",
    "openocd": "~/.arduino15/packages/esp32/tools/openocd-esp32/v0.12.0-esp32-20260424/bin/openocd",
    "openocdArgs": ["-f", "board/esp32s3-builtin.cfg"]
  }
]
```

| Key | Description |
| --- | --- |
| `request` | Only `attach`: connects to the firmware on the chip and halts it. Upload first, for example with a `build` task. |
| `gdb` | GDB 14+ with Python, for your chip: a path (`~` is expanded) or a name on your `PATH`. |
| `openocd` | OpenOCD to start for the session: a path or a name on your `PATH`. Needs `openocdArgs`. |
| `openocdArgs` | OpenOCD arguments selecting the probe and chip, e.g. `["-f", "board/esp32s3-builtin.cfg"]`. |
| `openocdScripts` | OpenOCD scripts directory. Defaults to `share/openocd/scripts` next to OpenOCD's `bin`. |
| `resumeOnDisconnect` | Resume the firmware when the session ends. Defaults to `true`. |
| `target` | A GDB server that is already running, e.g. `localhost:3333`, instead of `openocd`. |
| `program` | Firmware ELF, absolute or relative to `cwd`. Optional in a PlatformIO project. |
| `pioEnv` | PlatformIO environment to debug, when `program` is not set. |
| `cwd` | Base for a relative `program`. Defaults to the worktree root. |

Zembed starts `gdb -q -i=dap` and Zed talks to it on stdin/stdout. GDB then
runs `target remote | openocd ... -c "gdb_port pipe"`, so OpenOCD is a child of
GDB and stops with the session. Two workarounds are applied for you:

- GDB's DAP mode (seen in 17.1) marks the target as running whenever a new
  thread appears, which breaks every request once OpenOCD lists the RTOS
  tasks. Zembed patches that handler at startup with a `-iex` Python command.
- OpenOCD leaves the chip halted when GDB detaches. Zembed registers a
  `gdb-detach` handler that resumes every core.

## Known limits

- **Lightly tested on hardware.** Everything was tried on one board, a Seeed
  XIAO ESP32S3 over its built-in USB-JTAG, on Linux, with probe-rs 0.32,
  Espressif GDB 17.1 and Espressif OpenOCD 0.12. Please open an issue with
  your chip, probe and tool versions if a session fails to start.
- **`gdb-openocd`: not validated end to end yet.** On the board, GDB's DAP
  mode with Espressif's OpenOCD gave working C++ line breakpoints, call
  stacks, variables and repeated continues, with OpenOCD started separately.
  Letting GDB start OpenOCD through a pipe, which is what the adapter does,
  has not been run on the board yet, nor has a full session from Zed.
- **`gdb-openocd`: GDB and OpenOCD paths are set by hand.** A Zed extension
  cannot search your disk for installed toolchains.
- **`gdb-openocd`: attach only, Linux and macOS only.** There is no
  reset-and-halt at startup yet, and the OpenOCD command is quoted for a POSIX
  shell, so Windows is not supported.
- **No RTT output in the debug session.** probe-rs sends RTT data through
  custom DAP events designed for its VS Code extension, and Zed ignores them.
  Use the `Zembed: flash and run (RTT logs)` or `Zembed: attach RTT logs`
  tasks to read RTT / defmt logs in a terminal. A probe is used by one process
  at a time, so this cannot run alongside a debug session.
- **PlatformIO debugging does not use `pio debug`'s own debugger.**
  PlatformIO drives GDB through GDB/MI and writes its own messages into that
  stream, so it cannot be plugged into Zed as a debug adapter. Zembed debugs
  the ELF PlatformIO builds with its own adapters instead, which means
  `debug_tool` and other `debug_*` options of `platformio.ini` are ignored.
- **`chip` is not derived from the PlatformIO `board`.** It has to be set.
- **`probe-rs`: no line breakpoints in C++ code** (tested with probe-rs 0.32 on an ESP32-S3
  running Arduino). probe-rs only accepts a source breakpoint after the
  function prologue, and it detects the prologue of GCC-compiled code for C
  only, so every line of a `.cpp` file is refused with "No valid breakpoint
  information found". Breakpoints in `.c` files work. Use the `gdb-openocd`
  adapter for Arduino and other C++ firmware.
- **`probe-rs`, dual-core ESP32: one core per session.** probe-rs debugs a single core,
  core 0 by default, while Arduino runs `setup()` and `loop()` on core 1. Add
  `"coreConfigs": [{ "coreIndex": 1 }]` to follow core 1.
- **No peripheral (SVD) view.** Zed extensions cannot add custom panels.
  `svdFile` is still passed to probe-rs, which exposes peripheral registers as
  a variable scope when the client displays it.
- **Tasks are templates.** Zed only lets an extension ship tasks for a language
  it defines, so Zembed's tasks have to be copied into the project.
- **probe-rs is not downloaded automatically.** It must be installed.

## Roadmap

- Validate on real boards (STM32, nRF52, RP2040, ESP32-C3).
- Debug locator: turn `cargo run` / `cargo build` tasks into debug sessions
  without writing `program` by hand.
- RTT inside the debug console.
- `gdb-openocd`: launch with reset and halt, Windows, STM32 and other chips.
- PlatformIO: derive `chip` and the OpenOCD configuration from the board.

## Development

```sh
rustup target add wasm32-wasip2
cargo test                                      # configuration logic
cargo build --release --target wasm32-wasip2    # what Zed builds
```

Layout:

```
extension.toml               manifest; one [debug_adapters.<name>] per adapter
src/lib.rs                   routes Zed's calls to an adapter by name
src/adapters/probe_rs.rs     probe-rs adapter
src/adapters/gdb_openocd.rs  GDB (DAP mode) + OpenOCD adapter
src/paths.rs                 path helpers shared by the adapters
src/platformio.rs            platformio.ini helpers (environment, firmware path)
debug_adapter_schemas/       JSON schema per adapter (completion in debug.json)
templates/                   files to copy into a project's .zed/ folder
```

To add an adapter, create a module in `src/adapters/`, declare it in
`extension.toml`, add its schema, and route its name in `src/lib.rs`.

## License

[MIT](LICENSE)
