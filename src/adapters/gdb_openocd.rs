//! Debug adapter backed by GDB's own DAP mode (`gdb -i=dap`, GDB 14+), with
//! OpenOCD as the GDB server.
//!
//! This is the adapter for C and C++ firmware (Arduino, ESP-IDF, ...), where
//! probe-rs cannot place source breakpoints.
//!
//! Zed lets an extension start a single process, so GDB is that process and
//! it spawns OpenOCD itself, in pipe mode: `target remote | openocd ...`.
//! OpenOCD then lives and dies with the debug session.

use crate::paths::{expand_home, is_absolute, resolve_path, sh_quote};
use crate::platformio;
use zed_extension_api::{
    serde_json::{self, json, Value},
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
const RESUME_ON_DETACH: &str = "foreach t [target names] { $t configure -event gdb-detach { resume } }";

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

    let session = build_session(
        raw,
        &root,
        home.as_deref(),
        |file| worktree.read_text_file(file).ok(),
        |name| worktree.which(name),
    )?;

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
        Some("attach") => Ok(StartDebuggingRequestArgumentsRequest::Attach),
        Some(other) => Err(format!(
            "unsupported `request`: `{other}`. gdb-openocd only supports `attach`: \
             upload the firmware first, for example with a `build` task."
        )),
        None => Err("missing `request` (expected `attach`)".into()),
    }
}

pub fn config_to_scenario(_config: DebugConfig) -> Result<DebugScenario, String> {
    Err(
        "gdb-openocd needs the paths of GDB and OpenOCD, which this dialog cannot provide. \
         Add an entry to .zed/debug.json instead (see the Zembed README)."
            .into(),
    )
}

