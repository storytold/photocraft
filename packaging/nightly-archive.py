#!/usr/bin/env python3
"""Photo-owned flat Linux archive policy. No path alias/PAX/link records."""
import pathlib
import re
import sys
import tarfile

APP_DOCS = {"README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "COPYRIGHT", "NOTICE", "ATTRIBUTION.md"}


def notice(name):
    return name in APP_DOCS or re.fullmatch(r"OFL-[A-Za-z0-9_.-]+\.txt", name) is not None


def linux_archive(stage, output):
    entries = sorted(stage.iterdir(), key=lambda item: item.name)
    folded = set()
    for entry in entries:
        name = entry.name
        if name not in {"photocraft", "photocraft-nightly.png"} and not notice(name):
            raise ValueError(f"unexpected nightly member: {name}")
        if entry.is_symlink() or not entry.is_file() or len(name.encode("utf-8")) > 100:
            raise ValueError(f"nightly member must be a short regular file: {name}")
        if name.casefold() in folded:
            raise ValueError(f"case-colliding nightly member: {name}")
        folded.add(name.casefold())
    required = {"photocraft", "photocraft-nightly.png", "README.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md"}
    if not required.issubset({item.name for item in entries}):
        raise ValueError("required GUI/icon/notices missing")
    with tarfile.open(output, "x:gz", format=tarfile.USTAR_FORMAT) as archive:
        for entry in entries:
            archive.add(entry, arcname=entry.name, recursive=False)
    # Inspect the raw ordinary headers, not a transparent PAX/GNU-normalizing reader.
    import gzip
    with gzip.open(output, "rb") as stream:
        while True:
            header = stream.read(512)
            if header == b"\0" * 512:
                break
            if len(header) != 512 or header[156:157] not in {b"0", b"\0"}:
                raise ValueError("non-ordinary TAR record was emitted")
            size = int(header[124:136].rstrip(b"\0 ").lstrip(b" ") or b"0", 8)
            remaining = ((size + 511) // 512) * 512
            while remaining:
                chunk = stream.read(min(remaining, 1024 * 1024))
                if not chunk:
                    raise ValueError("truncated TAR member")
                remaining -= len(chunk)


if __name__ == "__main__":
    if len(sys.argv) != 4 or sys.argv[1] != "linux":
        raise SystemExit("usage: nightly-archive.py linux STAGE OUTPUT")
    linux_archive(pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]))
