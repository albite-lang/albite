# make/devcontainer/x86_64.mk — the x86_64 arm of the devcontainer lanes
#
# Split out of devcontainer.mk by ADR 31.7.26c. Nothing here is shared with the
# arm64 recipes beyond the `devcontainer` CLI itself; `make help` still prints
# both arms from help-devcontainer.

.PHONY: devcontainer-up-x86 devcontainer-build-x86 devcontainer-self-compile-x86 devcontainer-self-compile-verify-x86 devcontainer-check-reserved-link-names-x86 devcontainer-down-x86

# --- x86_64 devcontainer targets (QEMU emulation on ARM Mac) ---
# These use a separate devcontainer config at .devcontainer/x86_64/ with
# --platform linux/amd64. Builds are slower due to QEMU but produce real
# x86_64 binaries. Uses CARGO_TARGET_DIR=/tmp/target_x86 inside the
# container to avoid conflicting with host-arch build artifacts.

# Start the x86_64 dev container
# A private overlay may route the bring-up through a supervisor (ADR 22.10.26c D6).
DEVCONTAINER_UP_X86_CMD ?= devcontainer up --workspace-folder . --config .devcontainer/x86_64/devcontainer.json
devcontainer-up-x86:
	$(DEVCONTAINER_UP_X86_CMD)

# Build with codegen in x86_64 dev container
devcontainer-build-x86:
	devcontainer exec --workspace-folder . --config .devcontainer/x86_64/devcontainer.json bash -c 'CARGO_TARGET_DIR=/tmp/target_x86 cargo build --release'

# Self-compile in x86_64 container (bootstrap → albite1_x86)
devcontainer-self-compile-x86: devcontainer-build-x86
	devcontainer exec --workspace-folder . --config .devcontainer/x86_64/devcontainer.json bash -c '\
		/tmp/target_x86/release/albite compile src/compiler/main.tg -o $(DC_TUNGSTEN1_X86) -v'
	@echo "✓ Built $(DC_TUNGSTEN1_X86) (x86_64)"

# Self-compile + verify all examples on x86_64
# Checks output content (not just exit code) to prevent false positives (ADR 10.5.26c §2.2).
devcontainer-self-compile-verify-x86: devcontainer-self-compile-x86
	@echo "=== x86_64 self-compile-verify: testing albite1_x86 ==="
	@# Smoke test: version must not print help (argv parsing sentinel)
	@printf "  smoke %-35s" "version"; \
	v_out=$$(devcontainer exec --workspace-folder . --config .devcontainer/x86_64/devcontainer.json bash -c \
		"$(DC_TUNGSTEN1_X86) version" 2>&1); \
	if echo "$$v_out" | grep -q "USAGE:"; then \
		echo "❌ FATAL: version printed help — binary is broken"; \
		echo "$$v_out" | head -5; exit 1; \
	fi; \
	if ! echo "$$v_out" | grep -qi "tungsten"; then \
		echo "❌ FATAL: version did not print expected output"; \
		echo "$$v_out" | head -5; exit 1; \
	fi; \
	echo "✅"
	@# Content-verified check for each example
	@failed=0; for prog in examples/hello.tg examples/answer.tg examples/option.tg \
	             examples/arithmetic.tg examples/strings.tg examples/logic.tg \
	             examples/pair.tg examples/list_ops.tg examples/result.tg \
	             examples/ordering.tg; do \
		printf "  check %-35s" "$$prog"; \
		output=$$(devcontainer exec --workspace-folder . --config .devcontainer/x86_64/devcontainer.json bash -c \
			"$(DC_TUNGSTEN1_X86) check $$prog" 2>&1); \
		exit_code=$$?; \
		if [ "$$exit_code" -eq 0 ]; then echo "✅"; \
		else echo "❌ FAIL"; echo "$$output" | head -3; failed=1; fi; \
	done; \
	if [ "$$failed" -eq 1 ]; then echo "❌ x86_64 self-compile-verify FAILED"; exit 1; fi
	@echo "✅ x86_64 self-compile-verify passed"

# The reserved link-name gate (ADR 9.10.26f) as CI's Ubuntu leg runs it. On
# the same pinned rustc, the x86_64 runtime archive imports ~46 more C names
# from Rust std than the arm64 container's does, so a table that passes
# `make check-reserved-link-names` in the arm64 container can still fail CI.
devcontainer-check-reserved-link-names-x86:
	devcontainer exec --workspace-folder . --config .devcontainer/x86_64/devcontainer.json make check-reserved-link-names

# Stop and remove the x86_64 dev container. The CLI records config_file as an
# absolute path, so a relative filter never matches.
devcontainer-down-x86:
	@CONTAINER_ID=$$(docker ps -q --filter "label=devcontainer.local_folder=$$(pwd)" --filter "label=devcontainer.config_file=$$(pwd)/.devcontainer/x86_64/devcontainer.json"); \
	if [ -n "$$CONTAINER_ID" ]; then \
		docker stop $$CONTAINER_ID && docker rm $$CONTAINER_ID; \
		echo "x86_64 dev container stopped and removed"; \
	else \
		echo "No x86_64 dev container running"; \
	fi
