"""ci.yml and perf-nightly.yml build every job with one Rust release: TOOLCHAIN_PIN, and the gate calls' `toolchain`."""
from pathlib import Path
import re
import unittest

WORKFLOWS = Path(__file__).resolve().parent.parent / '.github' / 'workflows'
CI = WORKFLOWS / 'ci.yml'
PERF = WORKFLOWS / 'perf-nightly.yml'


class TestToolchainPin(unittest.TestCase):
    def setUp(self):
        self.text = CI.read_text()

    def test_the_gate_calls_use_the_pinned_release(self):
        pin = re.search(r'^  TOOLCHAIN_PIN: "([^"]+)"$', self.text, re.M)
        self.assertIsNotNone(pin, 'ci.yml sets no TOOLCHAIN_PIN')
        calls = re.findall(r'^      toolchain: "([^"]+)"$', self.text, re.M)
        self.assertEqual(calls, [pin.group(1)] * 2, 'the Linux and Windows gate calls must name the pin')

    def test_perf_nightly_uses_the_same_release(self):
        pins = [re.search(r'^  TOOLCHAIN_PIN: "([^"]+)"$', t, re.M) for t in (self.text, PERF.read_text())]
        self.assertTrue(all(pins), 'a workflow sets no TOOLCHAIN_PIN')
        self.assertEqual(pins[0].group(1), pins[1].group(1))

    def test_no_job_installs_a_floating_stable(self):
        self.assertNotIn('toolchain: stable', self.text)
        self.assertNotIn('toolchain: stable', PERF.read_text())


if __name__ == '__main__':
    unittest.main()
