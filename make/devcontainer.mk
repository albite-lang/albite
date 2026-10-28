# make/devcontainer.mk — Dev container targets (LLVM codegen testing)
#
# Commands for running the compiler inside a dev container with LLVM 18.
# Requires: npm install -g @devcontainers/cli
#
# The x86_64 (QEMU) arm lives in make/devcontainer/x86_64.mk (ADR 31.7.26c) and
# the self-host Core diagnostics in make/devcontainer/selfhost-core.mk (ADR
# 3.9.26h); help-devcontainer below still prints every arm as one section.

include make/devcontainer/x86_64.mk
include make/devcontainer/selfhost-core.mk
include make/devcontainer/check-cost.mk
include make/devcontainer/stack-budget.mk
include make/devcontainer/ro-probe.mk
include make/devcontainer/profile.mk

.PHONY: devcontainer-up devcontainer-test devcontainer-test-all devcontainer-build
.PHONY: devcontainer-compile devcontainer-compile-run devcontainer-run devcontainer-check devcontainer-eval
.PHONY: devcontainer-check-selfhost devcontainer-build-check-selfhost devcontainer-check-selfhost-nocodegen devcontainer-build-check-selfhost-nocodegen
.PHONY: devcontainer-check-l4 devcontainer-check-ir-determinism devcontainer-check-ir-determinism-fast devcontainer-check-ir-determinism-full
.PHONY: devcontainer-check-ir-fingerprint-full
.PHONY: devcontainer-ensure-lib-symlink
.PHONY: devcontainer-self-compile devcontainer-self-compile-with-check devcontainer-self-compile-step2
.PHONY: devcontainer-self-compile-split devcontainer-self-compile-ir devcontainer-self-compile-llc devcontainer-self-compile-link
.PHONY: devcontainer-build-dev-tool devcontainer-self-compile-verify devcontainer-self-compile-dev devcontainer-self-compile-dev-fast devcontainer-self-compile-fast
.PHONY: devcontainer-self-compile-verify-fast devcontainer-self-compile-direct devcontainer-full-bootstrap devcontainer-selfcompiled-stored-generics devcontainer-selfcompiled-conversion devcontainer-selfcompiled-dependent-signature
.PHONY: devcontainer-doctor-self-test devcontainer-doctor-self-test-full
.PHONY: devcontainer-down devcontainer-target-reset

# ADR 24.7.26b: the arm64 devcontainer isolates its Cargo build output in a
# container-local named volume (CARGO_TARGET_DIR=/build/target), so release
# artifacts no longer live under the bind-mounted ./target. Recipes that exec
# those artifacts must expand CARGO_TARGET_DIR *inside* the container — hence
# the `bash -c '...'` wrappers below. This snippet expands to that path in the
# container, falling back to ./target when unset (pre-24.7.26b containers /
# host). It is `$$`-escaped for make and must be used INSIDE a single-quoted
# bash -c payload so the host shell never expands it.
DC_TARGET = $${CARGO_TARGET_DIR:-target}

# `devcontainer exec` forwards no host environment, so ALBITE_NATIVE_CORE
# (ADR 4.10.26n D6) and ALBITE_CODEGEN_JOBS reach the self-compile only through
# this flag, and only when set: `make devcontainer-self-compile-verify-fast
# ALBITE_NATIVE_CORE=on` builds albite1 on the native concat. Unset, the
# compiler's default holds — never an empty assignment. The legacy TUNGSTEN_*
# spelling is read when the new one is unset or empty (ADR 20.10.26e D4).
ALBITE_NATIVE_CORE := $(or $(ALBITE_NATIVE_CORE),$(TUNGSTEN_NATIVE_CORE))
ALBITE_CODEGEN_JOBS := $(or $(ALBITE_CODEGEN_JOBS),$(TUNGSTEN_CODEGEN_JOBS))
DC_SETTINGS_ENV = $(if $(ALBITE_NATIVE_CORE),--remote-env ALBITE_NATIVE_CORE=$(ALBITE_NATIVE_CORE),) \
  $(if $(ALBITE_CODEGEN_JOBS),--remote-env ALBITE_CODEGEN_JOBS=$(ALBITE_CODEGEN_JOBS),)

