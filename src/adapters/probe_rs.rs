//! Debug adapter backed by `probe-rs dap-server`.
//!
//! probe-rs ships its own DAP server, so this module only has to:
//! 1. find the `probe-rs` binary,
//! 2. start it listening on a TCP port Zed connects to,
//! 3. translate the user's `.zed/debug.json` entry into the configuration
//!    probe-rs expects (see `normalize_config`).

use zed_extension_api::{
    resolve_tcp_template,
    serde_json::{self, json, Map, Value},
    DebugAdapterBinary, DebugConfig, DebugRequest, DebugScenario, DebugTaskDefinition,
    StartDebuggingRequestArguments, StartDebuggingRequestArgumentsRequest,
    TcpArgumentsTemplate, Worktree,
};

pub const ADAPTER_NAME: &str = "probe-rs";

/// Keys Zembed accepts as shortcuts and that probe-rs itself does not know.
const SHORTCUT_KEYS: &[&str] = &["program", "svdFile", "rttEnabled", "rttChannelFormats"];

const NOT_FOUND: &str = "`probe-rs` was not found on your PATH. Install it \
(https://probe.rs/docs/getting-started/installation/) or set its path in Zed settings: \
{\"dap\": {\"probe-rs\": {\"binary\": \"/path/to/probe-rs\"}}}";

pub fn get_dap_binary(
    task: DebugTaskDefinition,
    user_provided_path: Option<String>,
    worktree: &Worktree,
) -> Result<DebugAdapterBinary, String> {
    let command = user_provided_path
        .or_else(|| worktree.which("probe-rs"))
        .ok_or_else(|| NOT_FOUND.to_string())?;

    let root = worktree.root_path();
    let raw: Value = serde_json::from_str(&task.config)
        .map_err(|e| format!("invalid probe-rs debug configuration: {e}"))?;
    let request = request_kind(&raw)?;

    let detect_chip = || {
        ["Embed.toml", ".embed.toml", ".cargo/config.toml", ".cargo/config"]
            .iter()
            .filter_map(|file| worktree.read_text_file(file).ok())
            .find_map(|text| chip_from_text(&text))
    };
    let configuration = normalize_config(raw, &root, detect_chip)?;

    // probe-rs speaks DAP over TCP; let Zed pick a free port unless the user
    // pinned one with `tcp_connection` in debug.json.
    let connection = resolve_tcp_template(task.tcp_connection.unwrap_or(TcpArgumentsTemplate {
        port: None,
        host: None,
        timeout: None,
    }))?;

    Ok(DebugAdapterBinary {
        command: Some(command),
        arguments: vec![
            "dap-server".into(),
            "--port".into(),
            connection.port.to_string(),
        ],
        envs: Vec::new(),
        cwd: Some(root),
        connection: Some(connection),
        request_args: StartDebuggingRequestArguments {
            configuration: configuration.to_string(),
            request,
        },
    })
}

pub fn request_kind(config: &Value) -> Result<StartDebuggingRequestArgumentsRequest, String> {
    match config.get("request").and_then(Value::as_str) {
        Some("launch") => Ok(StartDebuggingRequestArgumentsRequest::Launch),
        Some("attach") => Ok(StartDebuggingRequestArgumentsRequest::Attach),
        Some(other) => Err(format!(
            "unsupported `request`: `{other}` (expected `launch` or `attach`)"
        )),
        None => Err("missing `request` (expected `launch` or `attach`)".into()),
    }
}

/// Used by Zed's "new debug session" modal, which only knows a program path.
/// The chip is filled in later by `get_dap_binary` from the project files.
pub fn config_to_scenario(config: DebugConfig) -> Result<DebugScenario, String> {
    let DebugRequest::Launch(launch) = config.request else {
        return Err(
            "probe-rs cannot attach to a process id. Add an `attach` entry with `chip` and \
             `program` to .zed/debug.json instead."
                .into(),
        );
    };
    let mut scenario = json!({
        "request": "launch",
        "program": launch.program,
    });
    if let Some(cwd) = launch.cwd {
        scenario["cwd"] = cwd.into();
    }
    Ok(DebugScenario {
        label: config.label,
        adapter: config.adapter,
        build: None,
        config: scenario.to_string(),
        tcp_connection: None,
    })
}

