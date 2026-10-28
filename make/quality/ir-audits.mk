# make/quality/ir-audits.mk — IR determinism, cache and emission audits
#
# The host-side canaries plus the `doctor check ir` audit family run over
# one freshly emitted corpus of the self-hosted compiler's own IR.

.PHONY: check-ir-determinism-canary check-cache-poisoning-canary check-ir-fingerprint-canary check-ir-fingerprint-selftest update-ir-fingerprint check-extern-map-ambiguity check-ir-audits check-indirect-buffers check-lowering-consistency check-int-parity check-nat-parity check-bytes-parity check-substring-trap check-native-core-parity check-native-core-split-bytes check-native-core-args devcontainer-native-concat-cost ir-small-adt-census

## Fast IR determinism canary (host-side, ~3s) — ADR 18.5.26b
## Compiles a small .tg file twice with --emit-llvm and diffs the output.
check-ir-determinism-canary:
	@bash scripts/check-ir-determinism-canary.sh

## Cache-poisoning canary (ADR 4.7.26d) — host-side, no LLVM, ~2s.
##
## Runs each fixture COLD (fresh isolated cache) then WARM (reusing it) and
## compares exit status + stdout. The 4.7.26c bug was exactly this divergence:
## a bodyless signature-cache entry read back by `run`/`test`, surfacing as
## "no tests found" or a spurious E0030 — silent, and invisible to every other
## gate. `--gate` makes a divergence exit non-zero.
##
## Both modes are covered on purpose: 4.7.26c bit `test` (empty def list), and
## `run` is the mode the playground/evaluator path uses. Fixtures are the ones
## that already back diff_exec_e2e, so they stay maintained; the StringBuilder
## fixture (ADR 14.9.26a) rides along so the six builder externs are run
## cold-then-warm on the evaluator too.
##
## NOTE: this is the cold-vs-warm class only. It does NOT catch a cache written
## by one compiler build and read by a NEWER one — the validity key carries
## CARGO_PKG_VERSION, which is constant across edits (see
## docs/repo-memory/common-pitfalls.md "Why it goes stale silently"). For that
## class the mitigation stays `albite cache clean`.
check-cache-poisoning-canary:
	@echo "=== Cache-poisoning canary (cold vs warm) ==="
	$(CARGO) run -q -p albite_bootstrap --no-default-features -- \
	  diff cache tests/console_println_run.tg --mode run --gate
	$(CARGO) run -q -p albite_bootstrap --no-default-features -- \
	  diff cache tests/string_builder_run.tg --mode run --gate
	$(CARGO) run -q -p albite_bootstrap --no-default-features -- \
	  diff cache tests/dead_arm_letelse_run.tg --mode test --gate

## Colliding-name extern-map misresolution audit (ADR 12.7.26b) — cost 4,
## codegen feature, no LLVM emission. Exit non-zero on any ambiguous
## (unit, name) reference — the CI regression gate for ADR 12.7.26a.
check-extern-map-ambiguity:
	$(CARGO) run -q -p albite_bootstrap --features codegen -- doctor check codegen extern-map-ambiguity $(COMPILER_MAIN)

## IR audits over a REAL corpus (ADRs 17.7.26e §6.6, 28.7.26e) — the standing
## oracles for the `doctor check ir` family. Cost 4, requires codegen.
##
## Every one of these audits existed for months and was only ever run by hand,
## so their unit fixtures drifted from the shape the compiler actually emits
## while their tests stayed green. Pointing them at the self-hosted compiler's
## own IR found THREE of six red — `indirect-buffers` with 32 false escapes
## (17.7.26e), `null-calls` with 15 (`@cstring_is_null(` matched `null(`), and
## `declares` with 5 (LLVM IR text living in `@str_lit` constants) — plus one
## false `wrapper-self-calls` finding. An audit nobody runs is not a safety net;
## it is a file that compiles.
##
## Both targets emit IR for $(COMPILER_MAIN) into a scratch dir and audit it in
## --strict mode, where a vacuous pass — candidates found, nothing parsed — is a
## failure (exit 2), so a rename in the emitter cannot quietly empty an audit's
## candidate set. The emit dominates the cost (~3,300 `.ll` in Oct 2026),
## so auditing seven times over one corpus is near-free. `direct-mt-callers`
## (ADR 3.10.26b) takes no --strict: it refuses a corpus with no `$direct_mt`
## define unconditionally.
##
## The native core's `__native_core/native_core.ll` is the one unit written
## AFTER an O2 pipeline (ADR 7.10.26p), and these audits lint unoptimised
## shapes — so the shared walker skips that directory, keeps it out of
## `files=`, and every audit prints a `note: skipped` line naming it and the
## gates that do check it: the compile/native_core tests (IR shape) and
## check-native-core-* (behaviour) (ADR 28.10.26e).
##
## The dir is re-emitted on every invocation and never cached across runs: a
## corpus that outlived a compiler rebuild would make the gate green over IR the
## compiler no longer emits — this ADR's own failure mode, reintroduced as a
## cost optimization.
IR_AUDIT_LL_DIR ?= $(WS_MEASURE_SCRATCH)/ll-audit
# Retained alias: ADR 17.7.26e named this after the one audit that used it.
INDIRECT_BUFFER_LL_DIR ?= $(IR_AUDIT_LL_DIR)

