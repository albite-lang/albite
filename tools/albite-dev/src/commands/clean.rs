use clap::{Args, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

const IR_DIR: &str = "/tmp/tungsten1_ll";
/// Under the writable `out/` bind (ADR 16.10.26p D2).
const WORKSPACE_IR_DIR: &str = "out/tungsten1_ll";
const LOG_DIR: &str = "/var/log/tungsten";

#[derive(Args)]
pub struct CleanArgs {
    #[command(subcommand)]
    pub command: CleanCommand,
}

#[derive(Subcommand)]
pub enum CleanCommand {
    /// Remove IR output directories (/tmp/tungsten1_ll/ and out/tungsten1_ll/)
    Ir,
    /// Remove captured log files (alias for `logs clean`)
    Logs,
    /// Remove all: IR directories, logs, and $WS_SCRATCH/ll/ (default target/ll/)
    All,
}

pub fn run(args: CleanArgs) -> Result<(), String> {
    match args.command {
        CleanCommand::Ir => clean_ir(),
        CleanCommand::Logs => clean_logs(),
        CleanCommand::All => {
            clean_ir()?;
            clean_logs()?;
            clean_target_ll()
        }
    }
}

fn clean_ir() -> Result<(), String> {
    let mut removed = Vec::new();

    for dir in [IR_DIR, WORKSPACE_IR_DIR] {
        let path = Path::new(dir);
        if path.exists() {
            fs::remove_dir_all(path).map_err(|e| format!("failed to remove {dir}: {e}"))?;
            removed.push(dir);
        }
    }

    if removed.is_empty() {
        println!("(no IR directories to clean)");
    } else {
        println!("Removed: {}", removed.join(", "));
    }
    Ok(())
}

fn clean_logs() -> Result<(), String> {
    let dir = Path::new(LOG_DIR);
    if !dir.exists() {
        println!("(no log directory)");
        return Ok(());
    }

    let mut count = 0;
    for entry in fs::read_dir(dir).map_err(|e| format!("read dir: {e}"))? {
        let entry = entry.map_err(|e| format!("entry: {e}"))?;
        if entry.path().is_file() && entry.file_name() != ".gitkeep" {
            fs::remove_file(entry.path())
                .map_err(|e| format!("remove {}: {e}", entry.path().display()))?;
            count += 1;
        }
    }

    println!("Removed {count} log file(s)");
    Ok(())
}

/// `$WS_SCRATCH/ll`, default `target/ll`: the container sets `WS_SCRATCH`
/// to `out/scratch`, because its `target/` is read-only (ADR 16.10.26p D1).
fn target_ll_dir(ws_scratch: Option<std::ffi::OsString>) -> PathBuf {
    match ws_scratch {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("ll"),
        _ => PathBuf::from("target/ll"),
    }
}

fn clean_target_ll() -> Result<(), String> {
    let path = target_ll_dir(std::env::var_os("WS_SCRATCH"));
    if path.exists() {
        fs::remove_dir_all(&path)
            .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
        println!("Removed {}/", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16.10.26p D2: the scratch `ll` directory follows `WS_SCRATCH`.
    #[test]
    fn target_ll_dir_follows_ws_scratch() {
        assert_eq!(target_ll_dir(None), PathBuf::from("target/ll"));
        assert_eq!(target_ll_dir(Some("".into())), PathBuf::from("target/ll"));
        assert_eq!(
            target_ll_dir(Some("out/scratch".into())),
            PathBuf::from("out/scratch/ll")
        );
        assert_eq!(WORKSPACE_IR_DIR, "out/tungsten1_ll");
    }
}
