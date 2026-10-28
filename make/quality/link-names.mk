# make/quality/link-names.mk — the reserved link-name gate (ADR 9.10.26f D2).
#
# A program's definition named like a C symbol the runtime imports is spelled
# apart by the elaborator, so the link cannot join it to the runtime's own
# call — but only for names in albite_core/src/link_names.rs. This gate
# keeps that table a superset of what is really imported.

.PHONY: check-reserved-link-names

RESERVED_NAMES_DIR := build/reserved-link-names

## Reserved link-name gate (ADR 9.10.26f AC5). Cost 4, requires codegen
## (host LLVM or the devcontainer). Builds the runtime archive and a codegen
## `tungsten`, compiles tests/reserved_link_names_probe.tg with the native
## core off and on, then hands the archive and both probe binaries to
## `doctor check link reserved-names`, which fails naming any imported C name
## the table lacks. A binary links libc dynamically, so every C name its
## objects and the native core import is still undefined in it. The archive
## is addressed through $$CARGO_TARGET_DIR when it is set, so in the
## devcontainer the recipe reads /build/target, not the host's ./target.
check-reserved-link-names:
	@echo "=== Reserved link names cover what the runtime and a program import (ADR 9.10.26f) ==="
	$(CARGO) build -q -p albite_core
	$(CARGO) build -q -p albite_bootstrap --features codegen --bin albite
	@mkdir -p $(RESERVED_NAMES_DIR)
	ALBITE_NATIVE_CORE=off $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  compile tests/reserved_link_names_probe.tg -o $(RESERVED_NAMES_DIR)/probe-off
	ALBITE_NATIVE_CORE=all $(CARGO) run -q -p albite_bootstrap --features codegen -- \
	  compile tests/reserved_link_names_probe.tg -o $(RESERVED_NAMES_DIR)/probe-all
	$(CARGO) run -q -p albite_bootstrap --features codegen -- doctor check link reserved-names \
	  "$${CARGO_TARGET_DIR:-target}/debug/libalbite_core.a" \
	  $(RESERVED_NAMES_DIR)/probe-off $(RESERVED_NAMES_DIR)/probe-all
