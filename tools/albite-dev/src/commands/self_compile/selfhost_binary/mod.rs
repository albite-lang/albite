//! Self-hosted binary location and host guard (ADR 16.10.26p D2).
//!
//! Split from `artifacts.rs` when ADR 20.10.26d renamed the stage to
//! `albite1` and added [`renamed_stage`], which that file — at its size cap —
//! had no room for.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where a self-compile writes `albite1`, under the container's writable
/// `out/` bind (its root is read-only); the root `albite1` links into it.
pub const SELF_COMPILE_OUTPUT: &str = "out/albite1";

/// The binary a command asked for: an explicit path, else `$TUNGSTEN1`,
/// else `./albite1` — never [`super::artifacts::Artifacts::output_binary`].
pub fn requested_selfhost_binary(
    explicit: Option<&str>,
    tungsten1_env: Option<OsString>,
) -> PathBuf {
    match (explicit, tungsten1_env) {
        (Some(path), _) => PathBuf::from(path),
        (None, Some(env)) if !env.is_empty() => renamed_stage(PathBuf::from(env)),
        _ => PathBuf::from("./albite1"),
    }
}

/// `$TUNGSTEN1` naming the pre-rename stage (`out/tungsten1`) means the stage a
/// self-compile now writes as `albite1` beside it (ADR 20.10.26d D3).
///
/// The devcontainer's `containerEnv` and a lane's `-e` still spell the old
/// name: `.devcontainer/` is a protected path that ADR could not write, and
/// 20.10.26e renames the variable itself. `make/config.mk` maps the same way.
/// An explicit `--binary` is never rewritten — it names a file on purpose.
pub fn renamed_stage(path: PathBuf) -> PathBuf {
    if path.file_name().is_some_and(|name| name == "tungsten1") {
        path.with_file_name("albite1")
    } else {
        path
    }
}

/// May `path` run? In the container (`ALBITE_IN_CONTAINER`) yes; outside
/// it `out/` is container-written, so a symlink or a canonical path under the
/// workspace's `out/` is refused on any OS. Pure: the caller stats.
pub fn resolve_selfhost_binary(
    in_container: bool,
    path: &Path,
    canonical: &Path,
    is_symlink: bool,
    workspace_root: &Path,
) -> Result<PathBuf, String> {
    if in_container {
        return Ok(path.to_path_buf());
    }
    let out_dir = workspace_root.join("out");
    if is_symlink || canonical.starts_with(&out_dir) {
        return Err(format!(
            "refusing to run {} on the host: it resolves into {}, which the devcontainer \
             writes. Build a host-native albite1 first (`make self-compile`). (ADR 16.10.26p D2)",
            path.display(),
            out_dir.display(),
        ));
    }
    Ok(path.to_path_buf())
}

/// Does the `IN_CONTAINER` setting's value say we are in the devcontainer?
/// Only the exact value `1` does. Pure: the caller reads the setting.
pub fn in_container_flag(setting: Option<&str>) -> bool {
    setting == Some("1")
}

/// [`resolve_selfhost_binary`] over the real environment and filesystem.
pub fn selfhost_binary(explicit: Option<&str>, root: &Path) -> Result<PathBuf, String> {
    let path = requested_selfhost_binary(explicit, std::env::var_os("TUNGSTEN1"));
    let in_container = in_container_flag(albite_runtime::settings::read("IN_CONTAINER").as_deref());
    let is_symlink = std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
    let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    resolve_selfhost_binary(in_container, &path, &canonical, is_symlink, &root)
}

#[cfg(test)]
mod tests;