/// Turns a `.zed/debug.json` entry into a probe-rs DAP configuration.
///
/// Native probe-rs keys pass through untouched. On top of that Zembed accepts
/// top-level shortcuts (`program`, `svdFile`, `rttEnabled`,
/// `rttChannelFormats`) that are folded into `coreConfigs[0]`, defaults `cwd`
/// to the worktree root, enables flashing for `launch`, and detects the chip
/// from the project when it is not given.
pub fn normalize_config(
    raw: Value,
    root: &str,
    detect_chip: impl FnOnce() -> Option<String>,
) -> Result<Value, String> {
    let Value::Object(mut config) = raw else {
        return Err("probe-rs debug configuration must be a JSON object".into());
    };
    let is_launch = config.get("request").and_then(Value::as_str) == Some("launch");

    let cwd = match config.get("cwd").and_then(Value::as_str) {
        Some(cwd) => resolve_path(cwd, root),
        None => root.to_string(),
    };
    config.insert("cwd".into(), cwd.clone().into());

    if !config.get("chip").is_some_and(Value::is_string) {
        let chip = detect_chip().ok_or(
            "no `chip` in the debug configuration, and none could be detected from \
             Embed.toml or the `probe-rs run --chip <CHIP>` runner in .cargo/config.toml. \
             Add `\"chip\": \"<CHIP>\"` to the entry in .zed/debug.json \
             (see `probe-rs chip list`).",
        )?;
        config.insert("chip".into(), chip.into());
    }

    // Fold the shortcuts into coreConfigs[0]; explicit coreConfigs values win.
    let mut shortcuts = Map::new();
    for key in SHORTCUT_KEYS {
        if let Some(value) = config.remove(*key) {
            let key = if *key == "program" { "programBinary" } else { key };
            shortcuts.insert(key.to_string(), value);
        }
    }
    let mut cores = match config.remove("coreConfigs") {
        Some(Value::Array(cores)) if !cores.is_empty() => cores,
        Some(Value::Array(_)) | None => vec![json!({ "coreIndex": 0 })],
        Some(_) => return Err("`coreConfigs` must be an array".into()),
    };
    if let Some(Value::Object(first)) = cores.first_mut() {
        for (key, value) in shortcuts {
            first.entry(key).or_insert(value);
        }
    }
    for core in &mut cores {
        for key in ["programBinary", "svdFile"] {
            if let Some(path) = core.get(key).and_then(Value::as_str) {
                core[key] = resolve_path(path, &cwd).into();
            }
        }
    }
    if !cores
        .iter()
        .any(|core| core.get("programBinary").is_some_and(Value::is_string))
    {
        return Err(
            "no program to debug: set `program` to the path of your firmware ELF \
             (for example target/thumbv7em-none-eabihf/debug/<crate-name>)"
                .into(),
        );
    }
    config.insert("coreConfigs".into(), Value::Array(cores));

    // A launch without flashing would debug whatever is already on the chip.
    if is_launch && !config.contains_key("flashingConfig") {
        config.insert("flashingConfig".into(), json!({ "flashingEnabled": true }));
    }

    Ok(Value::Object(config))
}

/// Finds a chip name in an `Embed.toml` (`chip = "X"`) or a Cargo config
/// (`runner = "probe-rs run --chip X"`, `PROBE_RS_CHIP = "X"`). This is a
/// line-based scan on purpose: it avoids a TOML dependency in the WASM module.
pub fn chip_from_text(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let tokens: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '[' | ']' | '='))
            .filter(|token| !token.is_empty())
            .collect();
        let value = match tokens.as_slice() {
            ["chip" | "PROBE_RS_CHIP", value, ..] => Some(*value),
            _ => tokens
                .iter()
                .position(|token| *token == "--chip")
                .and_then(|i| tokens.get(i + 1).copied()),
        };
        if let Some(value) = value.filter(|v| !v.starts_with('-') && !v.starts_with('#')) {
            return Some(value.to_string());
        }
    }
    None
}

fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

fn resolve_path(path: &str, base: &str) -> String {
    if is_absolute(path) {
        return path.to_string();
    }
    let path = path.strip_prefix("./").unwrap_or(path);
    format!("{}/{}", base.trim_end_matches(['/', '\\']), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chip() {
        let cargo = "[target.thumbv7em-none-eabihf]\nrunner = \"probe-rs run --chip STM32F411CEUx\"\n";
        assert_eq!(chip_from_text(cargo).as_deref(), Some("STM32F411CEUx"));
        let array = "runner = [\"probe-rs\", \"run\", \"--chip=nRF52840_xxAA\"]";
        assert_eq!(chip_from_text(array).as_deref(), Some("nRF52840_xxAA"));
        let embed = "[default.general]\n# chip = \"old\"\nchip = \"RP2040\"\n";
        assert_eq!(chip_from_text(embed).as_deref(), Some("RP2040"));
        assert_eq!(chip_from_text("[env]\nPROBE_RS_CHIP = \"esp32c3\"").as_deref(), Some("esp32c3"));
        assert_eq!(chip_from_text("runner = \"probe-rs run\"\nchips = 3"), None);
    }

    #[test]
    fn normalizes_shortcuts() {
        let raw = json!({
            "request": "launch",
            "program": "target/thumbv7em-none-eabihf/debug/app",
            "svdFile": "/svd/chip.svd",
            "rttEnabled": true,
        });
        let config = normalize_config(raw, "/work", || Some("STM32F411CEUx".into())).unwrap();
        assert_eq!(
            config,
            json!({
                "request": "launch",
                "cwd": "/work",
                "chip": "STM32F411CEUx",
                "flashingConfig": { "flashingEnabled": true },
                "coreConfigs": [{
                    "coreIndex": 0,
                    "programBinary": "/work/target/thumbv7em-none-eabihf/debug/app",
                    "svdFile": "/svd/chip.svd",
                    "rttEnabled": true,
                }],
            })
        );
    }

    #[test]
    fn keeps_native_config_and_never_flashes_on_attach() {
        let raw = json!({
            "request": "attach",
            "chip": "RP2040",
            "program": "ignored.elf",
            "coreConfigs": [{ "coreIndex": 0, "programBinary": "C:\\fw\\app.elf" }],
        });
        let config = normalize_config(raw, "/work", || panic!("chip was given")).unwrap();
        assert_eq!(config["coreConfigs"][0]["programBinary"], "C:\\fw\\app.elf");
        assert!(config.get("flashingConfig").is_none());
        assert!(config.get("program").is_none());
    }

    #[test]
    fn reports_missing_chip_and_program() {
        let no_chip = json!({ "request": "launch", "program": "a.elf" });
        assert!(normalize_config(no_chip, "/w", || None).unwrap_err().contains("chip"));
        let no_program = json!({ "request": "launch", "chip": "RP2040" });
        assert!(normalize_config(no_program, "/w", || None).unwrap_err().contains("program"));
    }

    #[test]
    fn reads_request_kind() {
        assert!(request_kind(&json!({ "request": "attach" })).is_ok());
        assert!(request_kind(&json!({ "request": "run" })).is_err());
        assert!(request_kind(&json!({})).is_err());
    }
}
