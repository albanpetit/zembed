//! What `zembed` learns about a project and the machine.
//!
//! Chip detection and tool discovery are the extension's own code
//! (`src/esp.rs`, `src/platformio.rs`) and scripts, so the command and the
//! debug adapter always agree.

use crate::esp::{self, Chip};
use crate::platformio;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct PioEnv {
    pub name: String,
    pub board: Option<String>,
    /// Human name of the board, from PlatformIO's board definition.
    pub board_name: Option<String>,
    pub chip: Option<Chip>,
    pub build_type: Option<String>,
}

pub struct RustProject {
    pub crate_name: String,
    /// Chip set in Embed.toml or .cargo/config.toml, which the extension
    /// reads itself.
    pub chip: Option<String>,
    /// target/<triple>/debug/<crate>, when the target is set.
    pub program: Option<String>,
}

pub enum Project {
    PlatformIo {
        envs: Vec<PioEnv>,
        default_env: String,
    },
    Rust(RustProject),
}

impl Project {
    pub fn read(root: &Path) -> Result<Project, String> {
        let ini_path = root.join(platformio::PROJECT_FILE);
        if let Ok(ini) = std::fs::read_to_string(&ini_path) {
            let names = env_names(&ini);
            if names.is_empty() {
                return Err("platformio.ini has no [env:...] section".into());
            }
            let default_env = platformio::default_env(&ini)
                .filter(|env| names.contains(env))
                .unwrap_or_else(|| names[0].clone());
            let envs = names
                .into_iter()
                .map(|name| PioEnv::read(&ini, name))
                .collect();
            return Ok(Project::PlatformIo { envs, default_env });
        }
        if let Ok(cargo) = std::fs::read_to_string(root.join("Cargo.toml")) {
            let read = |file: &str| std::fs::read_to_string(root.join(file)).unwrap_or_default();
            let config = read(".cargo/config.toml");
            let embed = read("Embed.toml");
            let crate_name = toml_string(&cargo, "name").unwrap_or_else(|| "firmware".into());
            let target = toml_string(&config, "target");
            let chip = toml_string(&embed, "chip").or_else(|| runner_chip(&config));
            let program = target.map(|t| format!("target/{t}/debug/{crate_name}"));
            return Ok(Project::Rust(RustProject {
                crate_name,
                chip,
                program,
            }));
        }
        Err(format!(
            "{}: neither platformio.ini nor Cargo.toml. Run zembed from the firmware project's folder.",
            root.display()
        ))
    }
}

impl PioEnv {
    fn read(ini: &str, name: String) -> PioEnv {
        let value = |key| platformio::env_value(ini, &name, key);
        let board = value("board");
        let definition = board.as_deref().and_then(board_definition);
        let board_name = definition
            .as_deref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .and_then(|json| json.get("name")?.as_str().map(str::to_string));
        let chip = value("board_build.mcu")
            .and_then(|mcu| esp::chip(&mcu))
            .or_else(|| definition.as_deref().and_then(esp::chip_from_board_json))
            .or_else(|| board.as_deref().and_then(esp::chip_from_board_id));
        PioEnv {
            build_type: value("build_type"),
            name,
            board,
            board_name,
            chip,
        }
    }
}

