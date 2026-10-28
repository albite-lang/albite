use clap::{Parser, ValueEnum};
use std::path::PathBuf;

mod capture;
mod compare;
mod compiler;
mod discover;
mod json_parity;
mod normalize;
mod preflight;
mod run;
mod update;

use run::{clean_cache, run_category, summary_line, CategoryTally};

/// Golden test runner for the Tungsten compiler.
///
/// Compares compiler output against `.expected` snapshot files.
/// Strips ANSI color codes and normalizes paths before comparison.
///
/// The discovery (`discover`), compiler invocation (`compiler`), comparison
/// (`compare`), execution (`run`), and `--update` (`update`) logic live in
/// sibling modules (ADR 16.7.26b file-size paydown).
#[derive(Parser)]
#[command(
    name = "golden",
    about = "Golden test runner for the Tungsten compiler"
)]
pub(crate) struct Cli {
    /// Test category to run (omit to run all)
    #[arg(value_enum)]
    category: Option<Category>,

    /// Regenerate .expected files from current compiler output
    #[arg(long)]
    update: bool,

    /// Path to the tungsten compiler binary
    #[arg(long, default_value = "./target/release/albite")]
    compiler: PathBuf,

    /// Path to the golden tests directory
    #[arg(long, default_value = "tests/golden")]
    dir: PathBuf,

    /// Skip cache cleaning between categories
    #[arg(long)]
    no_clean: bool,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum Category {
    Check,
    Run,
    Error,
    Compile,
    /// `compile` invocations that must FAIL, one per `.args` (ADR 30.9.26j):
    /// the captured stderr is the snapshot. Needs codegen, as `compile` does.
    CompileError,
    Test,
    /// Every `error/` fixture under both `--error-format`s (ADR 28.9.26e).
    JsonParity,
}

impl Category {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Category::Check => "check",
            Category::Run => "run",
            Category::Error => "error",
            Category::Compile => "compile",
            Category::CompileError => "compile-error",
            Category::Test => "test",
            Category::JsonParity => "json-parity",
        }
    }

    /// The fixture directory under `--dir`. Only `compile-error` differs from
    /// [`Category::as_str`]: the CLI spells it with a hyphen, the tree with an
    /// underscore, as the other multi-word fixture names are.
    pub(crate) fn dir_name(&self) -> &str {
        match self {
            Category::CompileError => "compile_error",
            other => other.as_str(),
        }
    }

    /// Whether the category exercises `compile`, which exists only in a
    /// codegen-featured binary — such fixtures are skipped, not failed, without.
    pub(crate) fn needs_codegen(&self) -> bool {
        matches!(self, Category::Compile | Category::CompileError)
    }

    pub(crate) fn tungsten_cmd(&self) -> &str {
        match self {
            Category::Check | Category::Error | Category::JsonParity => "check",
            Category::Run => "run",
            Category::Compile | Category::CompileError => "compile",
            Category::Test => "test",
        }
    }

    fn all() -> Vec<Category> {
        vec![
            Category::Check,
            Category::Run,
            Category::Error,
            Category::Compile,
            Category::CompileError,
            Category::Test,
            Category::JsonParity,
        ]
    }
}

fn main() {
    let cli = Cli::parse();

    if !cli.compiler.exists() {
        eprintln!(
            "Compiler not found at {}. Build with: cargo build --release -p albite_bootstrap",
            cli.compiler.display()
        );
        std::process::exit(1);
    }

    let categories = match &cli.category {
        Some(c) => vec![c.clone()],
        None => Category::all(),
    };

    println!("Golden Test Runner");
    println!("==================");
    println!();

    // Probe the binary once, before any category runs (ADR 21.7.26f / D3).
    // An unrunnable binary would otherwise false-red every test in every
    // category with a per-test "failed to run compiler" diff.
    let preflight = preflight::probe(&cli.compiler);
    println!("{}", preflight.banner(&cli.compiler));
    println!();
    if preflight.is_fatal() {
        std::process::exit(1);
    }
    let codegen_available = preflight.codegen_available();

    let mut total = CategoryTally::default();

    for category in &categories {
        if !cli.no_clean {
            clean_cache(&cli.compiler);
        }
        total.add(run_category(&cli, category, codegen_available));
    }

    if categories.len() > 1 {
        println!("==================");
        print!("Total: {}", summary_line(&total).trim_start());
    }

    if total.is_failure() {
        std::process::exit(1);
    }
}
