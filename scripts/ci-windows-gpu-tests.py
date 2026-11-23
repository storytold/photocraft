"""What a pull request's Windows legs skip (.github/workflows/ci.yml): photocraft-gpu's tests, and the
tests of packages with no platform-specific code.

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

The same rule picks the platform-neutral packages (NEUTRAL) to skip: one is skipped when neither it nor
any workspace package it depends on has a changed file, and nothing shared changed (any manifest, or
a file outside the packages that is not documentation). They have no `cfg(windows)`, `cfg(unix)` or
`cfg(target_os ...)` in their sources, so Windows adds no coverage over the Linux gate for them.

The Linux gate skips by the same rule for every package except photocraft-gpu, whose tests the Linux
gate runs on every pull request (`--skip-linux`).

Usage:
  ci-windows-gpu-tests.py BASE               prints `true` (run photocraft-gpu's tests) or `false`.
  ci-windows-gpu-tests.py --skip-neutral BASE  prints the neutral packages to skip, space-separated
                                               (nothing when none).
  ci-windows-gpu-tests.py --skip-linux BASE    prints the packages the Linux gate skips, space-separated
                                               (nothing when none).
"""

import json
import subprocess
import sys

GPU = 'photocraft-gpu'

# Outside every workspace package and never read by a build or a test. Anything else outside the
# packages counts as related, so an unknown new input runs the tests rather than skipping them.
UNRELATED_PREFIXES = ('docs/', 'log/', 'contributors/', 'scorecard/')
UNRELATED_SUFFIXES = ('.md',)

# Workspace packages whose sources and tests have no platform-specific code (checked with
# `grep -rE 'cfg\(.*(windows|unix|target_os|target_family)'` over each package; the only `cfg` in their
# manifests is `not(target_arch = "wasm32")`). Add a package only after the same check.
NEUTRAL = (
    'photocraft-algo',
    'photocraft-color',
    'photocraft-compose',
    'photocraft-doc',
    'photocraft-geom',
    'photocraft-ops',
    'photocraft-paint',
    'photocraft-raw',
    'photocraft-vector',
)


def workspace_deps(packages):
    """Each workspace package's workspace dependencies (every kind, from `cargo metadata`)."""
    names = {p['name'] for p in packages}
    return {p['name']: {d['name'] for d in p['dependencies'] if d['name'] in names} for p in packages}


def inputs_of(name, deps):
    """`name` and every workspace package it depends on, directly or not."""
    seen, todo = {name}, [name]
    while todo:
        for d in deps[todo.pop()]:
            if d not in seen:
                seen.add(d)
                todo.append(d)
    return seen


def gpu_inputs(packages):
    """The workspace packages photocraft-gpu is built from (itself included).

    `packages` is `cargo metadata`'s `packages` list. Raises ValueError when photocraft-gpu is not in
    the workspace, so a rename cannot silently turn the selection off.
    """
    if GPU not in {p['name'] for p in packages}:
        raise ValueError(f'{GPU} is not a workspace package')
    return inputs_of(GPU, workspace_deps(packages))


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


def changed_packages(changed, packages, root):
    """The workspace packages that own a changed file (paths relative to the workspace root), or None
    when a change can affect every package: any package manifest, or a file outside the packages that
    is not documentation (module docstring)."""
    # Longest directory first, so a nested package wins over the one around it.
    dirs = sorted(((d, n) for n, d in package_dirs(packages, root).items() if d), key=lambda x: -len(x[0]))
    owners = set()
    for path in changed:
        # Feature unification: any member manifest can change the build of every other package.
        if path == 'Cargo.toml' or path.endswith('/Cargo.toml'):
            return None
        owner = next((n for d, n in dirs if path.startswith(d + '/')), None)
        if owner is not None:
            owners.add(owner)
            continue
        if path.startswith(UNRELATED_PREFIXES) or path.endswith(UNRELATED_SUFFIXES):
            continue
        return None
    return owners


def needs_gpu_tests(changed, packages, root):
    """Whether the changed files (paths relative to the workspace root) can affect the GPU tests."""
    inputs = gpu_inputs(packages)
    owners = changed_packages(changed, packages, root)
    return owners is None or bool(owners & inputs)


def neutral_to_skip(changed, packages, root):
    """The NEUTRAL packages whose tests the Windows legs skip for these changed files, sorted."""
    owners = changed_packages(changed, packages, root)
    if owners is None:
        return []
    deps = workspace_deps(packages)
    return sorted(n for n in NEUTRAL if n in deps and not (inputs_of(n, deps) & owners))


def linux_to_skip(changed, packages, root):
    """Every workspace package but photocraft-gpu whose tests the Linux gate skips for these changed
    files, sorted: nothing it depends on (itself included) has a changed file, and nothing shared
    changed."""
    owners = changed_packages(changed, packages, root)
    if owners is None:
        return []
    deps = workspace_deps(packages)
    return sorted(n for n in deps if n != GPU and not (inputs_of(n, deps) & owners))


def changed_files(base, cwd=None):
    """The files changed between `base` and HEAD. `--no-renames` lists both sides of a move: by
    default git names only the destination, so moving a file out of photocraft-gpu would look like a
    change in the package it moved to."""
    out = subprocess.run(['git', 'diff', '--name-only', '--no-renames', base, 'HEAD'], check=True, capture_output=True, text=True, cwd=cwd)
    return out.stdout.split()


def main(argv):
    args = argv[1:]
    skip_neutral = args[:1] == ['--skip-neutral']
    skip_linux = args[:1] == ['--skip-linux']
    if skip_neutral or skip_linux:
        args = args[1:]
    if len(args) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    changed = changed_files(args[0])
    meta = json.loads(subprocess.run(['cargo', 'metadata', '--no-deps', '--format-version', '1'], check=True, capture_output=True, text=True).stdout)
    if skip_linux:
        print(' '.join(linux_to_skip(changed, meta['packages'], meta['workspace_root'])))
    elif skip_neutral:
        print(' '.join(neutral_to_skip(changed, meta['packages'], meta['workspace_root'])))
    else:
        print('true' if needs_gpu_tests(changed, meta['packages'], meta['workspace_root']) else 'false')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
