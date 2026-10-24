#!/usr/bin/env bash
# Acquire the audit toolchain on a Linux cloud container (Debian/Ubuntu, Go, Node, JRE
# preinstalled) without touching the PhotoCraft repo. Idempotent; prints every version.
#   scc 3.5.0           go install (no repo change)            -> ~/go/bin/scc
#   Graphviz            apt-get                                 -> dot
#   PlantUML 1.2025.4   release jar (bundles the C4 stdlib)     -> ~/.local/share/plantuml/plantuml.jar
#   jscpd 4.0.5         npx, run on demand (not installed in the repo)
#   helpers             rustc, std-only single files            -> $AUDIT_BIN (default ~/.cache/cartography/bin)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="${AUDIT_BIN:-$HOME/.cache/cartography/bin}"
mkdir -p "$bin" "$HOME/.local/share/plantuml"

if ! command -v scc >/dev/null && [ ! -x "$HOME/go/bin/scc" ]; then
  go install github.com/boyter/scc/v3@v3.5.0
fi
if ! command -v dot >/dev/null; then
  apt-get install -y -qq graphviz >/dev/null 2>&1 || { apt-get update -qq && apt-get install -y -qq graphviz; }
fi
jar="$HOME/.local/share/plantuml/plantuml.jar"
[ -s "$jar" ] || curl -sSfL -o "$jar" \
  https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml-1.2025.4.jar

for h in metrics sizing testlist fpcount; do
  rustc -O --edition 2021 "$here/$h.rs" -o "$bin/$h"
done

scc_bin="$(command -v scc || echo "$HOME/go/bin/scc")"
echo "scc:      $("$scc_bin" --version)  ($scc_bin)"
echo "graphviz: $(dot -V 2>&1)"
echo "plantuml: $(java -jar "$jar" -version 2>/dev/null | head -1)  ($jar)"
echo "jscpd:    $(npx -y jscpd@4.0.5 --version 2>/dev/null | tail -1) (npx)"
echo "rustc:    $(rustc --version)"
echo "clippy:   $(cargo clippy --version)"
echo "helpers:  $bin/{metrics,sizing,testlist,fpcount}"
