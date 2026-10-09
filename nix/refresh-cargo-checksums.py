#!/usr/bin/env nix-shell
#!nix-shell -i python3 -p python3
"""Record file hashes after reviewed patches; retain Cargo.lock's original package hash."""
import hashlib
import json
import sys
from pathlib import Path

vendor = Path(sys.argv[1])
for name in sys.argv[2:]:
    crate = vendor / name
    checksum = crate / '.cargo-checksum.json'
    manifest = json.loads(checksum.read_text())
    manifest['files'] = {
        path.relative_to(crate).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(crate.rglob('*'))
        if path.is_file() and path != checksum
    }
    checksum.write_text(json.dumps(manifest, sort_keys=True) + '\n')
