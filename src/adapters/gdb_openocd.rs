//! Debug adapter backed by GDB's own DAP mode (`gdb -i=dap`, GDB 14+), with
//! OpenOCD as the GDB server.
//!
//! This is the adapter for C and C++ firmware (Arduino, ESP-IDF, ...), where
//! probe-rs cannot place source breakpoints.
//!
//! Zed lets an extension start a single process, so GDB is that process and
//! it spawns OpenOCD itself, in pipe mode: `target remote | openocd ...`.
//! OpenOCD then lives and dies with the debug session.

use crate::esp;
use crate::paths::{expand_home, is_absolute, resolve_path, sh_quote};
use crate::platformio;
use std::{cell::RefCell, collections::HashMap};
use zed_extension_api::{
    process::Command,
    serde_json::{self, json, Map, Value},
    DebugAdapterBinary, DebugConfig, DebugScenario, DebugTaskDefinition,
    StartDebuggingRequestArguments, StartDebuggingRequestArgumentsRequest, Worktree,
};

pub const ADAPTER_NAME: &str = "gdb-openocd";

/// Works around a bug in GDB's DAP mode (seen in GDB 17.1): every newly
/// discovered thread marks the target as running. OpenOCD reports RTOS tasks
/// as threads, discovered while the target is halted, so every later request
/// failed with `notStopped`. The fix reports the thread without touching the
/// running state. It is a no-op if GDB's internals differ.
const NEW_THREAD_FIX: &str = "python exec(\"import gdb\\nimport gdb.dap.events as e\\ntry:\\n \
gdb.events.new_thread.disconnect(e._new_thread)\\n \
gdb.events.new_thread.connect(lambda ev: e.thread_event(ev, 'started'))\\n\
except Exception:\\n pass\")";

/// Resumes every core when GDB detaches, so the firmware is not left frozen
/// after the session.
const RESUME_ON_DETACH: &str =
    "foreach t [target names] { $t configure -event gdb-detach { resume } }";

/// What the adapter needs from the user's machine. Implemented over Zed's
/// worktree and `process:exec` in the extension, and faked in tests.
pub trait Host {
    /// Reads a project file, relative to the worktree root.
    fn read_file(&self, path: &str) -> Option<String>;
    /// Looks a program up on the user's PATH.
    fn which(&self, name: &str) -> Option<String>;
    fn home(&self) -> Option<String>;
    /// Runs a POSIX shell script with positional arguments.
    fn sh(&self, script: &str, args: &[String]) -> Result<ShOutput, String>;
}

pub struct ShOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

struct WorktreeHost<'a> {
    worktree: &'a Worktree,
    home: Option<String>,
}

