#!/usr/bin/env python3
"""The nightly cold run deletes the rolling crates entries just before it saves its own.

    python3 scripts/ci-clear-rolling-cache.py        needs GH_TOKEN, GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT

The cold run (ci.yml, `cold-build`) never restores the rolling entry, so the old one only adds to
the cache while the new one is saved: two entries of about 4 GB each, beside the others, go over the
repository's 10 GB limit and GitHub evicts the Windows, web and corpus entries. The gate's build job
saves in its `Post` steps; this deletes the old entries when the first of them starts. Pull requests
and pushes that start in the minutes between restore nothing from the deleted entry and build cold
once. Deleting is the only write; the entry the cold run saves does not exist yet when this runs.
"""

import json
import os
import re
import subprocess
import sys
import time

REF = 'refs/heads/main'
BUILD_JOB = re.compile(r'^gate / build \(')
ROLLING = '-gate-crates-'
POLL_SECONDS = 20
GIVE_UP_SECONDS = 100 * 60


def saving(job):
    """True once the job's post steps (the cache save) have started."""
    return any(s['name'].startswith('Post ') and s['status'] in ('in_progress', 'completed') for s in job.get('steps', []))


def build_job(jobs):
    return next((j for j in jobs if BUILD_JOB.match(j['name'])), None)


def rolling_entries(caches):
    return [c for c in caches if ROLLING in c['key']]


def gh(*args):
    out = subprocess.run(['gh', 'api', *args], check=True, capture_output=True, text=True).stdout
    return json.loads(out) if out.strip() else None


def main():
    repo, run, attempt = os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_RUN_ID'], os.environ.get('GITHUB_RUN_ATTEMPT', '1')
    deadline = time.monotonic() + GIVE_UP_SECONDS
    while True:
        jobs = gh(f'repos/{repo}/actions/runs/{run}/attempts/{attempt}/jobs?per_page=100') or {}
        job = build_job(jobs.get('jobs', []))
        if job and saving(job):
            break
        if job and job['status'] == 'completed':
            print('the build job finished without a save step; nothing deleted')
            return
        if time.monotonic() > deadline:
            print('::warning::the build job never reached its save; nothing deleted')
            return
        time.sleep(POLL_SECONDS)
    caches = (gh(f'repos/{repo}/actions/caches?ref={REF}&per_page=100') or {}).get('actions_caches', [])
    for c in rolling_entries(caches):
        print(f"delete {c['key']} ({c['size_in_bytes'] / 1e9:.2f} GB)")
        subprocess.run(['gh', 'api', '-X', 'DELETE', f"repos/{repo}/actions/caches/{c['id']}"], check=True)


if __name__ == '__main__':
    sys.exit(main())
