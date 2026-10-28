//! Tests for `selfhost_binary.rs`: where the self-hosted stage is written,
//! which one a command runs, and the host guard (ADR 16.10.26p D2), plus the
//! pre-rename `$TUNGSTEN1` mapping (ADR 20.10.26d D3).

use super::*;
use crate::commands::self_compile::artifacts::Artifacts;

/// 16.10.26p AC4c: the self-compile writes under `out/`, and every binary
/// albite-dev runs is `--binary`, else `$TUNGSTEN1`, else `./albite1`.
#[test]
fn output_is_under_out_and_the_requested_binary_falls_back_in_order() {
    let arts = Artifacts::at_root(PathBuf::from("/ws"), PathBuf::from("/build/target"));
    assert_eq!(arts.output_binary, PathBuf::from("/ws/out/albite1"));
    let env = || Some(OsString::from("out/albite1"));
    assert_eq!(
        requested_selfhost_binary(Some("x/t1"), env()),
        PathBuf::from("x/t1")
    );
    assert_eq!(
        requested_selfhost_binary(None, env()),
        PathBuf::from("out/albite1")
    );
    for unset in [None, Some(OsString::new())] {
        assert_eq!(
            requested_selfhost_binary(None, unset),
            PathBuf::from("./albite1")
        );
    }
}

/// 20.10.26d D3: the devcontainer still exports `TUNGSTEN1=out/tungsten1`, and
/// that means the `albite1` a self-compile now writes beside it.
#[test]
fn a_pre_rename_stage_in_the_environment_resolves_to_albite1() {
    assert_eq!(
        requested_selfhost_binary(None, Some(OsString::from("out/tungsten1"))),
        PathBuf::from("out/albite1")
    );
    assert_eq!(
        requested_selfhost_binary(None, Some(OsString::from("./tungsten1"))),
        PathBuf::from("./albite1")
    );
}

/// 20.10.26d D3: only the exact stage name maps, and an explicit `--binary`
/// is never rewritten.
#[test]
fn renamed_stage_maps_only_the_old_stage_name() {
    assert_eq!(
        renamed_stage(PathBuf::from("tungsten1")),
        PathBuf::from("albite1")
    );
    assert_eq!(
        renamed_stage(PathBuf::from("/x/tungsten1_x86")),
        PathBuf::from("/x/tungsten1_x86")
    );
    assert_eq!(
        renamed_stage(PathBuf::from("/opt/tungsten1/bin")),
        PathBuf::from("/opt/tungsten1/bin")
    );
    assert_eq!(
        requested_selfhost_binary(Some("out/tungsten1"), None),
        PathBuf::from("out/tungsten1")
    );
}

/// 16.10.26p AC4c: outside the container a symlink, or a canonical path under
/// the workspace's own `out/`, is refused; another `out` component is not;
/// inside the container the path is returned as given.
#[test]
fn the_host_refuses_a_symlink_or_the_workspace_out_dir_and_the_container_does_not() {
    let (root, link) = (Path::new("/ws"), Path::new("./albite1"));
    let in_out = Path::new("/ws/out/albite1");
    let resolve = |inside, canonical: &Path, symlink| {
        resolve_selfhost_binary(inside, link, canonical, symlink, root)
    };
    let err = resolve(false, in_out, false).unwrap_err();
    assert!(
        err.contains("/ws/out") && err.contains("make self-compile"),
        "{err}"
    );
    assert!(resolve(false, Path::new("/ws/albite1"), true).is_err());
    assert_eq!(
        resolve(false, Path::new("/ws/albite1"), false),
        Ok(link.to_path_buf())
    );
    assert!(resolve(false, Path::new("/ws/src/out/albite1"), false).is_ok());
    assert!(resolve(false, Path::new("/elsewhere/out/albite1"), false).is_ok());
    assert_eq!(resolve(true, in_out, true), Ok(link.to_path_buf()));
}

/// 20.10.26e: the `IN_CONTAINER` setting means "in the container" only when
/// its value is exactly `1`; unset, empty and any other value are the host.
#[test]
fn only_an_in_container_value_of_one_means_the_container() {
    assert!(in_container_flag(Some("1")));
    for host in [None, Some(""), Some("0"), Some("true"), Some("11")] {
        assert!(!in_container_flag(host), "{host:?}");
    }
}
