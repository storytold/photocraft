import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('gpu_select', Path(__file__).with_name('ci-windows-gpu-tests.py'))
select = importlib.util.module_from_spec(spec)
spec.loader.exec_module(select)

ROOT = '/w'


def pkg(name, path, deps=()):
    return {'name': name, 'manifest_path': f'{ROOT}/{path}/Cargo.toml', 'dependencies': [{'name': d} for d in deps]}


# gpu builds on compose (directly) and raster (through compose, and as a dev-dependency kind:
# cargo metadata lists every kind in one list); ui-egui builds on gpu, so it is not an input.
PACKAGES = [
    pkg('photocraft-raster', 'crates/raster'),
    pkg('photocraft-compose', 'crates/compose', ['photocraft-raster', 'serde']),
    pkg('photocraft-gpu', 'crates/gpu', ['photocraft-compose', 'wgpu']),
    pkg('photocraft-ui-egui', 'crates/ui-egui', ['photocraft-gpu']),
    pkg('photocraft', 'apps/photocraft', ['photocraft-ui-egui']),
]


def needs(*changed, packages=PACKAGES):
    return select.needs_gpu_tests(list(changed), packages, ROOT)


class Selection(unittest.TestCase):
    def test_gpu_and_what_it_builds_on_run_the_tests(self):
        for path in ['crates/gpu/src/lib.rs', 'crates/gpu/tests/it/parity.rs', 'crates/compose/src/blend.rs',
                     'crates/raster/Cargo.toml', 'crates/raster/build.rs']:
            with self.subTest(path=path):
                self.assertTrue(needs(path))

    def test_packages_built_on_gpu_skip_them(self):
        self.assertFalse(needs('crates/ui-egui/src/canvas.rs', 'apps/photocraft/src/main.rs'))

    def test_shared_build_inputs_run_them(self):
        for path in ['Cargo.toml', 'Cargo.lock', '.config/nextest.toml', '.github/workflows/ci.yml',
                     '.cargo/config.toml', 'rust-toolchain.toml', 'scripts/ci-windows-gpu-tests.py']:
            with self.subTest(path=path):
                self.assertTrue(needs('crates/ui-egui/src/canvas.rs', path))

    def test_any_package_manifest_runs_them(self):
        # A feature enabled in ui-egui's manifest unifies into the workspace build gpu is tested in.
        for path in ['crates/ui-egui/Cargo.toml', 'apps/photocraft/Cargo.toml']:
            with self.subTest(path=path):
                self.assertTrue(needs(path))

    def test_documentation_skips_them(self):
        self.assertFalse(needs('docs/development.md', 'README.md', 'crates/ui-egui/README.md', 'scorecard/ui.toml'))

    def test_unknown_files_outside_packages_run_them(self):
        for path in ['assets/shaders/x.wgsl', 'scripts/other.py', 'i18n/de.tsv', 'deleted-crate/src/lib.rs']:
            with self.subTest(path=path):
                self.assertTrue(needs(path))

    def test_nothing_changed_skips_them(self):
        self.assertFalse(needs())

    def test_a_nested_package_belongs_to_itself(self):
        packages = PACKAGES + [pkg('photocraft-gpu-fuzz', 'crates/gpu/fuzz', ['photocraft-gpu'])]
        self.assertFalse(needs('crates/gpu/fuzz/src/main.rs', packages=packages))
        self.assertTrue(needs('crates/gpu/src/lib.rs', packages=packages))

    def test_a_new_dependency_is_followed_without_editing_the_script(self):
        packages = PACKAGES + [pkg('photocraft-new', 'crates/new')]
        self.assertFalse(needs('crates/new/src/lib.rs', packages=packages))
        packages[2] = pkg('photocraft-gpu', 'crates/gpu', ['photocraft-compose', 'photocraft-new'])
        self.assertTrue(needs('crates/new/src/lib.rs', packages=packages))

    def test_windows_paths_from_cargo_metadata(self):
        packages = [dict(p, manifest_path=p['manifest_path'].replace('/', '\\')) for p in PACKAGES]
        self.assertTrue(select.needs_gpu_tests(['crates/compose/src/blend.rs'], packages, ROOT.replace('/', '\\')))
        self.assertFalse(select.needs_gpu_tests(['crates/ui-egui/src/canvas.rs'], packages, ROOT.replace('/', '\\')))

    def test_a_file_moved_out_of_gpu_counts_for_gpu(self):
        with tempfile.TemporaryDirectory() as repo:
            def git(*args):
                subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', *args], cwd=repo, check=True, capture_output=True)
            src = Path(repo, 'crates/gpu/tests/fixtures/reference.bin')
            src.parent.mkdir(parents=True)
            src.write_bytes(b'reference pixels ' * 64)
            git('init', '-q')
            git('add', '-A')
            git('commit', '-q', '-m', 'base')
            dst = Path(repo, 'crates/ui-egui/tests/fixtures/reference.bin')
            dst.parent.mkdir(parents=True)
            src.rename(dst)
            git('add', '-A')
            git('commit', '-q', '-m', 'move')
            changed = select.changed_files('HEAD~1', cwd=repo)
        self.assertIn('crates/gpu/tests/fixtures/reference.bin', changed)
        self.assertTrue(needs(*changed))

    def test_a_missing_gpu_package_is_an_error(self):
        with self.assertRaises(ValueError):
            needs('crates/ui-egui/src/canvas.rs', packages=[p for p in PACKAGES if p['name'] != 'photocraft-gpu'])


