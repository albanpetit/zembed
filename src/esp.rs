//! Espressif chips: which GDB and OpenOCD they need, and how to find them on
//! the user's machine.
//!
//! The extension runs in WASM and cannot list directories, so discovery is a
//! small POSIX shell script run on the user's machine (`process:exec`
//! capability). Its output is parsed here, which keeps the logic testable.

/// What Zembed needs to know about an Espressif chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// probe-rs / PlatformIO style name, e.g. `esp32s3`.
    pub name: String,
    /// Toolchain package holding GDB, e.g. `xtensa-esp-elf-gdb`.
    pub gdb_package: &'static str,
    /// GDB executable, e.g. `xtensa-esp32s3-elf-gdb`.
    pub gdb_binary: String,
    /// OpenOCD board file for the chip's built-in USB-JTAG, if it has one.
    pub builtin_jtag_config: Option<String>,
}

/// Chips from the most specific name to the least, so that `esp32` is tried
/// last when matching a board name.
const KNOWN: &[&str] = &[
    "esp32s3", "esp32s2", "esp32c61", "esp32c6", "esp32c5", "esp32c3", "esp32c2", "esp32h2",
    "esp32p4", "esp32",
];

/// Chips whose USB port doubles as a JTAG probe.
const WITH_USB_JTAG: &[&str] = &[
    "esp32s3", "esp32c3", "esp32c5", "esp32c6", "esp32h2", "esp32p4",
];

/// Builds the chip description from a name such as `esp32s3`, `ESP32-S3` or
/// `esp32_s3`.
pub fn chip(name: &str) -> Option<Chip> {
    let name = normalize(name);
    let name = KNOWN.iter().find(|known| **known == name)?;
    let xtensa = matches!(*name, "esp32" | "esp32s2" | "esp32s3");
    Some(Chip {
        name: name.to_string(),
        gdb_package: if xtensa {
            "xtensa-esp-elf-gdb"
        } else {
            "riscv32-esp-elf-gdb"
        },
        gdb_binary: if xtensa {
            format!("xtensa-{name}-elf-gdb")
        } else {
            "riscv32-esp-elf-gdb".to_string()
        },
        builtin_jtag_config: WITH_USB_JTAG
            .contains(name)
            .then(|| format!("board/{name}-builtin.cfg")),
    })
}

/// Guesses the chip from a PlatformIO board id such as `seeed_xiao_esp32s3`
/// or `esp32-c3-devkitm-1`.
pub fn chip_from_board_id(board: &str) -> Option<Chip> {
    let board = normalize(board);
    KNOWN
        .iter()
        .find(|known| board.contains(*known))
        .and_then(|known| chip(known))
}

/// Reads `build.mcu` from a PlatformIO board definition (JSON).
pub fn chip_from_board_json(json: &str) -> Option<Chip> {
    let board: serde_json::Value = serde_json::from_str(json).ok()?;
    chip(board.get("build")?.get("mcu")?.as_str()?)
}

fn normalize(name: &str) -> String {
    name.to_ascii_lowercase().replace(['-', '_', ' '], "")
}

/// Prints the board definition PlatformIO uses for board `$1`.
pub const BOARD_SCRIPT: &str = r#"for f in "$HOME"/.platformio/platforms/*/boards/"$1".json; do
  [ -f "$f" ] && cat "$f" && exit 0
done
exit 0"#;

