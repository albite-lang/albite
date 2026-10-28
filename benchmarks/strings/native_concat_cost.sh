#!/usr/bin/env bash
# The native concat's cost (ADR 4.10.26n P0.3 and Step 7; ADR 6.10.26m D1).
# Runs in the devcontainer through `make devcontainer-native-concat-cost`;
# results are copied by hand into benchmarks/results/<adr>.native-concat-cost.md.
#
#   MODE=prototype  the port as an ordinary function over the checked
#                   conversion (string_concat*_native.tg) against Rust.
#   MODE=export     the real export: the same `++` programs built with
#                   ALBITE_NATIVE_CORE=off and =all (ADR 9.10.26n D4: since
#                   7.10.26q `on` enables no concat row, so `off` against `on`
#                   would be Rust against Rust).
#   MODE=selfcheck  `tungsten1 check src/compiler/main.tg`, a tungsten1 built
#                   with one setting against one built with another
#                   (6.10.26m D1; SETTINGS="off on" by default, "off all" for
#                   6.10.26n D6): bench-compare's counts (--warmup 2 --runs 10)
#                   after an A/A run, peak RSS, and whether both print the same
#                   diagnostics.
#   MODE=arena      MODE=export's long shape under ALBITE_ARENA=bump.
#   MODE=owned      ADR 9.10.26n D4: every shape `off` (Rust) against `all`
#                   (both concats native) — string_concat_owned.tg after an A/A
#                   run on `owned_off`, then the long and short shapes; the
#                   owned shape under ALBITE_ARENA=bump for `off`, `on` and
#                   `all`; peak RSS and page faults per binary; a perf profile
#                   of `short_all`, `owned_off` and `owned_all` (13.10.26p);
#                   and the --alloc-profile
#                   `string` class row `on` against `all` (6.10.26n D7).
#
# The micro shapes are built by `tungsten compile`, whose per-module objects
# are written at LLVM's `None` level (6.10.26m D1 records the level rather
# than assuming D8's "-O2"). `hyperfine --warmup 3 --runs 20`, an A/A run of
# the Rust build against itself first.
#
# Every run keeps its results (ADR 7.10.26p D5): it writes into
# build/native-concat-cost/<short sha>/<mode>/<run>/ beside a provenance.txt
# (the commit, the bootstrap binary's mtime, `git status --porcelain`), and
# never deletes an earlier run — the old `rm -rf` wiped 6.10.26m's baseline.
# <run> is a fresh timestamp-and-pid name, except under MODE=selfcheck, whose
# steps share one directory: RUN names it (default `steps`).
#   MODE=replay     ADR 16.10.26j D4: owned_grow_replay.c over 13.10.26p's
#                   eight shapes and D4's factors, then each factor's verdict
#                   under the allocator-work rule and the smallest that
#                   qualifies.
#   MODE=provenance  allocate and describe a run directory, and nothing else.
#
# Every mode records the container's load average, and the host's when the
# recipe passes it in as HOST_LOAD (the container cannot see the host's), at
# start and end (ADR 16.10.26j D6); the recipe appends the host's end line.
set -euo pipefail
TG="${TG:-${CARGO_TARGET_DIR:-target}/release/tungsten}"
MODE="${MODE:-prototype}"
# The container sees the bind-mounted checkout as another user's, so git
# trusts it for these reads only.
git() { command git -c safe.directory="$PWD" "$@"; }
SHA="$(git rev-parse --short HEAD)"
MODE_ROOT="build/native-concat-cost/$SHA/$MODE"
if [ "$MODE" = selfcheck ]; then RUN="${RUN:-steps}"; else RUN="run-$(date +%Y%m%dT%H%M%S)-$$"; fi
OUT="$MODE_ROOT/$RUN"
mkdir -p "$OUT"
{
  echo "commit: $(git rev-parse HEAD)"
  echo "bootstrap: $TG"
  echo "bootstrap mtime: $(stat -c %y "$TG" 2> /dev/null || stat -f %Sm "$TG" 2> /dev/null || echo absent)"
  echo "recorded: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "git status --porcelain:"
  git status --porcelain
  echo "host load (start): ${HOST_LOAD:-not passed in}"
  echo "container load (start): $(cat /proc/loadavg 2> /dev/null || echo n/a)"
} >> "$OUT/provenance.txt"
echo "$OUT" > build/native-concat-cost/last-run
trap 'echo "container load (end): $(cat /proc/loadavg 2> /dev/null || echo n/a)" >> "$OUT/provenance.txt"' EXIT
echo "== results in $OUT"
if [ "$MODE" = provenance ]; then exit 0; fi

