#!/usr/bin/env bash
set -euo pipefail

# The pre-rename name of albite-docker.sh, kept for one release (ADR 20.10.26d).
# It prints one notice and hands every argument to the script beside it.

echo "note: \`tungsten-docker.sh\` is now \`albite-docker.sh\`; this name is removed in 2.1" >&2
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/albite-docker.sh" "$@"