class NeutralSelection(unittest.TestCase):
    PACKAGES = [
        pkg('photocraft-geom', 'crates/geom'),
        pkg('photocraft-algo', 'crates/algo', ['photocraft-geom']),
        pkg('photocraft-compose', 'crates/compose', ['photocraft-geom']),
        pkg('photocraft-engine', 'crates/engine', ['photocraft-compose']),
        pkg('photocraft-ui-egui', 'crates/ui-egui', ['photocraft-engine']),
    ]

    def skipped(self, *changed):
        return select.neutral_to_skip(list(changed), self.PACKAGES, ROOT)

    def test_a_change_elsewhere_skips_every_neutral_package(self):
        self.assertEqual(self.skipped('crates/ui-egui/src/lib.rs'), ['photocraft-algo', 'photocraft-compose', 'photocraft-geom'])

    def test_a_changed_package_and_its_dependents_are_not_skipped(self):
        self.assertEqual(self.skipped('crates/compose/src/lib.rs'), ['photocraft-algo', 'photocraft-geom'])
        # geom is a dependency of algo and compose, so its change keeps all three.
        self.assertEqual(self.skipped('crates/geom/src/lib.rs'), [])

    def test_shared_inputs_skip_nothing(self):
        for path in ['crates/ui-egui/Cargo.toml', 'Cargo.lock', '.github/workflows/ci.yml']:
            with self.subTest(path=path):
                self.assertEqual(self.skipped('crates/ui-egui/src/lib.rs', path), [])

    def test_documentation_only_skips_all_of_them(self):
        self.assertEqual(self.skipped('docs/a.md'), ['photocraft-algo', 'photocraft-compose', 'photocraft-geom'])


class TestLinuxSkip(unittest.TestCase):
    def skipped(self, *changed):
        return select.linux_to_skip(list(changed), PACKAGES, ROOT)

    def test_a_leaf_change_skips_what_does_not_build_on_it(self):
        self.assertEqual(self.skipped('crates/ui-egui/src/lib.rs'), ['photocraft-compose', 'photocraft-raster'])

    def test_a_change_reaches_the_packages_built_on_it_but_never_skips_gpu(self):
        self.assertEqual(self.skipped('crates/compose/src/lib.rs'), ['photocraft-raster'])
        self.assertNotIn('photocraft-gpu', self.skipped('crates/ui-egui/src/lib.rs'))

    def test_shared_inputs_skip_nothing(self):
        for path in ['crates/raster/Cargo.toml', 'Cargo.lock', '.github/workflows/ci.yml']:
            with self.subTest(path=path):
                self.assertEqual(self.skipped('crates/ui-egui/src/lib.rs', path), [])

    def test_documentation_only_skips_everything_but_gpu(self):
        self.assertEqual(self.skipped('docs/a.md'), ['photocraft', 'photocraft-compose', 'photocraft-raster', 'photocraft-ui-egui'])


if __name__ == '__main__':
    unittest.main()
