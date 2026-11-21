#!/usr/bin/env python3
"""Generated metadata/sentinel fixtures; no native binary, signature, archive or network."""
import importlib.util
from pathlib import Path
import plistlib
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("photo_nightly_plist", HERE / "nightly_plist.py")
POLICY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(POLICY)
COMMIT = "a" * 40
VERSION = "0.23.1-nightly.20261121.12345.1.g" + COMMIT[:12]


class NightlyMetadata(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.template = self.root / "Info.plist.in"
        self.original = (HERE / "Info.plist.in").read_bytes()
        self.template.write_bytes(self.original)
        self.output = self.root / "Info.plist"

    def test_nightly_transform_removes_associations_without_changing_stable_or_platform_metadata(self):
        stable = plistlib.loads(self.original)
        stable["CFBundleURLTypes"] = [{"CFBundleURLSchemes": ["generated-test-scheme"]}]
        prepared = plistlib.dumps(stable)
        self.template.write_bytes(prepared)
        POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        rendered = plistlib.loads(self.output.read_bytes())
        for key in POLICY.ASSOCIATIONS:
            self.assertNotIn(key, rendered)
        self.assertNotEqual(rendered["CFBundleIdentifier"], stable["CFBundleIdentifier"])
        self.assertEqual(rendered["CFBundleIdentifier"], "ai.storyteller.photocraft.nightly")
        self.assertEqual(rendered["CFBundleExecutable"], "PhotoCraft")
        self.assertEqual(rendered["PhotoCraftVersion"], VERSION)
        self.assertEqual(rendered["PhotoCraftBuildCommit"], COMMIT)
        self.assertTrue(VERSION.startswith(rendered["CFBundleVersion"] + "-nightly."))
        for key in ("LSMinimumSystemVersion", "CFBundleLocalizations", "NSHighResolutionCapable", "NSHumanReadableCopyright"):
            self.assertEqual(rendered[key], stable[key])
        self.assertEqual(self.template.read_bytes(), prepared)
        self.assertEqual((HERE / "Info.plist.in").read_bytes(), self.original)

    def test_bad_identity_or_template_refuses_before_creating_output(self):
        for version, commit in (("0.23.1", COMMIT), (VERSION, "b" * 40), (VERSION, "not-a-commit"), (VERSION.replace("20261121", "20261399"), COMMIT)):
            with self.subTest(version=version, commit=commit):
                with self.assertRaises(ValueError):
                    POLICY.write_plist(self.template, self.output, version, commit)
                self.assertFalse(self.output.exists())
                self.assertEqual(self.template.read_bytes(), self.original)
        self.template.write_bytes(plistlib.dumps({"CFBundlePackageType": "generated-not-an-app"}))
        with self.assertRaises(ValueError):
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        self.assertFalse(self.output.exists())

    def test_exclusive_output_preserves_existing_bytes_and_symlink_targets(self):
        sentinel = b"generated owned sentinel, not an existing app or user file"
        self.output.write_bytes(sentinel)
        with self.assertRaises(FileExistsError):
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        self.assertEqual(self.output.read_bytes(), sentinel)
        self.output.unlink()
        external = self.root / "sentinel"
        external.write_bytes(sentinel)
        self.output.symlink_to(external)
        with self.assertRaises(FileExistsError):
            POLICY.write_plist(self.template, self.output, VERSION, COMMIT)
        self.assertEqual(external.read_bytes(), sentinel)
        self.assertTrue(self.output.is_symlink())


if __name__ == "__main__":
    unittest.main()
