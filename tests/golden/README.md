# Golden Tests

This directory contains golden test files for verifying the self-hosted driver produces correct output.

The runner is the `tools/golden` Rust crate, driven through `make golden`. A
second, shell implementation lived in this directory until ADR 31.7.26b retired
it: it consumed every per-test status as an `if` condition, so a failing golden
test printed a red ✗, was counted in "N failed", and exited **0** — and it was
the runner both CI workflows invoked. Category selection, `--update` and the
comparison semantics below are unchanged; only the implementation and the entry
point are.

## Structure

```
tests/golden/
├── README.md           # This file
├── check/              # Files that should type-check successfully
│   └── *.expected      # Expected output for each .tg file
├── run/                # Files that should run and produce output
│   └── *.expected      # Expected output for each .tg file
├── error/              # Files that should produce errors
│   └── *.expected      # Expected error output for each .tg file
├── compile/            # Files that should compile, then run (needs codegen)
│   └── *.expected      # Expected output of the compiled binary
├── compile_error/      # `compile` invocations that must fail, one `.args` each (needs codegen)
│   └── *.expected      # Expected stderr; `json_*` twins pin --error-format json
└── test/               # Files exercising `tungsten test` discovery
    └── *.expected      # Expected runner output for each .tg file
```

Compiling a `compile`-category fixture emits its executable as an extensionless
sibling of the `.tg` file. Those are gitignored (see `.gitignore`); five
devcontainer-built ELF binaries reached git before that rule existed.

The `compile-error` category (ADR 30.9.26j) runs `tungsten compile <fixture>
<.args>` and snapshots its output, so it is skipped without codegen as
`compile` is. Its human fixtures were snapshotted from the binary *before*
`compile`'s failures became one structured value, and are the oracle that the
human text did not change. Each `json_*` fixture is a golden twin with the same
`.tg` and `--error-format json` appended to its `.args`; the `json-parity`
category's `compile` arm runs every other fixture under both formats.

## Running Tests

`make golden` expects a release compiler at `./target/release/albite`
(`cargo build --release`); the `compile` category additionally needs it built
with the `codegen` feature, and is reported as **skipped** rather than passed
when it is not.

```bash
# Run all golden tests
make golden

# Update expected files (regenerate golden output from the current compiler)
make golden-update

# Run a specific category (check | run | error | compile | compile-error | test | json-parity)
cargo run --release -p golden -- error

# Update one category
cargo run --release -p golden -- --update error
```

## Adding Tests

1. Create a `.tg` file in the appropriate subdirectory
2. Run `make golden-update` to generate expected output
3. Review the `.expected` file to ensure it's correct
4. Commit both files

**A `.tg` with no `.expected` is reported `? (no expected file)` and counted as
PASSED** — the runner exits 0. So step 2 is not optional: skip it and the
fixture is invisible to CI while looking green locally.

### Multi-file tests need `pub mod <sibling>;`

A subdirectory test runs `main.tg`, and a sibling is only elaborated if
`main.tg` **declares** it:

```tungsten
pub mod types;        // <- without this line, types.tg is never parsed
use types::{Bad};
```

The workspace scan *discovers* siblings (the driver prints
`Discovered N sibling module(s)`), which makes the omission easy to miss:
without the `pub mod`, the run reports `Parsed 1 module(s)`, collects **0
types**, and a fixture written to prove a cross-module diagnostic fires
silently proves nothing. Cross-check against `Parsed N module(s)` in
`--verbose` output when a multi-file fixture behaves unexpectedly.

## Color Stripping

Output comparison strips ANSI color codes to avoid false negatives from color differences between bootstrap and self-hosted drivers.
