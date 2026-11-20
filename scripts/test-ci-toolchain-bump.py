from importlib.machinery import SourceFileLoader
from pathlib import Path
import unittest

bump = SourceFileLoader('bump', str(Path(__file__).with_name('ci-toolchain-bump.py'))).load_module()

FILE = '# note\n[toolchain]\nchannel = "1.100.0"\ncomponents = ["clippy"]\n'


def manifest(version):
    return f'[pkg.rust]\nversion = "{version} (0ef1bc386 2026-11-09)"\n'


class TestBump(unittest.TestCase):
    def test_a_newer_stable_is_reported_and_the_same_or_older_is_not(self):
        self.assertEqual(bump.newer(FILE, manifest('1.101.0')), '1.101.0')
        self.assertEqual(bump.newer(FILE, manifest('1.110.0')), '1.110.0')
        self.assertIsNone(bump.newer(FILE, manifest('1.100.0')))
        self.assertIsNone(bump.newer(FILE, manifest('1.99.2')))

    def test_a_pin_that_is_not_a_release_moves_to_stable(self):
        self.assertEqual(bump.newer(FILE.replace('1.100.0', 'stable'), manifest('1.100.0')), '1.100.0')


if __name__ == '__main__':
    unittest.main()
