# make/devcontainer/ro-probe.mk — the kernel probe for the read-only
# devcontainer root (ADR 16.10.26p D5).
#
# In the devcontainer the workspace is mounted read-only with the directories
# in .devcontainer/writable-dirs.txt writable on top, so no program a container
# runs can write the guardrail (.claude/, tools/claude-hooks, target/). This
# asks the KERNEL rather than reading the config: every case spelling of
# `.claude`, a cross-mount rename, a symlink out of a writable dir, a remount
# from a user namespace — each row asserting its own errno — as `vscode` and
# as root. `make/private/docker-vm.mk` adds the live mount check
# (`docker-vm ro-check --live`) as a prerequisite where that tool exists.
#
# Its own file because make/devcontainer.mk sits at the `mk-size` cap.

.PHONY: devcontainer-ro-probe

RO_PROBE_CONFIG = $(CURDIR)/.devcontainer/devcontainer.json

devcontainer-ro-probe:
	devcontainer exec --workspace-folder . sh scripts/devcontainer-ro-probe.sh vscode
	@id=$$(docker ps -q --filter "label=devcontainer.local_folder=$(CURDIR)" --filter "label=devcontainer.config_file=$(RO_PROBE_CONFIG)" | head -n 1); \
	if [ -z "$$id" ]; then echo "devcontainer-ro-probe: no running devcontainer for $(CURDIR)" >&2; exit 1; fi; \
	docker exec -u root -w /workspaces/$(notdir $(CURDIR)) "$$id" sh scripts/devcontainer-ro-probe.sh root
