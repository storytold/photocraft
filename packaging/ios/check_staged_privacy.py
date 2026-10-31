"""Reject common private/signing data in staged changes; never print matched values.

Run from a Git checkout before committing. This is a heuristic, not a secrets scanner
or a guarantee that arbitrary personal information can be recognized automatically.
"""
import pathlib
import re
import subprocess
import sys


def git(*args):
    return subprocess.check_output(["git", *args])


def main():
    paths = git("diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z").split(b"\0")
    patterns = {
        "home directory": rb"/(?:Users|home)/[A-Za-z0-9_.-]+/",
        "email address": rb"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
        "private key": rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----",
        "device identifier": rb"\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{16}\b",
        "signing team": rb"DEVELOPMENT_TEAM\s*=\s*[A-Z0-9]{10}\b",
        "GitHub token": rb"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b",
        "Apple API key id": rb"AuthKey_[A-Z0-9]{10}\.p8",
    }
    findings = []
    checked = 0
    for raw in filter(None, paths):
        path = raw.decode("utf-8")
        checked += 1
        parts = pathlib.PurePosixPath(path).parts
        if ("xcuserdata" in parts or "target" in parts or "secrets" in parts
                or any(part.endswith(".xcarchive") for part in parts)
                or path.endswith(("Signing.local.xcconfig", ".ipa", ".mobileprovision",
                                  ".provisionprofile", ".p12", ".p8", ".cer", ".key"))):
            findings.append((path, "private file or build output"))
            continue
        data = git("show", ":" + path)
        for label, pattern in patterns.items():
            if re.search(pattern, data):
                findings.append((path, label))
    for path, label in findings:
        print(f"BLOCKED: {path}: {label}", file=sys.stderr)
    print(f"Privacy check: {checked} staged files, {len(findings)} findings.")
    return bool(findings)


if __name__ == "__main__":
    sys.exit(main())
