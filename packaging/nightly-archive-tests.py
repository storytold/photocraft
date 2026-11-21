#!/usr/bin/env python3
"""Synthetic app-owned archive/output tests; no native build or executable launch."""
import gzip
import importlib.util
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("photo_nightly_archive", HERE / "nightly-archive.py")
ARCHIVE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ARCHIVE)


class ArchivePolicy(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.stage = self.root / "stage"
        self.stage.mkdir()
        self.output = self.root / "nightly.tar.gz"
        self.required = {"photocraft", "photocraft-nightly.png", "README.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md"}
        for name in self.required:
            (self.stage / name).write_bytes(b"generated fixture: " + name.encode())
        (self.stage / "photocraft").chmod(0o755)

    def test_flat_regular_ustar_with_all_notices(self):
        (self.stage / "OFL-Font.txt").write_text("generated fixture license", encoding="utf-8")
        ARCHIVE.linux_archive(self.stage, self.output)
        with tarfile.open(self.output) as archive:
            self.assertEqual({m.name for m in archive}, self.required | {"OFL-Font.txt"})
            self.assertTrue(all(m.isfile() and not m.pax_headers for m in archive))
            self.assertEqual(archive.extractfile("photocraft").read(), (self.stage / "photocraft").read_bytes())
        with gzip.open(self.output, "rb") as stream:
            while (header := stream.read(512)) != bytes(512):
                self.assertEqual(len(header), 512)
                self.assertIn(header[156:157], {b"0", b"\0"})
                self.assertEqual(header[257:263], b"ustar\0")
                size = int(header[124:136].rstrip(b"\0 ").lstrip(b" ") or b"0", 8)
                stream.read(((size + 511) // 512) * 512)

    def test_each_required_member_is_required_before_output(self):
        for name in self.required:
            with self.subTest(name=name):
                content = (self.stage / name).read_bytes()
                (self.stage / name).unlink()
                with self.assertRaisesRegex(ValueError, "required"):
                    ARCHIVE.linux_archive(self.stage, self.output)
                self.assertFalse(self.output.exists())
                (self.stage / name).write_bytes(content)

    def test_unexpected_cli_portable_or_installer_member_refuses(self):
        for name in ["photocraft-cli", "portable.txt", "photocraft.desktop"]:
            with self.subTest(name=name):
                bad = self.stage / name
                bad.write_bytes(b"generated fixture")
                with self.assertRaisesRegex(ValueError, "unexpected"):
                    ARCHIVE.linux_archive(self.stage, self.output)
                self.assertFalse(self.output.exists())
                bad.unlink()

    def test_symlink_refuses_before_output(self):
        icon = self.stage / "photocraft-nightly.png"
        icon.unlink()
        icon.symlink_to(self.stage / "README.md")
        with self.assertRaisesRegex(ValueError, "regular file"):
            ARCHIVE.linux_archive(self.stage, self.output)
        self.assertFalse(self.output.exists())

    def test_directory_refuses_before_output(self):
        icon = self.stage / "photocraft-nightly.png"
        icon.unlink()
        icon.mkdir()
        with self.assertRaisesRegex(ValueError, "regular file"):
            ARCHIVE.linux_archive(self.stage, self.output)
        self.assertFalse(self.output.exists())

    def test_long_notice_refuses_before_output(self):
        (self.stage / ("OFL-" + "x" * 100 + ".txt")).write_bytes(b"fixture")
        with self.assertRaisesRegex(ValueError, "short regular"):
            ARCHIVE.linux_archive(self.stage, self.output)
        self.assertFalse(self.output.exists())

    def test_case_colliding_notices_refuse_before_output(self):
        for name in ["OFL-Font.txt", "OFL-font.txt"]:
            (self.stage / name).write_bytes(b"fixture")
        with self.assertRaisesRegex(ValueError, "case-colliding"):
            ARCHIVE.linux_archive(self.stage, self.output)
        self.assertFalse(self.output.exists())

    def test_existing_archive_is_unchanged(self):
        self.output.write_bytes(b"owned earlier artifact")
        with self.assertRaises(FileExistsError):
            ARCHIVE.linux_archive(self.stage, self.output)
        self.assertEqual(self.output.read_bytes(), b"owned earlier artifact")


class ExclusiveOutput(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.source = self.root / "input archive"
        self.output = self.root / "final archive"
        self.source.write_bytes(b"generated archive fixture" * 1024)

    def publish(self):
        return subprocess.run(["bash", "-c", 'set -euo pipefail; source "$1"; nightly_publish_archive "$2" "$3"', "test", str(HERE / "nightly-common.sh"), str(self.source), str(self.output)], capture_output=True, text=True, timeout=15)

    def test_success_copies_exact_bytes(self):
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output.read_bytes(), self.source.read_bytes())

    def test_existing_output_is_never_replaced(self):
        self.output.write_bytes(b"existing artifact")
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual(self.output.read_bytes(), b"existing artifact")

    def test_missing_source_does_not_leave_a_fake_empty_artifact(self):
        self.source.unlink()
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
