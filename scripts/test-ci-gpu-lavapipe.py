import importlib.util
import io
from pathlib import Path
import sys
import unittest

spec = importlib.util.spec_from_file_location('gpu_check', Path(__file__).with_name('ci-gpu-lavapipe.py'))
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class LogChecks(unittest.TestCase):
    def test_requires_vulkan_evidence(self):
        self.assertTrue(check.validate_log(['GPU parity backend: Gl']))
        self.assertTrue(check.validate_log([]))

    def test_success_and_nextest_summary(self):
        self.assertFalse(check.validate_log(['GPU parity backend: Vulkan (llvmpipe, Cpu)',
                                            'Summary 100 tests run: 100 passed, 3 skipped']))

    def test_rejects_missing_comparisons(self):
        for message in ['skipping GPU parity tests: no device', 'skipping: unsupported format',
                        'no GPU adapter', 'SKIP gpu timing: no adapter',
                        'SKIP gpu timing: no device with timestamp queries', 'SKIP unknown test']:
            with self.subTest(message=message):
                self.assertTrue(check.validate_log(['GPU parity backend: Vulkan', message]))

    def test_allows_only_known_timing_exceptions(self):
        self.assertFalse(check.validate_log(['GPU parity backend: Vulkan',
            'SKIP gpu timing: llvmpipe is a software adapter, which cannot show a GPU/CPU split',
            'SKIP the comparison: this GPU finishes a 6 MP composite in 1 ms against 2 ms of encoding',
            'SKIP the sum: this refresh still fit one command buffer']))


class Wrapper(unittest.TestCase):
    """The gate's GPU shard runs xtask through `run`: its output is shown and checked."""

    def status(self, script, icds=('/icd/lvp_icd.x86_64.json',)):
        out = io.StringIO()
        return check.run([sys.executable, '-c', script], list(icds), out), out.getvalue()

    def test_runs_on_the_given_vulkan_driver_and_shows_the_output(self):
        status, out = self.status(
            "import os; print('GPU parity backend: Vulkan', os.environ['WGPU_BACKEND'], os.environ['VK_ICD_FILENAMES'])")
        self.assertEqual(status, 0)
        self.assertIn('GPU parity backend: Vulkan vulkan /icd/lvp_icd.x86_64.json', out)

    def test_a_passing_run_with_a_skip_fails(self):
        status, _ = self.status("print('GPU parity backend: Vulkan'); print('skipping GPU parity tests: no device')")
        self.assertEqual(status, 1)
        status, _ = self.status("print('no parity test ran')")
        self.assertEqual(status, 1)

    def test_a_failing_run_keeps_its_status(self):
        status, _ = self.status("print('GPU parity backend: Vulkan'); raise SystemExit(100)")
        self.assertEqual(status, 100)


if __name__ == '__main__':
    unittest.main()