/// `[env:<name>]` sections, in file order.
pub fn env_names(ini: &str) -> Vec<String> {
    ini.lines()
        .filter_map(|line| line.trim().strip_prefix("[env:")?.split(']').next())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

fn toml_string(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// The chip of a `runner = "probe-rs run --chip <CHIP>"` line.
fn runner_chip(config: &str) -> Option<String> {
    let rest = config.split("--chip").nth(1)?;
    let chip = rest
        .trim_start_matches(['=', ' '])
        .split(|c: char| c.is_whitespace() || c == '"')
        .next()?;
    (!chip.is_empty()).then(|| chip.to_string())
}

// ------------------------------------------------------------- the machine

/// Runs one of the extension's shell scripts, as the extension does.
fn sh(script: &str, args: &[&str]) -> Option<std::process::Output> {
    Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("zembed")
        .args(args)
        .output()
        .ok()
}

/// PlatformIO's JSON definition of a board.
fn board_definition(board: &str) -> Option<String> {
    let out = sh(esp::BOARD_SCRIPT, &[board])?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub struct Tools {
    pub gdb: Result<String, String>,
    /// GDBs found but not usable (no DAP mode), for `doctor`.
    pub rejected_gdb: Vec<String>,
    pub openocd: Result<String, String>,
}

/// GDB and OpenOCD for an Espressif chip, picked like the debug adapter does.
pub fn esp_tools(chip: &Chip) -> Tools {
    let output = sh(esp::DISCOVER_SCRIPT, &[chip.gdb_package, &chip.gdb_binary])
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let (gdbs, openocds) = esp::parse_discovery(&output);
    let gdb = esp::pick_gdb(&gdbs, &chip.gdb_binary);
    let rejected_gdb = gdbs
        .iter()
        .filter(|g| gdb.as_ref().map_or(true, |chosen| *chosen != g.path))
        .map(|g| g.path.clone())
        .collect();
    Tools {
        gdb,
        rejected_gdb,
        openocd: esp::pick_openocd(&openocds),
    }
}

/// The debugger that holds the probe, if any (the extension's own check).
pub fn probe_busy() -> Option<String> {
    let out = sh(esp::PROBE_CHECK_SCRIPT, &["true"])?;
    (out.status.code() == Some(3)).then(|| {
        let text = String::from_utf8_lossy(&out.stderr);
        text.trim()
            .trim_start_matches("Error: zembed: ")
            .to_string()
    })
}

pub fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|dir| Path::new(dir).join(name))
        .find(|path| is_executable(path))
}

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The `pio` command to write in the files, and a warning when it is not on
/// the PATH (Zed then would not find it either).
pub fn pio_command() -> (String, Option<String>) {
    if which("pio").is_some() {
        return ("pio".into(), None);
    }
    let home = home();
    for candidate in [
        home.join(".platformio/penv/bin/pio"),
        home.join(".local/bin/pio"),
    ] {
        if is_executable(&candidate) {
            let path = candidate.display().to_string();
            return (
                path.clone(),
                Some(format!("`pio` is not on your PATH: the files use {path}.")),
            );
        }
    }
    (
        "pio".into(),
        Some("PlatformIO (`pio`) was not found. Install PlatformIO Core: https://docs.platformio.org/en/latest/core/installation/".into()),
    )
}

/// A `~/.bashrc` that replaces bash with another shell makes Zed's tasks
/// open an empty terminal, since Zed runs them with `bash -i -c`.
pub fn shell_problems() -> Vec<String> {
    let Ok(bashrc) = std::fs::read_to_string(home().join(".bashrc")) else {
        return Vec::new();
    };
    bashrc
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim_start();
            ["exec zsh", "exec fish", "exec nu"].iter().any(|exec| {
                line.strip_prefix(exec).is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
            })
        })
        .map(|(i, line)| {
            format!(
                "~/.bashrc line {} runs `{}` unconditionally: Zed's tasks will open an empty terminal. \
                 Replace it with: [ -z \"$BASH_EXECUTION_STRING\" ] && {}",
                i + 1,
                line.trim(),
                line.trim()
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_environments() {
        let ini = "[platformio]\ndefault_envs = b\n[env]\nx = 1\n[env:a]\n[env:b] ; comment\n";
        assert_eq!(env_names(ini), ["a", "b"]);
    }

    #[test]
    fn reads_rust_settings() {
        assert_eq!(
            runner_chip("runner = \"probe-rs run --chip RP2040\"").as_deref(),
            Some("RP2040")
        );
        assert_eq!(
            runner_chip("runner = \"probe-rs run --chip=STM32F411RETx --x\"").as_deref(),
            Some("STM32F411RETx")
        );
        assert_eq!(
            toml_string("[build]\ntarget = \"thumbv6m-none-eabi\"\n", "target").as_deref(),
            Some("thumbv6m-none-eabi")
        );
    }
}
