//! One module per debug adapter.
//!
//! To add an adapter: create a module here, declare it in
//! `extension.toml` under `[debug_adapters.<name>]`, add its JSON schema in
//! `debug_adapter_schemas/`, and route its name in `lib.rs`.

pub mod gdb_openocd;
pub mod probe_rs;
