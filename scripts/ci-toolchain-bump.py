#!/usr/bin/env python3
"""Prints the newest stable Rust release when it is newer than rust-toolchain.toml's channel.

CI's nightly run (ci.yml `toolchain-check`) warns with it. Prints nothing when the file is current.
"""

import re
import sys
import tomllib
import urllib.request
from pathlib import Path

FILE = Path(__file__).resolve().parent.parent / 'rust-toolchain.toml'
MANIFEST = 'https://static.rust-lang.org/dist/channel-rust-stable.toml'
RELEASE = re.compile(r'^\d+\.\d+\.\d+$')


def numbers(version):
    return tuple(int(part) for part in version.split('.'))


def current(text):
    return tomllib.loads(text)['toolchain']['channel']


def latest(manifest_text):
    """`1.101.0` from the stable manifest's `[pkg.rust] version = "1.101.0 (<hash> <date>)"`."""
    version = tomllib.loads(manifest_text)['pkg']['rust']['version'].split()[0]
    if not RELEASE.match(version):
        raise ValueError(f'unexpected stable version {version!r}')
    return version


def newer(file_text, manifest_text):
    """The stable release when it is newer than the file's channel (a `1.x.y` release), else None."""
    pinned = current(file_text)
    stable = latest(manifest_text)
    if RELEASE.match(pinned) and numbers(stable) <= numbers(pinned):
        return None
    return stable


def main(argv):
    with urllib.request.urlopen(MANIFEST, timeout=30) as response:
        stable = newer(FILE.read_text(), response.read().decode())
    if stable:
        print(stable)


if __name__ == '__main__':
    main(sys.argv[1:])
