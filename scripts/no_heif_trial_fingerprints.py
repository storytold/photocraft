"""Report the Cargo fingerprint records written by the isolated no-HEIF trial."""

import json
import sys
from collections import Counter
from pathlib import Path


def records(root):
    result = {}
    for pattern in ("debug/build/*/*/fingerprint/*.json", "debug/.fingerprint/*/*.json"):
        for path in root.glob(pattern):
            result[path.relative_to(root).as_posix()] = path.stat().st_mtime_ns
    return result


def package(path):
    parts = Path(path).parts
    if parts[1] == "build":
        return parts[2]
    return parts[2].rsplit("-", 1)[0]


if __name__ == "__main__":
    mode, build_dir, snapshot = sys.argv[1:]
    current = records(Path(build_dir))
    snapshot = Path(snapshot)
    if mode == "before":
        snapshot.write_text(json.dumps(current), encoding="utf-8")
        print(f"Restored {len(current)} fingerprint records")
    elif mode == "after":
        before = json.loads(snapshot.read_text(encoding="utf-8"))
        restored_packages = {package(path) for path in before}
        counts = Counter()
        for path, timestamp in sorted(current.items()):
            if before.get(path) == timestamp:
                continue
            if path in before:
                reason = "existing record rewritten"
            elif package(path) in restored_packages:
                reason = "new record for restored package name"
            else:
                reason = "no restored record for package name"
            counts[reason] += 1
            print(f"{reason}\t{path}")
        print(f"Fingerprint records written by category: {dict(counts)}")
    else:
        raise SystemExit(f"Unknown snapshot mode: {mode}")
