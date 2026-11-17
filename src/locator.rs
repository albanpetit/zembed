//! Debug locator: turns PlatformIO tasks into debug sessions, so a project
//! needs no `.zed/debug.json`.
//!
//! When a C or C++ file is open, Zed offers every task to the locators. A
//! `pio run` task (with or without `-t upload`) becomes "upload, then debug":
//! the task runs as the build step and the adapter attaches to the freshly
//! flashed firmware. Everything else (monitor, clean, test, ...) is left
//! alone.

use crate::adapters::{gdb_openocd, probe_rs};
use zed_extension_api::{
    serde_json::json, BuildTaskDefinition, BuildTaskDefinitionTemplatePayload, DebugScenario,
    TaskTemplate,
};

pub const LOCATOR_NAME: &str = "platformio";

/// PlatformIO targets that never return or do not flash the firmware. A task
/// using one of them cannot be the build step of a debug session.
const BLOCKING_OR_UNRELATED_TARGETS: &[&str] = &[
    "monitor",
    "clean",
    "cleanall",
    "erase",
    "menuconfig",
    "size",
    "compiledb",
    "envdump",
    "idedata",
    "fullclean",
    "uploadfs",
    "buildfs",
];

pub fn create_scenario(
    task: TaskTemplate,
    resolved_label: String,
    debug_adapter_name: String,
) -> Option<DebugScenario> {
    // The full command line: tasks may give `pio` and its arguments apart,
    // or the whole `pio run -t upload` as the command.
    let mut words: Vec<String> = task.command.split_whitespace().map(String::from).collect();
    words.extend(task.args.iter().cloned());

    let program = words.first()?;
    let program = program.rsplit('/').next().unwrap_or(program);
    if !matches!(program, "pio" | "platformio" | "pio.exe" | "platformio.exe") {
        return None;
    }
    if words.get(1).map(String::as_str) != Some("run") {
        return None;
    }

    let mut env = None;
    let mut targets = Vec::new();
    let mut rest = words.iter().skip(2);
    while let Some(word) = rest.next() {
        match word.as_str() {
            "-t" | "--target" => targets.extend(rest.next().cloned()),
            "-e" | "--environment" => env = rest.next().cloned(),
            other => {
                if let Some(t) = other.strip_prefix("--target=") {
                    targets.push(t.to_string());
                } else if let Some(e) = other.strip_prefix("--environment=") {
                    env = Some(e.to_string());
                }
            }
        }
    }
    if targets
        .iter()
        .any(|t| BLOCKING_OR_UNRELATED_TARGETS.contains(&t.as_str()))
    {
        return None;
    }

    // A plain `pio run` only builds: flash too, or the session would debug
    // whatever firmware is already on the chip.
    let mut build = task.clone();
    let mut label = resolved_label;
    if !targets.iter().any(|t| t == "upload") {
        if task.args.is_empty() {
            build.command.push_str(" -t upload");
        } else {
            build.args.extend(["-t".to_string(), "upload".to_string()]);
        }
        label.push_str(" + upload");
    }

    // Keep the user's choice when it is one of ours (e.g. probe-rs set as
    // the language's debugger), otherwise GDB + OpenOCD, which handles C++.
    let adapter = match debug_adapter_name.as_str() {
        probe_rs::ADAPTER_NAME | gdb_openocd::ADAPTER_NAME => debug_adapter_name,
        _ => gdb_openocd::ADAPTER_NAME.to_string(),
    };

    let mut config = json!({ "request": "attach" });
    if let Some(env) = env {
        config["pioEnv"] = env.into();
    }

    Some(DebugScenario {
        label: format!("Debug: {label}"),
        adapter,
        // No locator name: the configuration is already complete, so Zed
        // runs the task and then starts the session.
        build: Some(BuildTaskDefinition::Template(
            BuildTaskDefinitionTemplatePayload {
                locator_name: None,
                template: build,
            },
        )),
        config: config.to_string(),
        tcp_connection: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zed_extension_api::serde_json::{self, Value};

    fn task(command: &str, args: &[&str]) -> TaskTemplate {
        TaskTemplate {
            label: "task".into(),
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env: vec![],
            cwd: Some("$ZED_WORKTREE_ROOT".into()),
        }
    }

    fn scenario(command: &str, args: &[&str]) -> Option<DebugScenario> {
        create_scenario(task(command, args), "pio".into(), "CodeLLDB".into())
    }

    fn build_task(s: &DebugScenario) -> &TaskTemplate {
        match s.build.as_ref().unwrap() {
            BuildTaskDefinition::Template(p) => {
                assert!(p.locator_name.is_none());
                &p.template
            }
            other => panic!("unexpected build {other:?}"),
        }
    }

    #[test]
    fn upload_task_with_separate_args_is_kept() {
        let s = scenario("pio", &["run", "-t", "upload"]).unwrap();
        assert_eq!(s.adapter, "gdb-openocd");
        assert_eq!(s.label, "Debug: pio");
        let b = build_task(&s);
        assert_eq!(b.command, "pio");
        assert_eq!(b.args, ["run", "-t", "upload"]);
        assert_eq!(b.cwd.as_deref(), Some("$ZED_WORKTREE_ROOT"));
        let config: Value = serde_json::from_str(&s.config).unwrap();
        assert_eq!(config, json!({ "request": "attach" }));
    }

    #[test]
    fn whole_command_line_in_one_string() {
        let s = scenario("/home/u/.local/bin/pio run -t upload -e xiao", &[]).unwrap();
        assert_eq!(
            build_task(&s).command,
            "/home/u/.local/bin/pio run -t upload -e xiao"
        );
        let config: Value = serde_json::from_str(&s.config).unwrap();
        assert_eq!(config["pioEnv"], "xiao");
    }

    #[test]
    fn plain_build_gets_an_upload() {
        let s = scenario("pio run", &[]).unwrap();
        assert_eq!(build_task(&s).command, "pio run -t upload");
        assert_eq!(s.label, "Debug: pio + upload");

        let s = scenario("platformio", &["run", "--environment=dev"]).unwrap();
        assert_eq!(
            build_task(&s).args,
            ["run", "--environment=dev", "-t", "upload"]
        );
        let config: Value = serde_json::from_str(&s.config).unwrap();
        assert_eq!(config["pioEnv"], "dev");
    }

    #[test]
    fn other_tasks_are_ignored() {
        assert!(scenario("pio", &["run", "-t", "upload", "-t", "monitor"]).is_none());
        assert!(scenario("pio run -t clean", &[]).is_none());
        assert!(scenario("pio", &["device", "monitor"]).is_none());
        assert!(scenario("pio", &["test"]).is_none());
        assert!(scenario("cargo", &["run"]).is_none());
        assert!(scenario("make pio", &[]).is_none());
    }

    #[test]
    fn our_adapter_choice_is_kept() {
        let s = create_scenario(
            task("pio run -t upload", &[]),
            "x".into(),
            "probe-rs".into(),
        );
        assert_eq!(s.unwrap().adapter, "probe-rs");
    }
}
