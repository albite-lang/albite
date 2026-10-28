# make/native.mk — Native self-compile targets (Mac LLVM 18)
#
# Self-compilation pipeline: bootstrap → albite1 → albite2
# Requires: brew install llvm@18

.PHONY: self-compile self-compile-fast build-bootstrap ensure-lib-symlink self-compile-step2 full-bootstrap self-compile-ir self-compile-llc self-compile-llc-fast self-compile-link self-compile-verify self-compile-verify-quick self-compile-dev selfcompiled-stored-generics-check selfcompiled-conversion-check selfcompiled-dependent-signature-check selfcompiled-rss-gate selfcompiled-rss-gate-arena selfhost-tools-fresh

# Full self-compile: build albite1 via the in-process codegen pipeline.
#
# HISTORY: this used to hand-roll --emit-llvm → llc → cc. Per-function
# emission (ADR 9.5.26b) made that flow doubly broken here: the `.ll` files
# land in a mirror DIRECTORY TREE (a flat `*.ll` glob linked 1 of ~2000
# units), and on case-insensitive APFS same-named-modulo-case units
# (`char_A.ll` / `char_a.ll`) silently overwrite each other in the tree, so
# even a `find`-based llc pass links with missing symbols. The real compile
# pipeline avoids both: in-process codegen emits index-prefixed object files
# (`0_char_A.o`, `1_char_a.o`) and links them itself.
self-compile:
	@echo "=== Stage 1/2: Building bootstrap compiler and FFI library ==="
	$(CARGO) build --release -p albite_bootstrap -p albite_core -p albite_codegen
	@echo ""
	@echo "=== Stage 2/2: Compiling the self-hosted compiler (in-process codegen) ==="
	@# ADR 16.10.26p D2: a root `albite1` that is a symlink points into the
	@# container's out/; replace it with a regular file rather than write through it.
	@if [ -L albite1 ]; then rm -f albite1; fi
	./target/release/albite compile src/compiler/main.tg -o albite1 -v
	@echo ""
	@echo "✓ Built albite1"

# Alias retained for muscle memory: the in-process pipeline has no llc -O0/-O2
# split, so "fast" and full are the same flow now.
self-compile-fast: self-compile

build-bootstrap:
	$(CARGO) build --release -p albite_bootstrap -p albite_core -p albite_codegen

# No longer needed — static linking eliminates runtime library dependency
ensure-lib-symlink:
	@true

# Step 2: albite1 compiles itself to albite2 (native Mac)
self-compile-step2:
	@if [ ! -f $(TUNGSTEN1) ]; then \
		echo "Error: $(TUNGSTEN1) not found. Run 'make self-compile' first."; \
		exit 1; \
	fi
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@if [ -L albite2 ]; then rm -f albite2; fi
	$(TUNGSTEN1) compile src/compiler/main.tg -o albite2 -v
	@echo "✓ Built albite2 (self-hosted compiler compiled by albite1)"

# Full bootstrap: Step 1 + Step 2 (native Mac)
full-bootstrap: self-compile self-compile-step2
	@echo ""
	@echo "=== Bootstrap Complete ==="
	@ls -lh albite1 albite2
	@echo ""
	@echo "To verify L4: ./albite2 check src/compiler/main.tg"

# Generate IR only (native) — writes per-module .ll files to tungsten1_ll/
self-compile-ir:
	mkdir -p tungsten1_ll
	$(CARGO) run -p albite_bootstrap --release -- compile src/compiler/main.tg --emit-llvm -o tungsten1_ll/ -v

# Compile per-module IR to object files (requires tungsten1_ll/ from
# self-compile-ir). Tree-aware: --emit-llvm writes a mirror directory tree
# (ADR 9.5.26b), so walk it with find — a flat glob sees ~1 of ~2000 units.
# CAVEAT (IR analysis only, not a link source of truth): on case-insensitive
# APFS, same-named-modulo-case units (char_A.ll / char_a.ll) overwrite each
# other in the tree, so a binary linked from it is missing those few symbols.
# Build runnable binaries with `make self-compile` (in-process pipeline,
# index-prefixed objects).
self-compile-llc:
	@echo "Using $(LLC)"
	@find tungsten1_ll -name '*.ll' -print0 | xargs -0 -I{} sh -c 'echo "  llc $$1"; $(LLC) -filetype=obj "$$1" -o "$${1%.ll}.o" -O2 -time-passes --stats' _ {}

