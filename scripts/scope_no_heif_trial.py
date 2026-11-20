"""Restrict the disposable CI workspace-hack to pins that preserve the I/O graph."""

import argparse
import json
import tomllib
from pathlib import Path


# Keep pins that preserve both the test and production I/O package sets.
PINS = set(
    """bitflags digest ecolor either emath font-types icu_collections
    icu_normalizer icu_properties icu_provider_blob indexmap linux-raw-sys
    proc-macro2 rustix serde_json skrifa syn toml typenum uuid wayland-sys""".split()
)


def exclusions(manifest):
    packages = {}
    for table in [manifest, *manifest.get("target", {}).values()]:
        for kind in ("dependencies", "build-dependencies"):
            for alias, spec in table.get(kind, {}).items():
                name = spec.get("package", alias)
                if name not in PINS:
                    source = {k: spec[k] for k in ("git", "rev", "tag", "branch") if k in spec}
                    packages[name, tuple(source.items())] = {"name": name, **source}
    return [packages[key] for key in sorted(packages)]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exclude-codecs", action="store_true", help="exclude the workspace's two codec dependencies before generation")
    if parser.parse_args().exclude_codecs:
        with Path("Cargo.toml").open("rb") as manifest:
            workspace_deps = tomllib.load(manifest)["workspace"]["dependencies"]
        packages = []
        for alias in ("photocraft-codecs", "photocraft-formats"):
            dep = workspace_deps[alias]
            packages.append({"name": dep["package"], "git": dep["git"], "rev": dep["rev"]})
    else:
        with Path("crates/workspace-hack/Cargo.toml").open("rb") as generated:
            packages = exclusions(tomllib.load(generated))
    # This config starts as a fresh copy of the checked-in trial template.
    with Path(".config/hakari.toml").open("a", encoding="utf-8") as config:
        for package in packages:
            config.write("\n[[final-excludes.third-party]]\n")
            for key, value in package.items():
                config.write(f"{key} = {json.dumps(value)}\n")