if [ "$MODE" = replay ]; then
  # Where a block moves depends on the heap's starting offset as much as on
  # the policy (P0.2: Rust's own export moves 0 to 1 times a round on the
  # owned shape by offset alone), so every (policy, shape) runs at 33 offsets
  # and D4's rule reads each shape's MEAN; the strict per-offset count is
  # printed beside it.
  gcc -O2 -static -o "$OUT/owned_grow_replay" benchmarks/strings/owned_grow_replay.c
  factors="5/4 3/2 7/4 2/1 5/2 3/1"
  policies="rust today usable $(for f in $factors; do printf 'geo%s cap%s ' "$f" "$f"; done)"
  for shape in "64 3 4" "16 3 4" "100 3 4" "256 3 4" "64 40 40" "16 40 40" "100 40 40" "256 40 40"; do
    # shellcheck disable=SC2086 # one argument per policy and per shape field
    for policy in $policies; do for pad in 0 $(seq 8 16 504); do
      "$OUT/owned_grow_replay" "$policy" $shape "$pad"
    done; done
  done > "$OUT/replay.txt"
  python3 benchmarks/strings/owned_grow_verdict.py "$OUT/replay.txt" | tee "$OUT/verdict.txt"
  exit 0
fi

medians() { # <json name>...
  python3 - "$OUT" "$@" <<'PY'
import glob, json, statistics, sys
out = sys.argv[1]
for name in sys.argv[2:]:
    # A comparison taken in PART halves pools both halves' samples.
    files = sorted(glob.glob(f"{out}/{name}.part*.json")) or [f"{out}/{name}.json"]
    parts = [json.load(open(f))["results"] for f in files]
    medians = [statistics.median(t for p in parts for t in p[i]["times"]) * 1000 for i in range(2)]
    print(f"{name}: median ms {medians[0]:.2f} vs {medians[1]:.2f}  ratio {medians[1] / medians[0]:.3f}")
PY
}

peak_rss() { # <command...>
  # `-o`, not a stderr pipe: the piped form printed nothing under pipefail.
  /usr/bin/time -v -o "$OUT/rss.txt" "$@" > /dev/null 2>&1
  sed -n 's/.*Maximum resident set size (kbytes): //p' "$OUT/rss.txt"
}

page_faults() { # <command...>: minor + major, from `perf stat` (9.10.26n D4)
  perf stat -x, -e page-faults -o "$OUT/faults.txt" "$@" > /dev/null 2>&1 || { echo "n/a"; return; }
  sed -n 's/^\([0-9]*\),.*page-faults.*/\1/p' "$OUT/faults.txt"
}

perf_shares() { # <binary in $OUT>: every symbol down to 0.5%
  perf record -q -o "$OUT/$1.perf" "$OUT/$1" > /dev/null 2>&1 || { echo "  perf unavailable"; return; }
  echo "  -- $1"
  perf report -i "$OUT/$1.perf" --stdio --sort symbol 2> /dev/null \
    | grep -E '^ +([1-9][0-9]*|0\.[5-9])[0-9]*\.?[0-9]*%' | head -40 || true
}

