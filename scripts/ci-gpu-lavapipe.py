#!/usr/bin/env python3
"""Run a command with wgpu on lavapipe, and fail if a GPU comparison in its output was skipped.

The Linux gate runs nextest's `gpu` test group in a shard of its own through this script
(`ci-gpu-lavapipe.py <xtask> ci tests --from <dir> --gpu`, scripts/ci-test-shard.sh): every wgpu
device in the run is Vulkan on Mesa's CPU rasterizer, each test's output is shown, and the output
is checked here for authored skips and for the parity tests' Vulkan backend line.

Usage: ci-gpu-lavapipe.py COMMAND [ARGS...]
"""
import glob
import os
import re
import subprocess
import sys
import tempfile


def validate_log(lines):
    vulkan = False
    rejected = []
    for line in lines:
        vulkan |= 'GPU parity backend: Vulkan' in line
        # Authored skips, not nextest's summary of ignored/filtered-out tests.
        if re.search(r'\bskipping\b|\bno (?:GPU )?adapter\b', line):
            rejected.append(line.rstrip())
        if 'SKIP ' in line and not (
            ('SKIP gpu timing:' in line and 'is a software adapter' in line)
            or 'SKIP the comparison: this GPU finishes' in line
            or 'SKIP the sum: this refresh still fit one command buffer' in line
        ):
            rejected.append(line.rstrip())
    if not vulkan:
        rejected.append('GPU parity tests did not report the Vulkan backend')
    return rejected


def run(command, icds, out=sys.stdout):
    """Runs `command` on the Vulkan drivers in `icds`; returns its exit status, or 1 when it passed
    but its output shows a skipped comparison."""
    env = dict(os.environ, WGPU_BACKEND='vulkan', VK_ICD_FILENAMES=os.pathsep.join(icds))
    # Stream output and keep it on disk, rather than buffering every successful test in memory.
    with tempfile.TemporaryFile(mode='w+t') as log:
        with subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              text=True) as process:
            for line in process.stdout:
                out.write(line)
                log.write(line)
            result = process.wait()
        log.seek(0)
        errors = validate_log(log)
    for error in errors:
        print(f'::error::{error}', file=sys.stderr)
    return result if result else int(bool(errors))


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    icds = sorted(glob.glob('/usr/share/vulkan/icd.d/lvp_icd*.json'))
    if not icds:
        raise RuntimeError('Install mesa-vulkan-drivers and libvulkan1 before this check')
    return run(argv, icds)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