# Start the dev container
DEVCONTAINER_UP_CMD ?= devcontainer up --workspace-folder .# a private overlay may supervise it (ADR 22.10.26c D6)
devcontainer-up:
	$(DEVCONTAINER_UP_CMD)

# Run codegen tests in dev container
devcontainer-test:
	devcontainer exec --workspace-folder . cargo test -p albite_codegen

# Run all tests in dev container
devcontainer-test-all:
	devcontainer exec --workspace-folder . cargo test

# Build with codegen in dev container
devcontainer-build: devcontainer-build-dev-tool
	devcontainer exec --workspace-folder . cargo build --release

# Build `albite-dev`, the in-container self-compile orchestrator. It lives
# OUTSIDE the workspace `default-members`, so a bare `cargo build --release`
# does NOT produce it — the self-compile targets below then fail with a bare
# "No such file or directory" that says nothing about the cause (ADR 17.7.26e
# retrospective). Every target that invokes it depends on this one; the build is
# a no-op (~2 s) once warm.
devcontainer-build-dev-tool:
	@devcontainer exec --workspace-folder . cargo build --release -p albite-dev

# Compile to native binary in dev container
devcontainer-compile:
ifndef FILE
	@echo "Usage: make devcontainer-compile FILE=<file> [OUT=<output>]"
	@echo "Example: make devcontainer-compile FILE=examples/hello.tg OUT=hello"
else
	devcontainer exec --workspace-folder . cargo run -p albite_bootstrap --release -- compile $(FILE) -o out/$(or $(OUT),$(basename $(notdir $(FILE))))
endif

# Compile and run a native binary in dev container
# Supports: make devcontainer-compile-run FILE=examples/hello.tg  OR  make devcontainer-compile-run examples/hello.tg
devcontainer-compile-run:
	$(eval _FILE := $(or $(FILE),$(filter %.tg,$(MAKECMDGOALS))))
	@if [ -z "$(_FILE)" ]; then \
		echo "Usage: make devcontainer-compile-run FILE=<file>"; \
		echo "   or: make devcontainer-compile-run <file>"; \
		echo "Example: make devcontainer-compile-run examples/hello.tg"; \
		exit 1; \
	fi && \
	BASENAME=$$(basename $(_FILE) .tg) && \
	devcontainer exec --workspace-folder . bash -c "\
		cargo run -p albite_bootstrap --release -- compile $(_FILE) -o out/$$BASENAME && \
		echo 'Running out/$$BASENAME...' && \
		out/$$BASENAME && \
		rm -f out/$$BASENAME"

# Run a file in dev container (interpreter)
devcontainer-run:
ifndef FILE
	@echo "Usage: make devcontainer-run FILE=<file>"
	@echo "Example: make devcontainer-run FILE=src/compiler/lexer_all.tg"
else
	devcontainer exec --workspace-folder . cargo run -p albite_bootstrap -- run $(FILE)
endif

# Check a file in dev container
devcontainer-check:
ifndef FILE
	@echo "Usage: make devcontainer-check FILE=<file>"
	@echo "Example: make devcontainer-check FILE=src/compiler/lexer_all.tg"
else
	devcontainer exec --workspace-folder . cargo run -p albite_bootstrap -- check $(FILE)
endif

# Eval an expression in dev container
devcontainer-eval:
ifndef EXPR
	@echo "Usage: make devcontainer-eval EXPR=<expr>"
	@echo "Example: make devcontainer-eval EXPR='char_at(\"hello\", 0)'"
else
	devcontainer exec --workspace-folder . cargo run -p albite_bootstrap -- eval "$(EXPR)"