/// Lists the GDB (`$1` package, `$2` executable) and OpenOCD installations
/// it can find, one per line:
/// `gdb<TAB>path<TAB>first line of --version<TAB>1 if Python works`
/// `openocd<TAB>path<TAB>first line of --version`
pub const DISCOVER_SCRIPT: &str = r#"pkg="$1"; bin="$2"; ard="$HOME/.arduino15/packages/esp32/tools"; mac="$HOME/Library/Arduino15/packages/esp32/tools"; idf="$HOME/.espressif/tools"; pio="$HOME/.platformio/packages"
seen=""
for p in "$ard/$pkg"/*/bin/"$bin" "$mac/$pkg"/*/bin/"$bin" "$idf/$pkg"/*/"$pkg"/bin/"$bin" "$pio/tool-$pkg/bin/$bin" "$(command -v "$bin" 2>/dev/null)"; do
  [ -n "$p" ] && [ -x "$p" ] || continue
  case "$seen" in *"|$p|"*) continue;; esac; seen="$seen|$p|"
  v=$("$p" --version 2>/dev/null | head -n 1)
  py=$("$p" -nx -batch -ex 'python print("zembed-python-ok")' 2>/dev/null | grep -c zembed-python-ok)
  printf 'gdb\t%s\t%s\t%s\n' "$p" "$v" "$py"
done
for p in "$ard"/openocd-esp32/*/bin/openocd "$mac"/openocd-esp32/*/bin/openocd "$idf"/openocd-esp32/*/openocd-esp32/bin/openocd "$pio/tool-openocd-esp32/bin/openocd" "$(command -v openocd 2>/dev/null)"; do
  [ -n "$p" ] && [ -x "$p" ] || continue
  case "$seen" in *"|$p|"*) continue;; esac; seen="$seen|$p|"
  v=$("$p" --version 2>&1 | head -n 1)
  printf 'openocd\t%s\t%s\n' "$p" "$v"
done
exit 0"#;

/// A GDB found on the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundGdb {
    pub path: String,
    pub version: Option<(u32, u32)>,
    pub python: bool,
}

/// An OpenOCD found on the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundOpenocd {
    pub path: String,
    pub version: String,
}

pub fn parse_discovery(output: &str) -> (Vec<FoundGdb>, Vec<FoundOpenocd>) {
    let mut gdbs = Vec::new();
    let mut openocds = Vec::new();
    for line in output.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["gdb", path, version, python, ..] => gdbs.push(FoundGdb {
                path: path.to_string(),
                version: gdb_version(version),
                python: python.trim() != "0" && !python.trim().is_empty(),
            }),
            ["openocd", path, version, ..] => openocds.push(FoundOpenocd {
                path: path.to_string(),
                version: version.trim().to_string(),
            }),
            _ => {}
        }
    }
    (gdbs, openocds)
}

/// `GNU gdb (esp-gdb) 17.1_20260402` -> `(17, 1)`.
fn gdb_version(line: &str) -> Option<(u32, u32)> {
    let last = line.split_whitespace().last()?;
    let mut numbers = last
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().ok());
    Some((numbers.next()??, numbers.next().flatten().unwrap_or(0)))
}

/// GDB's DAP mode appeared in GDB 14 and needs Python.
pub const MIN_GDB: (u32, u32) = (14, 0);

/// The newest GDB able to run in DAP mode, or an explanation.
pub fn pick_gdb(found: &[FoundGdb], binary: &str) -> Result<String, String> {
    let usable = found
        .iter()
        .filter(|gdb| gdb.python && gdb.version.is_some_and(|v| v >= MIN_GDB))
        .max_by_key(|gdb| gdb.version);
    if let Some(gdb) = usable {
        return Ok(gdb.path.clone());
    }
    let mut message =
        format!("no usable `{binary}` found. Zembed needs GDB 14 or newer with Python support. ");
    if found.is_empty() {
        message.push_str(
            "None was found in the Arduino IDE, ESP-IDF or PlatformIO tools, nor on your PATH. ",
        );
    } else {
        message.push_str("Found, but not usable: ");
        let rejected: Vec<String> = found
            .iter()
            .map(|gdb| {
                let why = match gdb.version {
                    Some(v) if v < MIN_GDB => format!("version {}.{} is too old", v.0, v.1),
                    None => "unknown version".to_string(),
                    Some(_) => "no Python support for your Python version".to_string(),
                };
                format!("{} ({why})", gdb.path)
            })
            .collect();
        message.push_str(&rejected.join("; "));
        message.push_str(". ");
    }
    message.push_str(
        "Install a recent \"esp32\" board package in the Arduino IDE (Boards Manager), or set \
         `gdb` in .zed/debug.json.",
    );
    Err(message)
}

