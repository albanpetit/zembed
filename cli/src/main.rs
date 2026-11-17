//! `zembed`: sets up Zed (Zembed extension) for an embedded firmware project.
//!
//!     zembed init [PROJECT]     write .zed/debug.json, tasks.json, settings.json
//!     zembed doctor [PROJECT]   check the project and the debugging tools

// The extension's own modules: same chip table, same tool discovery.
#[allow(dead_code)]
#[path = "../../src/esp.rs"]
mod esp;
#[allow(dead_code)]
#[path = "../../src/platformio.rs"]
mod platformio;

mod generate;
mod jsonc;
mod project;
mod text;

use generate::{Choices, ALL_TASKS, DEFAULT_TASKS};
use project::Project;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use text::{Lang, Text};

const HELP: &str = "\
zembed: sets up Zed (Zembed extension) for an embedded firmware project.

Usage:
  zembed init [PROJECT] [options]   write .zed/debug.json, tasks.json, settings.json
  zembed doctor [PROJECT]           check the project and the debugging tools

PROJECT defaults to the current folder (with platformio.ini or Cargo.toml).

Options of init:
  --lang en|fr          language of labels and comments (default: from $LANG)
  --env NAME            PlatformIO environment (default: the project's default)
  --tasks LIST          comma-separated, among build, upload, upload_monitor,
                        monitor, clean, compiledb, test, free_probe
                        (\"\" for no tasks.json)
  --label TEXT          prefix of the debug session names (default: the board)
  --chip CHIP           chip for probe-rs (non-ESP32 boards, Rust)
  --program PATH        firmware ELF (Rust)
  --no-debug            no debug.json
  --no-upload-debug     no \"upload and debug\" session
  --no-attach           no \"attach\" session
  --no-clangd           no settings.json
  --debug-build         add build_type = debug to platformio.ini
  --no-debug-build      leave platformio.ini alone (default with --yes)
  -y, --yes             ask nothing, take the defaults
  -n, --dry-run         show what would be written, write nothing
  -f, --force           replace existing files without asking (.bak copies kept)
";

#[derive(Default)]
struct Options {
    project: Option<PathBuf>,
    lang: Option<Lang>,
    env: Option<String>,
    tasks: Option<Vec<String>>,
    label: Option<String>,
    chip: Option<String>,
    program: Option<String>,
    no_debug: bool,
    no_upload_debug: bool,
    no_attach: bool,
    no_clangd: bool,
    debug_build: Option<bool>,
    yes: bool,
    dry_run: bool,
    force: bool,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut i = 0;
    let value = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or(format!("{name} needs a value"))
    };
    while i < args.len() {
        let arg = args[i].as_str();
        // Accept `--name=value` too.
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg, None),
        };
        let take = |i: &mut usize| inline.clone().map(Ok).unwrap_or_else(|| value(i, name));
        match name {
            "--lang" => {
                o.lang = Some(match take(&mut i)?.as_str() {
                    "fr" => Lang::Fr,
                    "en" => Lang::En,
                    other => return Err(format!("--lang: `{other}` is not en or fr")),
                })
            }
            "--env" => o.env = Some(take(&mut i)?),
            "--tasks" => {
                let list: Vec<String> = take(&mut i)?
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if let Some(bad) = list.iter().find(|t| !ALL_TASKS.contains(&t.as_str())) {
                    return Err(format!(
                        "unknown task `{bad}`; choose among {}",
                        ALL_TASKS.join(", ")
                    ));
                }
                o.tasks = Some(list);
            }
            "--label" => o.label = Some(take(&mut i)?),
            "--chip" => o.chip = Some(take(&mut i)?),
            "--program" => o.program = Some(take(&mut i)?),
            "--no-debug" => o.no_debug = true,
            "--no-upload-debug" => o.no_upload_debug = true,
            "--no-attach" => o.no_attach = true,
            "--no-clangd" => o.no_clangd = true,
            "--debug-build" => o.debug_build = Some(true),
            "--no-debug-build" => o.debug_build = Some(false),
            "-y" | "--yes" => o.yes = true,
            "-n" | "--dry-run" => o.dry_run = true,
            "-f" | "--force" => o.force = true,
            _ if !arg.starts_with('-') && o.project.is_none() => {
                o.project = Some(PathBuf::from(arg))
            }
            _ => return Err(format!("unknown option `{arg}` (see zembed --help)")),
        }
        i += 1;
    }
    Ok(o)
}