# Compile per-module IR to object files with -O0 (fast, requires tungsten1_ll/)
self-compile-llc-fast:
	@echo "Using $(LLC) with -O0"
	@find tungsten1_ll -name '*.ll' -print0 | xargs -0 -P4 -I{} sh -c '$(LLC) -filetype=obj "$$1" -o "$${1%.ll}.o" -O0' _ {}

# Link object files to binary (requires tungsten1_ll/ objects from
# self-compile-llc). See the APFS caveat above — prefer `make self-compile`.
self-compile-link:
	@if [ -L albite1 ]; then rm -f albite1; fi
	cc $$(find tungsten1_ll -name '*.o') -o albite1 target/release/libalbite_core.a -lSystem -lc -lm
	@echo "✓ Linked albite1"

# Single-command confidence check: build albite1 + verify it type-checks all test programs
self-compile-verify: self-compile
	@echo "=== Self-compile-verify: testing albite1 ==="
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@failed=0; for prog in examples/hello.tg examples/answer.tg examples/option.tg \
	             examples/arithmetic.tg examples/strings.tg examples/logic.tg \
	             examples/pair.tg examples/list_ops.tg examples/result.tg \
	             examples/ordering.tg; do \
		printf "  check %-35s" "$$prog"; \
		if $(TUNGSTEN1) check "$$prog" >/dev/null 2>&1; then echo "✅"; \
		else echo "❌ FAIL"; failed=1; fi; \
	done; \
	if [ "$$failed" -eq 1 ]; then echo "❌ Self-compile-verify FAILED"; exit 1; fi
	@echo "✅ Self-compile-verify: albite1 passed all 10 checks"

# Quick variant (default tier — 3 programs, for iteration speed)
self-compile-verify-quick: self-compile
	@echo "=== Self-compile-verify-quick: testing albite1 ==="
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@failed=0; for prog in examples/hello.tg examples/answer.tg examples/option.tg; do \
		printf "  check %-35s" "$$prog"; \
		if $(TUNGSTEN1) check "$$prog" >/dev/null 2>&1; then echo "✅"; \
		else echo "❌ FAIL"; failed=1; fi; \
	done; \
	if [ "$$failed" -eq 1 ]; then echo "❌ Self-compile-verify-quick FAILED"; exit 1; fi
	@echo "✅ Self-compile-verify-quick: albite1 passed smoke test"

# Stored-generic regression battery under the self-compiled compiler (ADR
# 21.7.26k D3): the bootstrap reproducer matrix (ADR 21.7.26e §2.1) mirrored as
# fixtures that albite1 itself elaborates — `.tg` tests under `albite test`
# run on the already-fixed bootstrap and would guard nothing. Assumes
# ./albite1 exists (run in-container after a self-compile). The non-regular
# backstop fixture must FAIL with a rendered error; everything else must pass.
selfcompiled-stored-generics-check:
	@echo "=== Stored-generic battery: albite1 checking the 21.7.26e matrix ==="
	@test -x $(TUNGSTEN1) || { echo "❌ $(TUNGSTEN1) missing — run a self-compile first"; exit 1; }
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@failed=0; for prog in tests/selfcompiled_stored_generics/field_list.tg \
	             tests/selfcompiled_stored_generics/field_tree.tg \
	             tests/selfcompiled_stored_generics/payload_list.tg \
	             tests/selfcompiled_stored_generics/payload_tree.tg \
	             tests/selfcompiled_stored_generics/poly3.tg \
	             tests/golden/check/cross_module_stored_generic_field/main.tg \
	             tests/golden/check/cross_module_stored_generic_payload/main.tg; do \
		printf "  check %-60s" "$$prog"; \
		if $(TUNGSTEN1) check "$$prog" >/dev/null 2>&1; then echo "✅"; \
		else echo "❌ FAIL"; failed=1; fi; \
	done; \
	printf "  reject %-59s" "tests/selfcompiled_stored_generics/nonregular_backstop.tg"; \
	if $(TUNGSTEN1) check tests/selfcompiled_stored_generics/nonregular_backstop.tg >/dev/null 2>&1; then \
		echo "❌ FAIL (accepted a non-regular generic)"; failed=1; \
	else echo "✅ (rendered error)"; fi; \
	if [ "$$failed" -eq 1 ]; then echo "❌ Stored-generic battery FAILED"; exit 1; fi
	@echo "✅ Stored-generic battery: all cells green"