/// The newest Espressif OpenOCD, or an explanation.
pub fn pick_openocd(found: &[FoundOpenocd]) -> Result<String, String> {
    found
        .iter()
        .filter(|openocd| openocd.version.contains("esp32"))
        .max_by(|a, b| a.version.cmp(&b.version))
        .map(|openocd| openocd.path.clone())
        .ok_or_else(|| {
            let generic: Vec<&str> = found.iter().map(|o| o.path.as_str()).collect();
            let mut message = "no Espressif OpenOCD (openocd-esp32) found".to_string();
            if !generic.is_empty() {
                message.push_str(&format!(
                    "; these are generic builds without ESP32 support: {}",
                    generic.join(", ")
                ));
            }
            message.push_str(
                ". Install a recent \"esp32\" board package in the Arduino IDE (Boards \
                 Manager), or set `openocd` in .zed/debug.json.",
            );
            message
        })
}

/// Refuses to touch the probe while another debugger holds it: OpenOCD
/// would then write to the USB device in the middle of that session. Else
/// runs `$@`, with a time limit when `timeout` exists (not on stock macOS).
/// The brackets keep the patterns from matching this script's own command
/// line.
pub const PROBE_CHECK_SCRIPT: &str = r#"if command -v pgrep >/dev/null 2>&1; then
  busy=$(pgrep -f 'openocd.*gdb_port [p]ipe|probe-rs [d]ap-server|probe-rs (run|attach|[g]db)' | tr '\n' ' ' | sed 's/ $//')
  if [ -n "$busy" ]; then
    echo "Error: zembed: the probe is already used by another debugger (process $busy)" >&2
    exit 3
  fi
fi
if command -v timeout >/dev/null 2>&1; then exec timeout 20 "$@"; else exec "$@"; fi"#;

/// Messages of OpenOCD that mean the probe or chip cannot be reached.
const PROBE_FAILURES: &[&str] = &[
    "already used by another debugger",
    "could not find or open device",
    "Examination failed",
    "LIBUSB_ERROR",
    "unable to open",
    "Can't find",
    "couldn't open",
    "No device found",
    "Error: libusb",
];

