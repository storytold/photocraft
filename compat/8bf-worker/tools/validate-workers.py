#!/usr/bin/env python3
"""Original conformance fixture across native Windows or Wine x86/x64. No vendor code or binaries."""
import argparse
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import time


def exchange(worker, fixture, mode, width, height, pixels, environment,
             channels=3, depth=8, selected_plane=None, runner="wine"):
    fixture_path = str(fixture.resolve()) if runner == "native" else 'Z:' + str(fixture.resolve()).replace('/', '\\')
    request = dict(path=fixture_path, entry='PluginMain',
                   width=width, height=height, channels=channels, depth=depth, show_ui=False)
    metadata = json.dumps(request).encode()
    frame = b'PC8BF001' + struct.pack('<IQ', len(metadata), len(pixels)) + metadata + pixels
    started = time.monotonic()
    process = subprocess.Popen(
        ([str(worker)] if runner == 'native' else ['wine', str(worker)]), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, env=dict(environment, PHOTOCRAFT_8BF_FIXTURE_MODE=mode),
        start_new_session=(os.name != 'nt'),
    )
    try:
        # Allow cold Wine initialization, but fail quickly if a fault opens a debugger.
        response, stderr = process.communicate(frame, timeout=15 if mode == 'crash' else 60)
    except subprocess.TimeoutExpired as error:
        try:
            if os.name == 'nt':
                process.kill()
            else:
                os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=2)
        except subprocess.TimeoutExpired:
            # Wine services may escape the worker's process group and retain pipes.
            # Reap our direct child without waiting indefinitely for those services.
            for stream in (process.stdin, process.stdout, process.stderr):
                stream.close()
            process.wait(timeout=2)
        raise RuntimeError(f'{worker.parent.parent.name} {mode} timed out') from error
    # Stop timing before Python decodes/validates output; include launch, pipes and worker exit.
    elapsed = time.monotonic() - started
    if mode in ('crash', 'exit_error'):
        assert process.returncode != 0, 'crash fixture unexpectedly succeeded'
        return elapsed
    assert process.returncode == 0, (process.returncode, stderr[-1000:])
    assert response[:8] == b'PC8BF001', (response[:100], stderr[-1000:])
    meta_len, pixel_len = struct.unpack('<IQ', response[8:20])
    assert len(response) == 20 + meta_len + pixel_len, 'incorrect frame length'
    meta = json.loads(response[20:20 + meta_len])
    data = response[20 + meta_len:]
    if mode in ('invert', 'noise', 'threaded_alloc', 'identity', 'single_plane', 'mask_request', 'ignored_mask_callback', 'mask_with_pixels'):
        assert meta['status'] == 'ok', meta
        assert (meta['width'], meta['height'], meta['channels'], meta['depth']) == (width, height, channels, depth), meta
        expected = expected_pixels(pixels, channels, depth, 'identity' if mode in ('mask_request', 'ignored_mask_callback') else mode, selected_plane)
        assert data == expected, 'incorrect output pixels'
    elif mode == 'cancel':
        assert meta['status'] == 'cancelled' and not data, meta
    else:
        assert meta['status'] == 'error' and not data, meta
    expected_finish = 0 if mode in ('cancel', 'start_error') else 1
    assert stderr.count(b'CONFORMANCE_FINISH') == expected_finish, 'Finish must run once after successful Start, including host callback errors'
    if mode == 'ignored_callback':
        assert '-32124' not in meta['message'], 'cleanup error hid the original callback failure'
    return elapsed