endif

# Self-host check: the codegen (release) bootstrap type-checks the self-hosted
# compiler source. Codegen is the unmarked default; the -nocodegen sibling
# below carries the marker.
devcontainer-check-selfhost:
	devcontainer exec --workspace-folder . bash -c '$(DC_TARGET)/release/albite check src/compiler/main.tg --max-errors=0'

# Build the bootstrap, then run the self-host check
devcontainer-build-check-selfhost: devcontainer-build devcontainer-check-selfhost

# The four self-host Core diagnostics — dump-core, closed-terms,
# well-typed-terms and tyvar-escape — are in make/devcontainer/selfhost-core.mk.

# Self-host check, no-codegen variant: the --no-default-features bootstrap
# type-checks the same self-hosted source. A build-config difference only —
# it does NOT run the self-compiled albite1.
devcontainer-check-selfhost-nocodegen:
	devcontainer exec --workspace-folder . cargo run -p albite_bootstrap --no-default-features -- check src/compiler/main.tg --max-errors=0

# Build the no-codegen bootstrap, then run the no-codegen self-host check
devcontainer-build-check-selfhost-nocodegen:
	devcontainer exec --workspace-folder . cargo build -p albite_bootstrap --no-default-features
	devcontainer exec --workspace-folder . cargo run -p albite_bootstrap --no-default-features -- check src/compiler/main.tg --max-errors=0

# No longer needed — static linking eliminates runtime library dependency
devcontainer-ensure-lib-symlink:
	@true

# Compile the self-hosted compiler to native in dev container (Step 1: bootstrap → albite1)
devcontainer-self-compile:
	devcontainer exec --workspace-folder . bash -c '$(DC_TARGET)/release/albite compile src/compiler/main.tg -o $(DC_TUNGSTEN1) -v'

# Fast self-compile, then the SELF-HOSTED compiler type-checks its own source
# (ADR 18.9.26d). The only target that gates on `albite1 check main.tg` —
# `-verify-fast` stops at the examples, which is how a refusal reached `main`.
# The check is the last command, so its exit code is the target's: no pipe (a
# `| tail` replaces it with the filter's), and `--max-errors 0` with a SPACE —
# the self-hosted CLI takes the value from the next argument and silently
# skips `--max-errors=0`.
devcontainer-self-compile-with-check:
	$(MAKE) devcontainer-self-compile-fast
	devcontainer exec --workspace-folder . $(DC_TUNGSTEN1) check src/compiler/main.tg --max-errors 0

# Step 2: albite1 compiles itself to albite2. albite1 delegates codegen to
# the BOOTSTRAP, a second prerequisite this target used to leave implicit — and
# that is the one that broke (ADR 3.9.26b D4). Assert it through the same
# DC_TARGET as every other recipe; no ALBITE_BOOTSTRAP_BIN, because the
# resolution order is precisely what this exercises.
devcontainer-self-compile-step2:
	@if [ ! -f $(DC_TUNGSTEN1) ]; then \
		echo "Error: $(DC_TUNGSTEN1) not found. Run 'make devcontainer-self-compile' first."; \
		exit 1; \
	fi
	@devcontainer exec --workspace-folder . bash -c 'test -x $(DC_TARGET)/release/albite' || { \
		echo 'Error: no bootstrap at $$CARGO_TARGET_DIR/release/albite — albite1 delegates codegen to it. Run: make devcontainer-build'; exit 1; }
	# No ulimit: albite1 runs its body on a stack the runtime sizes (ADR 19.10.26c D5).
	devcontainer exec --workspace-folder . bash -c '$(DC_TUNGSTEN1) compile src/compiler/main.tg -o $(DC_TUNGSTEN2) -v'
	@echo "✓ Built albite2 (self-hosted compiler compiled by albite1)"

