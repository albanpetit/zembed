"""Tests of the `zembed` command. Run: python3 -m unittest discover cli"""

import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ZEMBED = Path(__file__).with_name("zembed")


def jsonc(text):
    return json.loads(re.sub(r"(?m)^\s*//.*$", "", text))


class Init(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())

    def zembed(self, *args):
        result = subprocess.run([sys.executable, str(ZEMBED), *args], capture_output=True, text=True, stdin=subprocess.DEVNULL)
        return result

    def project(self, ini):
        (self.dir / "platformio.ini").write_text(ini)

    def read(self, name):
        return jsonc((self.dir / ".zed" / name).read_text())

    def test_esp32s3_needs_no_tool_path(self):
        self.project("[env:xiao]\nplatform = espressif32\nboard = seeed_xiao_esp32s3\n")
        self.assertEqual(self.zembed("init", str(self.dir), "--yes", "--lang", "fr").returncode, 0)
        debug = self.read("debug.json")
        self.assertEqual([d["adapter"] for d in debug], ["gdb-openocd", "gdb-openocd"])
        self.assertEqual(debug[0]["build"]["args"], ["run", "-t", "upload"])
        self.assertNotIn("gdb", debug[0])
        self.assertNotIn("openocdArgs", debug[0])
        labels = [t["label"] for t in self.read("tasks.json")]
        self.assertIn("pio: upload + monitor", labels)
        self.assertIn("Debug: libérer la sonde", labels)
        self.assertIn("query-driver", (self.dir / ".zed/settings.json").read_text())

    def test_keeps_existing_files_unless_forced(self):
        self.project("[env:xiao]\nboard = seeed_xiao_esp32s3\n")
        (self.dir / ".zed").mkdir()
        (self.dir / ".zed/tasks.json").write_text("[]\n")
        out = self.zembed("init", str(self.dir), "--yes").stdout
        self.assertIn("--force", out)
        self.assertEqual((self.dir / ".zed/tasks.json").read_text(), "[]\n")
        self.zembed("init", str(self.dir), "--yes", "--force")
        self.assertEqual((self.dir / ".zed/tasks.json.bak").read_text(), "[]\n")
        self.assertTrue(self.read("tasks.json"))
        again = self.zembed("init", str(self.dir), "--yes", "--force", "--lang", "en").stdout
        self.assertEqual(again.count("unchanged"), 3)

    def test_other_environment_and_external_probe(self):
        self.project("[platformio]\ndefault_envs = a\n[env]\nframework = arduino\n[env:a]\nboard = esp32-s3-devkitc-1\n[env:b]\nboard = esp32dev\n")
        self.zembed("init", str(self.dir), "--yes", "--env", "b", "--tasks", "build,monitor")
        debug = self.read("debug.json")
        self.assertEqual(debug[0]["pioEnv"], "b")
        self.assertEqual(debug[0]["build"]["args"], ["run", "-t", "upload", "-e", "b"])
        self.assertEqual(debug[0]["openocdArgs"][0], "-f")
        self.assertEqual([t["command"] for t in self.read("tasks.json")], ["pio run -e b", "pio device monitor -e b"])

    def test_probe_rs_board_and_debug_build_type(self):
        self.project("[env:nucleo]\nplatform = ststm32\nboard = nucleo_f411re\n\n")
        self.zembed("init", str(self.dir), "--yes", "--chip", "STM32F411RETx", "--debug-build", "--tasks", "")
        debug = self.read("debug.json")
        self.assertEqual(debug[0]["adapter"], "probe-rs")
        self.assertEqual(debug[0]["chip"], "STM32F411RETx")
        self.assertFalse((self.dir / ".zed/tasks.json").exists())
        self.assertIn("board = nucleo_f411re\nbuild_type = debug\n", (self.dir / "platformio.ini").read_text())

    def test_never_edits_platformio_ini_by_default(self):
        self.project("[env:xiao]\nboard = seeed_xiao_esp32s3\n")
        self.zembed("init", str(self.dir), "--yes")
        self.assertNotIn("build_type", (self.dir / "platformio.ini").read_text())

    def test_dry_run_writes_nothing(self):
        self.project("[env:xiao]\nboard = seeed_xiao_esp32s3\n")
        out = self.zembed("init", str(self.dir), "--yes", "--dry-run").stdout
        self.assertIn('"adapter": "gdb-openocd"', out)
        self.assertFalse((self.dir / ".zed").exists())

    def test_rust(self):
        (self.dir / ".cargo").mkdir()
        (self.dir / "Cargo.toml").write_text('[package]\nname = "blinky"\n')
        (self.dir / ".cargo/config.toml").write_text('[build]\ntarget = "thumbv6m-none-eabi"\nrunner = "probe-rs run --chip RP2040"\n')
        self.zembed("init", str(self.dir), "--yes", "--no-clangd")
        debug = self.read("debug.json")
        self.assertEqual(debug[0]["program"], "target/thumbv6m-none-eabi/debug/blinky")
        self.assertNotIn("chip", debug[0])  # detected by the extension from .cargo/config.toml
        self.assertEqual([t["command"] for t in self.read("tasks.json")][:2], ["cargo build", "cargo run"])
        self.assertFalse((self.dir / ".zed/settings.json").exists())

    def test_rejects_unknown_tasks_and_folders(self):
        self.assertIn("unknown task", self.zembed("init", str(self.dir), "--tasks", "foo").stderr)
        self.assertNotEqual(self.zembed("init", str(self.dir), "--yes").returncode, 0)


if __name__ == "__main__":
    unittest.main()
