# make/devcontainer/stack-budget.mk — does the self-host still fit its stack?
#
# ADR 19.10.26d D3. Since 19.10.26c a compiled program sizes its own stack
# (512 MiB by default), so the self-host no longer crashes as its recursion
# deepens, and the next lapse past 8 MB would go unnoticed. This target runs
# `$(DC_TUNGSTEN1)check` and `$(DC_TUNGSTEN1)compile` on main.tg under
# ALBITE_STACK_SIZE=$(STACK_BUDGET), and again at $(STACK_HEADROOM) for a
# headroom line that never fails the target:
#
#   make docker-vm-run TARGET=devcontainer-stack-budget [STACK_BUDGET=8M]
#
# Each command prints one `stack-budget: <command> <verdict>` line, and the
# LINE is the verdict, as in check-cost.mk: make collapses a failing exit to 2.
#   ok                 exit 0
#   overflow           exit 134 with `*** Stack overflow detected ***` (the
#                      self-host's runtime), or exit 139 (a raw SIGSEGV)
#   bootstrap-overflow `has overflowed its stack`: the Rust bootstrap `compile`
#                      delegates codegen to, which runs on the shell's limit
#                      and never reads the variable — not a self-host result
#   error              any other non-zero exit — not a stack result, so a
#                      compile error never reads as a pass
# Anything but `ok` at the budget fails the target. Advisory: no gate runs it.
#
# Shell rather than a albite-dev subcommand: the classification is a handful
# of exit-code and stderr matches, too small to earn a Rust module.
# `devcontainer exec` forwards no host environment, so STACK_BUDGET is
# interpolated into the `bash -c` string rather than passed through.

.PHONY: devcontainer-stack-budget

STACK_BUDGET ?= 8M
STACK_HEADROOM ?= 6M

# One shell program, run in the container. `run_at <size> <name> <cmd…>` runs
# one command at one size, keeping its stderr for the classifier, and prints
# the verdict word. The probe comes first: 19.10.26c's runtime refuses a
# malformed ALBITE_STACK_SIZE with exit 2 before reading its arguments, so a
# binary that does anything else predates it and every result below would be
# the shell's limit, not the budget's.
STACK_BUDGET_SCRIPT = \
  classify() { \
    if [ "$$1" -eq 0 ]; then echo ok; \
    elif grep -q "has overflowed its stack" "$$2"; then echo bootstrap-overflow; \
    elif [ "$$1" -eq 139 ]; then echo overflow; \
    elif [ "$$1" -eq 134 ] && grep -q "Stack overflow detected" "$$2"; then echo overflow; \
    else echo error; fi; \
  }; \
  run_at() { \
    size=$$1; shift; err=$$(mktemp); \
    ALBITE_STACK_SIZE=$$size "$$@" >/dev/null 2>"$$err"; rc=$$?; \
    verdict=$$(classify "$$rc" "$$err"); \
    if [ "$$verdict" != ok ]; then echo "  [$$size exit $$rc] $$(tail -n 3 "$$err" | tr "\n" " ")" >&2; fi; \
    rm -f "$$err" /tmp/stack-budget-tungsten2; echo "$$verdict"; \
  }; \
  ALBITE_STACK_SIZE=banana $(DC_TUNGSTEN1)>/dev/null 2>&1; probe=$$?; \
  if [ "$$probe" -ne 2 ]; then \
    echo "stack-budget: binary ignores ALBITE_STACK_SIZE (built before 19.10.26c?) [probe exit $$probe]"; exit 1; \
  fi; \
  failed=0; thin=0; \
  for cmd in check compile; do \
    if [ "$$cmd" = check ]; then set -- $(DC_TUNGSTEN1)check src/compiler/main.tg --max-errors 0; \
    else set -- $(DC_TUNGSTEN1)compile src/compiler/main.tg -o /tmp/stack-budget-tungsten2; fi; \
    verdict=$$(run_at $(STACK_BUDGET) "$$@"); \
    case "$$verdict" in \
      ok) echo "stack-budget: $$cmd ok ($(STACK_BUDGET))";; \
      bootstrap-overflow) echo "stack-budget: $$cmd bootstrap-overflow (not a self-host stack result)"; failed=1;; \
      error) echo "stack-budget: $$cmd error (not a stack result)"; failed=1;; \
      *) echo "stack-budget: $$cmd overflow ($(STACK_BUDGET))"; failed=1;; \
    esac; \
    if [ "$$verdict" = ok ] && [ "$$(run_at $(STACK_HEADROOM) "$$@")" != ok ]; then thin=1; fi; \
  done; \
  if [ "$$failed" -eq 0 ]; then \
    if [ "$$thin" -eq 0 ]; then echo "headroom: ok ($(STACK_HEADROOM))"; \
    else echo "headroom: thin (fits $(STACK_BUDGET), not $(STACK_HEADROOM))"; fi; \
  fi; \
  exit $$failed

# Guards as devcontainer-self-compile-step2's: `compile` delegates codegen to
# the bootstrap, which is addressed through DC_TARGET only.
devcontainer-stack-budget:
	@if [ ! -f $(DC_TUNGSTEN1) ]; then \
		echo "Error: albite1 not found. Run 'make devcontainer-self-compile-fast' first."; \
		exit 1; \
	fi
	@devcontainer exec --workspace-folder . bash -c 'test -x $(DC_TARGET)/release/albite' || { \
		echo 'Error: no bootstrap at $$CARGO_TARGET_DIR/release/albite — albite1 delegates codegen to it. Run: make devcontainer-build'; exit 1; }
	@devcontainer exec --workspace-folder . bash -c '$(STACK_BUDGET_SCRIPT)'
