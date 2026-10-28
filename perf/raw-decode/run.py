#!/usr/bin/env python3
"""Fetch SHA-pinned CC0 samples and run the native, decode-only Rust benchmark.

Run from any directory: python3 perf/raw-decode/run.py --output target/raw-decode-results
Downloaded originals stay in gitignored corpus/. Results include failures, never a zero time.
"""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def fetch(sample):
    path = (ROOT / sample['path']).resolve()
    if not path.is_relative_to(ROOT / 'corpus'):
        raise ValueError('sample path must be inside corpus/: ' + sample['path'])
    if path.is_file() and path.stat().st_size == sample['bytes'] and digest(path) == sample['sha256']:
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    # Only a .part file is resumed; an old invalid original is never trusted or appended to.
    partial = path.with_suffix(path.suffix + '.part')
    for _ in range(8):
        subprocess.run(['curl', '-L', '--fail', '--silent', '--show-error',
                        '--continue-at', '-', '--max-time', '60', '--speed-time', '10',
                        '--speed-limit', '128', '-A', 'Photocraft-dev',
                        '-o', str(partial), sample['url']], check=False)
        if partial.is_file() and partial.stat().st_size == sample['bytes'] and digest(partial) == sample['sha256']:
            partial.replace(path)
            return path
    raise RuntimeError('download failed SHA-256/length verification: ' + sample['source'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=pathlib.Path, default=pathlib.Path(__file__).with_name('samples.json'))
    parser.add_argument('--output', type=pathlib.Path, default=ROOT / 'target/raw-decode-results')
    parser.add_argument('--cooldown-seconds', type=float, default=0, help='idle after building, from 0 to 60 seconds')
    args = parser.parse_args()
    if not 0 <= args.cooldown_seconds <= 60:
        parser.error('--cooldown-seconds must be between 0 and 60')
    manifest = json.loads(args.manifest.read_text())
    samples = manifest['samples']
    args.output.mkdir(parents=True, exist_ok=True)
    # Finish all downloads/hash checks before compiling or timing.
    for index, sample in enumerate(samples, 1):
        print(f'Checking {index}/{len(samples)}: {sample["source"]}', file=sys.stderr, flush=True)
        fetch(sample)
    subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'photocraft-raw',
                    '--example', 'rawdecodebench'], cwd=ROOT, check=True)
    target = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    if not target.is_absolute():
        target = ROOT / target
    binary = target / 'release/examples' / ('rawdecodebench.exe' if os.name == 'nt' else 'rawdecodebench')
    if args.cooldown_seconds:
        print(f'Build complete; cooling down for {args.cooldown_seconds:g} seconds', file=sys.stderr, flush=True)
        time.sleep(args.cooldown_seconds)
    source_files = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'crates/raw/Cargo.toml',
                    ROOT / 'crates/raw/examples/rawdecodebench.rs', *sorted((ROOT / 'crates/raw/src').glob('*.rs'))]
    meta = {
        'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'platform': platform.platform(), 'architecture': platform.machine(),
        'logical_cpus': os.cpu_count(), 'rustc': command('rustc', '-Vv'),
        'cooldown_seconds': args.cooldown_seconds,
        'performance_environment': {k: os.environ.get(k) for k in ['RAYON_NUM_THREADS','RUSTFLAGS','CARGO_PROFILE_RELEASE_LTO','CARGO_PROFILE_RELEASE_CODEGEN_UNITS']},
        'git_head': command('git', 'rev-parse', 'HEAD'),
        'git_dirty': bool(command('git', 'status', '--porcelain')),
        'binary_sha256': digest(binary), 'manifest_sha256': digest(args.manifest),
        'source_sha256': {str(p.relative_to(ROOT)): digest(p) for p in source_files},
        'features': 'photocraft-raw defaults: flashraw 0.1.1 CPU/SIMD/rayon; native JPEG XL enabled; no FlashRaw GPU/ICC/mmap',
        'method': 'read once; untimed exact validation; 2 warmups; 9 interleaved decode-only runs/backend/file',
    }
    if platform.system() == 'Darwin':
        meta['cpu'] = command('sysctl', '-n', 'machdep.cpu.brand_string')
        meta['physical_cpus'] = int(command('sysctl', '-n', 'hw.physicalcpu'))
        meta['memory_bytes'] = int(command('sysctl', '-n', 'hw.memsize'))
    (args.output / 'machine.json').write_text(json.dumps(meta, indent=2) + '\n')
    with (args.output / 'results.jsonl').open('w') as output, (args.output / 'stderr.log').open('w') as errors:
        for index, sample in enumerate(samples, 1):
            print(f'Benchmark {index}/{len(samples)}: {sample["source"]}', file=sys.stderr, flush=True)
            try:
                run = subprocess.run([str(binary), sample['path']], cwd=ROOT, capture_output=True, text=True, timeout=180)
                errors.write(sample['source'] + '\n' + run.stderr + '\n')
                errors.flush()
                if run.returncode:
                    result = {'status': 'process-error', 'returncode': run.returncode, 'error': run.stderr[-4000:]}
                else:
                    result = json.loads(run.stdout)
            except subprocess.TimeoutExpired:
                result = {'status': 'timeout', 'error': 'file exceeded the 180-second process limit'}
            output.write(json.dumps({'sample': sample, 'benchmark': result}) + '\n')
            output.flush()
    meta['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (args.output / 'machine.json').write_text(json.dumps(meta, indent=2) + '\n')
    print('Results: ' + str(args.output.resolve()), file=sys.stderr)


if __name__ == '__main__':
    main()