# δ-free conversion battery (ADR 30.9.26g D7): every fixture through BOTH
# ./albite1 and the bootstrap, which must agree — except the one pinned
# part-(b) divergence, `delta_needs_admission` (bootstrap δ accepts, albite1
# refuses). Each rejection is grepped for its own message, so a red arm cannot
# pass by firing for the wrong reason. The bootstrap is addressed through
# $CARGO_TARGET_DIR, never ./target, which in the container holds the HOST's
# MachO build. Spec: file|bootstrap verdict|albite1 verdict|bootstrap
# message|albite1 message.
selfcompiled-conversion-check:
	@echo "=== δ-free conversion battery: albite1 against the bootstrap ==="
	@test -x $(TUNGSTEN1) || { echo "❌ $(TUNGSTEN1) missing — run a self-compile first"; exit 1; }
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@test -n "$$CARGO_TARGET_DIR" || { echo "❌ CARGO_TARGET_DIR unset — run in the devcontainer (./target holds host builds)"; exit 1; }
	$(CARGO) build -p albite_bootstrap --bin albite
	@logs=$$(mktemp -d); d=tests/selfcompiled_conversion; refl_b="requires both sides to be equal"; refl_s="terms are not definitionally equal"; printf '%s\n' \
		"tests/golden/check/conversion_without_delta.tg|accept|accept||" \
		"$$d/left_zero.tg|reject|reject|$$refl_b|$$refl_s" \
		"$$d/successor_is_not_self.tg|reject|reject|$$refl_b|$$refl_s" \
		"$$d/closed_sum.tg|reject|reject|$$refl_b|$$refl_s" \
		"$$d/times_one.tg|reject|reject|$$refl_b|$$refl_s" \
		"$$d/trans_left_zero.tg|reject|reject|endpoint mismatch: first proof ends|trans: endpoint mismatch" \
		"$$d/delta_needs_admission.tg|accept|reject||$$refl_s" \
	| { failed=0; while IFS='|' read -r prog want_boot want_self msg_boot msg_self; do \
		for side in bootstrap albite1; do \
			if [ "$$side" = bootstrap ]; then bin="$$CARGO_TARGET_DIR/debug/albite"; want="$$want_boot"; msg="$$msg_boot"; \
			else bin=$(TUNGSTEN1); want="$$want_self"; msg="$$msg_self"; fi; \
			log="$$logs/$$side.$$(basename "$$prog").log"; \
			if "$$bin" check "$$prog" >"$$log" 2>&1; then got=accept; else got=reject; fi; \
			printf "  %-9s %-6s %-55s" "$$side" "$$want" "$$prog"; \
			if [ "$$got" != "$$want" ]; then echo "❌ FAIL ($$got)"; cat "$$log"; failed=1; \
			elif [ "$$want" = reject ] && ! grep -qF "$$msg" "$$log"; then echo "❌ FAIL (rejected without: $$msg)"; cat "$$log"; failed=1; \
			else echo "✅"; fi; \
		done; \
	done; \
	if [ "$$failed" -eq 1 ]; then echo "❌ Conversion battery FAILED (logs: $$logs)"; exit 1; fi; \
	echo "✅ Conversion battery: both compilers agree (one pinned part-(b) divergence)"; }

