"""Host-side regressions for iPadOS startup and portable signing. No extra dependencies."""
import pathlib
import plistlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
APP = ROOT.parent.parent / "apps" / "photocraft-ios"


class PackagingTests(unittest.TestCase):
    def test_ios_photos_required_callback_is_registered_with_protocol(self):
        # objc2 aborts class registration when a required protocol method is only
        # registered as an inherent method (the shipped macOS binding is incomplete on iOS).
        bridge = (APP / "src/scene.rs").read_text()
        implementation = bridge.split("unsafe impl IosPhotosDelegate for ImagePickerDelegate", 1)[1]
        self.assertIn('#[unsafe(method(picker:didFinishPicking:))]', implementation.split("\n);", 1)[0])
        self.assertIn("let _ = ImagePickerDelegate::class();", bridge)

    def test_scene_manifest_matches_registered_rust_class(self):
        with (ROOT / "Info.plist").open("rb") as source:
            manifest = plistlib.load(source)["UIApplicationSceneManifest"]
        self.assertIs(manifest["UIApplicationSupportsMultipleScenes"], False)
        scenes = manifest["UISceneConfigurations"]["UIWindowSceneSessionRoleApplication"]
        self.assertEqual(len(scenes), 1)
        self.assertEqual(scenes[0]["UISceneClassName"], "UIWindowScene")
        name = scenes[0]["UISceneDelegateClassName"]
        self.assertIn(f'#[name = "{name}"]', (APP / "src/scene.rs").read_text())
        main = (APP / "src/main.rs").read_text()
        self.assertLess(main.index("scene::register()"), main.index("eframe::run_native("))
        self.assertIn("scene::attach_window(cc)?", main)

    def test_personal_signing_is_not_in_shared_project(self):
        project = (ROOT / "PhotoCraft.xcodeproj/project.pbxproj").read_text()
        self.assertNotRegex(project, r"DEVELOPMENT_TEAM\s*=\s*[A-Z0-9]{10}")
        self.assertNotRegex(project, r"photocraft\.local\.[a-z0-9]+")
        self.assertNotIn("/Users/", project)
        self.assertNotIn("TEMP_", project)
        members = re.findall(r"^\s+[A-F0-9]{24} /\* Signing\.xcconfig \*/,\s*$", project, re.MULTILINE)
        self.assertEqual(len(members), 1, "Signing config must belong to exactly one Xcode group")
        self.assertEqual(project.count("baseConfigurationReference ="), 2)
        self.assertIn("Signing.xcconfig", project)
        config = (ROOT / "Signing.xcconfig").read_text()
        self.assertIn('#include? "Signing.local.xcconfig"', config)
        self.assertIn("Signing.local.xcconfig", (ROOT / ".gitignore").read_text())

    def test_unsafe_boundary_is_scoped_to_scene_module(self):
        for source in (APP / "src").glob("*.rs"):
            if source.name != "scene.rs":
                self.assertNotIn("allow(unsafe_code)", source.read_text())
                self.assertIsNone(re.search(r"\bunsafe\s*\{", source.read_text()))
        bridge = (APP / "src/scene.rs").read_text()
        self.assertIn("MainThreadMarker::new()", bridge)
        self.assertIn("deny(unsafe_op_in_unsafe_fn)", bridge)

    def test_rust_output_participates_in_xcode_signing(self):
        # Otherwise an incremental Run can overwrite the signed executable without resigning.
        project = (ROOT / "PhotoCraft.xcodeproj/project.pbxproj").read_text()
        self.assertIn('"$(TARGET_BUILD_DIR)/$(EXECUTABLE_PATH)"', project)
        self.assertIn("outputPaths = (", project)


if __name__ == "__main__":
    unittest.main()
