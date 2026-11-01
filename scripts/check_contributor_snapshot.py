"""Read-only contributor snapshot diagnostic; not a generator or release gate.

Allow the snapshot commit itself and narrowly defined version-only operations.
Inspect each intervening commit, so reverting a change cannot hide missing credit.
The official generator is still required to verify statistics and consent.
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

SNAPSHOT = 'contributors/contributors.json'
REFRESH = 'python3 ../../craftrules/scripts/contributors.py .'


def git(root, *args):
    result = subprocess.run(
        ['git', '-C', str(root), *args], capture_output=True, text=True,
        encoding='utf-8', env={**os.environ, 'GIT_NO_LAZY_FETCH': '1'},
    )
    if result.returncode:
        raise ValueError('Git history/object unavailable: ' + ' '.join(args[:2]))
    return result.stdout


def version_only(root, parent, commit):
    """Compare parsed TOML, allowing only the workspace/local lock version change.

    xtask version sets workspace.package.version and updates workspace lock entries.
    Never exempt arbitrary Cargo changes or changes to source-backed dependencies.
    """
    before = tomllib.loads(git(root, 'show', parent + ':Cargo.toml'))
    after = tomllib.loads(git(root, 'show', commit + ':Cargo.toml'))
    old = before['workspace']['package']['version']
    new = after['workspace']['package']['version']
    if not isinstance(old, str) or not isinstance(new, str) or old == new:
        return False
    after['workspace']['package']['version'] = old
    if before != after:
        return False
    left = tomllib.loads(git(root, 'show', parent + ':Cargo.lock'))
    right = tomllib.loads(git(root, 'show', commit + ':Cargo.lock'))
    local = {p['name'] for p in left['package']
             if 'source' not in p and p.get('version') == old}
    for package in right['package']:
        if ('source' not in package and package['name'] in local
                and package.get('version') == new):
            package['version'] = old
        if 'dependencies' in package:
            package['dependencies'] = [
                d[:-len(new)] + old if d in {n + ' ' + new for n in local} else d
                for d in package['dependencies']
            ]
    return left == right


def check(root, revision='HEAD'):
    target = git(root, 'rev-parse', '--verify', revision + '^{commit}').strip()
    data = json.loads(git(root, 'show', target + ':' + SNAPSHOT))
    if (not isinstance(data, dict) or type(data.get('schema')) is not int
            or data['schema'] != 1 or data.get('repo') != 'storytold/photocraft'
            or not isinstance(data.get('contributors'), list)
            or type(data.get('commits')) is not int or data['commits'] < 0
            or not isinstance(data.get('head_date'), str)
            or not isinstance(data.get('branch'), str)):
        raise ValueError('Missing or unsupported snapshot fields (schema 1, PhotoCraft repo required)')
    if any(not isinstance(p, dict) or not isinstance(p.get('login'), str)
           for p in data['contributors']):
        raise ValueError('Invalid contributor records')
    head = data.get('head')
    if not isinstance(head, str) or not re.fullmatch(r'[0-9a-f]{40}', head):
        raise ValueError('Snapshot head must be a full Git commit ID')
    git(root, 'cat-file', '-e', head + '^{commit}')
    try:
        git(root, 'merge-base', '--is-ancestor', head, target)
    except ValueError as error:
        raise ValueError('Snapshot head is not an ancestor of the candidate, or history is incomplete') from error
    commits = git(root, 'rev-list', '--reverse', '--topo-order', head + '..' + target).splitlines()
    for commit in commits:
        parents = git(root, 'rev-list', '--parents', '-n', '1', commit).split()[1:]
        if not parents:
            raise ValueError('Uncovered root commit ' + commit)
        parent = parents[0]
        paths = set(git(root, 'diff', '--name-only', '--no-renames', parent, commit).splitlines())
        other = paths - {SNAPSHOT}
        if not other:
            continue
        if other <= {'Cargo.toml', 'Cargo.lock'} and version_only(root, parent, commit):
            continue
        raise ValueError('Snapshot omits change at ' + commit + ': ' + ', '.join(sorted(other)))
    return head, target


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--revision', default='HEAD')
    args = parser.parse_args(argv)
    try:
        head, target = check(args.repo, args.revision)
    except (ValueError, OSError, KeyError, TypeError) as error:
        print(f'Snapshot needs attention: {error}.\nRefresh on latest main: {REFRESH}\n'
              'Review and commit the generated JSON, then recheck before updating release.\n'
              'If Git objects are missing, fetch complete history first.', file=sys.stderr)
        return 1
    print(f'No uncovered changes detected: {head} -> {target} (diagnostic only).')
    return 0


if __name__ == '__main__':
    sys.exit(main())
