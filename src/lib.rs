//! Zembed: embedded development support for Zed.
//!
//! The extension is a thin dispatcher: every debug adapter lives in its own
//! module under `adapters/` and is selected by the adapter name declared in
//! `extension.toml`.

mod adapters;
mod paths;
mod platformio;

use adapters::{gdb_openocd, probe_rs};
use zed_extension_api::{
    self as zed, serde_json::Value, DebugAdapterBinary, DebugConfig, DebugScenario,
    DebugTaskDefinition, StartDebuggingRequestArgumentsRequest, Worktree,
};

struct Zembed;

impl zed::Extension for Zembed {
    fn new() -> Self {
        Zembed
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: DebugTaskDefinition,
        user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> Result<DebugAdapterBinary, String> {
        match adapter_name.as_str() {
            probe_rs::ADAPTER_NAME => {
                probe_rs::get_dap_binary(config, user_provided_debug_adapter_path, worktree)
            }
            gdb_openocd::ADAPTER_NAME => {
                gdb_openocd::get_dap_binary(config, user_provided_debug_adapter_path, worktree)
            }
            other => Err(unknown_adapter(other)),
        }
    }

    fn dap_request_kind(
        &mut self,
        adapter_name: String,
        config: Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        match adapter_name.as_str() {
            probe_rs::ADAPTER_NAME => probe_rs::request_kind(&config),
            gdb_openocd::ADAPTER_NAME => gdb_openocd::request_kind(&config),
            other => Err(unknown_adapter(other)),
        }
    }

    fn dap_config_to_scenario(&mut self, config: DebugConfig) -> Result<DebugScenario, String> {
        match config.adapter.as_str() {
            probe_rs::ADAPTER_NAME => probe_rs::config_to_scenario(config),
            gdb_openocd::ADAPTER_NAME => gdb_openocd::config_to_scenario(config),
            other => Err(unknown_adapter(other)),
        }
    }
}

fn unknown_adapter(name: &str) -> String {
    format!("Zembed does not provide a debug adapter named `{name}`")
}

zed::register_extension!(Zembed);