impl Host for WorktreeHost<'_> {
    fn read_file(&self, path: &str) -> Option<String> {
        self.worktree.read_text_file(path).ok()
    }

    fn which(&self, name: &str) -> Option<String> {
        self.worktree.which(name)
    }

    fn home(&self) -> Option<String> {
        self.home.clone()
    }

    fn sh(&self, script: &str, args: &[String]) -> Result<ShOutput, String> {
        let mut command = Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("zembed")
            .args(args);
        if let Some(home) = &self.home {
            command = command.env("HOME", home);
        }
        let output = command.output()?;
        Ok(ShOutput {
            success: output.status == Some(0),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// A debug session: the GDB process to start and the DAP `attach` arguments.
#[derive(Debug, PartialEq)]
pub struct Session {
    pub gdb: String,
    pub gdb_args: Vec<String>,
    pub attach: Value,
}

pub fn get_dap_binary(
    task: DebugTaskDefinition,
    user_provided_path: Option<String>,
    worktree: &Worktree,
) -> Result<DebugAdapterBinary, String> {
    let root = worktree.root_path();
    let mut raw: Value = serde_json::from_str(&task.config)
        .map_err(|e| format!("invalid gdb-openocd debug configuration: {e}"))?;
    let request = request_kind(&raw)?;
    // The `dap.gdb-openocd.binary` setting of Zed wins over `gdb`.
    if let (Some(path), Some(config)) = (user_provided_path, raw.as_object_mut()) {
        config.insert("gdb".into(), path.into());
    }
    let home = worktree
        .shell_env()
        .into_iter()
        .find_map(|(key, value)| (key == "HOME").then_some(value));
    let host = WorktreeHost { worktree, home };

    let session = build_session(raw, &root, &host)?;

    Ok(DebugAdapterBinary {
        command: Some(session.gdb),
        arguments: session.gdb_args,
        envs: Vec::new(),
        cwd: Some(root),
        // No TCP connection: GDB speaks DAP on stdin/stdout.
        connection: None,
        request_args: StartDebuggingRequestArguments {
            configuration: session.attach.to_string(),
            request,
        },
    })
}

pub fn request_kind(config: &Value) -> Result<StartDebuggingRequestArgumentsRequest, String> {
    match config.get("request").and_then(Value::as_str) {
        Some("attach") | None => Ok(StartDebuggingRequestArgumentsRequest::Attach),
        Some(other) => Err(format!(
            "unsupported `request`: `{other}`. gdb-openocd only supports `attach`: \
             upload the firmware first, for example with a `build` task."
        )),
    }
}

/// Used by Zed's "new debug session" dialog: everything else is detected.
pub fn config_to_scenario(config: DebugConfig) -> Result<DebugScenario, String> {
    let program = match config.request {
        zed_extension_api::DebugRequest::Launch(launch) => Some(launch.program),
        zed_extension_api::DebugRequest::Attach(_) => None,
    };
    let mut scenario = json!({ "request": "attach" });
    if let Some(program) = program.filter(|p| !p.is_empty()) {
        scenario["program"] = program.into();
    }
    Ok(DebugScenario {
        label: config.label,
        adapter: config.adapter,
        build: None,
        config: scenario.to_string(),
        tcp_connection: None,
    })
}

thread_local! {
    /// Tools found per GDB executable name, so discovery runs once per Zed
    /// session.
    static FOUND_TOOLS: RefCell<HashMap<String, (String, String)>> = RefCell::new(HashMap::new());
}

/// Turns a `.zed/debug.json` entry into the GDB command line and the DAP
/// `attach` arguments. Everything that is not given is detected: the chip
/// from platformio.ini, GDB and OpenOCD from the usual install locations,
/// and the firmware from the PlatformIO build directory.
pub fn build_session(raw: Value, root: &str, host: &dyn Host) -> Result<Session, String> {
    let Value::Object(config) = raw else {
        return Err("gdb-openocd debug configuration must be a JSON object".into());
    };
    let home = host.home();
    let home = home.as_deref();
    let string = |key: &str| -> Result<Option<&str>, String> {
        match config.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(format!("`{key}` must be a string")),
        }
    };
    // A bare name is looked up on PATH; anything else is a path.
    let program_path = |key: &str, value: &str| -> Result<String, String> {
        if value.contains(['/', '\\']) || value.starts_with('~') {
            Ok(expand_home(value, home))
        } else {
            host.which(value)
                .ok_or_else(|| format!("`{key}`: `{value}` was not found on your PATH"))
        }
    };

    let cwd = match string("cwd")? {
        Some(cwd) => resolve_path(&expand_home(cwd, home), root),
        None => root.to_string(),
    };

    let ini = host.read_file(platformio::PROJECT_FILE);
    let pio_env = match string("pioEnv")? {
        Some(env) => Some(env.to_string()),
        None => ini.as_deref().and_then(platformio::default_env),
    };

    let program = match string("program")? {
        Some(program) => expand_home(program, home),
        None => pio_env.as_deref().map(platformio::firmware_elf).ok_or(
            "no program to debug: set `program` to the path of your firmware ELF \
             (detected automatically in PlatformIO projects)",
        )?,
    };
    let program = resolve_path(&program, &cwd);

    let given_gdb = string("gdb")?;
    let given_target = string("target")?;
    let given_openocd = string("openocd")?;
    let given_args = openocd_args(&config)?;
    let needs_chip = given_gdb.is_none()
        || (given_target.is_none() && (given_openocd.is_none() || given_args.is_none()));
    let chip = if needs_chip {
        Some(detect_chip(
            string("chip")?,
            ini.as_deref(),
            pio_env.as_deref(),
            host,
        )?)
    } else {
        None
    };

    let found = match (
        &chip,
        given_gdb.is_none() || (given_target.is_none() && given_openocd.is_none()),
    ) {
        (Some(chip), true) => Some(find_tools(chip, host)?),
        _ => None,
    };

    let gdb = match given_gdb {
        Some(gdb) => program_path("gdb", gdb)?,
        None => found
            .as_ref()
            .map(|(gdb, _)| gdb.clone())
            .ok_or("missing `gdb`")?,
    };

    let target = match given_target {
        // A GDB server the user started themselves.
        Some(target) => target.to_string(),
        None => {
            let openocd = match given_openocd {
                Some(openocd) => program_path("openocd", openocd)?,
                None => found
                    .as_ref()
                    .map(|(_, o)| o.clone())
                    .ok_or("missing `openocd`")?,
            };
            let args = match given_args {
                Some(args) => args,
                None => default_openocd_args(chip.as_ref())?,
            };
            let scripts = match string("openocdScripts")? {
                Some(scripts) => Some(expand_home(scripts, home)),
                None => openocd
                    .rsplit_once("/bin/")
                    .filter(|_| is_absolute(&openocd))
                    .map(|(prefix, _)| format!("{prefix}/share/openocd/scripts")),
            };
            let args: Vec<String> = args.iter().map(|arg| expand_home(arg, home)).collect();
            if flag(&config, "checkProbe", true)? {
                check_probe(&openocd, scripts.as_deref(), &args, host)?;
            }
            openocd_pipe(
                &openocd,
                scripts.as_deref(),
                &args,
                flag(&config, "resumeOnDisconnect", true)?,
            )
        }
    };

    Ok(Session {
        gdb,
        gdb_args: vec![
            "-q".into(),
            "-i=dap".into(),
            "-iex".into(),
            NEW_THREAD_FIX.into(),
        ],
        // GDB runs `target remote <target>` itself, so `target` must not start
        // with `remote` or `extended-remote`.
        attach: json!({ "program": program, "target": target }),
    })
}

fn flag(config: &Map<String, Value>, key: &str, default: bool) -> Result<bool, String> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

fn openocd_args(config: &Map<String, Value>) -> Result<Option<Vec<String>>, String> {
    match config.get("openocdArgs") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(args)) if args.is_empty() => Ok(None),
        Some(Value::Array(args)) => args
            .iter()
            .map(|arg| {
                arg.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "`openocdArgs` must be an array of strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err("`openocdArgs` must be an array of strings".into()),
    }
}

fn default_openocd_args(chip: Option<&esp::Chip>) -> Result<Vec<String>, String> {
    let chip = chip.ok_or("missing `openocdArgs`")?;
    match &chip.builtin_jtag_config {
        Some(config) => Ok(vec!["-f".into(), config.clone()]),
        None => Err(format!(
            "the {} has no built-in USB-JTAG, so Zembed cannot guess your probe. Set \
             `openocdArgs`, for example [\"-f\", \"board/esp32-wrover-kit-3.3v.cfg\"] for an \
             ESP-Prog, or [\"-f\", \"interface/<probe>.cfg\", \"-f\", \"target/{}.cfg\"].",
            chip.name, chip.name
        )),
    }
}

/// The chip from `chip`, else from the PlatformIO board of the environment.
fn detect_chip(
    given: Option<&str>,
    ini: Option<&str>,
    env: Option<&str>,
    host: &dyn Host,
) -> Result<esp::Chip, String> {
    if let Some(name) = given {
        return esp::chip(name).ok_or_else(|| {
            format!(
                "`chip`: `{name}` is not an Espressif chip Zembed knows. For other chips, set \
                 `gdb`, `openocd` and `openocdArgs` yourself."
            )
        });
    }
    let board = match (ini, env) {
        (Some(ini), Some(env)) => platformio::env_value(ini, env, "board"),
        _ => None,
    };
    let Some(board) = board else {
        return Err(
            "cannot tell which chip to debug: this is not a PlatformIO project with a `board`. \
             Set `chip` (for example \"esp32s3\"), or `gdb`, `openocd` and `openocdArgs`."
                .into(),
        );
    };
    // Board definitions: the project's own `boards/` folder, then PlatformIO's.
    let from_json = host
        .read_file(&format!("boards/{board}.json"))
        .and_then(|json| esp::chip_from_board_json(&json))
        .or_else(|| {
            host.sh(esp::BOARD_SCRIPT, std::slice::from_ref(&board))
                .ok()
                .and_then(|out| esp::chip_from_board_json(&out.stdout))
        });
    from_json
        .or_else(|| esp::chip_from_board_id(&board))
        .ok_or_else(|| {
            format!(
                "board `{board}` does not use an Espressif chip Zembed knows. Set `gdb`, \
                 `openocd` and `openocdArgs` in .zed/debug.json."
            )
        })
}

/// Finds GDB and OpenOCD for the chip, once per Zed session.
fn find_tools(chip: &esp::Chip, host: &dyn Host) -> Result<(String, String), String> {
    if let Some(found) = FOUND_TOOLS.with(|cache| cache.borrow().get(&chip.gdb_binary).cloned()) {
        return Ok(found);
    }
    let output = host.sh(
        esp::DISCOVER_SCRIPT,
        &[chip.gdb_package.to_string(), chip.gdb_binary.clone()],
    )?;
    let (gdbs, openocds) = esp::parse_discovery(&output.stdout);
    let gdb = esp::pick_gdb(&gdbs, &chip.gdb_binary)?;
    let openocd = esp::pick_openocd(&openocds)?;
    let found = (gdb, openocd);
    FOUND_TOOLS.with(|cache| {
        cache
            .borrow_mut()
            .insert(chip.gdb_binary.clone(), found.clone());
    });
    Ok(found)
}

/// Starts OpenOCD once to make sure the probe and chip answer, so a missing
/// board gives a clear error instead of an empty debug session.
fn check_probe(
    openocd: &str,
    scripts: Option<&str>,
    args: &[String],
    host: &dyn Host,
) -> Result<(), String> {
    let mut argv = vec![openocd.to_string()];
    if let Some(scripts) = scripts {
        argv.extend(["-s".to_string(), scripts.to_string()]);
    }
    argv.extend(args.iter().cloned());
    argv.extend(["-c".to_string(), "init; shutdown".to_string()]);
    let output = host.sh(esp::PROBE_CHECK_SCRIPT, &argv)?;
    let log = format!("{}\n{}", output.stderr, output.stdout);
    match esp::probe_check_error(output.success, &log) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The `target remote` argument that makes GDB spawn OpenOCD and talk to it
/// over a pipe. GDB runs it through `/bin/sh`.
fn openocd_pipe(openocd: &str, scripts: Option<&str>, args: &[String], resume: bool) -> String {
    let mut command = vec!["|".to_string(), sh_quote(openocd)];
    if let Some(scripts) = scripts {
        command.extend(["-s".to_string(), sh_quote(scripts)]);
    }
    command.extend(args.iter().map(|arg| sh_quote(arg)));
    command.extend([
        "-c".to_string(),
        sh_quote("gdb_port pipe; tcl_port disabled; telnet_port disabled"),
    ]);
    if resume {
        command.extend(["-c".to_string(), sh_quote(RESUME_ON_DETACH)]);
    }
    command.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const GDB: &str = "/ard/xtensa-esp-elf-gdb/17.1/bin/xtensa-esp32s3-elf-gdb";
    const OPENOCD: &str = "/ard/openocd-esp32/v0.12.0-esp32-20260424/bin/openocd";

    /// A machine with a PlatformIO project for a XIAO ESP32S3.
    struct Fake {
        files: Vec<(&'static str, &'static str)>,
        discovery: String,
        probe_ok: bool,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn xiao() -> Self {
            Fake {
                files: vec![(
                    "platformio.ini",
                    "[env:seeed_xiao_esp32s3]\nplatform = espressif32\nboard = seeed_xiao_esp32s3\n",
                )],
                discovery: format!(
                    "gdb\t{GDB}\tGNU gdb (esp-gdb) 17.1_20260402\t1\n\
                     openocd\t{OPENOCD}\tOpen On-Chip Debugger v0.12.0-esp32-20260424\n"
                ),
                probe_ok: true,
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Host for Fake {
        fn read_file(&self, path: &str) -> Option<String> {
            self.files
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, c)| c.to_string())
        }
        fn which(&self, name: &str) -> Option<String> {
            (name == "gdb-multiarch").then(|| "/usr/bin/gdb-multiarch".to_string())
        }
        fn home(&self) -> Option<String> {
            Some("/home/me".into())
        }
        fn sh(&self, script: &str, args: &[String]) -> Result<ShOutput, String> {
            let (kind, stdout, success, stderr) = if script == esp::DISCOVER_SCRIPT {
                ("discover", self.discovery.clone(), true, String::new())
            } else if script == esp::BOARD_SCRIPT {
                ("board", String::new(), true, String::new())
            } else if script == esp::PROBE_CHECK_SCRIPT {
                let log = if self.probe_ok {
                    "Info : [esp32s3.cpu0] Examination succeed\n"
                } else {
                    "Error: esp_usb_jtag: could not find or open device!\n"
                };
                ("probe", String::new(), self.probe_ok, log.to_string())
            } else {
                panic!("unexpected script");
            };
            self.calls
                .borrow_mut()
                .push(format!("{kind} {}", args.join(" ")));
            Ok(ShOutput {
                success,
                stdout,
                stderr,
            })
        }
    }

    fn build(raw: Value, host: &Fake) -> Result<Session, String> {
        FOUND_TOOLS.with(|cache| cache.borrow_mut().clear());
        build_session(raw, "/work", host)
    }

    #[test]
    fn needs_no_configuration_in_a_platformio_project() {
        let host = Fake::xiao();
        let session = build(json!({}), &host).unwrap();
        assert_eq!(session.gdb, GDB);
        assert_eq!(session.gdb_args[..3], ["-q", "-i=dap", "-iex"]);
        assert_eq!(
            session.attach,
            json!({
                "program": "/work/.pio/build/seeed_xiao_esp32s3/firmware.elf",
                "target": "| '/ard/openocd-esp32/v0.12.0-esp32-20260424/bin/openocd' \
                           -s '/ard/openocd-esp32/v0.12.0-esp32-20260424/share/openocd/scripts' \
                           '-f' 'board/esp32s3-builtin.cfg' \
                           -c 'gdb_port pipe; tcl_port disabled; telnet_port disabled' \
                           -c 'foreach t [target names] { $t configure -event gdb-detach { resume } }'",
            })
        );
        let calls = host.calls.borrow();
        assert_eq!(calls[0], "board seeed_xiao_esp32s3");
        assert_eq!(
            calls[1],
            "discover xtensa-esp-elf-gdb xtensa-esp32s3-elf-gdb"
        );
        assert!(
            calls[2].starts_with(&format!("probe {OPENOCD} -s ")),
            "{}",
            calls[2]
        );
        assert!(calls[2].ends_with("-f board/esp32s3-builtin.cfg -c init; shutdown"));
    }

    #[test]
    fn reports_an_unreachable_probe() {
        let mut host = Fake::xiao();
        host.probe_ok = false;
        let error = build(json!({ "request": "attach" }), &host).unwrap_err();
        assert!(error.contains("could not find or open device"), "{error}");
        assert!(error.contains("plugged in"), "{error}");
        let skipped = build(json!({ "checkProbe": false }), &host);
        assert!(skipped.is_ok());
    }

    #[test]
    fn explains_missing_tools() {
        let mut host = Fake::xiao();
        host.discovery = "openocd\t/usr/bin/openocd\tOpen On-Chip Debugger 0.12.0\n".into();
        let error = build(json!({}), &host).unwrap_err();
        assert!(error.contains("xtensa-esp32s3-elf-gdb"), "{error}");
        assert!(error.contains("Arduino IDE"), "{error}");
    }

    #[test]
    fn explicit_settings_skip_detection() {
        let host = Fake::xiao();
        let session = build(
            json!({
                "gdb": "gdb-multiarch",
                "target": "localhost:3333",
                "program": "build/app.elf",
            }),
            &host,
        )
        .unwrap();
        assert_eq!(session.gdb, "/usr/bin/gdb-multiarch");
        assert_eq!(
            session.attach,
            json!({ "program": "/work/build/app.elf", "target": "localhost:3333" })
        );
        assert!(host.calls.borrow().is_empty());
    }

    #[test]
    fn honours_options() {
        let host = Fake::xiao();
        let session = build(
            json!({
                "gdb": "~/gdb",
                "openocd": "/opt/ocd/openocd",
                "openocdScripts": "~/scripts",
                "openocdArgs": ["-f", "it's.cfg"],
                "resumeOnDisconnect": false,
                "checkProbe": false,
                "pioEnv": "other",
            }),
            &host,
        )
        .unwrap();
        assert_eq!(session.gdb, "/home/me/gdb");
        assert_eq!(
            session.attach["program"],
            "/work/.pio/build/other/firmware.elf"
        );
        assert_eq!(
            session.attach["target"],
            "| '/opt/ocd/openocd' -s '/home/me/scripts' '-f' 'it'\\''s.cfg' \
             -c 'gdb_port pipe; tcl_port disabled; telnet_port disabled'"
        );
        assert!(host.calls.borrow().is_empty());
    }

    #[test]
    fn detects_the_chip() {
        // An explicit chip outside PlatformIO.
        let mut host = Fake::xiao();
        host.files.clear();
        let error = build(json!({ "program": "a.elf" }), &host).unwrap_err();
        assert!(error.contains("`chip`"), "{error}");
        let session = build(json!({ "program": "a.elf", "chip": "esp32-s3" }), &host).unwrap();
        assert_eq!(session.gdb, GDB);
        // Chips without built-in USB-JTAG need the probe to be named.
        let error = build(json!({ "program": "a.elf", "chip": "esp32" }), &host).unwrap_err();
        assert!(error.contains("openocdArgs"), "{error}");
        // Non-Espressif boards.
        let mut host = Fake::xiao();
        host.files = vec![("platformio.ini", "[env:nucleo]\nboard = nucleo_f411re\n")];
        let error = build(json!({}), &host).unwrap_err();
        assert!(error.contains("nucleo_f411re"), "{error}");
    }

    #[test]
    fn new_thread_fix_is_one_gdb_command() {
        // GDB reads `-iex` as a single command line: real newlines would cut
        // the Python in two. Python sees the `\n` escapes inside its string.
        assert!(!NEW_THREAD_FIX.contains('\n'));
        assert!(NEW_THREAD_FIX.starts_with("python exec(\"import gdb\\nimport gdb.dap.events"));
        assert!(NEW_THREAD_FIX.ends_with("except Exception:\\n pass\")"));
    }

    #[test]
    fn only_attaches() {
        assert!(request_kind(&json!({ "request": "attach" })).is_ok());
        assert!(request_kind(&json!({})).is_ok());
        assert!(request_kind(&json!({ "request": "launch" }))
            .unwrap_err()
            .contains("attach"));
    }
}