# Full bootstrap: Step 1 + Step 2
devcontainer-full-bootstrap: devcontainer-self-compile devcontainer-self-compile-step2
	@echo ""
	@echo "=== Bootstrap Complete ==="
	@devcontainer exec --workspace-folder . ls -lh $(DC_TUNGSTEN1) $(DC_TUNGSTEN2)
	@echo ""
	@echo "To verify L4: make devcontainer-check-l4"

# L4 verification: albite2 checks itself
devcontainer-check-l4:
	devcontainer exec --workspace-folder . $(DC_TUNGSTEN2) check src/compiler/main.tg --max-errors=0

# IR Determinism check (ADR 17.5.26c): verify bootstrap produces byte-identical .ll files
# across consecutive compilations. Uses the bootstrap binary (not albite1).
# Aliases to -full for backward compatibility (ADR 18.5.26c).
devcontainer-check-ir-determinism: devcontainer-check-ir-determinism-full

# Full IR determinism gate (ADR 18.5.26c): two independent clean compiles, includes __mono.ll.
devcontainer-check-ir-determinism-full:
	devcontainer exec --workspace-folder . bash scripts/check-ir-determinism-v2.sh --mode=full

# Fast IR determinism check (ADR 18.5.26c): reuses elab cache, excludes __mono.ll, parallel diff.
devcontainer-check-ir-determinism-fast:
	devcontainer exec --workspace-folder . bash scripts/check-ir-determinism-v2.sh --mode=fast

# Full-source IR fingerprint check (ADR 18.5.26d): single compile, compare against baseline.
devcontainer-check-ir-fingerprint-full:
	devcontainer exec --workspace-folder . bash scripts/check-ir-fingerprint.sh \
		--entry src/compiler/main.tg --baseline tests/golden/ir_fingerprint_full.manifest

# Compile self-hosted compiler with split IR/object/link stages (shows progress)
# Stage 1: Generate LLVM IR (per-file, ~2-3 min)
# Stage 2: Compile each .ll to .o with llc (slow, shows stats)
# Stage 3: Link all .o files to final binary
devcontainer-self-compile-split:
	@echo "=== Stage 1/3: Generating LLVM IR (per-file) ==="
	devcontainer exec --workspace-folder . bash -c 'rm -rf /tmp/tungsten1_ll && mkdir -p /tmp/tungsten1_ll && $(DC_TARGET)/release/albite compile src/compiler/main.tg --emit-llvm -o /tmp/tungsten1_ll/ -v'
	@echo ""
	@echo "=== Stage 2/3: Compiling IR to object files (llc) ==="
	@echo "This may take several minutes..."
	devcontainer exec --workspace-folder . bash -c 'find /tmp/tungsten1_ll -name "*.ll" | while read f; do echo "  llc $$f"; llc -filetype=obj "$$f" -o "$${f%.ll}.o" -O2 --stats; done'
	@echo ""
	@echo "=== Stage 3/3: Linking ==="
	devcontainer exec --workspace-folder . bash -c 'cc -o $(DC_TUNGSTEN1) $$(find /tmp/tungsten1_ll -name "*.o") $(DC_TARGET)/release/libalbite_core.a -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc'
	devcontainer exec --workspace-folder . rm -rf /tmp/tungsten1_ll
	@echo ""
	@echo "✓ Built albite1"

# Generate IR only (for debugging) — writes per-file .ll files to out/tungsten1_ll/
devcontainer-self-compile-ir:
	devcontainer exec --workspace-folder . bash -c 'rm -rf out/tungsten1_ll && mkdir -p out/tungsten1_ll && $(DC_TARGET)/release/albite compile src/compiler/main.tg --emit-llvm -o out/tungsten1_ll/ -v'

# Compile per-file IR to object files (requires out/tungsten1_ll/ from devcontainer-self-compile-ir)
devcontainer-self-compile-llc:
	devcontainer exec --workspace-folder . bash -c 'find out/tungsten1_ll -name "*.ll" | while read f; do echo "  llc $$f"; llc-18 -filetype=obj "$$f" -o "$${f%.ll}.o" -O2 -time-passes --stats; done'

