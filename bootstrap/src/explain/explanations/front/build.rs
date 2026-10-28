//! Explanations for `compile`'s build-failure codes, E0140–E0146
//! (ADR 30.9.26j D7). The codes come from `BuildErrorKind::code()` in
//! `driver/diagnostics/compile_failure/`.

use crate::explain::error_catalogue::ErrorExplanation;

const CATEGORY: &str = "Build";

/// `albite explain error E0140` … `E0146`.
pub(in crate::explain) fn build(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "IncompatibleFlags" => ErrorExplanation {
            name: "IncompatibleFlags",
            code: "E0140",
            category: CATEGORY,
            summary: "two `compile` flags that cannot be combined",
            detail: "\
`albite compile` was given two flags whose effects contradict each other, \
so it stops before reading the source. Today the one such pair is \
`--no-codegen`, which stops after the Core IR is generated, with \
`--emit-llvm`, which asks for the LLVM IR that step would have produced.\n\
\n\
Keep the flag that matches what you want: `--no-codegen` to type-check and \
lower without LLVM, `--emit-llvm` to write the `.ll` instead of linking.",
            example: "\
albite compile main.tg --no-codegen --emit-llvm\n\
// error: --no-codegen and --emit-llvm are incompatible.",
            see_also: &["MissingRequiredFlag"],
        },

        "MissingRequiredFlag" => ErrorExplanation {
            name: "MissingRequiredFlag",
            code: "E0141",
            category: CATEGORY,
            summary: "a `compile` flag that needs another flag",
            detail: "\
A `albite compile` flag only makes sense together with another one that \
was not given. `--only-unit` compiles a subset of the codegen units, and a \
partial set of object files cannot be linked into a program, so it requires \
`--emit-llvm`, which writes the selected units' `.ll` files instead.",
            example: "\
albite compile main.tg --only-unit parser\n\
// error: --only-unit requires --emit-llvm.",
            see_also: &["IncompatibleFlags"],
        },

        "CodeGenerationFailed" => ErrorExplanation {
            name: "CodeGenerationFailed",
            code: "E0142",
            category: CATEGORY,
            summary: "LLVM code generation failed",
            detail: "\
The program type-checked, but lowering it to LLVM failed: declaring or \
compiling a definition, building the `main` wrapper, compiling one module's \
unit, or writing the object file. The message names the definition or step.\n\
\n\
A program that passes `albite check` should not reach this, so it usually \
marks a compiler limitation or bug rather than a mistake in your source. \
`albite compile --dump-ir=<name>` shows the Core term the backend was given, \
and `--codegen-backtrace` adds the backend's own trace.",
            example: "\
albite compile main.tg\n\
// error: codegen failed for 'f': <the backend's reason>",
            see_also: &["OutputNotWritable"],
        },

        "ToolUnavailable" => ErrorExplanation {
            name: "ToolUnavailable",
            code: "E0143",
            category: CATEGORY,
            summary: "an external build tool could not be started",
            detail: "\
Linking a native program runs external tools: `cc` to link, `llc` to \
assemble per-module `.ll` files, and `clang` for `--sanitize`. One of them \
could not be started at all — usually because it is not installed or not on \
`PATH`. The `help:` line names the tool to install.",
            example: "\
albite compile main.tg   // with no `cc` on PATH\n\
// error: could not run linker: No such file or directory (os error 2)\n\
// help: make sure 'cc' (C compiler) is installed and in PATH",
            see_also: &["ToolFailed"],
        },

        "ToolFailed" => ErrorExplanation {
            name: "ToolFailed",
            code: "E0144",
            category: CATEGORY,
            summary: "an external build tool exited with an error",
            detail: "\
The linker, `llc` or `clang -fsanitize=address` started but exited non-zero. \
Its own output says why: in human mode it is printed above this line, and \
with `--error-format json` it is captured into the record's notes. A common \
cause is an output path whose directory does not exist.",
            example: "\
albite compile main.tg -o missing/dir/out\n\
// error: linker failed with status exit status: 1",
            see_also: &["ToolUnavailable", "RuntimeLibraryUnusable"],
        },

        "RuntimeLibraryUnusable" => ErrorExplanation {
            name: "RuntimeLibraryUnusable",
            code: "E0145",
            category: CATEGORY,
            summary: "the runtime library cannot be linked",
            detail: "\
Every native program links `libalbite_core.a`, which the compiler looks for \
next to its own executable. Either the archive is missing there, or it was \
built for the other platform — a devcontainer build and a host build share \
`target/`, so one can overwrite the other's archive. The `help:` lines give \
the rebuild command.",
            example: "\
albite compile main.tg\n\
// error: libalbite_core.a not found at target/release/libalbite_core.a",
            see_also: &["ToolFailed"],
        },

        "OutputNotWritable" => ErrorExplanation {
            name: "OutputNotWritable",
            code: "E0146",
            category: CATEGORY,
            summary: "an output or temporary file could not be written",
            detail: "\
`albite compile` could not write one of its files: the `.ll` from \
`--emit-llvm`, a per-module `.ll` copy or its directory, or a temporary \
directory for intermediates. The message names the path and the operating \
system's reason. Check that the directory exists and is writable.\n\
\n\
Writing the single `.ll` exits with status 3, as an I/O failure; every other \
site exits with 1.",
            example: "\
albite compile main.tg --emit-llvm -o missing/out.ll\n\
// error: could not write 'missing/out.ll': No such file or directory (os error 2)",
            see_also: &["ToolFailed"],
        },

        _ => return None,
    };
    Some(exp)
}
