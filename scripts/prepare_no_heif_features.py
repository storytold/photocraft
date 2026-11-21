"""Prepare the isolated no-HEIF feature trial and check its Cargo graphs."""

import argparse
import shutil
import subprocess
import tomllib
from pathlib import Path


PACKAGE = "no-heif-ci-features"


def packages(edges, *extra, locked=True):
    args = ["cargo", "tree", "--color", "never", "--prefix", "none", "--format", "{p}",
            "--edges", edges, "-p", "photocraft-io"]
    if locked:
        args.append("--locked")
    for package in extra:
        args.extend(["-p", package])
    lines = subprocess.check_output(args, text=True).splitlines()
    return {line.removesuffix(" (*)") for line in lines if line}


def lock_packages():
    with Path("Cargo.lock").open("rb") as lock:
        return tomllib.load(lock)["package"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--dev', action='store_true')
    args = parser.parse_args()
    stock_lock = lock_packages()
    stock_tests = packages("normal,build,dev")
    stock_build = packages("normal,build")
    helper = Path("crates") / PACKAGE
    helper.mkdir()
    (helper / "src").mkdir()
    (helper / "src/lib.rs").touch()
    # A build script makes Cargo select the helper's host dependencies.
    (helper / "build.rs").write_text("fn main() {}\n", encoding="utf-8")
    shutil.copyfile(".config/no-heif-ci-features.toml", helper / "Cargo.toml")
    if args.dev:
        manifest = helper / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('[dependencies]', '[dev-dependencies]', 1))
        (helper / 'src/lib.rs').write_text('#[cfg(test)] mod tests { #[test] fn feature_carrier() {} }\n')

    selected = packages("normal,build,dev", PACKAGE, locked=False)
    selected = {pkg for pkg in selected if not pkg.startswith(f"{PACKAGE} v")}
    if selected != stock_tests:
        raise SystemExit(f"test package set changed: added {sorted(selected-stock_tests)}, "
                         f"removed {sorted(stock_tests-selected)}")
    if packages("normal,build") != stock_build:
        raise SystemExit("the normal/build I/O package set changed")
    if args.dev:
        combined = {pkg for pkg in packages('normal,build', PACKAGE) if not pkg.startswith(f'{PACKAGE} v')}
        if combined != stock_build:
            raise SystemExit(f'combined normal/build set changed: added {sorted(combined-stock_build)}, removed {sorted(stock_build-combined)}')
    if [pkg for pkg in lock_packages() if pkg["name"] != PACKAGE] != stock_lock:
        raise SystemExit("existing Cargo.lock packages or dependency edges changed")
    print(f"Preserved {len(stock_tests)} I/O test packages, {len(stock_build)} normal/build "
          "packages and every existing Cargo.lock entry.")


if __name__ == "__main__":
    main()
