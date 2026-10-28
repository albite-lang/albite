# make/devcontainer/check-cost.mk — `albite1 check main.tg` against a stored cost
#
# ADR 15.10.26f. The gate compares the check's wall time and peak RSS with the
# committed record (tools/tungsten1-check-cost.toml, read AT THE WINDOW'S BASE)
# and, when the record is stale, too close to call or past its bound, A/Bs the
# tree against the base in a kept container-local worktree. It runs as five
# foreground phases, each under a batch child's 600 s Bash cap; every phase
# prints one `check-cost:` line — the verdict, or the next command to run:
#
#   make docker-vm-run TARGET=devcontainer-check-cost-gate COST_ARGS="--phase prepare"
#
# The LINE, not the exit, is the verdict: make collapses every failing exit
# to 2. `devcontainer exec` forwards no host environment (DC_SETTINGS_ENV),
# so the batch window and deadline cross with --remote-env, and only when set.
#
# Split out of devcontainer.mk the way selfhost-core.mk was (ADR 3.9.26h); `make
# help` prints both targets from help-devcontainer.

.PHONY: devcontainer-check-cost-gate devcontainer-check-cost-bless

# Passed verbatim to `albite-dev check-cost gate` (D7).
COST_ARGS ?=
# The window's base outside a batch; inside one, ACTION_BATCH_SINCE wins.
BASE ?= HEAD

DC_COST_ENV = $(if $(ACTION_BATCH_SINCE),--remote-env ACTION_BATCH_SINCE=$(ACTION_BATCH_SINCE),) \
  $(if $(ACTION_BATCH_DEADLINE),--remote-env ACTION_BATCH_DEADLINE=$(ACTION_BATCH_DEADLINE),)
DC_COST_BASE = $(if $(ACTION_BATCH_SINCE),,--base $(BASE))

# Compare with the stored record; A/B against the base when it cannot settle it.
devcontainer-check-cost-gate: devcontainer-build-dev-tool
	devcontainer exec --workspace-folder . $(DC_COST_ENV) bash -c '$(DC_TARGET)/release/albite-dev check-cost gate $(DC_COST_BASE) $(COST_ARGS)'

# The operator's re-bless of [measured] and [environment] — never [bound] (D6).
devcontainer-check-cost-bless: devcontainer-build-dev-tool
	devcontainer exec --workspace-folder . $(DC_COST_ENV) bash -c '$(DC_TARGET)/release/albite-dev check-cost bless'