## Emit the audit corpus. Both audit targets depend on this recipe, so a change
## to the emit flags reaches both (there is one definition, two entry points).
define emit-ir-audit-corpus
	@rm -rf $(IR_AUDIT_LL_DIR)
	$(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  compile --emit-llvm $(COMPILER_MAIN) -o $(IR_AUDIT_LL_DIR)
endef

## The audit driver. A single-line variable, not a `define`: a canned recipe's
## embedded newline would split the aggregating shell below into one shell per
## audit, and `exit $$rc` would then run with `rc` unset — a gate that reports
## "✗ failed" and exits 0.
IR_AUDIT_CMD = $(CARGO) run -q -p albite_bootstrap --no-default-features -- doctor check ir

## All seven dir-scanning IR audits over one freshly emitted corpus (ADR 28.7.26e
## D1, 3.10.26b D4). Every audit runs as `… || rc=1` so make's abort-on-first-failure does
## not hide the rest: a developer who breaks two audits should learn both in one
## run. Exits non-zero iff any audit is red.
check-ir-audits:
	@echo "=== IR audits over the self-hosted compiler's emitted IR (emit once, audit seven) ==="
	$(emit-ir-audit-corpus)
	@rc=0; \
	$(IR_AUDIT_CMD) declares $(IR_AUDIT_LL_DIR) --strict || rc=1; \
	$(IR_AUDIT_CMD) null-calls $(IR_AUDIT_LL_DIR) --strict || rc=1; \
	$(IR_AUDIT_CMD) indirect-buffers $(IR_AUDIT_LL_DIR) --strict || rc=1; \
	$(IR_AUDIT_CMD) sret-stores $(IR_AUDIT_LL_DIR) --strict || rc=1; \
	$(IR_AUDIT_CMD) merge-truncation $(IR_AUDIT_LL_DIR) --strict || rc=1; \
	$(IR_AUDIT_CMD) wrapper-self-calls $(IR_AUDIT_LL_DIR) --strict || rc=1; \
	$(IR_AUDIT_CMD) direct-mt-callers $(IR_AUDIT_LL_DIR) || rc=1; \
	if [ $$rc -ne 0 ]; then echo "✗ one or more IR audits failed"; else echo "✓ all seven IR audits clean and non-vacuous"; fi; \
	exit $$rc

## Single-audit fast path for ABI work — what an indirect-param change wants.
## Shares check-ir-audits' emit recipe and corpus dir; superseded by it in CI.
check-indirect-buffers:
	@echo "=== Class-P indirect-buffer audit (emit + --strict) ==="
	$(emit-ir-audit-corpus)
	$(CARGO) run -q -p albite_bootstrap --no-default-features -- \
	  doctor check ir indirect-buffers $(IR_AUDIT_LL_DIR) --strict

## Assert every non-recursive ADT lowers identically via every route — the
## named-vs-structural split-brain gate (ADR 12.7.26c P3). Cost 4, requires
## codegen; kept OFF check-health's no-codegen hot path as a dedicated target.
check-lowering-consistency:
	$(CARGO) run -q -p albite_bootstrap --features codegen -- doctor check type lowering-consistency $(COMPILER_MAIN)

## Int parity + trap gate (ADR 14.9.26c AC 3). Cost 5, requires codegen
## (host LLVM). Two `diff exec` runs over two fixtures:
##   1. tests/int_parity_run.tg — every Int node, printed; the evaluator's
##      checked arms and the native intrinsics/sdiv/srem must agree on stdout.
##   2. tests/int_overflow_trap_run.tg — prints, then overflows; `diff exec`
##      classifies both sides failing at runtime with equal stdout as parity,
##      so this passes only if BOTH sides trap after the print. A backend that
##      wrapped would print a third line and diverge.
## The static library is rebuilt first: the trap block calls `tg_int_trap`,
## which a stale `libalbite_core.a` lacks. The trap runs again under
## ALBITE_NATIVE_CORE=all, where it calls the core's `tg_native_int_trap`
## (ADR 7.10.26q D5).
check-int-parity:
	@echo "=== Int parity (evaluator vs native) + overflow-trap gate (ADR 14.9.26c) ==="
	$(CARGO) build -q -p albite_core
	ALBITE_NATIVE_CORE=off $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  diff exec tests/int_parity_run.tg
	ALBITE_NATIVE_CORE=off $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  diff exec tests/int_overflow_trap_run.tg
	ALBITE_NATIVE_CORE=all $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  diff exec tests/int_overflow_trap_run.tg
	ALBITE_NATIVE_CORE=off $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  diff exec tests/a_program_write_trap_run.tg
	ALBITE_NATIVE_CORE=all $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  diff exec tests/a_program_write_trap_run.tg
	@echo "✓ Int parity holds and the overflow trap fires on both sides, off and all"

## Nat division-by-zero parity (ADR 9.10.26g AC 3). Cost 5, requires codegen.
## `n / 0` and `n % 0` are `0` on the evaluator; the native lowering guards a
## run-time zero divisor, and this `diff exec` is what pins the two together.
check-nat-parity:
	@echo "=== Nat division-by-zero parity (evaluator vs native) (ADR 9.10.26g) ==="
	$(CARGO) build -q -p albite_bootstrap --features codegen --bin albite
	ALBITE_NATIVE_CORE=off $${CARGO_TARGET_DIR:-target}/debug/albite diff exec tests/nat_div_zero_run.tg
	@echo "✓ Nat / and % by zero agree: 0 on both sides"

## Byte-buffer parity + trap gate (ADR 3.10.26n AC 5). Cost 5, requires
## codegen (host LLVM or the devcontainer). The `tg_bytes_*` intrinsics run
## from a handle table on the evaluator and inline natively, so:
##   1. tests/bytes_parity_run.tg — every intrinsic in range; `diff exec` parity.
##   2. tests/bytes_trap_run.tg — eight refusals, one per copy: each case is
##      stamped into $(BYTES_TRAP_DIR) with its `BYTES_TRAP_CASE` rewritten.
##      `diff exec` reads "both sides failed with equal stdout" as parity and
##      exits 4 when only the evaluator failed, so each case must (a) fail
##      under `albite run` with an `error: byte` line, its `before` line
##      printed and no `no trap` line, and (b) exit 0 under `diff exec` —
##      which together say the native side trapped after the same print.
BYTES_TRAP_DIR := build/bytes-trap
check-bytes-parity:
	@echo "=== Byte-buffer parity (evaluator vs native) + trap gate (ADR 3.10.26n) ==="
	$(CARGO) build -q -p albite_core
	$(CARGO) build -q -p albite_bootstrap --features codegen --bin albite
	@tg=$${CARGO_TARGET_DIR:-target}/debug/albite; dir=$(BYTES_TRAP_DIR); \
	$$tg diff exec tests/bytes_parity_run.tg || exit 1; \
	rm -rf $$dir && mkdir -p $$dir || exit 1; \
	for c in 0 1 2 3 4 5 6 7; do \
	  case_tg=$$dir/case_$$c.tg; \
	  sed "s|{ 0 } // BYTES_TRAP_CASE|{ $$c } // BYTES_TRAP_CASE|" tests/bytes_trap_run.tg > $$case_tg; \
	  grep -q "{ $$c } // BYTES_TRAP_CASE" $$case_tg || { echo "✗ case $$c: marker not rewritten"; exit 1; }; \
	  if $$tg run $$case_tg > $$dir/case_$$c.out 2> $$dir/case_$$c.err; then \
	    echo "✗ case $$c: the evaluator did not trap"; exit 1; fi; \
	  grep -q "before the refusal" $$dir/case_$$c.out || { echo "✗ case $$c: no line before the trap"; exit 1; }; \
	  if grep -q "no trap" $$dir/case_$$c.out; then echo "✗ case $$c: the call was let through"; exit 1; fi; \
	  grep -q "error: byte" $$dir/case_$$c.err || { echo "✗ case $$c: no byte trap line"; cat $$dir/case_$$c.err; exit 1; }; \
	  ALBITE_NATIVE_CORE=off $$tg diff exec $$case_tg > $$dir/case_$$c.diff 2>&1 || { echo "✗ case $$c: diff exec"; cat $$dir/case_$$c.diff; exit 1; }; \
	  ALBITE_NATIVE_CORE=all $$tg diff exec $$case_tg > $$dir/case_$$c.all.diff 2>&1 || { echo "✗ case $$c: diff exec under all"; cat $$dir/case_$$c.all.diff; exit 1; }; \
	  echo "  case $$c: $$(cat $$dir/case_$$c.err)"; \
	done
	@echo "✓ Byte parity holds and all eight refusals trap on both sides, off and all"

## Substring cut trap gate (ADR 16.10.26h AC 5). Cost 5, requires codegen.
## tests/substring_trap_run.tg's three cuts inside a character (the end, the
## start, both), stamped one per run as check-bytes-parity's cases are: each
## must fail under `albite run` with the substring trap line after its
## `before the cut` line, and `diff exec` must read parity under off and all.
SUBSTRING_TRAP_DIR := build/substring-trap
check-substring-trap:
	@echo "=== Substring cut inside a character traps on every reader (ADR 16.10.26h) ==="
	$(CARGO) build -q -p albite_core
	$(CARGO) build -q -p albite_bootstrap --features codegen --bin albite
	@tg=$${CARGO_TARGET_DIR:-target}/debug/albite; dir=$(SUBSTRING_TRAP_DIR); \
	rm -rf $$dir && mkdir -p $$dir || exit 1; \
	for c in 0 1 2; do \
	  case_tg=$$dir/case_$$c.tg; \
	  sed "s|{ 0 } // SUBSTRING_TRAP_CASE|{ $$c } // SUBSTRING_TRAP_CASE|" tests/substring_trap_run.tg > $$case_tg; \
	  grep -q "{ $$c } // SUBSTRING_TRAP_CASE" $$case_tg || { echo "✗ case $$c: marker not rewritten"; exit 1; }; \
	  if $$tg run $$case_tg > $$dir/case_$$c.out 2> $$dir/case_$$c.err; then \
	    echo "✗ case $$c: the evaluator did not trap"; exit 1; fi; \
	  grep -q "before the cut" $$dir/case_$$c.out || { echo "✗ case $$c: no line before the trap"; exit 1; }; \
	  if grep -q "no trap" $$dir/case_$$c.out; then echo "✗ case $$c: the cut was let through"; exit 1; fi; \
	  grep -q "error: substring cuts inside a character" $$dir/case_$$c.err || { echo "✗ case $$c: no substring trap line"; cat $$dir/case_$$c.err; exit 1; }; \
	  ALBITE_NATIVE_CORE=off $$tg diff exec $$case_tg > $$dir/case_$$c.diff 2>&1 || { echo "✗ case $$c: diff exec"; cat $$dir/case_$$c.diff; exit 1; }; \
	  ALBITE_NATIVE_CORE=all $$tg diff exec $$case_tg > $$dir/case_$$c.all.diff 2>&1 || { echo "✗ case $$c: diff exec under all"; cat $$dir/case_$$c.all.diff; exit 1; }; \
	  echo "  case $$c: $$(cat $$dir/case_$$c.err)"; \
	done
	@echo "✓ All three cuts inside a character trap on both sides, off and all"

## Native-core parity gate (ADR 4.10.26n AC 7, 6.10.26n AC 5). Cost 5,
## requires codegen. `on` enables the default rows — since 7.10.26q D5 the
## trap alone, so a `++` stays Rust's — and `all` every row, both concats
## included, so:
##   1. tests/native_core_parity_run.tg — `diff exec` parity, evaluator vs
##      native, under `on` and under `all`; only `all` must call the concats.
##      `all` runs under GLIBC_TUNABLES=glibc.malloc.perturb=165 (9.10.26n D3),
##      so an unwritten byte in a fresh concat block reads `Z`, not the text.
##   2. tests/native_core_bytes_run.tg — a split multi-byte character, which
##      the evaluator cannot hold: built `off` and `all`, stdout byte-identical.
##      Its halves arrive through argv as the lone bytes C3 and A9, since a
##      cut inside a character traps (ADR 16.10.26h D5).
##   3. tests/native_core_owned_grow_run.tg — owned chains across the
##      amortised grow's in-place and moving paths (ADR 16.10.26j AC 4), under
##      `all` with ALBITE_ARENA unset and at `bump:1`; its IR must call
##      __albite_realloc_amortised.
NATIVE_CORE_DIR := build/native-core-parity
check-native-core-parity:
	@echo "=== Native-core parity (ALBITE_NATIVE_CORE=on, =all) (ADR 4.10.26n, 6.10.26n) ==="
	$(CARGO) build -q -p albite_core
	$(CARGO) build -q -p albite_bootstrap --features codegen --bin albite
	@tg=$${CARGO_TARGET_DIR:-target}/debug/albite; dir=$(NATIVE_CORE_DIR); \
	rm -rf $$dir && mkdir -p $$dir || exit 1; \
	for setting in on all; do perturb=; [ $$setting = all ] && perturb=glibc.malloc.perturb=165; \
	  GLIBC_TUNABLES=$$perturb ALBITE_NATIVE_CORE=$$setting $$tg diff exec tests/native_core_parity_run.tg || exit 1; \
	  ALBITE_NATIVE_CORE=$$setting $$tg compile --emit-llvm tests/native_core_parity_run.tg -o $$dir/ll_$$setting > /dev/null || exit 1; \
	done; \
	grep -rqs "call.*@tg_native_string_concat(" $$dir/ll_all || { echo "✗ all: the switch did not reach codegen: no tg_native_string_concat call"; exit 1; }; \
	grep -rqs "call.*@tg_native_string_concat_owned(" $$dir/ll_all || { echo "✗ all: no tg_native_string_concat_owned call"; exit 1; }; \
	for arena in "" bump:1; do \
	  GLIBC_TUNABLES=glibc.malloc.perturb=165 ALBITE_ARENA=$$arena ALBITE_NATIVE_CORE=all $$tg diff exec tests/native_core_owned_grow_run.tg || { echo "✗ the amortised grow diverged (ALBITE_ARENA=$$arena)"; exit 1; }; \
	done; \
	ALBITE_NATIVE_CORE=all $$tg compile --emit-llvm tests/native_core_owned_grow_run.tg -o $$dir/ll_grow > /dev/null || exit 1; \
	grep -rqs "call ptr @__albite_realloc_amortised(" $$dir/ll_grow || { echo "✗ all: the owned grow does not call __albite_realloc_amortised (ADR 16.10.26j)"; exit 1; }
	@$(MAKE) -s check-native-core-split-bytes
	@echo "✓ Native-core parity holds: evaluator = native under on and all, and off = all on split bytes"

## The second half alone, so a non-vacuity drill can show it red by itself.
check-native-core-split-bytes:
	@tg=$${CARGO_TARGET_DIR:-target}/debug/albite; dir=$(NATIVE_CORE_DIR)/split; \
	rm -rf $$dir && mkdir -p $$dir || exit 1; \
	ALBITE_NATIVE_CORE=off $$tg compile tests/native_core_bytes_run.tg -o $$dir/bytes_off > /dev/null || exit 1; \
	ALBITE_NATIVE_CORE=all $$tg compile tests/native_core_bytes_run.tg -o $$dir/bytes_all > /dev/null || exit 1; \
	$$dir/bytes_off "$$(printf '\303')" "$$(printf '\251')" > $$dir/bytes_off.out || exit 1; \
	$$dir/bytes_all "$$(printf '\303')" "$$(printf '\251')" > $$dir/bytes_all.out || { echo "✗ the all build did not complete"; exit 1; }; \
	LC_ALL=C grep -aq "^split-left .* 2$$" $$dir/bytes_off.out || { echo "✗ the lone lead byte did not arrive through argv"; cat $$dir/bytes_off.out; exit 1; }; \
	cmp $$dir/bytes_off.out $$dir/bytes_all.out || { echo "✗ off and all disagree on a split character"; exit 1; }

## The argument init's parity (ADR 7.10.26r D5): `off` (Rust's init) against
## `all` (the core's), native against native, over four argument sets.
check-native-core-args:
	@echo "=== Native-core argument init (ALBITE_NATIVE_CORE=off, =all) (ADR 7.10.26r) ==="
	$(CARGO) build -q -p albite_core
	$(CARGO) build -q -p albite_bootstrap --features codegen --bin albite
	@bash scripts/check-native-core-args.sh $${CARGO_TARGET_DIR:-target}/debug/albite $(NATIVE_CORE_DIR)/args
	@echo "✓ Argument init parity holds: off = all = the arguments given"

## The native concat's cost (ADR 4.10.26n P0.3, Step 7), in the devcontainer.
## MODE=prototype (the port as a function) or MODE=export (the real export,
## `off` against `all`), MODE=owned (every shape `off` against `all`, ADR
## 9.10.26n D4), MODE=replay (the owned grow's allocator replay and its
## factor verdict, ADR 16.10.26j D4), or MODE=selfcheck STEP=build|rss|bench
## [SWITCH=off|on|all] [SETTINGS="off all"] (`albite1 check main.tg`); see
## benchmarks/strings/native_concat_cost.sh. The host's `uptime` goes in as
## HOST_LOAD and is appended again at the end: the container cannot see the
## host's load, and a run above 8 is retaken (ADR 16.10.26j D6).
devcontainer-native-concat-cost:
	devcontainer exec --workspace-folder . env MODE=$(or $(MODE),prototype) STEP=$(STEP) SWITCH=$(SWITCH) PART=$(PART) SETTINGS="$(or $(SETTINGS),off on)" HOST_LOAD="$$(uptime)" bash benchmarks/strings/native_concat_cost.sh
	@run=$$(cat build/native-concat-cost/last-run) && echo "host load (end): $$(uptime)" | tee -a "$$run/provenance.txt"

## Small-ADT census (ADR 28.9.26f). Cost 5, requires codegen (host LLVM 18 or
## the devcontainer). Records, never gates on counts — it fails only when an
## emit or the --strict self-compile census fails. Under SMALL_ADT_DIR:
##   provenance.txt            the commit and `git status --porcelain`
##   selfcompile/ + .emit.log  $(COMPILER_MAIN)'s IR (a debug emit, as
##                             check-ir-audits') and its stderr
##   selfcompile.json          the census the B19 rule reads (--strict)
##   bench/<stem>/, bench-<stem>.json   each benchmark's IR and census (context)
##   bench-<stem>.alloc.log    its --alloc-profile run; a compile/link/run
##                             failure prints `profile failed: <stem>` and the
##                             loop goes on — the profile is context, not input
## A separate dir from IR_AUDIT_LL_DIR, so neither recipe clobbers the other.
## `--termination=report` is the benchmark harness's own workload level.
SMALL_ADT_DIR ?= $(WS_MEASURE_SCRATCH)/small-adt-census
SMALL_ADT_CENSUS = $(CARGO) run -q -p albite_bootstrap --no-default-features -- doctor check ir small-adts
SMALL_ADT_COMPILE = $(CARGO) run -q -p albite_bootstrap --features codegen -- compile --termination=report
ir-small-adt-census:
	@echo "=== Small-ADT census over the self-compile corpus + benchmarks (ADR 28.9.26f) ==="
	$(CARGO) build -q -p albite_core
	@set -e; d=$(SMALL_ADT_DIR); rm -rf $$d; mkdir -p $$d/bench; \
	{ echo "commit: $$(git rev-parse HEAD)"; echo "status --porcelain:"; git status --porcelain; } > $$d/provenance.txt; \
	$(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  compile --emit-llvm $(COMPILER_MAIN) -o $$d/selfcompile 2> $$d/selfcompile.emit.log; \
	$(SMALL_ADT_CENSUS) $$d/selfcompile --json --strict --emit-log $$d/selfcompile.emit.log > $$d/selfcompile.json; \
	for tg in $$(ls benchmarks/*/*.tg | sort); do \
	  stem=$$(basename $$tg .tg); \
	  $(SMALL_ADT_COMPILE) --emit-llvm $$tg -o $$d/bench/$$stem; \
	  $(SMALL_ADT_CENSUS) $$d/bench/$$stem --json > $$d/bench-$$stem.json; \
	  if $(SMALL_ADT_COMPILE) --alloc-profile $$tg -o $$d/bench/$$stem.bin > $$d/bench-$$stem.alloc.log 2>&1 \
	    && $$d/bench/$$stem.bin > /dev/null 2>> $$d/bench-$$stem.alloc.log; then :; \
	  else echo "profile failed: $$stem"; fi; \
	done; \
	echo "✓ census written under $$d (verdict: selfcompile.json)"

## Compare canary IR output against stored fingerprint baseline (ADR 18.5.26d)
check-ir-fingerprint-canary:
	@bash scripts/check-ir-fingerprint.sh

## Check the fingerprint's target-line normalisation without a compiler (ADR 9.10.26b D4)
check-ir-fingerprint-selftest:
	@bash scripts/check-ir-fingerprint.sh --selftest

## Update canary IR fingerprint baseline (ADR 18.5.26d)
update-ir-fingerprint:
	@bash scripts/check-ir-fingerprint.sh --update
