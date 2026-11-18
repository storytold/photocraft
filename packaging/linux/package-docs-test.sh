#!/usr/bin/env bash
# Exercise Linux staging and both Flatpak manifests without building Rust or an AppImage.
# Needs the normal Linux packaging tools, Python 3 and PyYAML (packaging-lint installs them).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - "$ROOT" <<'PY'
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import zipfile

import yaml

root = Path(sys.argv[1])
docs = ("mcp.md", "control-protocol.md")


def run(command, cwd, env):
    result = subprocess.run(command, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise AssertionError(f"{command!r} failed ({result.returncode}):\n{result.stdout}")


def assert_docs(directory):
    for name in docs:
        path = directory / name
        assert path.is_file(), f"missing packaged document: {path}"
        assert path.read_bytes() == (root / "docs" / name).read_bytes(), f"changed packaged document: {path}"


with tempfile.TemporaryDirectory(prefix="photocraft-package-docs-") as temporary:
    temp = Path(temporary)
    fixture = temp / "source with spaces"
    fixture.mkdir()
    # Keep the real scripts, metadata, icons and document bytes. Only replace the binary
    # build and AppImage download/tool boundary; no real tool or runtime is downloaded.
    for name in ("packaging", "assets/app-icon/hicolor", "docs/mcp.md", "docs/control-protocol.md", "Cargo.toml", "README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "COPYRIGHT", "NOTICE", "ATTRIBUTION.md"):
        source, destination = root / name, fixture / name
        if not source.exists():
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        if source.is_dir():
            shutil.copytree(source, destination)
        else:
            shutil.copy2(source, destination)

    binary = "#!/usr/bin/env bash\nprintf 'PhotoCraft packaging fixture\\n'\n"
    target = fixture / "target"
    for name in ("photocraft", "photocraft-cli"):
        path = target / "release" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(binary)
        path.chmod(0o755)
    (fixture / "packaging/linux/verified-download.sh").write_text('''
download_verified() {
  if [ "${4:-false}" = true ]; then
    cat >"$2" <<'TOOL'
#!/usr/bin/env bash
set -euo pipefail
# Stand in for appimagetool's output, leaving the real AppDir for assertions below.
touch "${@: -1}"
TOOL
    chmod 755 "$2"
  else
    printf 'fixture runtime\\n' >"$2"
  fi
}
verify_appimagetool_override() { return 1; }
''')
    env = os.environ.copy()
    for key in ("CROSS_TARGET", "CROSS_COMPILE", "EMULATOR", "APPIMAGETOOL", "CRAFT_FONTS_DIR"):
        env.pop(key, None)
    env.update(CARGO_TARGET_DIR=str(target), DIST=str(temp / "dist"), CROSS_ARCH="x86_64", PHOTOCRAFT_BUILD_DATE="2026-01-01")

    # Tiny prebuilt web artifacts exercise font copying without compiling Rust.
    web = fixture / "dist/web"
    for site in (web, fixture / "dist/web-boot"):
        site.mkdir(parents=True)
        (site / "index.html").write_text("<html></html>")
    (web / "font-fixture_bg.wasm").write_bytes(b"\0asm\x01\0\0\0")
    (web / "font-fixture.js").write_bytes(b"")
    font_source = temp / "served fonts"
    (font_source / "fonts").mkdir(parents=True)
    font_files = {"fonts/plain.ttf": b"plain font", "fonts/licensed.ttf": b"licensed font", "fonts/OFL.txt": b"licence"}
    for name, data in font_files.items():
        (font_source / name).write_bytes(data)
    font_manifest = (
        b"# Served fonts\n\n"
        b"Plain | Regular | fonts/plain.ttf\n"
        b"Licensed | Regular | fonts/licensed.ttf | Latn | OFL-1.1 | fonts/OFL.txt\n"
    )
    web_name = "photocraft-web-1.2.3-font-test"
    web_env = dict(env, PHOTOCRAFT_VERSION="1.2.3-font-test", PHOTOCRAFT_BUILD_SHA="fixture", PHOTOCRAFT_WEB_FONTS_DIR=str(font_source))
    for ending in (b"\n", b"\r\n"):
        manifest_bytes = font_manifest.replace(b"\n", ending)
        (font_source / "fonts/manifest.txt").write_bytes(manifest_bytes)
        run(["bash", "packaging/web/package.sh", "--skip-build"], fixture, web_env)
        with zipfile.ZipFile(temp / "dist" / f"{web_name}.zip") as package:
            assert package.read(f"{web_name}/fonts/manifest.txt") == manifest_bytes
            for name, data in font_files.items():
                assert package.read(f"{web_name}/{name}") == data, f"changed packaged font or licence: {name}"
    print("PASS: web packaging imports three/six-field font manifests with LF and CRLF")

    run(["bash", "packaging/linux/package.sh", "--skip-build", "--formats", "tar appimage"], fixture, env)
    assert_docs(target / "linux-package/root/usr/share/doc/photocraft")
    assert_docs(target / "linux-package/PhotoCraft.AppDir/usr/share/doc/photocraft")
    archives = list((temp / "dist").glob("*.tar.gz"))
    assert len(archives) == 1, f"expected one tarball, found {archives}"
    unpacked = temp / "unpacked"
    unpacked.mkdir()
    run(["tar", "-xzf", str(archives[0]), "-C", str(unpacked)], fixture, env)
    bundle = next(unpacked.iterdir())
    assert_docs(bundle / "share/doc/photocraft")
    assert (bundle / "bin/photocraft-cli").read_text() == binary
    print("PASS: FHS stage, tarball and AppDir retain both protocol documents")

    # The source manifest must request both packages; a missing -p photocraft-cli leaves
    # no CLI for its real install command to copy. Do not seed its build with staged bins.
    shutil.rmtree(target)
    tools = temp / "tools"
    tools.mkdir()
    cargo = tools / "cargo"
    cargo.write_text('''#!/usr/bin/env python3
from pathlib import Path
import sys
args = sys.argv[1:]
for index, arg in enumerate(args[:-1]):
    if arg == "-p":
        path = Path("target/release") / args[index + 1]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/usr/bin/env bash\\nprintf 'PhotoCraft packaging fixture\\\\n'\\n")
        path.chmod(0o755)
''')
    cargo.chmod(0o755)
    env["PATH"] = str(tools) + os.pathsep + env["PATH"]
    for suffix, directory in ((".yml", fixture), (".bundle.yml", bundle)):
        manifest = yaml.safe_load((root / f"packaging/linux/flatpak/ai.storyteller.photocraft{suffix}").read_text())
        app = temp / ("flatpak-source" if suffix == ".yml" else "flatpak-bundle")
        # Other modules, such as cups-client, build their own sources; only photocraft is checked here.
        module = next(m for m in manifest["modules"] if isinstance(m, dict) and m.get("name") == "photocraft")
        for command in module["build-commands"]:
            # Redirect only the sandbox output root, preserving the manifest's commands.
            command = command.replace("/app/", shlex.quote(str(app)) + "/")
            run(["bash", "-euo", "pipefail", "-c", command], directory, env)
        cli = app / "bin/photocraft-cli"
        assert cli.is_file() and os.access(cli, os.X_OK), f"missing executable CLI in {suffix}"
        assert cli.read_text() == binary, f"changed CLI bytes in {suffix}"
        assert_docs(app / "share/doc/photocraft")
        print(f"PASS: Flatpak {suffix} build commands install the CLI and both protocol documents")
PY