if [ "$MODE" = selfcheck ]; then
  read -r SETTING_A SETTING_B <<< "${SETTINGS:-off on}"
  # In steps, each under a batch's 600 s foreground cap but the last:
  #   STEP=build SWITCH=off|on|all  `tungsten compile src/compiler/main.tg` under
  #                             one switch setting (the level is recorded
  #                             with the results, not assumed)
  #   STEP=rss                  one timed check per build: wall time, peak
  #                             RSS, exit, and whether the diagnostics agree
  #   STEP=bench-aa, bench-ab   bench-compare's counts (--warmup 2 --runs
  #                             10), A/A first, then A/B; PART=1|2 splits
  #                             each into halves of five (~6 min each)
  check() { echo "$OUT/tungsten1_$1 check src/compiler/main.tg"; }
  case "${STEP:-}" in
    build)
      ALBITE_NATIVE_CORE="$SWITCH" "$TG" compile src/compiler/main.tg -o "$OUT/tungsten1_$SWITCH" \
        > "$OUT/build_$SWITCH.log" 2>&1
      for symbol in tg_native_string_concat tg_native_string_concat_owned; do
        echo "  tungsten1_$SWITCH defines $symbol:" \
          "$(nm "$OUT/tungsten1_$SWITCH" | grep -c " T $symbol\$" || true)"
      done ;;
    rss)
      # P0.6: the check may exit non-zero on pre-existing diagnostics, and
      # both builds must print the same answer.
      # The check overflows an 8 MB stack on both builds, so it runs as the
      # profiling recipes run it, under `ulimit -s unlimited`.
      ulimit -s unlimited
      for switch in "$SETTING_A" "$SETTING_B"; do
        set +e
        /usr/bin/time -v -o "$OUT/time_$switch.txt" $(check "$switch") > "$OUT/diag_$switch.txt" 2>&1
        echo "  tungsten1_$switch: exit $?," \
          "$(sed -n 's/.*Elapsed (wall clock) time.*: //p' "$OUT/time_$switch.txt") wall," \
          "$(sed -n 's/.*Maximum resident set size (kbytes): //p' "$OUT/time_$switch.txt") KiB"
        set -e
      done
      cmp -s "$OUT/diag_$SETTING_A.txt" "$OUT/diag_$SETTING_B.txt" && echo "  diagnostics: identical" \
        || echo "  diagnostics: DIFFER" ;;
    bench-aa | bench-ab)
      ulimit -s unlimited
      if [ "$STEP" = bench-aa ]; then name=aa_main other=$SETTING_A; else name=main other=$SETTING_B; fi
      # PART=1|2 takes the ten runs as two halves of five, each with its own
      # warmup, so each fits a batch's 600 s foreground cap at a ~27 s check
      # (ADR 16.10.26j); `medians` pools them.
      runs=10 json="$OUT/$name.json"
      if [ -n "${PART:-}" ]; then runs=5 json="$OUT/$name.part$PART.json"; fi
      hyperfine -N -i --warmup 2 --runs "$runs" --export-json "$json" \
        "$(check "$SETTING_A")" "$(check "$other")" > /dev/null 2>&1
      medians "$name" ;;
    *) echo "MODE=selfcheck needs STEP=build|rss|bench-aa|bench-ab" >&2; exit 2 ;;
  esac
  exit 0
fi

if [ "$MODE" = arena ]; then
  # 6.10.26m P0.5, context only: the long shape under ALBITE_ARENA=bump,
  # over MODE=export's two builds: this commit's latest export run, or
  # EXPORT_DIR.
  export_dir="${EXPORT_DIR:-$(ls -td "build/native-concat-cost/$SHA/export"/run-* | head -1)}"
  hyperfine -N --warmup 3 --runs 20 --export-json "$OUT/arena_long.json" \
    "env ALBITE_ARENA=bump $export_dir/long_rust" \
    "env ALBITE_ARENA=bump $export_dir/long_port" > /dev/null 2>&1
  medians arena_long
  exit 0
fi

build() { # <source stem> <binary name> <switch> [compile flag]
  ALBITE_NATIVE_CORE="$3" "$TG" compile --termination=report ${4:+"$4"} \
    "benchmarks/strings/$1.tg" -o "$OUT/$2" > /dev/null 2> "$OUT/$2.build.log"
  echo "  $2: $("$OUT/$2" 2> /dev/null)"
}

