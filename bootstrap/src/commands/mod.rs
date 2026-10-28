//! CLI command handlers for run, eval, repl, clean, and cache operations.

mod cache;
mod cache_inspect;
mod cache_outcome;
mod cache_root;
mod cache_stats;
pub use cache::{cmd_cache_clean_all, cmd_cache_prune, cmd_clean};
pub use cache_inspect::cmd_cache_inspect;
pub use cache_stats::cmd_cache_stats;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use albite_bootstrap::driver::diagnostics::{self, json, ErrorFormat};
use albite_bootstrap::driver::{self, Mode, PipelineOpts, PipelineResult};

/// Options for the check command.
pub struct CheckOptions {
    pub verbose: bool,
    pub no_cache: bool,
    pub max_errors: usize,
    pub dump_types: bool,
    pub error_format: ErrorFormat,
}

/// Check command: type-check a file.
///
/// In JSON mode (ADR 28.9.26e) the render entry points write the records, so
/// a clean or failed run prints nothing more here: stdout holds records only
/// (D3). A pipeline/I/O failure has no diagnostic to render, so it becomes
/// one location-less record; the exit codes are the human mode's.
pub fn cmd_check(file: &PathBuf, opts: &CheckOptions) -> ExitCode {
    let CheckOptions {
        verbose,
        no_cache,
        max_errors,
        dump_types,
        error_format,
    } = *opts;
    diagnostics::set_error_format(error_format);
    let json = error_format == ErrorFormat::Json;

    let pipeline_opts = PipelineOpts {
        mode: Mode::Check,
        verbose,
        dump_types,
    };
    match driver::run_file_with_options(file, &pipeline_opts, no_cache, max_errors) {
        Ok(PipelineResult::Checked {
            num_defs,
            sorry,
            resting_theorems,
            kernel_exempt,
        }) => {
            if !json {
                let file = file.display().to_string();
                let line = driver::check_verdict_line(
                    &file,
                    num_defs,
                    &sorry,
                    resting_theorems,
                    &kernel_exempt,
                );
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Ok(PipelineResult::Failed) => ExitCode::FAILURE,
        Ok(PipelineResult::Evaluated { .. }) => {
            // Shouldn't happen in check mode
            ExitCode::SUCCESS
        }
        Ok(PipelineResult::Tested { .. }) => {
            // Shouldn't happen in check mode
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                diagnostics::emit_record(&json::failure_record(&e.to_string()));
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(3) // IO error
        }
    }
}

/// Options for the run command.
pub struct RunOptions {
    pub verbose: bool,
    pub no_cache: bool,
    pub max_errors: usize,
    pub dump_types: bool,
    pub error_format: ErrorFormat,
}

/// Run command: type-check and evaluate `main()`.
///
/// In JSON mode (ADR 29.9.26f D8) every record goes to **stderr**: stdout is
/// the program's own output and then the value line, byte-identical to human
/// mode. Front-end records all precede the program's first write, because
/// evaluation starts only after rendering.
pub fn cmd_run(file: &PathBuf, opts: &RunOptions) -> ExitCode {
    let RunOptions {
        verbose,
        no_cache,
        max_errors,
        dump_types,
        error_format,
    } = *opts;
    diagnostics::set_error_format(error_format);
    diagnostics::set_record_stream(diagnostics::RecordStream::Stderr);
    let pipeline_opts = PipelineOpts {
        mode: Mode::Run,
        verbose,
        dump_types,
    };
    match driver::run_file_with_options(file, &pipeline_opts, no_cache, max_errors) {
        Ok(PipelineResult::Evaluated { value, ty }) => {
            let value_str = driver::format_value(&value);
            if verbose {
                let ty_str = driver::format_type(&ty);
                println!("{value_str} : {ty_str}");
            } else {
                println!("{value_str}");
            }
            ExitCode::SUCCESS
        }
        Ok(PipelineResult::Checked { .. }) => {
            // Shouldn't happen in run mode
            ExitCode::SUCCESS
        }
        Ok(PipelineResult::Tested { .. }) => {
            // Shouldn't happen in run mode
            ExitCode::SUCCESS
        }
        Ok(PipelineResult::Failed) => ExitCode::FAILURE,
        Err(e) => {
            if error_format == ErrorFormat::Json {
                diagnostics::emit_record(&json::failure_record(&e.to_string()));
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(3)
        }
    }
}

/// Eval command: evaluate a single expression.
pub fn cmd_eval(expr: &str, verbose: bool, max_errors: usize) -> ExitCode {
    match driver::eval_expr(expr, verbose, max_errors) {
        Ok(PipelineResult::Evaluated { value, ty }) => {
            let value_str = driver::format_value(&value);
            if verbose {
                let ty_str = driver::format_type(&ty);
                println!("{value_str} : {ty_str}");
            } else {
                println!("{value_str}");
            }
            ExitCode::SUCCESS
        }
        Ok(PipelineResult::Failed) => ExitCode::FAILURE,
        Ok(PipelineResult::Checked { .. }) => ExitCode::SUCCESS,
        Ok(PipelineResult::Tested { .. }) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(3)
        }
    }
}

/// REPL command: interactive mode.
///
/// The streams are parameters so a test can drive a session: bound to
/// `io::stdin()` inside, no test could reach this body without blocking on a
/// terminal, and every edit to it surfaced as a surviving mutant (ADR 20.10.26b).
/// Write errors on the two output streams are ignored; the `print!` calls
/// this replaced panicked on them instead.
pub fn cmd_repl(
    input: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let _ = writeln!(
        stderr,
        "Albite {} — Interactive Mode",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(
        stderr,
        "Type expressions to evaluate, or :help for commands."
    );
    let _ = writeln!(stderr);

    loop {
        let _ = write!(stdout, "tg> ");
        let _ = stdout.flush();

        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => {
                let _ = writeln!(stdout);
                break;
            }
            Ok(_) => {}
            Err(e) => {
                let _ = writeln!(stderr, "error: {e}");
                break;
            }
        }

        let line = line.trim();

        if line.starts_with(':') {
            match line {
                ":quit" | ":q" | ":exit" => break,
                ":help" | ":h" | ":?" => {
                    let _ = writeln!(stdout, "Commands:");
                    let _ = writeln!(stdout, "  :help, :h, :?    Show this help");
                    let _ = writeln!(stdout, "  :quit, :q        Exit the REPL");
                    let _ = writeln!(stdout);
                    let _ = writeln!(stdout, "Enter expressions to evaluate them.");
                }
                _ => {
                    let _ = writeln!(stderr, "Unknown command: {line}");
                    let _ = writeln!(stderr, "Type :help for available commands.");
                }
            }
            continue;
        }

        if line.is_empty() {
            continue;
        }

        match driver::eval_expr(line, false, 20) {
            Ok(PipelineResult::Evaluated { value, ty }) => {
                let value_str = driver::format_value(&value);
                let ty_str = driver::format_type(&ty);
                let _ = writeln!(stdout, "{value_str} : {ty_str}");
            }
            Ok(PipelineResult::Failed) => {}
            Ok(PipelineResult::Checked { .. }) => {}
            Ok(PipelineResult::Tested { .. }) => {}
            Err(e) => {
                let _ = writeln!(stderr, "error: {e}");
            }
        }
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod repl_tests {
    use super::cmd_repl;

    /// Runs one REPL session over `input`, returning (stdout, stderr).
    fn session(input: &str) -> (String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        cmd_repl(&mut input.as_bytes(), &mut out, &mut err);
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    // 20.10.26b D5: the REPL greets as Albite, and a session reads its
    // input — an expression is evaluated and `:q` ends it before the next line.
    #[test]
    fn the_repl_greets_as_albite_and_evaluates_until_quit() {
        let (out, err) = session("2 + 2\n:q\n3 + 3\n");
        assert!(err.starts_with("Albite "), "{err}");
        assert!(err.contains("Interactive Mode"), "{err}");
        assert!(out.contains("4 : "), "{out}");
        assert!(!out.contains("6 : "), "{out}");
    }

    #[test]
    fn help_goes_to_stdout_and_an_unknown_command_to_stderr() {
        let (out, err) = session(":help\n:nope\n");
        assert!(out.contains("Commands:"), "{out}");
        assert!(err.contains("Unknown command: :nope"), "{err}");
    }
}
