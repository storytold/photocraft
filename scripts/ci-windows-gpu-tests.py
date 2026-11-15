"""Whether a pull request's Windows legs run photocraft-gpu's tests (.github/workflows/ci.yml).

The Windows runners have no GPU: wgpu renders on WARP, the Direct3D 12 software rasterizer, where
photocraft-gpu's parity tests take about half of the integration leg's test time (#15247). The Linux
gate runs them on every pull request, and main runs them on Windows too, so a pull request's Windows
legs skip them unless the pull request changes something they are built from:

- photocraft-gpu or a workspace package it depends on (any dependency kind, from `cargo metadata`,
  so a new dependency is covered without editing this file);
- any package's `Cargo.toml`: `cargo test --workspace` unifies features across the members, so a
  feature turned on in another member's manifest can change how photocraft-gpu's dependencies are
  built, and `Cargo.lock` does not record features;
- any file outside the workspace packages, except the documentation-only paths below. That covers
  the shared build inputs (workspace manifest, lockfile, Cargo and nextest config, this workflow,
  this script) and any input added later.

Usage: ci-windows-gpu-tests.py BASE  prints `true` (run them) or `false` (skip them).
"""

import json
import subprocess
import sys

GPU = 'photocraft-gpu'

# Outside every workspace package and never read by a build or a test. Anything else outside the
# packages counts as related, so an unknown new input runs the tests rather than skipping them.
UNRELATED_PREFIXES = ('docs/', 'log/', 'contributors/', 'scorecard/')
UNRELATED_SUFFIXES = ('.md',)


def gpu_inputs(packages):
    """The workspace packages photocraft-gpu is built from (itself included).

    `packages` is `cargo metadata`'s `packages` list. Raises ValueError when photocraft-gpu is not in
    the workspace, so a rename cannot silently turn the selection off.
    """
    names = {p['name'] for p in packages}
    if GPU not in names:
        raise ValueError(f'{GPU} is not a workspace package')
    deps = {p['name']: {d['name'] for d in p['dependencies'] if d['name'] in names} for p in packages}
    seen, todo = {GPU}, [GPU]
    while todo:
        for d in deps[todo.pop()]:
            if d not in seen:
                seen.add(d)
                todo.append(d)
    return seen


def package_dirs(packages, root):
    """Each package's directory relative to the workspace root, with `/` separators."""
    root = root.replace('\\', '/').rstrip('/') + '/'
    out = {}
    for p in packages:
        manifest = p['manifest_path'].replace('\\', '/')
        if manifest.startswith(root):
            rel = manifest[len(root):]
            out[p['name']] = rel.rsplit('/', 1)[0] if '/' in rel else ''
    return out


def needs_gpu_tests(changed, packages, root):
    """Whether the changed files (paths relative to the workspace root) can affect the GPU tests."""
    inputs = gpu_inputs(packages)
    # Longest directory first, so a nested package wins over the one around it.
    dirs = sorted(((d, n) for n, d in package_dirs(packages, root).items() if d), key=lambda x: -len(x[0]))
    for path in changed:
        # Feature unification: any member manifest can change the GPU build (module docstring).
        if path == 'Cargo.toml' or path.endswith('/Cargo.toml'):
            return True
        owner = next((n for d, n in dirs if path.startswith(d + '/')), None)
        if owner is not None:
            if owner in inputs:
                return True
            continue
        if path.startswith(UNRELATED_PREFIXES) or path.endswith(UNRELATED_SUFFIXES):
            continue
        return True
    return False


def changed_files(base, cwd=None):
    """The files changed between `base` and HEAD. `--no-renames` lists both sides of a move: by
    default git names only the destination, so moving a file out of photocraft-gpu would look like a
    change in the package it moved to."""
    out = subprocess.run(['git', 'diff', '--name-only', '--no-renames', base, 'HEAD'], check=True, capture_output=True, text=True, cwd=cwd)
    return out.stdout.split()


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    base = argv[1]
    changed = changed_files(base)
    meta = json.loads(subprocess.run(['cargo', 'metadata', '--no-deps', '--format-version', '1'], check=True, capture_output=True, text=True).stdout)
    print('true' if needs_gpu_tests(changed, meta['packages'], meta['workspace_root']) else 'false')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