if [ "$MODE" = owned ]; then
  echo "== build (owned) with $TG"
  for switch in off on all; do build string_concat_owned "owned_$switch" "$switch"; done
  build string_concat long_off off
  build string_concat long_all all
  build string_concat_short short_off off
  build string_concat_short short_all all
  pair() { # <json name> <a> <b> [command prefix]
    hyperfine -N --warmup 3 --runs 20 --export-json "$OUT/$1.json" \
      "${4:-}$OUT/$2" "${4:-}$OUT/$3" > /dev/null 2>&1
  }
  pair aa_owned owned_off owned_off
  pair owned owned_off owned_all
  pair long long_off long_all
  pair short short_off short_all
  bump="env ALBITE_ARENA=bump "
  pair bump_on_vs_off owned_off owned_on "$bump"
  pair bump_all_vs_off owned_off owned_all "$bump"
  pair bump_all_vs_on owned_on owned_all "$bump"
  medians aa_owned owned long short bump_on_vs_off bump_all_vs_off bump_all_vs_on
  echo "== bump-mode checksums (off, on, all)"
  for switch in off on all; do
    echo "  owned_$switch: $(ALBITE_ARENA=bump "$OUT/owned_$switch")"
  done
  echo "== peak RSS (/usr/bin/time -v) and page faults (perf stat)"
  for bin in owned_off owned_all long_off long_all short_off short_all; do
    echo "  $bin: $(peak_rss "$OUT/$bin") KiB, $(page_faults "$OUT/$bin") page faults"
  done
  echo "== perf shares"
  # owned_off beside owned_all, so the record carries the differential
  # profile (ADR 13.10.26p §1.2 took it by hand).
  for bin in short_all owned_off owned_all; do perf_shares "$bin"; done
  echo "== --alloc-profile string class (D7)"
  for switch in on all; do
    build string_concat_owned "profiled_$switch" "$switch" --alloc-profile > /dev/null
    "$OUT/profiled_$switch" > /dev/null 2> "$OUT/profiled_$switch.err" || true
    # The report's `string` class row, or its empty-profile line.
    echo "  $switch: $(grep -E '^ +string |no allocations recorded' "$OUT/profiled_$switch.err" | head -1)"
  done
  exit 0
fi

echo "== build ($MODE) with $TG"
if [ "$MODE" = prototype ]; then
  build string_concat long_rust off
  build string_concat_native long_port off
  build string_concat_short short_rust off
  build string_concat_short_native short_port off
else
  build string_concat long_rust off
  build string_concat long_port all
  build string_concat_short short_rust off
  build string_concat_short short_port all
fi

bench() { # <json name> <a> <b>
  hyperfine -N --warmup 3 --runs 20 --export-json "$OUT/$1.json" "$OUT/$2" "$OUT/$3" > /dev/null 2>&1
}
bench aa_long long_rust long_rust
bench aa_short short_rust short_rust
bench long long_rust long_port
bench short short_rust short_port

names="aa_long aa_short long short"
if [ "$MODE" = export ]; then
  # D3: what the core's second code-generation run adds to one build.
  hyperfine -N --warmup 1 --runs 10 --export-json "$OUT/compile.json" \
    "env ALBITE_NATIVE_CORE=off $TG compile examples/hello.tg -o $OUT/hello_off" \
    "env ALBITE_NATIVE_CORE=on $TG compile examples/hello.tg -o $OUT/hello_on" > /dev/null 2>&1
  # 7.10.26p D3: and `all`, the owned row's would-ship setting.
  hyperfine -N --warmup 1 --runs 10 --export-json "$OUT/compile_all.json" \
    "env ALBITE_NATIVE_CORE=off $TG compile examples/hello.tg -o $OUT/hello_off" \
    "env ALBITE_NATIVE_CORE=all $TG compile examples/hello.tg -o $OUT/hello_all" > /dev/null 2>&1
  names="$names compile compile_all"
fi
# shellcheck disable=SC2086 # one argument per name
medians $names

echo "== peak RSS (/usr/bin/time -v)"
for bin in long_rust long_port short_rust short_port; do
  echo "  $bin: $(peak_rss "$OUT/$bin") KiB"
done

# Every symbol down to 0.5%, so 6.10.26m D4's S (the summed share of the
# core's bytes_* symbols and the shim) can be read off, and the Rust build's
# short-shape profile beside the port's as its reference.
echo "== perf shares"
for bin in long_port short_port short_rust; do
  perf record -q -o "$OUT/$bin.perf" "$OUT/$bin" > /dev/null 2>&1 || { echo "  perf unavailable"; break; }
  echo "  -- $bin"
  perf report -i "$OUT/$bin.perf" --stdio --sort symbol 2> /dev/null \
    | grep -E '^ +([1-9][0-9]*|0\.[5-9])[0-9]*\.?[0-9]*%' | head -40 || true
done
