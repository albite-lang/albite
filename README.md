# Albite

**A dependently-typed language where proofs and programs are one.**

Albite (formerly [Tungsten by Spinning Cat Studios](https://github.com/Spinning-Cat-Studios/tungsten)) is a self-hosted research language with dependent types, proofs, Rust-inspired syntax, and native compilation. Its 2.0-alpha is a pre-release: the language is usable for experiments and compiler work, but breaking changes are expected before 2.0.

## Highlights

- **Self-Hosted Compiler**: The Albite compiler is written in Albite itself, with a verified triple-compile fixed point
- **Small Trusted Core**: ~1.5K LOC trusted kernel — if the core is correct, your proofs are sound
- **AI-Friendly Diagnostics**: Levenshtein-based "did you mean?" suggestions, contextual hints, and structured error spans designed for both humans and LLMs
- **Lean + Rust Flavor**: Familiar syntax for Rust developers, with dependent types and proofs inspired by Lean
- **Native Compilation**: LLVM 18 backend for compiling to native binaries
- **Incremental Compilation**: Build cache with dependency tracking for fast iteration

## Quick Start

The 2.0-alpha pre-release has pre-built binaries for macOS (ARM64) and Linux (x86_64) on the [Releases](../../releases) page. The [language manual](https://spinningcatstudios.com/tungsten) covers the language and its limitations.

**The command is being renamed.** Each archive ships the compiler as both `albite` and `tungsten`, and both names work throughout 2.0.x. The examples below still spell it `tungsten`.

**macOS note:** The binaries are not code-signed. macOS will show a "cannot be verified" dialog on first run. To fix this:

```bash
xattr -d com.apple.quarantine albite tungsten albite-bootstrap
```

You can verify download integrity with the `checksums.txt` file included in each release:

```bash
shasum -a 256 -c checksums.txt
```

**Linux note:** The Linux binary is x86_64 and requires glibc ≥ 2.39 (Ubuntu 24.04+).
On older distros or non-x86 hosts (including Apple Silicon Macs), use the included
Docker tooling:

```bash
# Build the Docker image (done automatically on first run)
./albite-docker.sh build

# Run albite commands inside the container
./albite-docker.sh run check myfile.tg
./albite-docker.sh run compile hello.tg -o hello
./albite-docker.sh run run examples/hello.tg

# Open an interactive shell
./albite-docker.sh shell
```

Type-checking (`albite check`) and interpreted execution (`albite run`) work without LLVM. Only `albite compile` requires LLVM 18.

### Building from Source

**Without LLVM** — supports `check`, `run`, and `eval` (no native compilation):

```bash
cargo build --release -p albite_bootstrap --no-default-features

./target/release/albite run examples/hello.tg         # → hello world
./target/release/albite check examples/arithmetic.tg
./target/release/albite eval "2 + 2"                  # → 4
```

**With LLVM 18** — adds the `compile` subcommand for native binaries (see [LLVM Setup](#llvm-setup-for-native-compilation)):

```bash
# macOS with Homebrew (use /path/to/llvm@18 on other platforms)
LLVM_SYS_180_PREFIX=$(brew --prefix llvm@18) cargo build --release

./target/release/albite compile examples/hello.tg -o hello
./hello                                                 # → hello world
```

### Using Make

```bash
# Interpreted execution (no LLVM needed)
make run FILE=examples/hello.tg

# Native compilation (requires LLVM 18)
make compile FILE=examples/hello.tg OUT=hello
./hello

# Or compile and run in one step
make compile-run FILE=examples/hello.tg

# Don't have LLVM? Use the dev container:
make devcontainer-up
make devcontainer-compile-run FILE=examples/hello.tg
```

## Examples

**Hello World** (`examples/hello.tg`):
```tungsten
fn main() -> String {
    "hello world"
}
```

**Arithmetic** (`examples/arithmetic.tg`):
```tungsten
fn main() -> Nat {
    20 + 20 + 2
}
```

**Boolean Logic** (`examples/logic.tg`):
```tungsten
fn my_not(b: Bool) -> Bool {
    if b { false } else { true }
}

fn my_and(a: Bool, b: Bool) -> Bool {
    if a { b } else { false }
}
```

**Algebraic Data Types:**
```tungsten
type Option<T> = None | Some(T)
type List<T> = Nil | Cons(T, List<T>)
type Result<T, E> = Ok(T) | Err(E)

fn unwrap_or<T>(opt: Option<T>, default: T) -> T {
    match opt {
        None => default,
        Some(x) => x,
    }
}
```

**Record Types:**
```tungsten
type Point = { x: Nat, y: Nat }

fn origin() -> Point {
    { x: 0, y: 0 }
}
```

**Modules:**
```tungsten
mod utils;      // loads utils.tg or utils/mod.tg
mod math;

use utils::helper;
use math::{add, sub};
```

See the `examples/` directory for more, including proofs (`proof.tg`, `proofs_boolean.tg`, `proofs_natural.tg`).

## AI-Friendly Diagnostics

Albite provides rich, contextual error messages designed for both humans and AI assistants:

```text
[E0010] Error: expected `Nat`, found `String`
   ╭─[tests/golden/error/type_mismatch_hints.tg:3:10]
   │
 2 │ fn main() -> Nat {
   │              ─┬─
   │               ╰─── expected due to return type
 3 │     42 + "hello"
   │          ───┬───
   │             ╰───── expected `Nat`, found `String`
───╯
  hint: run `albite info type-encoding <Type> tests/golden/error/type_mismatch_hints.tg`
  hint: run `albite doctor suggest-tools "type mismatch"`
```

The full output is pinned by [a golden diagnostic test](tests/golden/error/type_mismatch_hints.expected).

- **Contextual hints**: "expected because of return type annotation at line 3"
- **Did-you-mean suggestions**: Levenshtein-based typo detection for identifiers
- **Constructor suggestions**: When you use the wrong constructor, shows valid options
- **Structured spans**: Precise source locations with context

## What Works / What Doesn't

### ✅ Working

- **Self-hosted compiler** — the compiler compiles itself, with verified triple-compile fixed point
- **Type checking** — dependent types, generics, ADTs, records, pattern matching
- **Native compilation** — LLVM 18 backend produces native binaries (macOS ARM64 + Linux x86_64)
- **Proofs** — dependent types enable theorem proving (see `examples/proof.tg`)
- **AI-friendly diagnostics** — rich errors with Levenshtein suggestions and contextual hints
- **Rust-style modules** — `mod`, `use`, with per-module compilation and elaboration caching
- **FFI** — `extern "C" fn` for calling C/Rust functions
- **Early return** — `return expr` and `?` operator for Result/Option
- **Pattern matching** — nested destructuring, `if let`, `let`-`else`, `try` blocks
- **Parallel codegen** — per-function LLVM IR emission with parallel compilation
- **Static linking** — single binary output, no `LD_LIBRARY_PATH` required
- **Termination checking** *(2.0-alpha)* — every recursive function must be seen to stop; `#[decreasing(arg)]` names the shrinking parameter, `#[partial]` opts executable code out, and a proof may not depend on a partial definition
- **Strict positivity** *(2.0-alpha)* — a type may refer to itself only to the right of an arrow
- **Signed integers** *(2.0-alpha)* — `Int` beside `Nat`, overflow traps, `to_int`/`from_int`, and `match` on numbers with guards
- **`StringBuilder`** *(2.0-alpha)* — linear-time string building through the runtime, via `extern "C"`
- **Arena allocation** *(2.0-alpha)* — `ALBITE_ARENA=bump` runs a compiled program on a bump-pointer arena
- **One error per mistake** *(2.0-alpha)* — a broken definition is reported once, not at every use

The rules the compiler now enforces — each with a rejected program, its
diagnostic and the rewrite — are documented at
[spinningcatstudios.com/tungsten](https://spinningcatstudios.com/tungsten/reference/limitations#rules-the-compiler-enforces).

### ❗ Known Limitations

- **No borrow checker** — memory safety relies on immutability; mutable references require manual care

### ❌ Not Yet (v2.0 / v2.1 / v2.2)

For the full, checked list see
[Current limitations](https://spinningcatstudios.com/tungsten/reference/limitations).

**v2.0** — production-readiness:
- Universe hierarchy (`Type 0 : Type 1 : ...`)
- Tactic language (simp, induction, rewrite)
- LSP (minimal: go-to-definition, hover)
- `let mut` syntax
- Or-patterns on constructors (on numbers they work since 2.0-alpha)
- Package manifest (`albite.toml`; `tungsten.toml` is still read through 2.0.x)
- DWARF source-level debugging

**v2.1** — ecosystem & adoption:
- Borrow checker
- Type classes / traits
- Indexed families (length-indexed vectors and the like)
- Proof irrelevance (Prop universe, compile-time proof erasure)
- Crypto & networking primitives (type-safe sockets, TLS)
- Provable network models (session types, failure reasoning)
- Full LSP with completions
- REPL
- Lean4 transpiler
- Community contributions (see [CONTRIBUTING.md](CONTRIBUTING.md))

**v2.2** — proof language maturity:
- Metaprogramming framework (custom tactics in Albite)
- User-defined notation and syntax extensions
- Quotient types
- Automatic type coercions

## Project Structure

```
tungsten/
├── albite_core/     # Trusted kernel (Rust) — type checker & evaluator
├── bootstrap/         # Bootstrap compiler (Rust) — lexer, parser, elaborator
├── albite_codegen/  # LLVM codegen crate (Rust)
├── src/compiler/      # Self-hosted compiler (Albite)
├── stdlib/            # Standard library (Albite)
├── examples/          # Example programs and proofs
└── tests/             # Test suite including golden tests
```

## Architecture

Albite uses a **small trusted kernel** architecture:

- **`albite_core`** (~1.5K LOC Rust): The trusted computing base. Contains the type checker and evaluator. If this code is correct, Albite is sound.

- **`bootstrap`** (Rust): The bootstrap compiler. Parses surface syntax and elaborates it to Core terms. Bugs here can't compromise soundness — they can only cause compilation to fail.

- **`src/compiler/`** (Albite): The self-hosted compiler. Written in Albite, compiled by the bootstrap compiler, then compiles itself. The triple-compile fixed point has been verified.

- **`albite_codegen`** (Rust): LLVM 18 code generation via inkwell.

## Commands

| Command | Description |
|---------|-------------|
| `albite <file>` | Run a file (evaluate `main()`) |
| `albite run <file>` | Same as above |
| `albite check <file>` | Type-check without running |
| `albite eval <expr>` | Evaluate an expression |
| `albite compile <file>` | Compile to native code (requires LLVM 18) |
| `albite clean` | Clear the build cache |
| `albite cache stats` | Show cache statistics |

### Native Compilation

```bash
# Compile to native binary
albite compile examples/hello.tg -o hello
./hello

# Emit LLVM IR instead of compiling
albite compile examples/hello.tg --emit-llvm
```

**Note:** Native compilation requires LLVM 18. Type-checking (`albite check`) and interpreted execution (`albite run`) do **not** require LLVM.

## Development

```bash
# Build (interpreter only, no LLVM required)
cargo build --release -p albite_bootstrap --no-default-features

# Build with native compilation support (requires LLVM 18)
# macOS with Homebrew (use /path/to/llvm@18 on other platforms)
LLVM_SYS_180_PREFIX=$(brew --prefix llvm@18) cargo build --release

# Run tests
make test

# Run golden tests
make golden
```

### Profiling & Benchmarking

Requires [mise](https://mise.jdx.dev/) for tool management. One-time setup:

```bash
mise install          # installs samply + hyperfine from .mise.toml
# or explicitly:
make setup-profiling
```

**Profile** — CPU sampling profiler, opens Firefox Profiler UI in your browser:

```bash
make profile                          # profile the self-hosted compiler check
make profile FILE=examples/hello.tg   # profile a specific file
```

**Benchmark** — statistical timing (N runs, warmup, mean/stddev/min/max):

```bash
make bench                            # benchmark the self-hosted compiler check
make bench FILE=examples/hello.tg     # benchmark a specific file
```

**Compare** — side-by-side benchmark of two binaries:

```bash
make bench-compare A=./albite1 B=./albite2
```

### LLVM Setup (for native compilation)

**macOS (Homebrew):**
```bash
brew install llvm@18
export LLVM_SYS_180_PREFIX=$(brew --prefix llvm@18)
```

**Linux (apt):**
```bash
apt install llvm-18 llvm-18-dev
export LLVM_SYS_180_PREFIX=/usr/lib/llvm-18
```

Then build with codegen:
```bash
cargo build --release
```

If building without LLVM, use `--no-default-features` to disable the codegen feature.

## Stability

Albite 2.0-alpha adds stricter checks for recursive code and types, signed integers, a string builder, and an opt-in arena allocator. It is a research pre-release, not a stability promise: programs may need changes before 2.0. Semver applies to tooling releases, not language surface stability. See the [current limitations](https://spinningcatstudios.com/tungsten/reference/limitations) before relying on a feature.

## Support

Albite is a personal research project; support is best-effort. Bug reports with minimal reproductions are welcome via GitHub Issues.

## Contributing

Bug reports, questions, and documentation feedback are welcome. Pull requests are not being accepted at this time — see [CONTRIBUTING.md](CONTRIBUTING.md) for details and the plan for v2.1.

## License

MIT
