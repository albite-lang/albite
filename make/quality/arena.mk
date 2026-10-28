# make/quality/arena.mk — the allocator's mode and region gates
#
# Moved from make/quality/ir-audits.mk, which had reached its 400-line cap
# (ADR 13.10.26q, 9.10.26o's retrospective), with the region copy-out
# battery added beside them.

.PHONY: check-arena-mode check-arena-mode-scan check-arena-branch-cost check-region-copy-out check-region-copy-million measure-region-rss

## Arena-mode parity + callee gate (ADR 14.9.26b AC 5 / AC 7, amended by ADR
## 18.9.26c AC 5). Cost 5, requires codegen (host LLVM). Two checks over one
## fixture, plus the region goldens:
##   1. `diff exec` under ALBITE_ARENA=bump:1 — the native side bump-
##      allocates from 1 MiB chunks (the fixture forces an oversized chunk and
##      a rollover) and must print what the evaluator prints. The variable
##      reaches only the compiled child: the bootstrap never calls
##      `__albite_arena_init` and stays `off`. `diff exec` passes it through
##      its cleared environment only since ADR 13.10.26q; before, this run
##      was the default mode's. Every `tests/golden/run/region_*.tg` runs too.
##   2. the non-profiled IR the bootstrap emits for the fixture loads
##      `@__albite_arena_mode`, calls `@__albite_alloc`, and calls `@malloc`
##      ONLY inside an `alloc.malloc[N]` block — the off arm of the mode branch.
##      `grep` cannot see block membership, so ARENA_MALLOC_SCAN remembers the
##      last label of each `.ll` and flags a `@malloc` call under any other.
## The static library is rebuilt first: the linker resolves
## `target/debug/libalbite_core.a`, and a stale one lacks the symbols.
ARENA_MODE_FIXTURE ?= tests/arena_mode_run.tg
ARENA_MODE_LL_DIR ?= $(WS_MEASURE_SCRATCH)/ll-arena-mode
## Exit 1 naming each `@malloc` call outside an `alloc.malloc[N]` block. The
## optimised native core inlines its allocation sites, and LLVM's inliner
## suffixes the copied block `.i` per level (`alloc.malloc.i.i`) — the same off
## arm, so the suffix is accepted (ADR 7.10.26s).
ARENA_MALLOC_SCAN = awk 'FNR == 1 || /^define / { label = "" } /^[^ ;\t]+:/ { label = $$1 } /call ptr @malloc\(/ && label !~ /^alloc\.malloc[0-9]*(\.i[0-9]*)*:$$/ { print "  " FILENAME ":" FNR ": @malloc call in block " (label == "" ? "<entry>" : label); bad = 1 } END { exit bad }'
check-arena-mode:
	@echo "=== Arena-mode parity (ALBITE_ARENA=bump:1) + allocation callee gate ==="
	$(CARGO) build -q -p albite_core
	ALBITE_ARENA=bump:1 $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  diff exec $(ARENA_MODE_FIXTURE)
	@set -e; for f in $(REGION_GOLDENS); do \
	  ALBITE_ARENA=bump:1 $(CARGO) run -q -p albite_bootstrap --features codegen -- diff exec $$f; done
	@rm -rf $(ARENA_MODE_LL_DIR)
	$(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  compile --emit-llvm $(ARENA_MODE_FIXTURE) -o $(ARENA_MODE_LL_DIR)
	@$(MAKE) -s check-arena-mode-scan ARENA_MODE_LL_DIR=$(ARENA_MODE_LL_DIR)

## The IR half of check-arena-mode alone, over an existing `.ll` tree — no
## emit, so AC 5's red-arm drill can point it at a planted copy:
##   make check-arena-mode-scan ARENA_MODE_LL_DIR=<dir with a stray @malloc>
check-arena-mode-scan:
	@if ! grep -rq "call ptr @__albite_alloc(" $(ARENA_MODE_LL_DIR); then \
	  echo "✗ no @__albite_alloc call in the emitted IR"; exit 1; fi; \
	if ! grep -rq "load i32, ptr @__albite_arena_mode" $(ARENA_MODE_LL_DIR); then \
	  echo "✗ no @__albite_arena_mode load in the emitted IR"; exit 1; fi; \
	if ! $(ARENA_MALLOC_SCAN) $$(find $(ARENA_MODE_LL_DIR) -name '*.ll' | sort); then \
	  echo "✗ a @malloc call outside an alloc.malloc block"; exit 1; fi; \
	echo "✓ arena-mode parity holds; @malloc is called only on the mode-off arm"

## The region copy-out battery (ADR 13.10.26q AC 4). Cost 5, host LLVM. Every
## region golden through `diff exec` with each released chunk poisoned
## (ALBITE_REGION_POISON=1), in mode `off` and `bump:1` — so a missing or
## wrong copy reads 0xA5 bytes instead of what `free(3)` happened to leave —
## then the 1,000,000-node list, native only, against its known sum.
## REGION_COPY_BIN is where that list's binary is built.
REGION_GOLDENS = $(sort $(wildcard tests/golden/run/region_*.tg))
REGION_COPY_MILLION ?= tests/region_copy_million.tg
REGION_COPY_BIN ?= $(WS_SCRATCH)/region-copy-out/million
check-region-copy-out:
	@echo "=== Region copy-out battery: poisoned releases, off and bump:1 (ADR 13.10.26q) ==="
	$(CARGO) build -q -p albite_core
	@set -e; for f in $(REGION_GOLDENS); do \
	  ALBITE_REGION_POISON=1 $(CARGO) run -q -p albite_bootstrap --features codegen -- diff exec $$f; \
	  ALBITE_REGION_POISON=1 ALBITE_ARENA=bump:1 $(CARGO) run -q -p albite_bootstrap --features codegen -- diff exec $$f; \
	done
	@echo "✓ every region golden agrees with the evaluator under poisoned releases"
	@$(MAKE) -s check-region-copy-million

## Peak RSS and wall time of the region-wrapped benchmarks against their
## region-free twins, off and bump, with an A/A pair and the Rust baselines
## (ADR 13.10.26q D7). Recorded, never gated: the table goes into
## benchmarks/results/13.10.26q.region-copy-out-rss.md.
REGION_RSS_DIR ?= $(WS_MEASURE_SCRATCH)/region-rss
REGION_RSS_RUNS ?= 5
measure-region-rss:
	@bash scripts/region-rss.sh $(REGION_RSS_DIR) $(REGION_RSS_RUNS)

## The million-node half of check-region-copy-out alone. REGION_COPY_COMPILER
## defaults to the release build, which compiles it in seconds.
REGION_COPY_COMPILER ?= $${CARGO_TARGET_DIR:-target}/release/albite
check-region-copy-million:
	@mkdir -p $(dir $(REGION_COPY_BIN))
	$(REGION_COPY_COMPILER) compile $(REGION_COPY_MILLION) -o $(REGION_COPY_BIN)
	@got=$$(ALBITE_REGION_POISON=1 $(REGION_COPY_BIN)); [ "$$got" = 4500000 ] || { echo "✗ the million-node list summed to $$got"; exit 1; }; \
	got=$$(ALBITE_REGION_POISON=1 ALBITE_ARENA=bump:1 $(REGION_COPY_BIN)); [ "$$got" = 4500000 ] || { echo "✗ bump: the million-node list summed to $$got"; exit 1; }; \
	echo "✓ the million-node list copies out, off and bump:1, poisoned"

## Three-variant same-IR A/B of the allocation-site mode branch (ADR 18.9.26c
## AC 1). IN-CONTAINER (hyperfine, llvm-link, opt, llc, the release build):
##   devcontainer exec --workspace-folder . make check-arena-branch-cost
## `closure_chain` is emitted once and linked into ONE `.ll`; three binaries
## are built from it by rewriting only the `@__albite_arena_mode` declaration:
##   emitted  `external hidden global i32` — this ADR's branch
##   0        `internal constant i32 0` — folds to the bare pre-14.9.26b @malloc
##   1        `internal constant i32 1` — folds to the parent's @__albite_alloc
## All three take the same `opt -passes=instcombine,simplifycfg` then `llc -O2`
## (llc alone will not reliably fold a load from a constant global) and the same
## link line; a grep proves each fold. `internal` never clashes with the
## runtime's real export, which `__albite_alloc` and the prologue keep reading.
## `--termination=report` is the benchmark harness's own workload flag
## (tools/albite-bench, WORKLOAD_TERMINATION_LEVEL): the workloads loop on Nat.
## Recorded, not gated (AC 1: *benchmark, recorded*) — the rows go into
## benchmarks/results/18.9.26c.arena-branch-cost.md.
## ADR 7.10.26s D6 (b): a fourth variant, `scoped`, rewrites the prologue to
## `@__albite_arena_init_regions()` — built only when the archive exports it.
## ARENA_BRANCH_ARCHIVE runs a kept (pre-change) runtime archive; ARENA_BRANCH_LL
## reuses a kept linked `.ll` instead of re-emitting, so A/A and A/B share one IR.
## ADR 9.10.26o D7: a fifth, `region`, is closure_chain with each run in a region.
## ADR 13.10.26q D8: a sixth, `region inverted`, is that IR with OFF_ARM_WEIGHT's
## `branch_weights` swapped by rewrite — G2a's lever, measured, not built.
ARENA_BRANCH_WEIGHTS = "branch_weights", i32 2000, i32 1}
ARENA_BRANCH_INVERTED = "branch_weights", i32 1, i32 2000}
ARENA_BRANCH_DIR ?= $(WS_MEASURE_SCRATCH)/arena-branch-cost
ARENA_BRANCH_BENCH ?= benchmarks/closures/closure_chain.tg
ARENA_BRANCH_REGION_BENCH ?= benchmarks/closures/closure_chain_region.tg
ARENA_BRANCH_RELEASE = $${CARGO_TARGET_DIR:-target}/release
ARENA_BRANCH_ARCHIVE ?= $(ARENA_BRANCH_RELEASE)/libalbite_core.a
ARENA_BRANCH_LL ?=
ARENA_BRANCH_LINK = -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
check-arena-branch-cost:
	@echo "=== Arena branch cost: same-IR A/B on closure_chain (ADR 18.9.26c, 7.10.26s) ==="
	@set -e; d=$(ARENA_BRANCH_DIR); rm -rf $$d; mkdir -p $$d; a=$(ARENA_BRANCH_ARCHIVE); \
	if [ -n "$(ARENA_BRANCH_LL)" ]; then cp $(ARENA_BRANCH_LL) $$d/emitted.ll; else \
	$(ARENA_BRANCH_RELEASE)/tungsten compile --termination=report --emit-llvm $(ARENA_BRANCH_BENCH) -o $$d/ll; \
	llvm-link -S $$(find $$d/ll -name '*.ll' | sort) -o $$d/emitted.ll; fi; \
	decl='@__albite_arena_mode = external hidden global i32'; \
	grep -qx "$$decl" $$d/emitted.ll || { echo "✗ no '$$decl' line in the linked IR"; exit 1; }; \
	scoped=; if nm -g $$a 2>/dev/null | grep -q ' T __albite_arena_init_regions$$'; then scoped=scoped; fi; \
	for v in emitted 0 1 $$scoped; do \
	  src=$$d/emitted.ll; \
	  if [ $$v = scoped ]; then src=$$d/vscoped.ll; \
	    sed 's/@__albite_arena_init()/@__albite_arena_init_regions()/' $$d/emitted.ll > $$src; \
	  elif [ $$v != emitted ]; then src=$$d/v$$v.ll; \
	    sed "s/^$$decl\$$/@__albite_arena_mode = internal constant i32 $$v/" $$d/emitted.ll > $$src; fi; \
	  opt -S -passes=instcombine,simplifycfg $$src -o $$d/$$v.opt.ll; \
	  llc -O2 -filetype=obj $$d/$$v.opt.ll -o $$d/$$v.o; \
	  cc $$d/$$v.o -o $$d/closure_chain.$$v $$a $(ARENA_BRANCH_LINK); \
	  [ "$$($$d/closure_chain.$$v)" = "$$(cat benchmarks/closures/closure_chain.expected)" ] || { echo "✗ variant $$v printed the wrong answer"; exit 1; }; \
	done; \
	! grep -q "load i32, ptr @__albite_arena_mode" $$d/0.opt.ll || { echo "✗ variant 0 kept the mode load"; exit 1; }; \
	! grep -q "call ptr @__albite_alloc(" $$d/0.opt.ll || { echo "✗ variant 0 kept a @__albite_alloc call"; exit 1; }; \
	! grep -q "call ptr @malloc(" $$d/1.opt.ll || { echo "✗ variant 1 kept a @malloc call"; exit 1; }; \
	echo "✓ three variants built from one .ll, each fold verified"; r=$$d/closure_chain.region; \
	$(ARENA_BRANCH_RELEASE)/tungsten compile --termination=report --emit-llvm $(ARENA_BRANCH_REGION_BENCH) -o $$d/rll && llvm-link -S $$(find $$d/rll -name '*.ll' | sort) -o $$d/region.ll && opt -S -passes=instcombine,simplifycfg $$d/region.ll -o $$d/region.opt.ll && llc -O2 -filetype=obj $$d/region.opt.ll -o $$d/region.o && cc $$d/region.o -o $$r $$a $(ARENA_BRANCH_LINK) && [ "$$($$r)" = "$$(cat benchmarks/closures/closure_chain.expected)" ] && grep -q "call void @__albite_arena_init_regions()" $$d/region.ll; \
	ri=$$d/closure_chain.region_inv; grep -qF '$(ARENA_BRANCH_WEIGHTS)' $$d/region.opt.ll || { echo "✗ no OFF_ARM_WEIGHT branch_weights in the region IR"; exit 1; }; \
	sed 's/$(ARENA_BRANCH_WEIGHTS)/$(ARENA_BRANCH_INVERTED)/' $$d/region.opt.ll > $$d/region_inv.ll && llc -O2 -filetype=obj $$d/region_inv.ll -o $$d/region_inv.o && cc $$d/region_inv.o -o $$ri $$a $(ARENA_BRANCH_LINK) && [ "$$($$ri)" = "$$(cat benchmarks/closures/closure_chain.expected)" ]; \
	hyperfine --warmup 5 --runs 40 --export-markdown $$d/ab.md --export-csv $$d/ab.csv \
	  -n 'emitted off' "$$d/closure_chain.emitted" \
	  -n '0 (bare malloc)' "$$d/closure_chain.0" \
	  -n '1 (parent symbol) off' "$$d/closure_chain.1" \
	  -n 'emitted bump' "ALBITE_ARENA=bump $$d/closure_chain.emitted" \
	  -n '1 (parent symbol) bump' "ALBITE_ARENA=bump $$d/closure_chain.1" \
	  $${scoped:+-n scoped $$d/closure_chain.scoped} -n region "$$r" -n 'region inverted' "$$ri"; \
	awk -F, 'NR > 1 { m[NR - 1] = $$2 } END { \
	  printf "emitted/0 off: %+.1f%%   1/emitted off: %+.1f%%   1/0 off: %+.1f%%   emitted/1 bump: %+.1f%%\n", \
	    (m[1] / m[2] - 1) * 100, (m[3] / m[1] - 1) * 100, (m[3] / m[2] - 1) * 100, (m[4] / m[5] - 1) * 100; \
	  if (7 in m) printf "scoped/1 off: %+.1f%%   region/1 off: %+.1f%%   region/0 off: %+.1f%%\n", (m[6] / m[3] - 1) * 100, (m[7] / m[3] - 1) * 100, (m[7] / m[2] - 1) * 100; \
	  if (8 in m) printf "region inverted/region: %+.1f%%   region inverted/1 off: %+.1f%%\n", (m[8] / m[7] - 1) * 100, (m[8] / m[3] - 1) * 100 }' $$d/ab.csv; \
	echo "Table: $$d/ab.md"