# Link object files to binary (requires out/tungsten1_ll/*.o from devcontainer-self-compile-llc)
devcontainer-self-compile-link:
	devcontainer exec --workspace-folder . bash -c 'cc -o $(DC_TUNGSTEN1) $$(find out/tungsten1_ll -name "*.o") $(DC_TARGET)/release/libalbite_core.a -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc'
	@echo "✓ Linked albite1"

# DevContainer self-compile-verify (full tier): self-compile + verify examples.
devcontainer-self-compile-verify: devcontainer-self-compile devcontainer-build-dev-tool
	devcontainer exec --workspace-folder . bash -c '$(DC_TARGET)/release/albite-dev verify'

# Fast self-compile via albite-dev (3-stage: IR → llc -O1 → link; -O1, not
# -O0, so albite1 fits an 8 MB stack — ADR 19.10.26d D4).
# Parallelism defaults to nproc/2. Override: ALBITE_CODEGEN_JOBS=N or --parallelism N.
devcontainer-self-compile-fast: devcontainer-build-dev-tool
	devcontainer exec --workspace-folder . $(DC_SETTINGS_ENV) bash -c '$(DC_TARGET)/release/albite-dev self-compile --fast'

# Direct self-compile: emit .o in-process (no llc), single-stage (ADR 9.5.26e §2.1).
devcontainer-self-compile-direct: devcontainer-build-dev-tool
	devcontainer exec --workspace-folder . bash -c '$(DC_TARGET)/release/albite-dev self-compile --direct'

# Fast self-compile + verify all examples (routine confidence check).
devcontainer-self-compile-verify-fast: devcontainer-self-compile-fast devcontainer-build-dev-tool
	devcontainer exec --workspace-folder . $(DC_SETTINGS_ENV) bash -c '$(DC_TARGET)/release/albite-dev verify'

# Self-compiled stored-generic battery (ADR 21.7.26k): run the tungsten1-elaborated
# 21.7.26e-matrix fixtures against the container's existing ./albite1.
devcontainer-selfcompiled-stored-generics:
	devcontainer exec --workspace-folder . make selfcompiled-stored-generics-check

# δ-free conversion battery (ADR 30.9.26g): albite1 and the bootstrap agree.
devcontainer-selfcompiled-conversion:
	devcontainer exec --workspace-folder . make selfcompiled-conversion-check

# Dependent-signature battery (ADR 2.10.26f): albite1 and the bootstrap agree.
devcontainer-selfcompiled-dependent-signature:
	devcontainer exec --workspace-folder . make selfcompiled-dependent-signature-check

# The self-compile tier a developer build rides on. `-dev` takes the default
# -O2 path; `-dev-fast` overrides it with the llc -O1 fast tier (ADR 21.8.26a). One
# recipe with a variable rather than two copies: the diagnostics swap has an
# error path that must restore the production stub, and a second copy of it is
# a second place to forget that.
DEV_SELF_COMPILE_TARGET ?= devcontainer-self-compile

# Developer build in devcontainer: self-compile with diagnostic tools enabled
devcontainer-self-compile-dev:
	@echo "=== Building albite1 in devcontainer (developer mode — diagnostic tools enabled) ==="
	@# Step 1: Swap in developer diagnostics
	@cp src/compiler/driver/ffi/diagnostics/mod.tg src/compiler/driver/ffi/diagnostics/mod.tg.prod
	@cp src/compiler/driver/ffi/diagnostics/dev.tg src/compiler/driver/ffi/diagnostics/mod.tg
	@# Step 2: Build (same as devcontainer-self-compile)
	@$(MAKE) $(DEV_SELF_COMPILE_TARGET) || { \
		cp src/compiler/driver/ffi/diagnostics/mod.tg.prod src/compiler/driver/ffi/diagnostics/mod.tg; \
		rm -f src/compiler/driver/ffi/diagnostics/mod.tg.prod; \
		exit 1; \
	}
	@# Step 3: Restore production stubs
	@cp src/compiler/driver/ffi/diagnostics/mod.tg.prod src/compiler/driver/ffi/diagnostics/mod.tg
	@rm -f src/compiler/driver/ffi/diagnostics/mod.tg.prod
	@echo "✓ Built albite1 in devcontainer (developer mode)"

