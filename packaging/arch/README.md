# Arch Linux (AUR)

Two AUR packages, both maintained from this directory:

| Package | What it installs | Build time |
|---|---|---|
| [`photocraft-bin`](https://aur.archlinux.org/packages/photocraft-bin) | the release's prebuilt `photocraft-<v>-linux-<arch>.tar.gz` (x86_64, aarch64) | seconds |
| [`photocraft`](https://aur.archlinux.org/packages/photocraft) | built from the release tag's source with the system `cargo` | 10 to 20 minutes |

Both conflict with each other and install the same tree as the `.deb`/`.rpm`: both binaries, the
desktop entry, MIME type, AppStream metainfo and hicolor icons. pacman's own hooks refresh the
desktop, MIME and icon caches, so there's no install script. Windowing and GPU libraries are
dlopen()ed, so the `depends` are listed by hand (why: `packaging/linux/nfpm.yaml`).

Users install with any AUR helper:

```sh
yay -S photocraft-bin    # or: yay -S photocraft
```

## Releasing

The PKGBUILDs here are templates: `update.sh <version>` points them at a published release
(pkgver, checksums, the tag's commit) and regenerates `.SRCINFO`. The AUR workflow
(`.github/workflows/aur.yml`) does this when a release is published and pushes both packages to
the AUR. Pre-releases are skipped. Details and the one-time setup: `docs/releasing.md` › AUR.

The `release` event runs the workflow file of the tagged commit, so the first release that
includes this directory is the first one published automatically. For an older release, run
*Actions → AUR → Run workflow* on `main` with its version.

By hand, on Arch:

```sh
packaging/arch/update.sh 0.2.0
cd packaging/arch/photocraft-bin
makepkg -si && namcap PKGBUILD *.pkg.tar.zst
git clone ssh://aur@aur.archlinux.org/photocraft-bin.git /tmp/aur-photocraft-bin
cp PKGBUILD .SRCINFO /tmp/aur-photocraft-bin/ && cd /tmp/aur-photocraft-bin
git commit -am "Update to 0.2.0" && git push
```

`pkgver` can't contain `-`, so `update.sh` maps a pre-release like `0.3.0-rc.1` to `0.3.0rc.1`
(which pacman sorts before `0.3.0`) and keeps the tag's spelling in `_version`.
