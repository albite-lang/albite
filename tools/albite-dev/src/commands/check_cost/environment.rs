//! Reading the record's `[environment]` fields in the container (D2, P0.4).
//!
//! Each source's parsing is a pure function over its captured text; only
//! [`read_current`] touches a process or a file.

use cost_baseline::Environment;
use std::process::Command;

/// `uname -r`, `rustc -V`: the trimmed output.
pub fn parse_trimmed(text: &str) -> String {
    text.trim().to_string()
}

/// `/etc/os-release`'s `PRETTY_NAME`, unquoted.
pub fn parse_os_release(text: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim().trim_matches('"').to_string())
        .unwrap_or_default()
}

/// `lscpu`'s `Model name:`; empty when it has none, or prints the `-`
/// placeholder Docker Desktop's arm64 VM gives it (P0.4).
pub fn parse_lscpu_model(text: &str) -> String {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("Model name:"))
        .map(str::trim)
        .filter(|v| *v != "-")
        .unwrap_or_default()
        .to_string()
}

/// `/proc/cpuinfo`'s first `CPU part`, the arm64 fallback for the model.
pub fn parse_cpuinfo_part(text: &str) -> String {
    text.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "CPU part").then(|| value.trim().to_string())
        })
        .unwrap_or_default()
}

/// The first non-empty line of `llc --version`.
pub fn parse_llc_version(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// `/proc/meminfo`'s `MemTotal`, in kB.
pub fn parse_mem_total_kb(text: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// `nproc`.
pub fn parse_nproc(text: &str) -> u32 {
    text.trim().parse().unwrap_or(0)
}

/// The running container's environment. An unreadable source yields an empty
/// field, which then differs from any recorded value: stale, never equal.
pub fn read_current() -> Environment {
    let lscpu = parse_lscpu_model(&output("lscpu", &[]));
    let cpu_model = if lscpu.is_empty() {
        parse_cpuinfo_part(&file("/proc/cpuinfo"))
    } else {
        lscpu
    };
    Environment {
        kernel: parse_trimmed(&output("uname", &["-r"])),
        os: parse_os_release(&file("/etc/os-release")),
        cpu_model,
        llc: parse_llc_version(&output("llc", &["--version"])),
        rustc: parse_trimmed(&output("rustc", &["-V"])),
        nproc: parse_nproc(&output("nproc", &[])),
        mem_total_kb: parse_mem_total_kb(&file("/proc/meminfo")),
    }
}

fn output(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn file(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
