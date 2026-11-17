//! Helpers to understand a PlatformIO project (`platformio.ini`).
//!
//! Debug adapters use them to find the firmware PlatformIO builds, so the
//! user does not have to spell out `.pio/build/<env>/firmware.elf`.

pub const PROJECT_FILE: &str = "platformio.ini";

/// The environment PlatformIO builds when none is given: the first entry of
/// `default_envs` in `[platformio]`, otherwise the first `[env:<name>]`.
pub fn default_env(ini: &str) -> Option<String> {
    let mut section = "";
    let mut first_env = None;
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
            section = name.trim();
            if let Some(env) = section.strip_prefix("env:") {
                first_env.get_or_insert(env.trim());
            }
        } else if section == "platformio" {
            if let Some(("default_envs", value)) =
                line.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
            {
                let first = value.split(',').next().map(str::trim).unwrap_or("");
                // `${...}` interpolation and multi-line lists are not resolved
                // here; fall back to the first [env:...] in that case.
                if !first.is_empty() && !first.contains("${") {
                    return Some(first.to_string());
                }
            }
        }
    }
    first_env.filter(|env| !env.is_empty()).map(str::to_string)
}

/// Value of `key` for an environment: from `[env:<env>]`, otherwise from
/// the shared `[env]` section. `extends` and `${...}` are not resolved.
pub fn env_value(ini: &str, env: &str, key: &str) -> Option<String> {
    let wanted = format!("env:{env}");
    let mut section = String::new();
    let mut own = None;
    let mut shared = None;
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
            section = name.trim().to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            if k.trim() != key {
                continue;
            }
            let value = v.split(" ;").next().unwrap_or("").trim().to_string();
            if value.is_empty() {
                continue;
            }
            if section == wanted {
                own.get_or_insert(value);
            } else if section == "env" {
                shared.get_or_insert(value);
            }
        }
    }
    own.or(shared)
}

/// Path of the ELF PlatformIO builds for an environment, relative to the
/// project root (with the default `build_dir`).
pub fn firmware_elf(env: &str) -> String {
    format!(".pio/build/{env}/firmware.elf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_default_env() {
        let ini = "; comment\n[platformio]\ndefault_envs = nucleo, esp32\n\n[env]\nframework = arduino\n\n[env:esp32]\nboard = esp32dev\n\n[env:nucleo]\nboard = nucleo_f411re\n";
        assert_eq!(default_env(ini).as_deref(), Some("nucleo"));
    }

    #[test]
    fn falls_back_to_first_env() {
        let ini = "[env:bluepill]\nboard = bluepill_f103c8\n[env:other]\n";
        assert_eq!(default_env(ini).as_deref(), Some("bluepill"));
        let interpolated = "[platformio]\ndefault_envs = ${sysenv.ENVS}\n[env:a]\n";
        assert_eq!(default_env(interpolated).as_deref(), Some("a"));
        assert_eq!(default_env("[platformio]\nsrc_dir = main\n"), None);
    }

    #[test]
    fn reads_env_values() {
        let ini = "[env]\nboard = shared\nframework = arduino\n\n[env:xiao]\nboard = seeed_xiao_esp32s3 ; comment\n[env:other]\n";
        assert_eq!(env_value(ini, "xiao", "board").as_deref(), Some("seeed_xiao_esp32s3"));
        assert_eq!(env_value(ini, "other", "board").as_deref(), Some("shared"));
        assert_eq!(env_value(ini, "xiao", "framework").as_deref(), Some("arduino"));
        assert_eq!(env_value(ini, "xiao", "platform"), None);
    }

    #[test]
    fn builds_elf_path() {
        assert_eq!(firmware_elf("nucleo"), ".pio/build/nucleo/firmware.elf");
    }
}
