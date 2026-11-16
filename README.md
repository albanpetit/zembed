# Zembed

Embedded development for the [Zed](https://zed.dev) editor.

Zembed is a Zed extension that lets you debug microcontrollers from Zed's
built-in debugger. This first version targets **embedded Rust** through
[probe-rs](https://probe.rs); PlatformIO support is planned next.

> **Status: prototype.** The extension compiles and its configuration logic is
> unit-tested, but it has not yet been validated against real hardware. See
> [Known limits](#known-limits).

## What you get

- A `probe-rs` debug adapter for Zed: breakpoints, stepping, call stack,
  variables, flashing before the session, attaching to a running target.
- A short configuration format for `.zed/debug.json`, with completion and
  validation, on top of the native probe-rs options.
- Chip auto-detection from your project (`Embed.toml`, or the
  `probe-rs run --chip <CHIP>` runner in `.cargo/config.toml`).
- Ready-to-copy [templates](templates/rust-embedded) for debug sessions and for
  build / flash / RTT tasks.

## Requirements

- A recent Zed, with the built-in debugger and debug adapter extensions.
- `probe-rs` on your `PATH`
  ([installation](https://probe.rs/docs/getting-started/installation/)).
- A debug probe supported by probe-rs (ST-Link, J-Link, CMSIS-DAP, ...).
- Rust with the `wasm32-wasip2` target, needed by Zed to build the extension
  while it is installed as a dev extension: `rustup target add wasm32-wasip2`.

## Install (dev extension)

Zembed is not published in the Zed extension registry yet.

1. Clone this repository.
2. In Zed, run `zed: install dev extension` from the command palette and pick
   the cloned folder.

## Quick start

1. Copy [`templates/rust-embedded/debug.json`](templates/rust-embedded/debug.json)
   to `.zed/debug.json` in your firmware project.
2. Set `program` to your ELF (`target/<target triple>/debug/<crate name>`) and,
   if needed, `chip`.
3. Run `debugger: start` (F4) and pick **Zembed: build, flash and debug**.

For build / flash / RTT tasks, copy
[`templates/rust-embedded/tasks.json`](templates/rust-embedded/tasks.json) to
`.zed/tasks.json` and run them with `task: spawn`.

## Configuration

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
| `program` | Firmware ELF, absolute or relative to `cwd`. |
| `svdFile` | CMSIS-SVD file, absolute or relative to `cwd`. |
| `rttEnabled`, `rttChannelFormats` | RTT settings (see the limits below). |
| `cwd` | Base for relative paths. Defaults to the worktree root. |
| `probe` | `VID:PID[:Serial]`, when several probes are plugged in. |
| `speed`, `wireProtocol`, `connectUnderReset` | Probe connection options. |
| `flashingConfig` | Flashing options for `launch`. Defaults to `{ "flashingEnabled": true }`. |
| `coreConfigs` | Native probe-rs per-core configuration (multi-core targets). Wins over the shortcuts above. |

`program`, `svdFile`, `rttEnabled` and `rttChannelFormats` are Zembed shortcuts
for `coreConfigs[0]`. Every other key is passed to probe-rs unchanged, so any
option from the
[probe-rs debug configuration](https://probe.rs/docs/tools/debugger/) works.

If `probe-rs` is not on your `PATH`, point Zed to it in `settings.json`:

```json
{ "dap": { "probe-rs": { "binary": "/path/to/probe-rs" } } }
```

Zembed starts `probe-rs dap-server --port <free port>` and Zed connects to it
over TCP. To pin the port, add `"tcp_connection": { "port": 50000 }` to the
entry.

## Known limits

- **Not yet tested on hardware.** Please open an issue with your chip, probe
  and probe-rs version if a session fails to start.
- **No RTT output in the debug session.** probe-rs sends RTT data through
  custom DAP events designed for its VS Code extension, and Zed ignores them.
  Use the `Zembed: flash and run (RTT logs)` or `Zembed: attach RTT logs`
  tasks to read RTT / defmt logs in a terminal. A probe is used by one process
  at a time, so this cannot run alongside a debug session.
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
- PlatformIO: build / upload / monitor tasks, then debugging through
  `pio debug` and a GDB-based adapter.

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
debug_adapter_schemas/       JSON schema per adapter (completion in debug.json)
templates/                   files to copy into a project's .zed/ folder
```

To add an adapter, create a module in `src/adapters/`, declare it in
`extension.toml`, add its schema, and route its name in `src/lib.rs`.

## License

[MIT](LICENSE)