def expected_pixels(pixels, channels, depth, mode, selected_plane=None):
    """Derive full-range wire pixels from the worker's documented native samples."""
    if mode == 'identity':
        return pixels
    if depth == 8:
        if selected_plane is None:
            return pixels.translate(bytes(reversed(range(256))))
        expected = bytearray(pixels)
        for index in range(selected_plane, len(expected), channels):
            expected[index] = 255 - expected[index]
        return bytes(expected)
    elif depth == 16:
        if len(pixels) % 2:
            raise ValueError('16-bit pixel payload has an odd byte count')
        source = list(struct.unpack('<' + 'H' * (len(pixels) // 2), pixels))
        maximum = 65535
    else:
        raise ValueError(f'unsupported fixture depth: {depth}')

    native_maximum = 255 if depth == 8 else 32768
    sample_count = len(source)
    if sample_count % channels:
        raise ValueError('pixel payload is not a whole number of pixels')
    selected = set(range(channels)) if selected_plane is None else {selected_plane}
    expected = []
    for index, value in enumerate(source):
        if index % channels not in selected:
            expected.append(value)
            continue
        native = value if depth == 8 else (value * native_maximum + 32767) // maximum
        native_output = native_maximum - native
        if depth == 8:
            expected.append(native_output)
        elif native_output == native:
            # The host keeps the original full-range value when the native result
            # is unchanged, avoiding a needless quantization round trip.
            expected.append(value)
        else:
            expected.append((native_output * maximum + 16384) // native_maximum)
    return struct.pack('<' + 'H' * len(expected), *expected)


def sample_pixels(width, height, channels, depth):
    count = width * height * channels
    if depth == 8:
        return (bytes(range(256)) * ((count + 255) // 256))[:count]
    edges = (0, 1, 2, 32767, 32768, 32769, 65534, 65535)
    # Repeated deterministic sample block keeps 24MP/16-bit fixtures bounded.
    values = [edges[i] if i < len(edges) else (i * 977 + 12345) & 0xFFFF
              for i in range(256)]
    block = struct.pack('<256H', *values)
    return (block * ((count + 255) // 256))[:count * 2]


def parse_args(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--prefix', help='Dedicated Wine prefix (required for the Wine runner)')
    parser.add_argument('--runner', choices=('wine', 'native'), default='wine')
    parser.add_argument('--workers-dir', type=Path, help='Directory containing worker32.exe and worker64.exe payloads')
    parser.add_argument('--fixtures-dir', type=Path, help='Prebuilt fixture directory containing i686/ and x86_64/ subdirectories')
    parser.add_argument('--large', action='store_true')
    args = parser.parse_args(argv)
    if args.runner == 'wine' and not args.prefix:
        parser.error('--prefix is required for --runner wine')
    return args


def main(argv=None):
    args = parse_args(argv)
    root = Path(__file__).resolve().parents[1]
    environment = dict(os.environ)
    if args.runner == 'wine':
        environment.update(WINEPREFIX=str(Path(args.prefix).resolve()), WINEDEBUG='-all')
        environment.pop('WINEARCH', None)
    results = []
    for arch in ('i686', 'x86_64'):
        suffix = '-pc-windows-msvc' if args.runner == 'native' else '-pc-windows-gnu'
        target = arch + suffix
        output = root / 'target' / target / 'release'
        worker = ((args.workers_dir.resolve() / ('photocraft-8bf-worker32.exe' if arch == 'i686' else 'photocraft-8bf-worker64.exe'))
                  if args.workers_dir else output / 'photocraft-8bf-worker.exe')
        fixture_output = args.fixtures_dir.resolve() / arch if args.fixtures_dir else output
        fixture_output.mkdir(parents=True, exist_ok=True)
        fixture_paths = []
        for source, name in (('fixture', 'conformance'), ('host_services', 'host-services'), ('handle_services', 'handle-services')):
            path = fixture_output / ('photocraft-' + name + '.8bf')
            if not args.fixtures_dir:
                command = ['rustc', '--edition=2024', '--crate-type=cdylib', '--target', target]
                if args.runner == 'wine':
                    linker = os.environ.get('CARGO_TARGET_' + target.upper().replace('-', '_') + '_LINKER', arch + '-w64-mingw32-gcc')
                    command += ['-C', 'linker=' + linker]
                command += ['-O', str(root / 'tests' / (source + '.rs')), '-o', str(path)]
                subprocess.run(command, check=True)
            if not path.is_file():
                raise FileNotFoundError(f'missing {arch} fixture: {path}')
            fixture_paths.append(path)
        if not worker.is_file():
            raise FileNotFoundError(f'missing {arch} worker: {worker}')
        fixture, services, handles = fixture_paths
        cases = [dict(name=mode, mode=mode, width=320, height=240, channels=3, depth=8)
                 for mode in ('invert', 'noise', 'cancel', 'crash', 'exit_error', 'bad_rect',
                              'ignored_callback', 'threaded_alloc', 'mask_request',
                              'ignored_mask_callback', 'mask_with_pixels', 'start_error')]
        for name, channels, depth, mode in (
            ('gray8_invert', 1, 8, 'invert'),
            ('gray8_identity', 1, 8, 'identity'),
            ('rgb16_invert', 3, 16, 'invert'),
            ('rgb16_identity', 3, 16, 'identity'),
            ('gray16_invert', 1, 16, 'invert'),
            ('gray16_identity', 1, 16, 'identity'),
            ('rgb16_single_plane', 3, 16, 'single_plane'),
        ):
            cases.append(dict(name=name, mode=mode, width=31, height=17,
                              channels=channels, depth=depth,
                              selected_plane=1 if mode == 'single_plane' else None))
        if args.large:
            cases.append(dict(name='24mp_invert', mode='invert', width=6000,
                              height=4000, channels=3, depth=8))
            cases.append(dict(name='24mp_rgb16_identity', mode='identity', width=6000,
                              height=4000, channels=3, depth=16))
        cases.append(dict(name='host_services', mode='identity', width=2, height=1,
                          channels=3, depth=8, fixture=services))
        for handle_case in ('owned', 'direct', 'indirect'):
            cases.append(dict(name='handle_' + handle_case, mode='identity', width=2, height=1,
                              channels=3, depth=8, fixture=handles, handle_case=handle_case))
        for case in cases:
            pixels = sample_pixels(case['width'], case['height'], case['channels'], case['depth'])
            elapsed = exchange(worker, case.get('fixture', fixture), case['mode'],
                               case['width'], case['height'], pixels, dict(environment, PHOTOCRAFT_8BF_HANDLE_CASE=case.get('handle_case', 'owned')),
                               channels=case['channels'], depth=case['depth'],
                               selected_plane=case.get('selected_plane'), runner=args.runner)
            result = dict(architecture=arch, case=case['name'],
                          passed=True, worker_round_trip_seconds=round(elapsed, 4),
                          pixels=case['width'] * case['height'])
            results.append(result)
            print(json.dumps(result), flush=True)
    (root / 'target').mkdir(parents=True, exist_ok=True)
    (root / 'target/conformance.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
