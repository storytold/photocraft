from importlib.machinery import SourceFileLoader
from pathlib import Path
import unittest

clear = SourceFileLoader('clear', str(Path(__file__).with_name('ci-clear-rolling-cache.py'))).load_module()


class TestClear(unittest.TestCase):
    def test_the_build_job_is_saving_once_a_post_step_starts(self):
        steps = lambda *s: {'name': 'gate / build (ubuntu-24.04)', 'steps': [{'name': n, 'status': st} for n, st in s]}
        self.assertFalse(clear.saving(steps(('Setup', 'completed'), ('Build the test archive', 'in_progress'))))
        self.assertTrue(clear.saving(steps(('Build the test archive', 'completed'), ('Post Setup', 'in_progress'))))

    def test_only_the_build_job_counts(self):
        jobs = [{'name': 'gate / lint (ubuntu-24.04)'}, {'name': 'gate / build (ubuntu-24.04)'}]
        self.assertEqual(clear.build_job(jobs), jobs[1])

    def test_only_rolling_crates_entries_are_deleted(self):
        caches = [
            {'key': 'v2-rust-ab-gate-crates-Linux-x64-1-1-Linux-x64-d8-99'},
            {'key': 'v2-rust-ab-gate-Windows_NT-x64-ae-88'},
            {'key': 'v1-rust-corpus-corpus-Linux-x64-62-99'},
        ]
        self.assertEqual(clear.rolling_entries(caches), caches[:1])

    def test_entries_created_after_the_run_started_are_kept(self):
        rolling = lambda created: {'key': 'v2-rust-ab-gate-crates-Linux-x64-1', 'created_at': created}
        old, same_minute_push = rolling('2026-11-21T03:00:00Z'), rolling('2026-11-22T03:05:00Z')
        self.assertEqual(clear.created_before([old, same_minute_push], '2026-11-22T03:00:10Z'), [old])


if __name__ == '__main__':
    unittest.main()
