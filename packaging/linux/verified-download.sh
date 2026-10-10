#!/usr/bin/env bash
# Source after packaging/env.sh, which supplies sha256(). Keep the verified cache usable when
# an official asset was pre-seeded by a tool that created it without an execute bit.

download_verified() {
  local url="$1" dest="$2" expected="$3" tmp
  if [ -f "$dest" ] && [ "$(sha256 "$dest")" = "$expected" ]; then
    chmod 755 "$dest"
    return 0
  fi
  tmp="$(mktemp "$dest.XXXXXX")"
  if ! curl -fsSL -o "$tmp" "$url"; then rm -f "$tmp"; return 1; fi
  if [ "$(sha256 "$tmp")" != "$expected" ]; then
    echo "error: SHA-256 mismatch for $url" >&2
    rm -f "$tmp"
    return 1
  fi
  chmod 755 "$tmp"
  mv -f "$tmp" "$dest"
}

verify_appimagetool_override() {
  local tool="$1" expected="$2" version="$3" arch="$4"
  if [ ! -f "$tool" ] || [ "$(sha256 "$tool")" != "$expected" ]; then
    echo "error: APPIMAGETOOL must be the verified $version $arch release asset" >&2
    return 1
  fi
  if [ ! -x "$tool" ]; then
    echo "error: APPIMAGETOOL must be executable (chmod +x $tool)" >&2
    return 1
  fi
}
