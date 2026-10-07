#!/usr/bin/env python3
"""Packaging contract tests using fixture binaries; no Rust builds or native tooling.

Run: python3 scripts/test_release_packaging.py
"""
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ReleasePackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="photocraft-packaging-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        self.root.mkdir()
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "9.8.7"\n')
        (self.root / "LICENSE-MIT").write_text("fixture license\n")
        (self.root / "README.md").write_text("PhotoCraft fixture\n")
        (self.root / "packaging").mkdir()
        shutil.copyfile(ROOT / "packaging/env.sh", self.root / "packaging/env.sh")
        for platform in ("linux", "macos"):
            shutil.copytree(ROOT / "packaging" / platform, self.root / "packaging" / platform)
        icon = self.root / "assets/app-icon/hicolor/256x256/apps/ai.storyteller.photocraft.png"
        icon.parent.mkdir(parents=True)
        icon.write_bytes(b"fixture icon")
        self.tools = Path(self.temp.name) / "tools"
        self.tools.mkdir()
        self.target = Path(self.temp.name) / "target"
        self.dist = Path(self.temp.name) / "dist"
        self.env = dict(os.environ, PATH=str(self.tools) + os.pathsep + os.environ["PATH"],
                        CARGO_TARGET_DIR=str(self.target), DIST=str(self.dist),
                        PHOTOCRAFT_BUILD_SHA="fixture", PHOTOCRAFT_BUILD_DATE="2026-10-06")
        self.env.pop("PHOTOCRAFT_VERSION", None)
        self.mock("uname", 'printf "%s\\n" "$FIXTURE_ARCH"')
        self.mock("objcopy", 'printf "private debug data\\n" > "$3"')
        self.mock("strip", "exit 0")
        # BSD install handles -D differently; emulate the GNU forms package.sh uses.
        installer = self.tools / "install"
        installer.write_text(
            "#!" + sys.executable + "\n"
            "import os, pathlib, shutil, sys\n"
            "flag, source, destination = sys.argv[1:]\n"
            "if flag not in ('-Dm755', '-Dm644'):\n"
            "    raise SystemExit('unexpected fixture install arguments: ' + repr(sys.argv[1:]))\n"
            "path = pathlib.Path(destination)\n"
            "path.parent.mkdir(parents=True, exist_ok=True)\n"
            "shutil.copyfile(source, path)\n"
            "os.chmod(path, int(flag[3:], 8))\n"
        )
        installer.chmod(0o755)
        # Host-installed validators must not affect fixture tests.
        self.mock("desktop-file-validate", "exit 0")
        self.mock("appstreamcli", "exit 0")
        self.mock("cargo", 'echo "unexpected cargo invocation" >&2; exit 99')

    def mock(self, name, body):
        path = self.tools / name
        path.write_text("#!/bin/sh\nset -eu\n" + body + "\n")
        path.chmod(0o755)

    def fixture_binaries(self, arch):
        self.env["FIXTURE_ARCH"] = arch
        normalized = "aarch64" if arch == "arm64" else arch
        triple = normalized + "-unknown-linux-gnu"
        folder = self.target / triple / "native-release"
        folder.mkdir(parents=True)
        for name in ("photocraft", "photocraft-cli"):
            executable = folder / name
            executable.write_text('#!/bin/sh\nprintf "%s\\n" "' + name + ' ' + triple + ' fixture"\n')
            executable.chmod(0o755)
        return normalized, triple

    def run_script(self, platform, *args):
        return subprocess.run(["bash", str(self.root / "packaging" / platform / "package.sh"), *args],
                              env=self.env, capture_output=True, text=True, timeout=30)

    def test_linux_separate_architecture_downloads(self):
        for arch in ("x86_64", "aarch64", "arm64"):
            with self.subTest(arch=arch):
                if self.target.exists():
                    shutil.rmtree(self.target)
                if self.dist.exists():
                    shutil.rmtree(self.dist)
                normalized, triple = self.fixture_binaries(arch)
                result = self.run_script("linux", "--skip-build", "--formats", "tar")
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("photocraft-cli " + triple + " fixture", result.stdout)
                self.assertIn("photocraft " + triple + " fixture", result.stdout)
                gui_name = "photocraft-9.8.7-linux-" + normalized
                cli_name = "photocraft-cli-9.8.7-linux-" + normalized
                self.assertEqual({p.name for p in self.dist.iterdir()},
                                 {gui_name + ".tar.gz", cli_name + ".tar.gz"})
                with tarfile.open(self.dist / (gui_name + ".tar.gz")) as archive:
                    names = archive.getnames()
                    self.assertIn(gui_name + "/bin/photocraft", names)
                    self.assertFalse(any(Path(name).name == "photocraft-cli" for name in names))
                    self.assertFalse(any(name.endswith(".debug") for name in names))
                with tarfile.open(self.dist / (cli_name + ".tar.gz")) as archive:
                    executable = archive.getmember(cli_name + "/bin/photocraft-cli")
                    self.assertTrue(executable.mode & 0o111)
                    self.assertIn(cli_name + "/share/doc/photocraft-cli/LICENSE-MIT", archive.getnames())
                    self.assertFalse(any(Path(name).name == "photocraft" for name in archive.getnames()))
                    payload = archive.extractfile(executable).read().decode()
                    self.assertIn(triple, payload)
                symbols = self.target / "release-symbols" / ("linux-" + normalized)
                for name in ("photocraft", "photocraft-cli"):
                    self.assertEqual((symbols / (name + ".debug")).read_text(), "private debug data\n")
                self.assertFalse(symbols.is_relative_to(self.dist))

    def test_font_licenses_survive_all_linux_payloads(self):
        self.fixture_binaries("x86_64")
        font_root = self.root / "craft-fonts"
        notice = font_root / "fonts/fixture/OFL.txt"
        notice.parent.mkdir(parents=True)
        notice.write_text("fixture OFL notice")
        self.env["CRAFT_FONTS_DIR"] = str(font_root)
        tool = self.tools / "appimagetool"
        self.mock("appimagetool", 'test -f "$2/usr/share/doc/photocraft/OFL-fixture.txt" || exit 24\n'
                  'test ! -f "$2/usr/bin/photocraft-cli" || exit 25\n'
                  'printf "fixture appimage\\n" > "$3"')
        self.env["APPIMAGETOOL"] = str(tool)
        result = self.run_script("linux", "--skip-build", "--formats", "tar appimage")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.dist / "photocraft-9.8.7-linux-x86_64.AppImage").is_file())
        for prefix, doc in (("photocraft", "photocraft"), ("photocraft-cli", "photocraft-cli")):
            with tarfile.open(self.dist / (prefix + "-9.8.7-linux-x86_64.tar.gz")) as archive:
                expected = prefix + "-9.8.7-linux-x86_64/share/doc/" + doc + "/OFL-fixture.txt"
                self.assertIn(expected, archive.getnames())

    def test_diagnostic_or_strip_failure_stops_archives(self):
        self.fixture_binaries("x86_64")
        for tool in ("objcopy", "strip"):
            with self.subTest(tool=tool):
                self.mock(tool, "exit 23")
                result = self.run_script("linux", "--skip-build", "--formats", "tar")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(list(self.dist.glob("*.tar.gz")), [])
                self.mock(tool, 'printf "private debug data\\n" > "$3"' if tool == "objcopy" else "exit 0")

    def test_mac_rejects_universal_before_build(self):
        self.env["FIXTURE_ARCH"] = "x86_64"
        result = self.run_script("macos", "--arch", "universal", "--skip-build")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("unknown --arch universal", result.stderr)
        self.assertEqual(list(self.dist.iterdir()), [])

    def test_mac_missing_arch_value(self):
        self.env["FIXTURE_ARCH"] = "x86_64"
        result = self.run_script("macos", "--arch")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("--arch needs a value", result.stderr)


if __name__ == "__main__":
    unittest.main()
