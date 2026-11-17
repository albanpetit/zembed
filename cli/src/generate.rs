//! Builds the content of `.zed/debug.json`, `tasks.json` and `settings.json`
//! from what is known about the project. No disk access here.

use crate::jsonc;
use crate::project::{PioEnv, RustProject};
use crate::text::Text;
use serde_json::{json, Map, Value};

/// key, label, `pio` arguments, opens its own terminal.
pub const PIO_TASKS: &[(&str, &str, &str, bool)] = &[
    ("build", "build", "run", false),
    ("upload", "upload", "run -t upload", false),
    (
        "upload_monitor",
        "upload + monitor",
        "run -t upload -t monitor",
        true,
    ),
    ("monitor", "monitor", "device monitor", true),
    ("clean", "clean", "run -t clean", false),
    ("compiledb", "refresh compiledb", "run -t compiledb", false),
    ("test", "test", "test", false),
];
pub const ALL_TASKS: &[&str] = &[
    "build",
    "upload",
    "upload_monitor",
    "monitor",
    "clean",
    "compiledb",
    "test",
    "free_probe",
];
pub const DEFAULT_TASKS: &[&str] = &[
    "build",
    "upload",
    "upload_monitor",
    "monitor",
    "clean",
    "compiledb",
    "free_probe",
];

/// Stops what can hold a probe. The brackets keep pkill from matching the
/// task's own shell, whose command line contains these words.
const FREE_PROBE: &str =
    "pkill -f 'probe-rs [d]ap-server'; pkill -f 'openocd.*gdb_port [p]ipe'; echo ok";

/// What goes in the files.
pub struct Choices {
    pub debug: bool,
    pub upload_debug: bool,
    pub attach: bool,
    pub tasks: Vec<String>,
    pub clangd: bool,
    pub label: Option<String>,
    /// probe-rs chip (non-Espressif boards, Rust without a detected chip).
    pub chip: Option<String>,
    pub program: Option<String>,
}

pub type Files = Vec<(&'static str, String)>;

/// ESP32-S3, ESP32-C3...
pub fn chip_display(name: &str) -> String {
    match name.strip_prefix("esp32") {
        Some("") | None => name.to_uppercase(),
        Some(rest) => format!("ESP32-{}", rest.to_uppercase()),
    }
}

fn object(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<Map<_, _>>(),
    )
}

pub fn platformio(
    env: &PioEnv,
    default_env: &str,
    pio: &str,
    choices: &Choices,
    t: &Text,
) -> Files {
    let mut files = Files::new();
    let is_default = env.name == default_env;
    let env_args: Vec<Value> = if is_default {
        vec![]
    } else {
        vec!["-e".into(), env.name.clone().into()]
    };
    let label = choices
        .label
        .clone()
        .or_else(|| env.board_name.clone())
        .or_else(|| env.board.clone())
        .unwrap_or_else(|| env.name.clone());

    if choices.debug {
        let mut entries: Vec<(&str, Value)> = Vec::new();
        let header: Vec<String>;
        if let Some(chip) = &env.chip {
            // gdb-openocd: everything else is detected by the extension.
            let mut extra: Vec<(&str, Value)> = Vec::new();
            if !is_default {
                extra.push(("pioEnv", env.name.clone().into()));
            }
            if chip.builtin_jtag_config.is_none() {
                extra.push((
                    "openocdArgs",
                    json!(["-f", "board/esp32-wrover-kit-3.3v.cfg"]),
                ));
            }
            if choices.upload_debug {
                let mut args = vec![Value::from("run"), "-t".into(), "upload".into()];
                args.extend(env_args.clone());
                let mut pairs = vec![
                    ("label", Value::from(format!("{label}: {}", t.upload_debug))),
                    ("adapter", "gdb-openocd".into()),
                    ("build", json!({ "command": pio, "args": args })),
                ];
                pairs.extend(extra.clone());
                entries.push((t.upload_debug_note, object(pairs)));
            }
            if choices.attach {
                let mut pairs = vec![
                    ("label", Value::from(format!("{label}: {}", t.attach))),
                    ("adapter", "gdb-openocd".into()),
                ];
                pairs.extend(extra);
                entries.push((t.attach_note, object(pairs)));
            }
            header = vec![
                t.debug_header.to_string(),
                if chip.builtin_jtag_config.is_some() {
                    t.esp_probe.replace("{chip}", &chip_display(&chip.name))
                } else {
                    t.external_probe.to_string()
                },
            ];
        } else {
            // probe-rs, which needs the chip's probe-rs name.
            let mut common: Vec<(&str, Value)> = vec![(
                "chip",
                choices.chip.clone().unwrap_or_else(|| "CHIP".into()).into(),
            )];
            if !is_default {
                common.push(("pioEnv", env.name.clone().into()));
            }
            if choices.upload_debug {
                let mut args = vec![Value::from("debug")];
                args.extend(env_args.clone());
                let mut pairs = vec![
                    ("label", Value::from(format!("{label}: {}", t.build_debug))),
                    ("adapter", "probe-rs".into()),
                    ("request", "launch".into()),
                    ("build", json!({ "command": pio, "args": args })),
                ];
                pairs.extend(common.clone());
                pairs.push((
                    "flashingConfig",
                    json!({ "flashingEnabled": true, "haltAfterReset": false }),
                ));
                entries.push((t.upload_debug_note, object(pairs)));
            }
            if choices.attach {
                let mut pairs = vec![
                    ("label", Value::from(format!("{label}: {}", t.attach))),
                    ("adapter", "probe-rs".into()),
                    ("request", "attach".into()),
                ];
                pairs.extend(common);
                entries.push((t.attach_note, object(pairs)));
            }
            header = vec![t.debug_header.to_string()];
        }
        if !entries.is_empty() {
            let header: Vec<&str> = header.iter().map(String::as_str).collect();
            files.push(("debug.json", jsonc::array(&header, &entries)));
        }
    }

    if !choices.tasks.is_empty() {
        let env_flag = if is_default {
            String::new()
        } else {
            format!(" -e {}", env.name)
        };
        let tasks: Vec<(&str, Value)> = choices
            .tasks
            .iter()
            .filter_map(|key| {
                if key == "free_probe" {
                    return Some((
                        t.task("free_probe"),
                        json!({ "label": t.free_probe_label, "command": FREE_PROBE, "reveal": "always" }),
                    ));
                }
                let (_, name, sub, own_terminal) = PIO_TASKS.iter().find(|task| task.0 == key)?;
                let mut pairs = vec![
                    ("label", Value::from(format!("pio: {name}"))),
                    ("command", format!("{pio} {sub}{env_flag}").into()),
                ];
                if *own_terminal {
                    pairs.push(("use_new_terminal", true.into()));
                }
                pairs.push(("reveal", "always".into()));
                Some((t.task(key), object(pairs)))
            })
            .collect();
        files.push(("tasks.json", jsonc::array(&[t.tasks_header], &tasks)));
    }

    if choices.clangd {
        files.push(("settings.json", settings(t)));
    }
    files
}

