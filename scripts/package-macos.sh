#!/usr/bin/env bash
set -euo pipefail

# Stage macOS release binaries for distribution.
#
# Usage: package-macos.sh <albite1> <bootstrap> <output-dir>
#
# Copies the self-hosted compiler (as `albite`), the bootstrap (as
# `albite-bootstrap`) and the runtime library into <output-dir>. The
# deprecated `tungsten` is a second copy of the self-hosted `albite`, which
# prints its own one-line notice when run under that name (ADR 20.10.26d D2).

if [ $# -ne 3 ]; then
    echo "Usage: $(basename "$0") <albite1> <bootstrap> <output-dir>" >&2
    exit 1
fi

SELFHOSTED="$1"
BOOTSTRAP="$2"
OUT="$3"

# libalbite_core.a lives next to the bootstrap binary (same directory).
BOOTSTRAP_DIR="$(dirname "$BOOTSTRAP")"
STATIC_LIB="$BOOTSTRAP_DIR/libalbite_core.a"
if [ ! -f "$STATIC_LIB" ]; then
    echo "error: $STATIC_LIB not found (needed for linking compiled programs)" >&2
    exit 1
fi

mkdir -p "$OUT"
cp "$SELFHOSTED" "$OUT/albite"
cp "$SELFHOSTED" "$OUT/tungsten"
cp "$BOOTSTRAP" "$OUT/albite-bootstrap"
cp "$STATIC_LIB" "$OUT/"

echo "✓ Packaged albite + tungsten + albite-bootstrap + libalbite_core.a into $OUT"
