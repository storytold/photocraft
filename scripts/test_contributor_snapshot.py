"""Synthetic Git regressions; only Python standard library and Git are executed."""

import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from check_contributor_snapshot import check, main, REFRESH, SNAPSHOT


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Synthetic Test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        self.write('source.txt', 'original')
        self.write('Cargo.toml', '[workspace.package]\nversion = "1.0.0"\n')
        self.write('Cargo.lock', 'version = 4\n[[package]]\nname = "local"\nversion = "1.0.0"\n'
                   '[[package]]\nname = "external"\nversion = "2.0.0"\nsource = "registry+test"\n')
        self.base = self.commit()

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.root), *args], text=True).strip()

    def write(self, path, text):
        p = self.root / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding='utf-8')

    def commit(self):
        self.git('add', '.')
        self.git('commit', '-qm', 'synthetic', '--allow-empty')
        return self.git('rev-parse', 'HEAD')

    def snapshot(self, **updates):
        data = dict(schema=1, repo='storytold/photocraft', head=self.base,
                    head_date='2026-10-23T00:00:00Z', branch='main', commits=1,
                    contributors=[{'login': 'test'}])
        data.update(updates)
        self.write(SNAPSHOT, json.dumps(data))
        return self.commit()

    def test_refresh_commit_and_empty_commit_are_allowed(self):
        self.snapshot()
        self.commit()
        self.assertEqual(check(self.root)[0], self.base)

    def test_explicit_revision_requires_committed_json(self):
        self.snapshot()
        head = self.git('rev-parse', 'HEAD')
        with self.assertRaises(ValueError):
            check(self.root, self.base)  # base predates the JSON
        self.assertEqual(check(self.root, head)[1], head)

    def test_stale_source(self):
        self.snapshot()
        self.write('source.txt', 'new work')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'omits change'):
            check(self.root)

    def test_reverted_work_still_needs_credit(self):
        self.snapshot()
        self.write('source.txt', 'new work')
        self.commit()
        self.write('source.txt', 'original')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'omits change'):
            check(self.root)

    def test_missing_and_malformed_json(self):
        for content in [None, '{', '[]', '{}', '{"schema": 2}']:
            with self.subTest(content=content):
                if content is not None:
                    self.write(SNAPSHOT, content)
                    self.commit()
                with contextlib.redirect_stderr(io.StringIO()) as err:
                    self.assertEqual(main(['--repo', str(self.root)]), 1)
                self.assertIn(REFRESH, err.getvalue())

    def test_required_fields(self):
        for field in ['head', 'head_date', 'repo', 'branch', 'commits', 'contributors', 'schema']:
            with self.subTest(field=field):
                self.snapshot(**{field: None})
                with self.assertRaises((ValueError, TypeError)):
                    check(self.root)

    def test_non_commit_and_unknown_head(self):
        blob = self.git('rev-parse', self.base + ':source.txt')
        for head in ['not-a-sha', 'f' * 40, blob]:
            with self.subTest(head=head):
                self.snapshot(head=head)
                with self.assertRaises(ValueError):
                    check(self.root)

    def test_non_ancestor(self):
        tree = self.git('rev-parse', self.base + '^{tree}')
        foreign = self.git('commit-tree', tree, '-m', 'unrelated root')
        self.snapshot(head=foreign)
        with self.assertRaises(ValueError):
            check(self.root)

    def test_version_bump(self):
        self.snapshot()
        for file in ['Cargo.toml', 'Cargo.lock']:
            self.write(file, (self.root / file).read_text().replace('1.0.0', '1.1.0-rc.1'))
        self.commit()
        self.assertEqual(check(self.root)[0], self.base)

    def test_combined_refresh_and_version_bump(self):
        for file in ['Cargo.toml', 'Cargo.lock']:
            self.write(file, (self.root / file).read_text().replace('1.0.0', '1.2.0'))
        self.snapshot()
        self.assertEqual(check(self.root)[0], self.base)

    def test_merged_development_needs_refresh(self):
        self.snapshot()
        branch = self.git('branch', '--show-current')
        self.git('checkout', '-qb', 'side')
        self.write('side.txt', 'new work')
        self.commit()
        self.git('checkout', '-q', branch)
        self.git('merge', '--no-ff', '-qm', 'synthetic merge', 'side')
        with self.assertRaisesRegex(ValueError, 'omits change'):
            check(self.root)

    def test_dependency_upgrade_is_not_a_version_only_change(self):
        self.snapshot()
        for file in ['Cargo.toml', 'Cargo.lock']:
            self.write(file, (self.root / file).read_text().replace('1.0.0', '1.1.0'))
        self.write('Cargo.lock', (self.root/'Cargo.lock').read_text().replace('2.0.0', '2.1.0'))
        self.commit()
        with self.assertRaisesRegex(ValueError, 'omits change'):
            check(self.root)

    def test_consent_change_requires_refresh(self):
        self.snapshot()
        self.write('contributors/people.toml', '# changed consent')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'omits change'):
            check(self.root)

    def test_uncommitted_files_do_not_change_candidate(self):
        self.snapshot()
        self.write(SNAPSHOT, '{bad uncommitted JSON')
        self.assertEqual(check(self.root)[0], self.base)


if __name__ == '__main__':
    unittest.main()