pub fn rust(project: &RustProject, choices: &Choices, t: &Text) -> Files {
    let mut files = Files::new();
    let name = &project.crate_name;
    let program = choices
        .program
        .clone()
        .or_else(|| project.program.clone())
        .unwrap_or_else(|| "target/<target triple>/debug/<crate>".into());
    let mut common: Vec<(&str, Value)> = vec![("program", program.into())];
    if project.chip.is_none() {
        common.insert(
            0,
            (
                "chip",
                choices.chip.clone().unwrap_or_else(|| "CHIP".into()).into(),
            ),
        );
    }
    if choices.debug {
        let mut entries: Vec<(&str, Value)> = Vec::new();
        if choices.upload_debug {
            let mut pairs = vec![
                ("label", Value::from(format!("{name}: {}", t.build_debug))),
                ("adapter", "probe-rs".into()),
                ("request", "launch".into()),
                ("build", json!({ "command": "cargo", "args": ["build"] })),
            ];
            pairs.extend(common.clone());
            pairs.push((
                "flashingConfig",
                json!({ "flashingEnabled": true, "haltAfterReset": false }),
            ));
            pairs.push(("rttEnabled", true.into()));
            entries.push((t.upload_debug_note, object(pairs)));
        }
        if choices.attach {
            let mut pairs = vec![
                ("label", Value::from(format!("{name}: {}", t.attach))),
                ("adapter", "probe-rs".into()),
                ("request", "attach".into()),
            ];
            pairs.extend(common);
            entries.push((t.attach_note, object(pairs)));
        }
        if !entries.is_empty() {
            files.push(("debug.json", jsonc::array(&[t.debug_header], &entries)));
        }
    }
    if !choices.tasks.is_empty() {
        files.push((
            "tasks.json",
            jsonc::array(
                &[t.tasks_header],
                &[
                    (t.task("build"), json!({ "label": "cargo: build", "command": "cargo build", "reveal": "always" })),
                    (
                        t.task("cargo_run"),
                        json!({ "label": "cargo: run", "command": "cargo run", "use_new_terminal": true, "reveal": "always" }),
                    ),
                    (t.task("free_probe"), json!({ "label": t.free_probe_label, "command": FREE_PROBE, "reveal": "always" })),
                ],
            ),
        ));
    }
    files
}

fn settings(t: &Text) -> String {
    format!(
        "{{\n  // {}\n  \"lsp\": {{\n    \"clangd\": {{\n      \"binary\": {{\n        \
         \"arguments\": [\"--query-driver=**/.platformio/packages/toolchain-*/bin/*\"]\n      }}\n    }}\n  }}\n}}\n",
        t.settings_note
    )
}