/// Asks questions only in a terminal and without --yes.
struct Prompt<'a> {
    interactive: bool,
    t: &'a Text,
}

impl Prompt<'_> {
    fn line(&self, question: &str) -> String {
        print!("{question} ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer).ok();
        answer.trim().to_string()
    }

    fn ask(&self, question: &str, default: &str) -> String {
        if !self.interactive {
            return default.to_string();
        }
        let answer = self.line(&if default.is_empty() {
            question.to_string()
        } else {
            format!("{question} [{default}]")
        });
        if answer.is_empty() {
            default.to_string()
        } else {
            answer
        }
    }

    fn confirm(&self, question: &str, default: bool) -> bool {
        if !self.interactive {
            return default;
        }
        let (y, n) = (self.t.yes, self.t.no);
        let choices = if default {
            format!("[{}/{n}]", y.to_uppercase())
        } else {
            format!("[{y}/{}]", n.to_uppercase())
        };
        match self
            .line(&format!("{question} {choices}"))
            .to_lowercase()
            .chars()
            .next()
        {
            None => default,
            Some(c) => c == 'y' || c == 'o',
        }
    }

    fn tasks(&self) -> Vec<String> {
        let defaults = || DEFAULT_TASKS.iter().map(|s| s.to_string()).collect();
        if !self.interactive {
            return defaults();
        }
        println!("{}:", self.t.q_tasks);
        for (i, key) in ALL_TASKS.iter().enumerate() {
            let mark = if DEFAULT_TASKS.contains(key) {
                "x"
            } else {
                " "
            };
            println!("  {}. [{mark}] {key:15} {}", i + 1, self.t.task(key));
        }
        let answer = self.line(self.t.q_tasks_pick);
        if answer.is_empty() {
            return defaults();
        }
        answer
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|n| n.parse::<usize>().ok())
            .filter_map(|n| ALL_TASKS.get(n.wrapping_sub(1)))
            .map(|s| s.to_string())
            .collect()
    }
}

fn unified_diff(name: &str, old: &str, new: &str) -> String {
    // A simple line diff: enough to see what changes.
    let (old_lines, new_lines): (Vec<&str>, Vec<&str>) =
        (old.lines().collect(), new.lines().collect());
    let mut out = format!("--- .zed/{name}\n+++ .zed/{name} (new)\n");
    let (mut i, mut j) = (0, 0);
    while i < old_lines.len() || j < new_lines.len() {
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            i += 1;
            j += 1;
        } else if j < new_lines.len() && !old_lines[i..].contains(&new_lines[j]) {
            out.push_str(&format!("+{}\n", new_lines[j]));
            j += 1;
        } else if i < old_lines.len() {
            out.push_str(&format!("-{}\n", old_lines[i]));
            i += 1;
        } else {
            out.push_str(&format!("+{}\n", new_lines[j]));
            j += 1;
        }
    }
    out
}