# Developer build on the fast (llc -O1) tier — the iteration loop for a
# diagnostic flag, where the binary's own speed does not matter.
devcontainer-self-compile-dev-fast:
	@$(MAKE) devcontainer-self-compile-dev DEV_SELF_COMPILE_TARGET=devcontainer-self-compile-fast

# Stop and remove the dev container.
# NOTE: this deliberately does NOT remove the tungsten-arm-target build volume
# (ADR 24.7.26b D6) — the warm incremental cache must survive a normal stop.
# Use `make devcontainer-target-reset` to wipe a poisoned cache.
devcontainer-down:
	@CONTAINER_ID=$$(docker ps -q --filter "label=devcontainer.local_folder=$$(pwd)"); \
	if [ -n "$$CONTAINER_ID" ]; then \
		docker stop $$CONTAINER_ID && docker rm $$CONTAINER_ID; \
		echo "Dev container stopped and removed"; \
	else \
		echo "No dev container running"; \
	fi

# Reset the isolated arm64 build volume (ADR 24.7.26b D6).
# The named volume persists a warm incremental cache across recreates — but
# that includes a POISONED cache (a toolchain bump, a corrupt object) that a
# `devcontainer rebuild` would previously have cleared. Wipe it here; the next
# container build is cold. The volume must be idle (container stopped) to
# remove — run `make devcontainer-down` first if this errors "volume in use".
devcontainer-target-reset:
	docker volume rm tungsten-arm-target
	@echo "Removed tungsten-arm-target volume — next container build is cold."

