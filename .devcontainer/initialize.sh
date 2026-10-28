#!/bin/sh
# .devcontainer/initialize.sh — the devcontainers' `initializeCommand` (ADR 16.10.26p).
#
# Runs on the HOST before every `devcontainer up` (docker-vm's DevcontainerUp
# included), with the workspace folder as $1. Two jobs:
#
#  D1  Every writable bind in devcontainer.json needs a source, and a missing
#      `type=bind` source is a hard error. So each allowlisted directory that
#      does not exist yet is created — except `.git`, which is never created.
#      A symlinked entry is refused: Docker resolves bind sources, so a
#      committed `benchmarks -> .` would bind the whole root writable. An
#      existing non-directory (a gitfile `.git`, a stray file) is left alone.
#
#  D2  Container outputs now live under out/. Relative root symlinks
#      (`tungsten1 -> out/tungsten1`, likewise tungsten2 and tungsten1_x86) keep
#      `./tungsten1 …` working; a relative target resolves inside the
#      container's writable out/ bind, where an absolute host path would dangle.
#      On macOS only, a root name that is a container-built ELF regular file is
#      moved into out/ first (when out/<name> is absent), so it cannot shadow
#      out/ forever. A Mach-O (host-native) build is never touched.
set -eu

ws=${1:?usage: initialize.sh <workspace folder>}
allowlist="$ws/.devcontainer/writable-dirs.txt"

while IFS= read -r entry || [ -n "$entry" ]; do
	case "$entry" in '' | '#'*) continue ;; esac
	path="$ws/$entry"
	if [ -L "$path" ]; then
		echo "initialize: refusing: allowlisted $entry is a symlink — Docker would bind its target writable (ADR 16.10.26p D1)" >&2
		exit 1
	fi
	if [ "$entry" = .git ]; then
		# devcontainer.json cannot omit a mount, so a checkout with no .git
		# cannot satisfy the .git bind: say so instead of Docker's error.
		if [ ! -e "$path" ]; then
			echo "initialize: no .git in $ws — the devcontainer binds .git writable for commits; run 'git init' (ADR 16.10.26p D1)" >&2
			exit 1
		fi
		continue
	fi
	[ -e "$path" ] || mkdir -p "$path"
done <"$allowlist"

is_elf() {
	[ "$(head -c 4 "$1" | od -An -tx1 | tr -d ' \n')" = 7f454c46 ]
}

for name in tungsten1 tungsten2 tungsten1_x86; do
	root="$ws/$name"
	if [ -f "$root" ] && [ ! -L "$root" ]; then
		if [ "$(uname -s)" = Darwin ] && is_elf "$root" && [ ! -e "$ws/out/$name" ]; then
			mv "$root" "$ws/out/$name"
		else
			continue
		fi
	fi
	if [ -L "$root" ] || [ ! -e "$root" ]; then
		ln -sfn "out/$name" "$root"
	fi
done