# Dependent-signature battery (ADR 2.10.26f D8): the same shape as the conversion
# battery above — every fixture through BOTH compilers, each refusal grepped for
# its own message on each side (codes differ between the two, and a self-host
# signature refusal renders as E0999 until 2.10.26i). One row is pinned as
# measured: `later_global` (both refuse, each for its own reason).
# `converts_at_annotation` was the second until ADR 3.10.26i D3 gave albite1
# the bootstrap's Eq-against-Eq endpoint conversion. ADR 3.10.26i adds the
# dependent `natind` rows, the arithmetic library itself, and the bootstrap's
# own refusal goldens as must-fail rows, read in place. A row whose sixth field
# is `count` must also report the same `N definition(s)` on both compilers.
# No message holds a backtick or a `|`.
selfcompiled-dependent-signature-check:
	@echo "=== Dependent-signature battery: albite1 against the bootstrap ==="
	@test -x $(TUNGSTEN1) || { echo "❌ $(TUNGSTEN1) missing — run a self-compile first"; exit 1; }
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@test -n "$$CARGO_TARGET_DIR" || { echo "❌ CARGO_TARGET_DIR unset — run in the devcontainer (./target holds host builds)"; exit 1; }
	$(CARGO) build -p albite_bootstrap --bin albite
	@logs=$$(mktemp -d); d=tests/selfcompiled_dependent_signature; g=tests/golden/error; nf_b="cannot find value"; sort="is a sort: it classifies propositions such as"; pred="type variable cannot have type arguments"; printf '%s\n' \
		"$$d/accepted.tg|accept|accept||" \
		"$$d/cross_module/main.tg|accept|accept||" \
		"$$d/statement_sugar.tg|accept|accept||" \
		"$$d/annotated_dependent_global.tg|reject|reject|(n: Nat) -> Eq<Nat, NatAdd|type mismatch" \
		"$$d/annotated_generic_dependent.tg|reject|reject|(x: T) -> Eq<|cannot instantiate polymorphic type" \
		"$$d/cong_dependent.tg|reject|reject|first argument must be a function|cong requires a function as first argument" \
		"$$d/higher_order_generic.tg|reject|reject|cannot infer type argument|cannot infer a type argument from a dependent function" \
		"$$d/phantom_parameter.tg|reject|reject|cannot infer type argument|cannot instantiate polymorphic type" \
		"$$d/wrong_argument.tg|reject|reject|Eq<Nat, NatAdd(Succ(Succ(Succ(Zero))), Zero)|type mismatch" \
		"$$d/not_a_parameter.tg|reject|reject|$$nf_b|unknown value: n" \
		"$$d/later_parameter.tg|reject|reject|$$nf_b|unknown value: n" \
		"$$d/term_binder_is_not_a_type.tg|reject|reject|cannot find type|unknown type: p" \
		"$$d/prop_binder.tg|reject|reject|$$sort|$$sort" \
		"$$d/prop_parameter.tg|reject|reject|$$sort|$$sort" \
		"$$d/scope_leak.tg|reject|reject|$$nf_b|unknown value: n" \
		"$$d/converts_at_annotation.tg|accept|accept||" \
		"$$d/later_global.tg|reject|reject|$$nf_b|terms are not definitionally equal" \
		"src/compiler/nat_arith.tg|accept|accept|||count" \
		"$$d/natind_dependent.tg|accept|accept||" \
		"$$d/natind_computational.tg|accept|accept||" \
		"$$d/natind_reverse_capture.tg|reject|reject|shadows|shadows" \
		"$$d/natind_step_not_a_lambda.tg|reject|reject|must be written fn(k: Nat, ih)|must be written fn(k: Nat, ih)" \
		"$$d/natind_wrong_ih.tg|reject|reject|NatAdd(Zero, NatAdd(Var(|type mismatch" \
		"$$g/natind_step_must_prove_the_successor_case.tg|reject|reject|NatAdd(Zero, Succ(Var(|type mismatch" \
		"$$g/nat_lemma_false_statements.tg|reject|reject|NatAdd(Succ(Var(|type mismatch" \
		"$$g/nat_mul_lemma_false_statements.tg|reject|reject|Succ(Var(|type mismatch" \
		"$$d/exists_pair.tg|accept|reject||expected type, found 'exists'" \
		"$$d/le_refl.tg|accept|reject||expected '=', found '('" \
		"src/compiler/nat_order.tg|accept|reject||expected '=', found '('" \
		"src/compiler/nat_divides.tg|accept|reject||expected '=', found '('" \
		"src/compiler/nat_binom.tg|accept|reject||expected '{', found 'by'" \
		"src/compiler/nat_divmod.tg|accept|reject||Module cycle detected" \
		"src/compiler/nat_pow.tg|accept|reject||expected '{', found 'by'" \
		"src/compiler/nat_sum.tg|accept|reject||Module cycle detected" \
		"src/compiler/nat_prime.tg|accept|reject||Module cycle detected" \
		"src/compiler/nat_binom_theorem.tg|accept|reject||Module cycle detected" \
		"$$d/disjunction.tg|accept|reject||found '+'" \
		"$$d/refutation.tg|accept|reject||unknown value: absurd" \
		"$$d/dependent_function_type.tg|accept|reject||expected ')', found ':'" \
		"$$d/predicate_binder.tg|accept|reject||$$pred" \
	| { failed=0; while IFS='|' read -r prog want_boot want_self msg_boot msg_self mark; do \
		for side in bootstrap albite1; do \
			if [ "$$side" = bootstrap ]; then bin="$$CARGO_TARGET_DIR/debug/albite"; want="$$want_boot"; msg="$$msg_boot"; \
			else bin=$(TUNGSTEN1); want="$$want_self"; msg="$$msg_self"; fi; \
			log="$$logs/$$side.$$(echo "$$prog" | tr / _).log"; \
			if NO_COLOR=1 "$$bin" check "$$prog" >"$$log" 2>&1; then got=accept; else got=reject; fi; \
			printf "  %-9s %-6s %-72s" "$$side" "$$want" "$$prog"; \
			if [ "$$got" != "$$want" ]; then echo "❌ FAIL ($$got)"; cat "$$log"; failed=1; \
			elif [ "$$want" = reject ] && ! grep -qF "$$msg" "$$log"; then echo "❌ FAIL (rejected without: $$msg)"; cat "$$log"; failed=1; \
			else echo "✅"; fi; \
		done; \
		if [ "$$mark" = count ]; then \
			n_boot=$$(sed -n 's/.*: \([0-9]*\) definition(s).*/\1/p' "$$logs/bootstrap.$$(echo "$$prog" | tr / _).log"); \
			n_self=$$(sed -n 's/.*: \([0-9]*\) definition(s).*/\1/p' "$$logs/albite1.$$(echo "$$prog" | tr / _).log"); \
			printf "  %-9s %-6s %-72s" count "$$n_boot" "$$prog"; \
			if [ -z "$$n_boot" ] || [ "$$n_boot" != "$$n_self" ]; then echo "❌ FAIL (bootstrap $$n_boot, albite1 $$n_self)"; failed=1; \
			else echo "✅"; fi; \
		fi; \
	done; \
	if [ "$$failed" -eq 1 ]; then echo "❌ Dependent-signature battery FAILED (logs: $$logs)"; exit 1; fi; \
	echo "✅ Dependent-signature battery: both compilers agree (one pinned row; exists_pair, le_refl, nat_order, nat_divides, nat_binom, nat_divmod, nat_pow, nat_sum, nat_prime, nat_binom_theorem, disjunction, refutation, dependent_function_type and predicate_binder are the self-host's recorded refusals, ADR 4.10.26h D7, 4.10.26i D7, 6.10.26g D1, 7.10.26j D1, 7.10.26k D1, 9.10.26i D1, 9.10.26j D1, 9.10.26k D1, 9.10.26m D1, 7.10.26l D6, 7.10.26m D7, 7.10.26n D10, 6.10.26h D10)"; }

