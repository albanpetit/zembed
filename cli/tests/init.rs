//! End-to-end tests: run the `zembed` binary on throwaway projects.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("zembed-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
    fn write(&self, file: &str, text: &str) {
        let path = self.0.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.0.join(file)).unwrap()
    }
    fn json(&self, file: &str) -> Value {
        let raw = self.read(file);
        let text: Vec<&str> = raw
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect();
        serde_json::from_str(&text.join("\n")).unwrap()
    }
    fn exists(&self, file: &str) -> bool {
        self.0.join(file).exists()
    }
    fn zembed(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_zembed"))
            .arg("init")
            .arg(&self.0)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn esp32s3_needs_no_tool_path() {
    let d = Dir::new("s3");
    d.write(
        "platformio.ini",
        "[env:xiao]\nplatform = espressif32\nboard = seeed_xiao_esp32s3\n",
    );
    assert!(d.zembed(&["--yes", "--lang", "fr"]).status.success());
    let debug = d.json(".zed/debug.json");
    assert_eq!(debug[0]["adapter"], "gdb-openocd");
    assert_eq!(
        debug[0]["build"]["args"],
        serde_json::json!(["run", "-t", "upload"])
    );
    assert!(debug[0].get("gdb").is_none());
    let tasks = d.json(".zed/tasks.json");
    assert!(tasks
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["label"] == "Debug: libérer la sonde"));
    assert!(d.read(".zed/settings.json").contains("query-driver"));
    assert!(
        !d.read("platformio.ini").contains("build_type"),
        "--yes never edits platformio.ini"
    );
}

#[test]
fn keeps_existing_files_unless_forced() {
    let d = Dir::new("keep");
    d.write("platformio.ini", "[env:xiao]\nboard = seeed_xiao_esp32s3\n");
    d.write(".zed/tasks.json", "[]\n");
    assert!(stdout(&d.zembed(&["--yes", "--lang", "en"])).contains("--force"));
    assert_eq!(d.read(".zed/tasks.json"), "[]\n");
    d.zembed(&["--yes", "--force"]);
    assert_eq!(d.read(".zed/tasks.json.bak"), "[]\n");
    assert!(d.json(".zed/tasks.json").as_array().unwrap().len() > 3);
    let again = stdout(&d.zembed(&["--yes", "--force", "--lang", "en"]));
    assert_eq!(again.matches("unchanged").count(), 3, "{again}");
}

#[test]
fn other_environment() {
    let d = Dir::new("env");
    d.write("platformio.ini", "[platformio]\ndefault_envs = a\n[env]\nframework = arduino\n[env:a]\nboard = esp32-s3-devkitc-1\n[env:b]\nboard = esp32dev\n");
    d.zembed(&["--yes", "--env", "b", "--tasks", "build,monitor"]);
    let debug = d.json(".zed/debug.json");
    assert_eq!(debug[0]["pioEnv"], "b");
    assert_eq!(debug[0]["openocdArgs"][0], "-f");
    let tasks = d.json(".zed/tasks.json");
    let commands: Vec<&str> = tasks
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["command"].as_str().unwrap())
        .collect();
    assert!(
        commands[0].ends_with(" run -e b") && commands[1].ends_with(" device monitor -e b"),
        "{commands:?}"
    );
}

#[test]
fn probe_rs_board_and_debug_build_type() {
    let d = Dir::new("stm32");
    d.write(
        "platformio.ini",
        "[env:nucleo]\nplatform = ststm32\nboard = nucleo_f411re\n\n",
    );
    d.zembed(&[
        "--yes",
        "--chip",
        "STM32F411RETx",
        "--debug-build",
        "--tasks",
        "",
    ]);
    let debug = d.json(".zed/debug.json");
    assert_eq!(
        (debug[0]["adapter"].as_str(), debug[0]["chip"].as_str()),
        (Some("probe-rs"), Some("STM32F411RETx"))
    );
    assert!(!d.exists(".zed/tasks.json"));
    assert!(d
        .read("platformio.ini")
        .contains("board = nucleo_f411re\nbuild_type = debug\n"));
}

#[test]
fn dry_run_writes_nothing() {
    let d = Dir::new("dry");
    d.write("platformio.ini", "[env:xiao]\nboard = seeed_xiao_esp32s3\n");
    let out = stdout(&d.zembed(&["--yes", "--dry-run", "--debug-build"]));
    assert!(out.contains("\"adapter\": \"gdb-openocd\""));
    assert!(out.contains("build_type = debug"));
    assert!(!d.exists(".zed"));
    assert!(!d.read("platformio.ini").contains("build_type"));
}

#[test]
fn rust_project() {
    let d = Dir::new("rust");
    d.write(
        "Cargo.toml",
        "[package]\nname = \"blinky\"\nversion = \"0.1.0\"\n",
    );
    d.write(".cargo/config.toml", "[build]\ntarget = \"thumbv6m-none-eabi\"\n[target.thumbv6m-none-eabi]\nrunner = \"probe-rs run --chip RP2040\"\n");
    d.zembed(&["--yes", "--no-clangd"]);
    let debug = d.json(".zed/debug.json");
    assert_eq!(
        debug[0]["program"],
        "target/thumbv6m-none-eabi/debug/blinky"
    );
    assert!(debug[0].get("chip").is_none());
    assert_eq!(d.json(".zed/tasks.json")[1]["command"], "cargo run");
    assert!(!d.exists(".zed/settings.json"));
}

#[test]
fn rejects_bad_input() {
    let d = Dir::new("bad");
    let out = d.zembed(&["--tasks", "foo"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown task"));
    let out = d.zembed(&["--yes"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("neither platformio.ini nor Cargo.toml"));
}