fn init(o: Options) -> Result<(), String> {
    let root = std::fs::canonicalize(o.project.clone().unwrap_or_else(|| PathBuf::from(".")))
        .map_err(|e| {
            format!(
                "{}: {e}",
                o.project.as_deref().unwrap_or(Path::new(".")).display()
            )
        })?;
    let lang = o.lang.unwrap_or_else(Lang::from_env);
    let t = lang.text();
    let prompt = Prompt {
        interactive: !o.yes && std::io::stdin().is_terminal(),
        t,
    };
    let mut warnings = project::shell_problems();
    let mut choices = Choices {
        debug: !o.no_debug,
        upload_debug: !o.no_upload_debug,
        attach: !o.no_attach,
        tasks: Vec::new(),
        clangd: !o.no_clangd,
        label: o.label.clone(),
        chip: o.chip.clone(),
        program: o.program.clone(),
    };
    let mut ini_change: Option<(String, String)> = None;

    let files = match Project::read(&root)? {
        Project::PlatformIo { envs, default_env } => {
            let names: Vec<&str> = envs.iter().map(|e| e.name.as_str()).collect();
            let env_name = match &o.env {
                Some(env) => env.clone(),
                None if envs.len() > 1 => {
                    prompt.ask(&format!("{} ({})", t.q_env, names.join(", ")), &default_env)
                }
                None => default_env.clone(),
            };
            let env = envs.iter().find(|e| e.name == env_name).ok_or(format!(
                "unknown environment `{env_name}`; available: {}",
                names.join(", ")
            ))?;
            let (pio, pio_warning) = project::pio_command();
            warnings.extend(pio_warning);
            choices.tasks = o.tasks.clone().unwrap_or_else(|| prompt.tasks());
            if choices.debug {
                choices.upload_debug =
                    choices.upload_debug && prompt.confirm(t.q_upload_debug, true);
                choices.attach = choices.attach && prompt.confirm(t.q_attach, true);
                if env.chip.is_none() && choices.chip.is_none() {
                    choices.chip = Some(prompt.ask(t.q_chip, "CHIP"));
                }
                if let Some(chip) = &env.chip {
                    let tools = project::esp_tools(chip);
                    warnings.extend(tools.gdb.err());
                    warnings.extend(tools.openocd.err());
                }
                let wants_debug_build = env.build_type.as_deref() != Some("debug")
                    && match o.debug_build {
                        Some(answer) => answer,
                        None if o.yes => false,
                        None => prompt.confirm(&t.q_build_type.replace("{env}", &env.name), true),
                    };
                if wants_debug_build {
                    let ini = std::fs::read_to_string(root.join("platformio.ini"))
                        .map_err(|e| e.to_string())?;
                    ini_change = generate::with_debug_build_type(&ini, &env.name)
                        .map(|new| (env.name.clone(), new));
                }
            }
            generate::platformio(env, &default_env, &pio, &choices, t)
        }
        Project::Rust(rust) => {
            choices.tasks = o.tasks.clone().unwrap_or_else(|| vec!["build".into()]);
            if choices.debug {
                if rust.program.is_none() && choices.program.is_none() {
                    choices.program =
                        Some(prompt.ask(t.q_program, "target/<target triple>/debug/<crate>"));
                }
                if rust.chip.is_none() && choices.chip.is_none() {
                    choices.chip = Some(prompt.ask(t.q_chip, "CHIP"));
                }
                if project::which("probe-rs").is_none()
                    && !project::is_executable(&project::home().join(".cargo/bin/probe-rs"))
                {
                    warnings.push("probe-rs is not installed: https://probe.rs/docs/getting-started/installation/".into());
                }
            }
            generate::rust(&rust, &choices, t)
        }
    };

    // Show and write.
    let zed = root.join(".zed");
    let mut report: Vec<String> = Vec::new();
    for (name, content) in &files {
        let target = zed.join(name);
        let old = std::fs::read_to_string(&target).ok();
        if old.as_deref() == Some(content.as_str()) {
            report.push(format!(".zed/{name}: {}", t.unchanged));
            continue;
        }
        if o.dry_run {
            println!("----- .zed/{name}");
            match &old {
                None => print!("{content}"),
                Some(old) => print!("{}", unified_diff(name, old, content)),
            }
            continue;
        }
        if let Some(old) = &old {
            if !o.force {
                if !prompt.interactive {
                    report.push(format!(".zed/{name}: {}", t.skipped_hint));
                    continue;
                }
                print!("{}", unified_diff(name, old, content));
                if !prompt.confirm(&t.q_overwrite.replace("{name}", name), false) {
                    report.push(format!(".zed/{name}: {}", t.skipped));
                    continue;
                }
            }
        }
        std::fs::create_dir_all(&zed).map_err(|e| format!("{}: {e}", zed.display()))?;
        if let Some(old) = &old {
            std::fs::write(zed.join(format!("{name}.bak")), old).map_err(|e| e.to_string())?;
        }
        std::fs::write(&target, content).map_err(|e| format!("{}: {e}", target.display()))?;
        report.push(format!(
            ".zed/{name}: {}{}",
            t.written,
            if old.is_some() { " (.bak)" } else { "" }
        ));
    }
    if let Some((env, ini)) = ini_change {
        if o.dry_run {
            println!("----- platformio.ini: + build_type = debug  [env:{env}]");
        } else {
            std::fs::write(root.join("platformio.ini"), ini).map_err(|e| e.to_string())?;
            report.push(format!("platformio.ini: build_type = debug [env:{env}]"));
        }
    }
    for line in report {
        println!("  {line}");
    }
    for warning in warnings {
        println!("  ! {warning}");
    }
    if !o.dry_run {
        println!("{}", t.next);
    }
    Ok(())
}