# Self-compiled self-check RSS-ceiling regression guard (ADR 23.7.26d AC4). The
# self-check runs on the never-freeing (leak-on-drop) runtime, so peak RSS is
# the integral of allocation; a hot-path retained structure can silently ramp it
# back toward the ~31 GiB container VM (as the O(M²) re-injection did: ~9.5 →
# ~25.8 GiB before this ADR's O(M) fix dropped it to ~2.1 GiB). This gate re-uses
# the selfcompiled-profile 1 Hz RSS sampler to run `albite1 check main.tg` under a cap and
# FAIL if the check is killed at the ceiling or exits non-zero — tripping well
# before the VM line. Assumes ./albite1 exists; `albite-dev` is rebuilt by
# `selfhost-tools-fresh` (ADR 24.9.26a) — the target dir is resolved rather than hard-coded because in the
# devcontainer it is a named volume (/build/target, ADR 24.7.26b) while the
# bind-mounted ./target holds HOST artefacts: hard-coding it ran a stale July
# binary that predates this subcommand, so the gate failed with "check did not
# exit 0" for a reason unrelated to the code under test (found by ADR 19.8.26d)
# (run in-container after a self-compile). Ceiling default 8 GiB (~4× the
# post-fix ~2.1 GiB peak, room for compiler growth); override
# ALBITE_SELFCOMPILED_RSS_CEILING_GB (the legacy TUNGSTEN_ spelling still read,
# ADR 20.10.26e D4).
ALBITE_SELFCOMPILED_RSS_CEILING_GB := $(or $(ALBITE_SELFCOMPILED_RSS_CEILING_GB),$(ALBITE_SELFCOMPILED_RSS_CEILING_GB),8)