/// platformio.ini with `build_type = debug` added at the end of `[env:<env>]`.
pub fn with_debug_build_type(ini: &str, env: &str) -> Option<String> {
    let mut lines: Vec<&str> = ini.split('\n').collect();
    let header = format!("[env:{env}]");
    let start = lines
        .iter()
        .position(|l| l.trim().split(';').next().unwrap_or("").trim() == header)?;
    let mut end = start + 1;
    while end < lines.len() && !lines[end].trim_start().starts_with('[') {
        end += 1;
    }
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    lines.insert(end, "build_type = debug");
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esp;
    use crate::jsonc::parse;
    use crate::text::{EN, FR};

    fn env(name: &str, board: &str) -> PioEnv {
        PioEnv {
            name: name.into(),
            board: Some(board.into()),
            board_name: None,
            chip: esp::chip_from_board_id(board),
            build_type: None,
        }
    }

    fn choices(tasks: &[&str]) -> Choices {
        Choices {
            debug: true,
            upload_debug: true,
            attach: true,
            tasks: tasks.iter().map(|s| s.to_string()).collect(),
            clangd: true,
            label: None,
            chip: None,
            program: None,
        }
    }

    fn file<'a>(files: &'a Files, name: &str) -> &'a str {
        &files.iter().find(|(n, _)| *n == name).unwrap().1
    }

    #[test]
    fn esp32s3_needs_no_tool_path() {
        let files = platformio(
            &env("xiao", "seeed_xiao_esp32s3"),
            "xiao",
            "pio",
            &choices(DEFAULT_TASKS),
            &FR,
        );
        let debug = parse(file(&files, "debug.json"));
        assert_eq!(debug[0]["adapter"], "gdb-openocd");
        assert_eq!(debug[0]["build"]["args"], json!(["run", "-t", "upload"]));
        assert!(debug[0].get("gdb").is_none() && debug[0].get("openocdArgs").is_none());
        assert_eq!(debug[1].as_object().unwrap().len(), 2);
        assert!(file(&files, "debug.json").contains("ESP32-S3, c'est son port USB"));
        let tasks = parse(file(&files, "tasks.json"));
        let labels: Vec<&str> = tasks
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["label"].as_str().unwrap())
            .collect();
        assert!(
            labels.contains(&"pio: upload + monitor")
                && labels.contains(&"Debug: libérer la sonde")
        );
        assert!(file(&files, "settings.json").contains("query-driver"));
    }

    #[test]
    fn other_environment_and_external_probe() {
        let files = platformio(
            &env("b", "esp32dev"),
            "a",
            "pio",
            &choices(&["build", "monitor"]),
            &EN,
        );
        let debug = parse(file(&files, "debug.json"));
        assert_eq!(debug[0]["pioEnv"], "b");
        assert_eq!(
            debug[0]["build"]["args"],
            json!(["run", "-t", "upload", "-e", "b"])
        );
        assert_eq!(debug[0]["openocdArgs"][0], "-f");
        assert!(file(&files, "debug.json").contains("No built-in USB-JTAG"));
        let commands: Vec<String> = parse(file(&files, "tasks.json"))
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["command"].as_str().unwrap().into())
            .collect();
        assert_eq!(commands, ["pio run -e b", "pio device monitor -e b"]);
    }

    #[test]
    fn probe_rs_board() {
        let mut c = choices(&[]);
        c.chip = Some("STM32F411RETx".into());
        c.clangd = false;
        let files = platformio(
            &env("nucleo", "nucleo_f411re"),
            "nucleo",
            "/opt/pio",
            &c,
            &EN,
        );
        assert_eq!(files.len(), 1);
        let debug = parse(file(&files, "debug.json"));
        assert_eq!(debug[0]["adapter"], "probe-rs");
        assert_eq!(debug[0]["chip"], "STM32F411RETx");
        assert_eq!(debug[0]["build"]["command"], "/opt/pio");
        assert_eq!(debug[1]["request"], "attach");
    }

    #[test]
    fn rust_project() {
        let project = RustProject {
            crate_name: "blinky".into(),
            chip: Some("RP2040".into()),
            program: Some("target/thumbv6m-none-eabi/debug/blinky".into()),
        };
        let files = rust(&project, &choices(&["build"]), &EN);
        let debug = parse(file(&files, "debug.json"));
        assert_eq!(
            debug[0]["program"],
            "target/thumbv6m-none-eabi/debug/blinky"
        );
        assert!(
            debug[0].get("chip").is_none(),
            "the extension reads it from the project"
        );
        assert_eq!(parse(file(&files, "tasks.json"))[1]["command"], "cargo run");
    }

    #[test]
    fn adds_debug_build_type() {
        let ini = "[env:a]\nboard = x\n\n[env:b] ; second\nboard = y\n";
        assert_eq!(
            with_debug_build_type(ini, "a").unwrap(),
            "[env:a]\nboard = x\nbuild_type = debug\n\n[env:b] ; second\nboard = y\n"
        );
        assert_eq!(
            with_debug_build_type(ini, "b").unwrap(),
            "[env:a]\nboard = x\n\n[env:b] ; second\nboard = y\nbuild_type = debug\n"
        );
        assert!(with_debug_build_type(ini, "c").is_none());
    }

    #[test]
    fn names_chips() {
        assert_eq!(chip_display("esp32s3"), "ESP32-S3");
        assert_eq!(chip_display("esp32"), "ESP32");
    }
}