# Help section for devcontainer commands
.PHONY: help-devcontainer
help-devcontainer:
	@echo ""
	@echo "Dev Container (LLVM codegen):"
	@echo "  make devcontainer-up       - Start dev container with LLVM 18"
	@echo "  make devcontainer-test     - Run codegen tests in container"
	@echo "  make devcontainer-test-all - Run all tests in container"
	@echo "  make devcontainer-build    - Build with codegen in container"
	@echo "  make devcontainer-compile FILE=<file>     - Compile to native"
	@echo "  make devcontainer-compile-run FILE=<file> - Compile and run"
	@echo "  make devcontainer-run FILE=<file>         - Run (interpreted)"
	@echo "  make devcontainer-check FILE=<file>       - Type-check a file"
	@echo "  make devcontainer-eval EXPR=<expr>        - Eval an expression"
	@echo "  make devcontainer-check-selfhost          - Self-host check (release bootstrap)"
	@echo "  make devcontainer-build-check-selfhost    - Build then run the self-host check"
	@echo "  make devcontainer-dump-core PATTERN=<pat> - Dump Core IR (self-host, comma-sep or *)"
	@echo "  make devcontainer-check-tyvar-escape      - Check TyVar escapes (self-host)"
	@echo "  make devcontainer-selfhost-closed-terms   - Every self-host Core term closed? (needs -dev albite1)"
	@echo "  make devcontainer-selfhost-well-typed-terms - Every eliminator over the right former? (needs -dev albite1)"
	@echo "  make devcontainer-check-selfhost-nocodegen       - Self-host check (no-codegen bootstrap)"
	@echo "  make devcontainer-build-check-selfhost-nocodegen - Build bootstrap then no-codegen self-host check"
	@echo "  make devcontainer-self-compile-step2      - Build albite2 from albite1"
	@echo "  make devcontainer-self-compile-dev         - Build albite1 with diagnostic tools"
	@echo "  make devcontainer-self-compile-dev-fast   - Same, on the fast (llc -O1) tier"
	@echo "  make devcontainer-self-compile-fast         - Fast self-compile (llc -O1)"
	@echo "  make devcontainer-self-compile-verify-fast  - Fast self-compile + verify examples"
	@echo "  make devcontainer-selfcompiled-stored-generics  - Stored-generic battery under albite1 (ADR 21.7.26k)"
	@echo "  make devcontainer-selfcompiled-conversion       - δ-free conversion: albite1 vs bootstrap (ADR 30.9.26g)"
	@echo "  make devcontainer-selfcompiled-dependent-signature - dependent signatures: albite1 vs bootstrap (ADR 2.10.26f)"
	@echo "  make devcontainer-check-cost-gate COST_ARGS=\"--phase prepare\" - albite1 check cost vs its record (ADR 15.10.26f)"
	@echo "  make devcontainer-check-cost-bless        - Re-bless the cost record (operator only)"
	@echo "  make devcontainer-stack-budget [STACK_BUDGET=8M] - albite1 check+compile within a stack budget (ADR 19.10.26d)"
	@echo "  make devcontainer-full-bootstrap          - Full bootstrap (albite1 + albite2)"
	@echo "  make devcontainer-check-l4                - Check L4 (albite2 checks itself)"
	@echo "  make devcontainer-check-ir-determinism    - Full IR determinism gate (aliases -full)"
	@echo "  make devcontainer-check-ir-determinism-full - Two clean compiles, includes __mono.ll"
	@echo "  make devcontainer-check-ir-determinism-fast - Fast: elab cache reuse, excludes __mono.ll"
	@echo "  make devcontainer-check-ir-fingerprint-full - Full-source IR fingerprint vs baseline"
	@echo "  make devcontainer-ro-probe                - Kernel probe: the workspace is read-only (ADR 16.10.26p)"
	@echo "  make devcontainer-down                    - Stop dev container"
	@echo "  make devcontainer-target-reset            - Wipe the isolated build volume (ADR 24.7.26b)"
	@echo ""
	@echo "  Build isolation (ADR 24.7.26b): the arm64 container builds into a"
	@echo "  container-local volume (CARGO_TARGET_DIR=/build/target), so host and"
	@echo "  container no longer clobber each other's target/. The volume persists"
	@echo "  a warm cache across recreates; reset a poisoned one with the target above."
	@echo ""
	@echo "  Log capture: stderr is captured to .devcontainer/logs/ (bind-mounted)."
	@echo "  Inspect from host: cat .devcontainer/logs/<command>.stderr.log"
	@echo ""
	@echo "  Memory: Docker Desktop needs ≥16GB for CODEGEN_JOBS=2, ≥8GB for CODEGEN_JOBS=1."
	@echo "  Override parallelism: ALBITE_CODEGEN_JOBS=N make <target>"
	@echo ""
	@echo "Dev Container (x86_64, QEMU emulation):"
	@echo "  make devcontainer-up-x86                  - Start x86_64 container"
	@echo "  make devcontainer-build-x86               - Build with codegen (x86_64)"
	@echo "  make devcontainer-self-compile-x86        - Self-compile → albite1_x86"
	@echo "  make devcontainer-self-compile-verify-x86 - Self-compile + verify examples (x86_64)"
	@echo "  make devcontainer-check-reserved-link-names-x86 - Reserved link-name gate as CI's Ubuntu leg sees it"
	@echo "  make devcontainer-down-x86                - Stop x86_64 container"
	@echo ""
	@echo "Profiling (ADR 10.5.26j):"
	@echo "  make devcontainer-profile                 - Capture Chrome trace via albite-dev profile"

HELP_SECTIONS += devcontainer