# A gate must prove its own tooling is fresh (ADR 24.9.26a). Every self-host
# gate below that drives the bootstrap `tungsten` or `albite-dev` depends on
# this rather than a `test -x` of whatever binary is on disk: it builds
# `albite-dev`, then runs the canonical bootstrap build `albite-dev
# self-compile` runs first, so Cargo — which knows every input — decides
# whether either is stale. A failed build stops the gate. There is no opt-out.
selfhost-tools-fresh:
	$(CARGO) build --release -p albite-dev
	$(DC_TARGET)/release/albite-dev ensure-bootstrap

selfcompiled-rss-gate: selfhost-tools-fresh
	@echo "=== Self-compiled RSS gate: albite1 check main.tg under $(ALBITE_SELFCOMPILED_RSS_CEILING_GB) GiB ==="
	@test -x $(TUNGSTEN1) || { echo "❌ $(TUNGSTEN1) missing — run a self-compile first"; exit 1; }
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@out=$$(TUNGSTEN1=$(TUNGSTEN1) $${CARGO_TARGET_DIR:-target}/release/albite-dev selfcompiled-profile --skip-build \
	          --max-rss-gb $(ALBITE_SELFCOMPILED_RSS_CEILING_GB) --top 1 2>&1); \
	echo "$$out" | grep -E "terminal:|outcome:"; \
	if echo "$$out" | grep -q "KILLED at"; then \
		echo "❌ Self-compiled RSS gate: check crossed the $(ALBITE_SELFCOMPILED_RSS_CEILING_GB) GiB ceiling — a hot-path retained structure regressed (ADR 23.7.26d)"; exit 1; \
	elif echo "$$out" | grep -q "outcome: check exited 0"; then \
		echo "✅ Self-compiled RSS gate: check completed under the ceiling"; \
	else \
		echo "❌ Self-compiled RSS gate: check did not exit 0"; exit 1; \
	fi

