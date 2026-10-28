#!/usr/bin/env bash
set -euo pipefail

# Stage Linux release binaries for distribution.
#
# Usage: package-linux.sh <albite> <output-dir>
#
# Copies the statically-linked compiler into <output-dir> as `albite`, the
# deprecated `tungsten` shim built beside it (ADR 20.10.26d D1/D2: it prints
# one notice and execs the `albite` in its own directory), and the
# Dockerfile + albite-docker.sh with its forwarding tungsten-docker.sh.

if [ $# -ne 2 ]; then
    echo "Usage: $(basename "$0") <albite> <output-dir>" >&2
    exit 1
fi

ALBITE="$1"
OUT="$2"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# libalbite_core.a and the `tungsten` shim live next to the binary: one
# `cargo build --release` writes all three into the same directory.
ALBITE_DIR="$(dirname "$ALBITE")"
STATIC_LIB="$ALBITE_DIR/libalbite_core.a"
SHIM="$ALBITE_DIR/tungsten"
for required in "$STATIC_LIB" "$SHIM"; do
    if [ ! -f "$required" ]; then
        echo "error: $required not found (built beside $ALBITE by \`cargo build --release\`)" >&2
        exit 1
    fi
done

mkdir -p "$OUT"
cp "$ALBITE" "$OUT/albite"
cp "$SHIM" "$OUT/tungsten"
cp "$STATIC_LIB" "$OUT/"
cp "$REPO_ROOT/Dockerfile" "$REPO_ROOT/albite-docker.sh" "$REPO_ROOT/tungsten-docker.sh" "$OUT/"
chmod +x "$OUT/albite-docker.sh" "$OUT/tungsten-docker.sh"

echo "✓ Packaged albite + tungsten (shim) + libalbite_core.a into $OUT"
