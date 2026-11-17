//! Debug adapter backed by `probe-rs dap-server`.
//!
//! probe-rs ships its own DAP server, so this module only has to:
//! 1. find the `probe-rs` binary,
//! 2. start it listening on a TCP port Zed connects to,
//! 3. translate the user's `.zed/debug.json` entry into the configuration
//!    probe-rs expects (see `normalize_config`).

use crate::paths::resolve_path;
use crate::platformio;
use zed_extension_api::{
    resolve_tcp_template,
    serde_json::{self, json, Map, Value},
    DebugAdapterBinary, DebugConfig, DebugRequest, DebugScenario, DebugTaskDefinition,
    StartDebuggingRequestArguments, StartDebuggingRequestArgumentsRequest, TcpArgumentsTemplate,
    Worktree,
};

pub const ADAPTER_NAME: &str = "probe-rs";

/// Keys Zembed accepts as shortcuts and that probe-rs itself does not know.
const SHORTCUT_KEYS: &[&str] = &["program", "svdFile", "rttEnabled", "rttChannelFormats"];

/// Project files that may name the chip, most specific first.
const CHIP_FILES: &[&str] = &[
    "Embed.toml",
    ".embed.toml",
    ".cargo/config.toml",
    ".cargo/config",
    platformio::PROJECT_FILE,
];

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

    let configuration = normalize_config(raw, &root, |file| worktree.read_text_file(file).ok())?;

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
            // Exit when the session ends. Without it probe-rs keeps listening
            // for another client, and the leftover process holds the probe.
            "--single-session".into(),
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
/// from the project when it is not given. In a PlatformIO project, `program`
/// defaults to the firmware of `pioEnv` (or of the default environment).
///
/// `read_file` reads a project file by its path relative to the worktree root.
pub fn normalize_config(
    raw: Value,
    root: &str,
    read_file: impl Fn(&str) -> Option<String>,
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
        let chip = CHIP_FILES
            .iter()
            .filter_map(|file| read_file(file))
            .find_map(|text| chip_from_text(&text))
            .ok_or(
                "no `chip` in the debug configuration, and none could be detected from \
                 the project (Embed.toml, or a `probe-rs ... --chip <CHIP>` command in \
                 .cargo/config.toml or platformio.ini). Add `\"chip\": \"<CHIP>\"` to the \
                 entry in .zed/debug.json (see `probe-rs chip list`).",
            )?;
        config.insert("chip".into(), chip.into());
    }

    // Fold the shortcuts into coreConfigs[0]; explicit coreConfigs values win.
    let mut shortcuts = Map::new();
    for key in SHORTCUT_KEYS {
        if let Some(value) = config.remove(*key) {
            let key = if *key == "program" {
                "programBinary"
            } else {
                key
            };
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
    // PlatformIO projects: the firmware location follows from the environment.
    let pio_env = match config.remove("pioEnv") {
        Some(Value::String(env)) => Some(env),
        Some(_) => return Err("`pioEnv` must be a string".into()),
        None => None,
    };
    let has_program = |cores: &[Value]| {
        cores
            .iter()
            .any(|core| core.get("programBinary").is_some_and(Value::is_string))
    };
    if !has_program(&cores) {
        let env = pio_env.or_else(|| {
            read_file(platformio::PROJECT_FILE).and_then(|ini| platformio::default_env(&ini))
        });
        if let (Some(env), Some(first)) = (env, cores.first_mut()) {
            first["programBinary"] = platformio::firmware_elf(&env).into();
        }
    }
    if !has_program(&cores) {
        return Err(
            "no program to debug: set `program` to the path of your firmware ELF \
             (for example target/thumbv7em-none-eabihf/debug/<crate-name>), or `pioEnv` \
             to a PlatformIO environment"
                .into(),
        );
    }
    for core in &mut cores {
        for key in ["programBinary", "svdFile"] {
            if let Some(path) = core.get(key).and_then(Value::as_str) {
                core[key] = resolve_path(path, &cwd).into();
            }
        }
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
        if line.starts_with('#') || line.starts_with(';') {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chip() {
        let cargo =
            "[target.thumbv7em-none-eabihf]\nrunner = \"probe-rs run --chip STM32F411CEUx\"\n";
        assert_eq!(chip_from_text(cargo).as_deref(), Some("STM32F411CEUx"));
        let array = "runner = [\"probe-rs\", \"run\", \"--chip=nRF52840_xxAA\"]";
        assert_eq!(chip_from_text(array).as_deref(), Some("nRF52840_xxAA"));
        let embed = "[default.general]\n# chip = \"old\"\nchip = \"RP2040\"\n";
        assert_eq!(chip_from_text(embed).as_deref(), Some("RP2040"));
        assert_eq!(
            chip_from_text("[env]\nPROBE_RS_CHIP = \"esp32c3\"").as_deref(),
            Some("esp32c3")
        );
        assert_eq!(chip_from_text("runner = \"probe-rs run\"\nchips = 3"), None);
        let pio = "; upload_command = probe-rs download --chip old\nupload_command = probe-rs download --chip STM32F103C8 $SOURCE\n";
        assert_eq!(chip_from_text(pio).as_deref(), Some("STM32F103C8"));
    }

    #[test]
    fn normalizes_shortcuts() {
        let raw = json!({
            "request": "launch",
            "program": "target/thumbv7em-none-eabihf/debug/app",
            "svdFile": "/svd/chip.svd",
            "rttEnabled": true,
        });
        let config = normalize_config(raw, "/work", |file| {
            (file == ".cargo/config.toml")
                .then(|| "runner = \"probe-rs run --chip STM32F411CEUx\"".to_string())
        })
        .unwrap();
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
        let config = normalize_config(raw, "/work", |_| panic!("nothing to look up")).unwrap();
        assert_eq!(config["coreConfigs"][0]["programBinary"], "C:\\fw\\app.elf");
        assert!(config.get("flashingConfig").is_none());
        assert!(config.get("program").is_none());
    }

    #[test]
    fn reports_missing_chip_and_program() {
        let no_chip = json!({ "request": "launch", "program": "a.elf" });
        assert!(normalize_config(no_chip, "/w", |_| None)
            .unwrap_err()
            .contains("chip"));
        let no_program = json!({ "request": "launch", "chip": "RP2040" });
        assert!(normalize_config(no_program, "/w", |_| None)
            .unwrap_err()
            .contains("program"));
    }

    #[test]
    fn finds_platformio_firmware() {
        let project = |file: &str| {
            (file == "platformio.ini")
                .then(|| "[env:release]\n[env:debug]\nboard = nucleo_f411re\n".to_string())
        };
        let raw = json!({ "request": "launch", "chip": "STM32F411RETx" });
        let config = normalize_config(raw, "/work", project).unwrap();
        assert_eq!(
            config["coreConfigs"][0]["programBinary"],
            "/work/.pio/build/release/firmware.elf"
        );

        let raw = json!({ "request": "attach", "chip": "STM32F411RETx", "pioEnv": "debug" });
        let config = normalize_config(raw, "/work", project).unwrap();
        assert_eq!(
            config["coreConfigs"][0]["programBinary"],
            "/work/.pio/build/debug/firmware.elf"
        );
        assert!(config.get("pioEnv").is_none());

        // An explicit program always wins over the PlatformIO default.
        let raw =
            json!({ "request": "attach", "chip": "X", "program": "a.elf", "pioEnv": "debug" });
        let config = normalize_config(raw, "/work", project).unwrap();
        assert_eq!(config["coreConfigs"][0]["programBinary"], "/work/a.elf");
    }

    #[test]
    fn reads_request_kind() {
        assert!(request_kind(&json!({ "request": "attach" })).is_ok());
        assert!(request_kind(&json!({ "request": "run" })).is_err());
        assert!(request_kind(&json!({})).is_err());
    }
}