# The same gate with the bump arena selected (ADR 14.9.26b AC 5). ALBITE_ARENA
# is exported into the RUN only: albite-dev and the bootstrap never call
# __albite_arena_init, so it reaches nothing but the albite1 child, whose
# prologue reads it. Same ceiling, same sampler, same target-dir resolution;
# override ALBITE_ARENA_MODE for a chunk size (e.g. bump:16).
ALBITE_ARENA_MODE := $(or $(ALBITE_ARENA_MODE),$(ALBITE_ARENA_MODE),bump)
selfcompiled-rss-gate-arena: selfhost-tools-fresh
	@echo "=== Self-compiled RSS gate (ALBITE_ARENA=$(ALBITE_ARENA_MODE)): albite1 check main.tg under $(ALBITE_SELFCOMPILED_RSS_CEILING_GB) GiB ==="
	@test -x $(TUNGSTEN1) || { echo "❌ $(TUNGSTEN1) missing — run a self-compile first"; exit 1; }
	@$(call SELFHOST_GUARD,$(TUNGSTEN1))
	@out=$$(TUNGSTEN1=$(TUNGSTEN1) ALBITE_ARENA=$(ALBITE_ARENA_MODE) $${CARGO_TARGET_DIR:-target}/release/albite-dev selfcompiled-profile --skip-build \
	          --max-rss-gb $(ALBITE_SELFCOMPILED_RSS_CEILING_GB) --top 1 2>&1); \
	echo "$$out" | grep -E "terminal:|outcome:"; \
	if echo "$$out" | grep -q "KILLED at"; then \
		echo "❌ Self-compiled RSS gate (arena): check crossed the $(ALBITE_SELFCOMPILED_RSS_CEILING_GB) GiB ceiling under ALBITE_ARENA=$(ALBITE_ARENA_MODE)"; exit 1; \
	elif echo "$$out" | grep -q "outcome: check exited 0"; then \
		echo "✅ Self-compiled RSS gate (arena): check completed under the ceiling"; \
	else \
		echo "❌ Self-compiled RSS gate (arena): check did not exit 0"; exit 1; \
	fi

# Developer build: self-compile with diagnostic tools enabled (ADR 18.4.26f §4.5)
self-compile-dev:
	@echo "=== Building albite1 (developer mode — diagnostic tools enabled) ==="
	@# Step 1: Swap in developer diagnostics
	@cp src/compiler/driver/ffi/diagnostics/mod.tg src/compiler/driver/ffi/diagnostics/mod.tg.prod
	@cp src/compiler/driver/ffi/diagnostics/dev.tg src/compiler/driver/ffi/diagnostics/mod.tg
	@# Step 2: Build (same as self-compile)
	@$(MAKE) self-compile || { \
		cp src/compiler/driver/ffi/diagnostics/mod.tg.prod src/compiler/driver/ffi/diagnostics/mod.tg; \
		rm -f src/compiler/driver/ffi/diagnostics/mod.tg.prod; \
		exit 1; \
	}
	@# Step 3: Restore production stubs
	@cp src/compiler/driver/ffi/diagnostics/mod.tg.prod src/compiler/driver/ffi/diagnostics/mod.tg
	@rm -f src/compiler/driver/ffi/diagnostics/mod.tg.prod
	@echo "✓ Built albite1 (developer mode)"

# Help section for native compilation
.PHONY: help-native
help-native:
	@echo ""
	@echo "Native Mac Self-Compile (brew install llvm@18):"
	@echo "  make self-compile             - Build albite1 (in-process codegen + link)"
	@echo "  make self-compile-fast        - Alias of self-compile (kept for muscle memory)"
	@echo "  make self-compile-dev         - Build albite1 with diagnostic tools"
	@echo "  make self-compile-ir          - Generate albite1.ll only"
	@echo "  make self-compile-llc         - Compile albite1.ll → .o (with stats)"
	@echo "  make self-compile-llc-fast    - Compile albite1.ll → .o (-O0)"
	@echo "  make self-compile-link        - Link albite1.o → albite1"
	@echo "  make self-compile-step2       - Step 2: albite1 → albite2"
	@echo "  make full-bootstrap           - Full bootstrap (albite1 + albite2)"
	@echo "  make selfcompiled-stored-generics-check - Stored-generic battery under albite1 (ADR 21.7.26k)"
	@echo "  make selfcompiled-conversion-check - δ-free conversion: albite1 vs the bootstrap (ADR 30.9.26g)"
	@echo "  make selfcompiled-rss-gate              - Self-compiled RSS-ceiling regression guard (ADR 23.7.26d)"
	@echo "  make selfcompiled-rss-gate-arena        - The same gate under ALBITE_ARENA=bump (ADR 14.9.26b)"

HELP_SECTIONS += native
