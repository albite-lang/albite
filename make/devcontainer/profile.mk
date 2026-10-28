# make/devcontainer/profile.mk — codegen profiling in the devcontainer
# (ADR 10.5.26j).
#
# Its own file because make/devcontainer.mk sits at the `mk-size` cap;
# help-devcontainer still prints it.

.PHONY: devcontainer-profile

# Profile codegen with Chrome tracing (ADR 10.5.26j §2.5)
# Trace lands in .devcontainer/logs/profiles/ on the host (bind mount).
devcontainer-profile:
	devcontainer exec --workspace-folder . cargo run -p albite-dev --release -- profile
