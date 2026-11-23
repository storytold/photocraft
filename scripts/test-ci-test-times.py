import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('test_times', Path(__file__).with_name('ci-test-times.py'))
times = importlib.util.module_from_spec(spec)
spec.loader.exec_module(times)

# As `gh run view --log` prints them: job, step, timestamp, and the escape character as `^[`.
LOG = '\n'.join([
    'gate / test 4/4 (ubuntu-24.04)\tUNKNOWN STEP\t2026-11-19T16:21:04.4143043Z ^[[32;1m        PASS^[[0m [   1.673s] (   1/4515) '
    '^[[35;1mphotocraft-engine^[[0m ^[[36mcmd::retouch::tests^[[0m^[[36m::^[[0m^[[34;1mslow_one^[[0m',
    'gate / test 4/4 (ubuntu-24.04)\tUNKNOWN STEP\t2026-11-19T16:21:05.0Z \x1b[32;1m        PASS\x1b[0m [   0.272s] (   2/4515) '
    '\x1b[35;1mphotocraft::it\x1b[0m \x1b[36mcontrol\x1b[0m\x1b[36m::\x1b[0m\x1b[34;1mquick\x1b[0m',
    'x\tUNKNOWN STEP\t2026-11-19T16:21:06.0Z         SLOW [> 60.000s] photocraft-engine cmd::still_running',
    'x\tUNKNOWN STEP\t2026-11-19T16:21:07.0Z         FAIL [   2.500s] (   3/4515) photocraft-engine cmd::fails',
    # nextest repeats each failure at the end of the run.
    'x\tUNKNOWN STEP\t2026-11-19T16:21:09.0Z         FAIL [   2.500s] (   3/4515) photocraft-engine cmd::fails',
    'x\tUNKNOWN STEP\t2026-11-19T16:21:08.0Z      Summary [ 527.727s] 4515 tests run: 4515 passed',
])


class TestTimes(unittest.TestCase):
    def test_reads_each_finished_test_once(self):
        self.assertEqual(times.parse_log(LOG), {
            ('photocraft-engine', 'cmd::retouch::tests::slow_one'): 1.673,
            ('photocraft::it', 'control::quick'): 0.272,
            ('photocraft-engine', 'cmd::fails'): 2.5,
        })

    def test_rows_total_each_binary_and_list_the_slow_tests(self):
        self.assertEqual(times.rows('linux', times.parse_log(LOG)), [
            'bin\tlinux\tphotocraft-engine\t2\t4.17',
            'bin\tlinux\tphotocraft::it\t1\t0.27',
            'test\tlinux\tphotocraft-engine\t2.50\tcmd::fails',
            'test\tlinux\tphotocraft-engine\t1.67\tcmd::retouch::tests::slow_one',
        ])

    def test_a_refresh_keeps_the_rows_of_the_os_it_did_not_read(self):
        old = '# old header\nbin\twindows\tw\t1\t9.00\nbin\tlinux\tstale\t1\t1.00\n'
        new = times.merge(old, {'linux': ['bin\tlinux\tfresh\t1\t2.00']}, 123)
        body = [line for line in new.splitlines() if not line.startswith('#')]
        self.assertEqual(body, ['bin\tlinux\tfresh\t1\t2.00', 'bin\twindows\tw\t1\t9.00'])
        self.assertIn('run 123', new)

    def test_only_runs_that_test_every_package_refresh_the_times(self):
        self.assertIsNone(times.refusal({'event': 'push', 'headBranch': 'main'}))
        self.assertIsNone(times.refusal({'event': 'schedule', 'headBranch': 'main'}))
        self.assertIn('pull_request', times.refusal({'event': 'pull_request', 'headBranch': 'ci-x'}))
        self.assertIsNotNone(times.refusal({'event': 'push', 'headBranch': 'ci-x'}))

    def test_names_the_binaries_a_refresh_drops(self):
        old = 'bin\twindows\tgpu::it\t1\t9.00\nbin\twindows\tui\t1\t9.00\nbin\tlinux\tgpu::it\t1\t1.00\n'
        self.assertEqual(times.vanished(old, {'windows': ['bin\twindows\tui\t1\t2.00']}), {'windows': ['gpu::it']})
        self.assertEqual(times.vanished(old, {'linux': ['bin\tlinux\tgpu::it\t1\t2.00']}), {})


if __name__ == '__main__':
    unittest.main()
