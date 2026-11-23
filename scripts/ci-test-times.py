#!/usr/bin/env python3
"""Refresh .config/ci-test-times.tsv from the gate's test shards of one CI run.

    python3 scripts/ci-test-times.py <run-id> [--repo storytold/photocraft] [--force]

The gate's build job balances its per-shard archives by these times (xtask/src/shards.rs). Take a
green main run, which runs every test; refresh after large test changes. The script refuses other
runs (pull requests leave packages out of the Windows tests) unless given --force. A run without Windows
shards refreshes the Linux rows only and keeps the Windows ones. Needs an authenticated `gh`.

Rows, tab-separated:
    bin   <os> <binary-id> <test count> <total seconds>
    test  <os> <binary-id> <seconds> <test name>     (tests of TEST_ROW_SECONDS or more)
Linux rows leave out shard 1, which runs nextest's `gpu` group alone (`--gpu-shard` in ci.yml):
the build job plans the other shards with them.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

TIMES = Path(__file__).resolve().parent.parent / '.config' / 'ci-test-times.tsv'
TEST_ROW_SECONDS = 1.0
# The gate's shard jobs per OS, and the shards whose tests the plan does not balance.
JOBS = {
    'linux': (re.compile(r'^gate / test (\d+)/(\d+) \(ubuntu'), {1}),
    'windows': (re.compile(r'^windows / test (\d+)/(\d+) \(windows'), set()),
}
# Colour codes; `gh run view --log` prints the escape character as the two characters `^[`.
ANSI = re.compile(r'(?:\x1b|\^\[)\[[0-9;]*m')
# nextest's final status line of a test: `PASS [   1.673s] (   1/4515) <binary-id> <test name>`.
RESULT = re.compile(r'\b(?:PASS|FAIL|FLAKY|LEAK|LEAK-FAIL|TIMEOUT|SIGSEGV|SIGABRT|SIGBUS|SIGILL|ABORT) \[\s*([\d.]+)s\] (?:\(\s*\d+/\d+\) )?(\S+) (.+?)\s*$')
HEADER = """\
# Test times of the CI gate's shards, which the build job balances its per-shard archives by
# (xtask/src/shards.rs). Written by scripts/ci-test-times.py from run {run}; refresh it after large
# test changes. A missing or stale row only costs balance: the build's own test listing decides
# which tests run.
# bin  <os> <binary-id> <test count> <total seconds>
# test <os> <binary-id> <seconds> <test name>
"""


def parse_log(text):
    """{(binary, test): seconds} from one shard job's log."""
    times = {}
    for line in text.splitlines():
        m = RESULT.search(ANSI.sub('', line))
        if m:
            times[(m.group(2), m.group(3))] = float(m.group(1))
    return times


def rows(os_name, times):
    """The file's rows of one OS from {(binary, test): seconds}."""
    bins = {}
    for (binary, _), seconds in times.items():
        count, total = bins.get(binary, (0, 0.0))
        bins[binary] = (count + 1, total + seconds)
    out = [f'bin\t{os_name}\t{b}\t{c}\t{t:.2f}' for b, (c, t) in sorted(bins.items())]
    out += [f'test\t{os_name}\t{b}\t{s:.2f}\t{t}' for (b, t), s in sorted(times.items()) if s >= TEST_ROW_SECONDS]
    return out


def merge(old_text, fresh, run):
    """The new file: fresh rows for the OSes in `fresh` ({os: rows}), the old rows of the others."""
    kept = [line for line in old_text.splitlines()
            if line and not line.startswith('#') and line.split('\t')[1] not in fresh]
    body = kept + [row for os_name in sorted(fresh) for row in fresh[os_name]]
    body.sort(key=lambda r: (r.split('\t')[1], r.split('\t')[0] != 'bin', r.split('\t')[2]))
    return HEADER.format(run=run) + '\n'.join(body) + '\n'


def refusal(run, default_branch='main'):
    """Why a run cannot refresh the times, or None: only main's pushes and nightly runs test every
    package."""
    if run['event'] not in ('push', 'schedule') or run['headBranch'] != default_branch:
        return (f"run {run.get('databaseId', '')} is a {run['event']} run on {run['headBranch']}: only main's push and "
                'nightly runs test every package (pull requests leave some out on Windows); --force takes it anyway')
    return None


def vanished(old_text, fresh):
    """Binaries the old file has for an OS in `fresh` ({os: rows}) that the fresh rows lack."""
    def binaries(lines):
        out = {}
        for line in lines:
            cols = line.split('\t')
            if len(cols) == 5 and cols[0] == 'bin':
                out.setdefault(cols[1], set()).add(cols[2])
        return out
    old = binaries(line for line in old_text.splitlines() if not line.startswith("#"))
    new = binaries(row for rows in fresh.values() for row in rows)
    return {os_name: sorted(old.get(os_name, set()) - new.get(os_name, set())) for os_name in fresh
            if old.get(os_name, set()) - new.get(os_name, set())}


def gh(*args):
    return subprocess.run(['gh', *args], check=True, capture_output=True, text=True).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('run')
    parser.add_argument('--repo', default='storytold/photocraft')
    parser.add_argument('--force', action='store_true', help='take a run that is not a main push or nightly run')
    args = parser.parse_args()
    run = json.loads(gh('run', 'view', args.run, '-R', args.repo, '--json', 'databaseId,event,headBranch,jobs'))
    reason = refusal(run)
    if reason and not args.force:
        sys.exit(reason)
    jobs = run['jobs']
    fresh = {}
    for os_name, (pattern, skip) in JOBS.items():
        times = {}
        for job in jobs:
            m = pattern.match(job['name'])
            if not m or int(m.group(1)) in skip:
                continue
            if job['conclusion'] != 'success':
                print(f"warning: {job['name']} ended {job['conclusion']}", file=sys.stderr)
            log = gh('run', 'view', '-R', args.repo, '--job', str(job['databaseId']), '--log')
            found = parse_log(log)
            print(f"{job['name']}: {len(found)} tests", file=sys.stderr)
            times.update(found)
        if times:
            fresh[os_name] = rows(os_name, times)
    if not fresh:
        sys.exit(f'run {args.run} has no gate test shards')
    old = TIMES.read_text() if TIMES.exists() else ''
    for os_name, binaries in vanished(old, fresh).items():
        print(f"warning: {os_name}: the run has no tests of {', '.join(binaries)}; their rows go", file=sys.stderr)
    TIMES.write_text(merge(old, fresh, args.run))
    print(f'wrote {TIMES}', file=sys.stderr)


if __name__ == '__main__':
    main()