/// Turns the result of `openocd ... -c "init; shutdown"` into an error
/// message when the probe or chip could not be reached.
pub fn probe_check_error(success: bool, log: &str) -> Option<String> {
    let failed_line = log
        .lines()
        .find(|line| PROBE_FAILURES.iter().any(|needle| line.contains(needle)));
    if success && failed_line.is_none() {
        return None;
    }
    let errors: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("Error"))
        .collect();
    let detail = match (failed_line, errors.last()) {
        (Some(line), _) => line.trim().to_string(),
        (None, Some(line)) => line.trim().to_string(),
        (None, None) => log
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string(),
    };
    Some(format!(
        "OpenOCD could not reach the chip: {detail}\nIs the board plugged in? Is another \
         debug session, probe-rs or OpenOCD still using the probe? Stop it (for example \
         `pkill -f \"probe-rs dap-server\"; pkill openocd`) and try again."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_chips() {
        let s3 = chip("ESP32-S3").unwrap();
        assert_eq!(s3.gdb_binary, "xtensa-esp32s3-elf-gdb");
        assert_eq!(s3.gdb_package, "xtensa-esp-elf-gdb");
        assert_eq!(
            s3.builtin_jtag_config.as_deref(),
            Some("board/esp32s3-builtin.cfg")
        );
        let c3 = chip("esp32c3").unwrap();
        assert_eq!(c3.gdb_binary, "riscv32-esp-elf-gdb");
        assert_eq!(
            c3.builtin_jtag_config.as_deref(),
            Some("board/esp32c3-builtin.cfg")
        );
        assert_eq!(chip("esp32").unwrap().builtin_jtag_config, None);
        assert_eq!(chip("stm32f411"), None);
    }

    #[test]
    fn guesses_chip_from_board() {
        assert_eq!(
            chip_from_board_id("seeed_xiao_esp32s3").unwrap().name,
            "esp32s3"
        );
        assert_eq!(
            chip_from_board_id("esp32-c3-devkitm-1").unwrap().name,
            "esp32c3"
        );
        assert_eq!(
            chip_from_board_id("esp32-c6-devkitc-1").unwrap().name,
            "esp32c6"
        );
        assert_eq!(chip_from_board_id("esp32dev").unwrap().name, "esp32");
        assert_eq!(chip_from_board_id("nucleo_f411re"), None);
        let json = r#"{"build": {"mcu": "esp32s3", "f_cpu": "240000000L"}, "name": "XIAO"}"#;
        assert_eq!(chip_from_board_json(json).unwrap().name, "esp32s3");
        assert_eq!(
            chip_from_board_json(r#"{"build": {"mcu": "stm32f411ret6"}}"#),
            None
        );
    }

    #[test]
    fn picks_newest_usable_gdb() {
        let output = "gdb\t/pio/xtensa-esp32s3-elf-gdb\tGNU gdb (esp-gdb) 12.1_20221002\t0\n\
                      gdb\t/ard/16/xtensa-esp32s3-elf-gdb\tGNU gdb (esp-gdb) 16.2_20250324\t1\n\
                      gdb\t/ard/17/xtensa-esp32s3-elf-gdb\tGNU gdb (esp-gdb) 17.1_20260402\t1\n\
                      openocd\t/usr/bin/openocd\tOpen On-Chip Debugger 0.12.0\n\
                      openocd\t/ard/a/openocd\tOpen On-Chip Debugger v0.12.0-esp32-20250707 (2025-07-07-12:00)\n\
                      openocd\t/ard/b/openocd\tOpen On-Chip Debugger v0.12.0-esp32-20260424 (2026-04-24-12:00)\n";
        let (gdbs, openocds) = parse_discovery(output);
        assert_eq!(gdbs[0].version, Some((12, 1)));
        assert!(!gdbs[0].python);
        assert_eq!(
            pick_gdb(&gdbs, "x").unwrap(),
            "/ard/17/xtensa-esp32s3-elf-gdb"
        );
        assert_eq!(pick_openocd(&openocds).unwrap(), "/ard/b/openocd");
    }

    #[test]
    fn explains_missing_tools() {
        let (gdbs, openocds) = parse_discovery(
            "gdb\t/pio/gdb\tGNU gdb (esp-gdb) 12.1_20221002\t1\n\
             gdb\t/new/gdb\tGNU gdb (esp-gdb) 17.1_20260402\t0\n\
             openocd\t/usr/bin/openocd\tOpen On-Chip Debugger 0.12.0\n",
        );
        let error = pick_gdb(&gdbs, "xtensa-esp32s3-elf-gdb").unwrap_err();
        assert!(error.contains("12.1 is too old"), "{error}");
        assert!(error.contains("no Python support"), "{error}");
        let error = pick_openocd(&openocds).unwrap_err();
        assert!(error.contains("/usr/bin/openocd"), "{error}");
        assert!(pick_gdb(&[], "g").unwrap_err().contains("None was found"));
    }

    #[test]
    fn reads_probe_check() {
        assert_eq!(
            probe_check_error(true, "Info : [esp32s3.cpu0] Examination succeed\n"),
            None
        );
        let busy = "Info : esp_usb_jtag: serial (1C:DB)\nError: esp_usb_jtag: could not find or open device!\n";
        assert!(probe_check_error(false, busy)
            .unwrap()
            .contains("could not find or open device"));
        let examination = "Error: [esp32s3.cpu0] Examination failed\n";
        assert!(probe_check_error(true, examination).is_some());
        assert!(probe_check_error(false, "Error: something else\n")
            .unwrap()
            .contains("something else"));
        let busy = "Error: zembed: the probe is already used by another debugger (process 42)\n";
        assert!(probe_check_error(false, busy)
            .unwrap()
            .contains("process 42"));
    }
}