fn doctor(o: Options) -> Result<bool, String> {
    let root = std::fs::canonicalize(o.project.clone().unwrap_or_else(|| PathBuf::from(".")))
        .map_err(|e| e.to_string())?;
    let mut ok = true;
    let mut line = |good: bool, text: &str| {
        ok &= good;
        println!("  {}  {text}", if good { "ok" } else { "!!" });
    };
    println!("Project: {}", root.display());
    match Project::read(&root) {
        Ok(Project::PlatformIo { envs, default_env }) => {
            for e in &envs {
                println!(
                    "  --  [env:{}]{}: board {}, chip {}, build_type {}",
                    e.name,
                    if e.name == default_env {
                        " (default)"
                    } else {
                        ""
                    },
                    e.board.as_deref().unwrap_or("?"),
                    e.chip
                        .as_ref()
                        .map(|c| generate::chip_display(&c.name))
                        .unwrap_or_else(|| "? (probe-rs)".into()),
                    e.build_type.as_deref().unwrap_or("release")
                );
            }
            let (pio, warning) = project::pio_command();
            line(
                warning
                    .as_deref()
                    .is_none_or(|w| w.contains("not on your PATH")),
                &warning.unwrap_or(format!(
                    "pio: {}",
                    project::which(&pio).map_or(pio.clone(), |p| p.display().to_string())
                )),
            );
            if let Some(env) = envs.iter().find(|e| e.name == default_env) {
                if let Some(chip) = &env.chip {
                    let tools = project::esp_tools(chip);
                    match &tools.gdb {
                        Ok(gdb) => line(true, &format!("{} with DAP mode: {gdb}", chip.gdb_binary)),
                        Err(e) => line(false, e),
                    }
                    for rejected in &tools.rejected_gdb {
                        println!("        (not used: {rejected})");
                    }
                    match &tools.openocd {
                        Ok(openocd) => line(true, &format!("Espressif OpenOCD: {openocd}")),
                        Err(e) => line(false, e),
                    }
                    let name = generate::chip_display(&chip.name);
                    match chip.builtin_jtag_config {
                        Some(_) => line(true, &format!("{name} built-in USB-JTAG")),
                        None => line(
                            false,
                            &format!("{name} has no built-in USB-JTAG: needs an external probe"),
                        ),
                    }
                }
                let debug = env.build_type.as_deref() == Some("debug");
                line(
                    debug,
                    &if debug {
                        "build_type = debug".to_string()
                    } else {
                        format!(
                            "no build_type = debug in [env:{}]: variables may be optimized out",
                            env.name
                        )
                    },
                );
            }
        }
        Ok(Project::Rust(rust)) => {
            let probe_rs = project::which("probe-rs").or_else(|| {
                Some(project::home().join(".cargo/bin/probe-rs"))
                    .filter(|p| project::is_executable(p))
            });
            line(
                probe_rs.is_some(),
                &format!(
                    "probe-rs: {}",
                    probe_rs.map_or("not found".into(), |p| p.display().to_string())
                ),
            );
            line(
                rust.chip.is_some(),
                &format!(
                    "chip: {}",
                    rust.chip
                        .as_deref()
                        .unwrap_or("not set (Embed.toml or the probe-rs runner)")
                ),
            );
        }
        Err(e) => line(false, &e),
    }
    for name in ["debug.json", "tasks.json", "settings.json"] {
        let present = root.join(".zed").join(name).is_file();
        println!(
            "  --  .zed/{name}: {}",
            if present {
                "present"
            } else {
                "missing (zembed init)"
            }
        );
    }
    match project::probe_busy() {
        None => line(true, "probe free"),
        Some(busy) => line(false, &busy),
    }
    for problem in project::shell_problems() {
        line(false, &problem);
    }
    Ok(ok)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("init") => parse_options(&args[1..]).and_then(init).map(|_| true),
        Some("doctor") => parse_options(&args[1..]).and_then(doctor),
        Some("-h" | "--help" | "help") | None => {
            print!("{HELP}");
            Ok(true)
        }
        Some("-V" | "--version") => {
            println!("zembed {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        Some(other) => Err(format!("unknown command `{other}` (see zembed --help)")),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("zembed: {e}");
            ExitCode::from(2)
        }
    }
}