/// Turns a `.zed/debug.json` entry into the GDB command line and the DAP
/// `attach` arguments.
///
/// `read_file` reads a project file relative to the worktree root, and
/// `which` looks a program up on the user's PATH.
pub fn build_session(
    raw: Value,
    root: &str,
    home: Option<&str>,
    read_file: impl Fn(&str) -> Option<String>,
    which: impl Fn(&str) -> Option<String>,
) -> Result<Session, String> {
    let Value::Object(config) = raw else {
        return Err("gdb-openocd debug configuration must be a JSON object".into());
    };
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
            which(value).ok_or_else(|| format!("`{key}`: `{value}` was not found on your PATH"))
        }
    };

    let cwd = match string("cwd")? {
        Some(cwd) => resolve_path(&expand_home(cwd, home), root),
        None => root.to_string(),
    };

    let gdb = string("gdb")?.ok_or(
        "missing `gdb`: set it to a GDB 14 or newer built for your chip and with Python \
         support, for example the `xtensa-esp32s3-elf-gdb` of a recent ESP32 toolchain",
    )?;
    let gdb = program_path("gdb", gdb)?;

    let program = match string("program")? {
        Some(program) => expand_home(program, home),
        None => {
            let env = match string("pioEnv")? {
                Some(env) => Some(env.to_string()),
                None => read_file(platformio::PROJECT_FILE)
                    .and_then(|ini| platformio::default_env(&ini)),
            };
            env.map(|env| platformio::firmware_elf(&env)).ok_or(
                "no program to debug: set `program` to the path of your firmware ELF, or \
                 `pioEnv` to a PlatformIO environment",
            )?
        }
    };
    let program = resolve_path(&program, &cwd);

    let target = match (string("target")?, string("openocd")?) {
        // A GDB server the user started themselves.
        (Some(target), _) => target.to_string(),
        (None, Some(openocd)) => {
            let openocd = program_path("openocd", openocd)?;
            openocd_pipe(&openocd, &config, home)?
        }
        (None, None) => {
            return Err(
                "set `openocd` to the path of OpenOCD so Zembed starts it for the session, or \
                 `target` to a GDB server that is already running (for example \
                 `localhost:3333`)"
                    .into(),
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

/// The `target remote` argument that makes GDB spawn OpenOCD and talk to it
/// over a pipe. GDB runs it through `/bin/sh`.
fn openocd_pipe(
    openocd: &str,
    config: &serde_json::Map<String, Value>,
    home: Option<&str>,
) -> Result<String, String> {
    let args = match config.get("openocdArgs") {
        Some(Value::Array(args)) if !args.is_empty() => args
            .iter()
            .map(|arg| arg.as_str().ok_or("`openocdArgs` must be an array of strings"))
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::Array(_)) | None => {
            return Err(
                "missing `openocdArgs`: give OpenOCD its configuration, for example \
                 [\"-f\", \"board/esp32s3-builtin.cfg\"]"
                    .into(),
            )
        }
        Some(_) => return Err("`openocdArgs` must be an array of strings".into()),
    };
    let resume = match config.get("resumeOnDisconnect") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(resume)) => *resume,
        Some(_) => return Err("`resumeOnDisconnect` must be true or false".into()),
    };

    let mut command = vec!["|".to_string(), sh_quote(openocd)];
    // OpenOCD installs its scripts next to the binary; pass the directory
    // explicitly for toolchains that are not installed system-wide.
    let scripts = match config.get("openocdScripts").and_then(Value::as_str) {
        Some(scripts) => Some(expand_home(scripts, home)),
        None => openocd
            .rsplit_once("/bin/")
            .filter(|_| is_absolute(openocd))
            .map(|(prefix, _)| format!("{prefix}/share/openocd/scripts")),
    };
    if let Some(scripts) = scripts {
        command.extend(["-s".to_string(), sh_quote(&scripts)]);
    }
    command.extend(args.iter().map(|arg| sh_quote(&expand_home(arg, home))));
    command.extend([
        "-c".to_string(),
        sh_quote("gdb_port pipe; tcl_port disabled; telnet_port disabled"),
    ]);
    if resume {
        command.extend(["-c".to_string(), sh_quote(RESUME_ON_DETACH)]);
    }
    Ok(command.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GDB: &str = "~/tools/gdb/bin/xtensa-esp32s3-elf-gdb";
    const OPENOCD: &str = "~/tools/openocd-esp32/bin/openocd";

    fn project(file: &str) -> Option<String> {
        (file == "platformio.ini").then(|| "[env:seeed_xiao_esp32s3]\n".to_string())
    }

    fn build(raw: Value) -> Result<Session, String> {
        build_session(raw, "/work", Some("/home/me"), project, |name| {
            (name == "gdb-multiarch").then(|| "/usr/bin/gdb-multiarch".to_string())
        })
    }

    #[test]
    fn spawns_openocd_through_a_pipe() {
        let session = build(json!({
            "request": "attach",
            "gdb": GDB,
            "openocd": OPENOCD,
            "openocdArgs": ["-f", "board/esp32s3-builtin.cfg"],
        }))
        .unwrap();
        assert_eq!(session.gdb, "/home/me/tools/gdb/bin/xtensa-esp32s3-elf-gdb");
        assert_eq!(session.gdb_args[..3], ["-q", "-i=dap", "-iex"]);
        assert_eq!(
            session.attach,
            json!({
                "program": "/work/.pio/build/seeed_xiao_esp32s3/firmware.elf",
                "target": "| '/home/me/tools/openocd-esp32/bin/openocd' \
                           -s '/home/me/tools/openocd-esp32/share/openocd/scripts' \
                           '-f' 'board/esp32s3-builtin.cfg' \
                           -c 'gdb_port pipe; tcl_port disabled; telnet_port disabled' \
                           -c 'foreach t [target names] { $t configure -event gdb-detach { resume } }'",
            })
        );
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
    fn connects_to_a_running_server() {
        let session = build(json!({
            "request": "attach",
            "gdb": "gdb-multiarch",
            "target": "localhost:3333",
            "program": "build/app.elf",
        }))
        .unwrap();
        assert_eq!(session.gdb, "/usr/bin/gdb-multiarch");
        assert_eq!(
            session.attach,
            json!({ "program": "/work/build/app.elf", "target": "localhost:3333" })
        );
    }

    #[test]
    fn honours_options() {
        let session = build(json!({
            "request": "attach",
            "gdb": GDB,
            "openocd": "/opt/ocd/openocd",
            "openocdScripts": "~/scripts",
            "openocdArgs": ["-f", "it's.cfg"],
            "resumeOnDisconnect": false,
            "pioEnv": "other",
        }))
        .unwrap();
        assert_eq!(session.attach["program"], "/work/.pio/build/other/firmware.elf");
        assert_eq!(
            session.attach["target"],
            "| '/opt/ocd/openocd' -s '/home/me/scripts' '-f' 'it'\\''s.cfg' \
             -c 'gdb_port pipe; tcl_port disabled; telnet_port disabled'"
        );
    }

    #[test]
    fn reports_what_is_missing() {
        let base = json!({ "request": "attach", "gdb": GDB, "openocd": OPENOCD });
        assert!(build(base.clone()).unwrap_err().contains("openocdArgs"));
        let no_server = json!({ "request": "attach", "gdb": GDB });
        assert!(build(no_server).unwrap_err().contains("`openocd`"));
        let no_gdb = json!({ "request": "attach", "target": ":3333" });
        assert!(build(no_gdb).unwrap_err().contains("`gdb`"));
        let not_on_path = json!({ "request": "attach", "gdb": "arm-none-eabi-gdb", "target": ":3333" });
        assert!(build(not_on_path).unwrap_err().contains("PATH"));
        let no_program = build_session(
            json!({ "request": "attach", "gdb": GDB, "target": ":3333" }),
            "/work",
            None,
            |_| None,
            |_| None,
        );
        assert!(no_program.unwrap_err().contains("program"));
    }

    #[test]
    fn only_attaches() {
        assert!(request_kind(&json!({ "request": "attach" })).is_ok());
        assert!(request_kind(&json!({ "request": "launch" })).unwrap_err().contains("attach"));
    }
}
