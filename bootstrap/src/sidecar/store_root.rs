//! Where the sidecar store lives, and its one-time move from the old name
//! (ADR 20.10.26i D4).
//!
//! The store holds feedback history nothing can regenerate, so unlike the
//! project cache it is moved, not abandoned: `~/.tungsten/sidecar` becomes
//! `~/.albite/sidecar` on first use. [`sidecar_root`] decides and
//! [`classify_rename`] reads the rename's result — both pure; only
//! [`resolve_sidecar_root`] touches the filesystem. Read-only commands migrate
//! too, because a read that saw an empty new store would mislead.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The user-home directory the store lives under.
const STORE_HOME: &str = ".albite";

/// That directory's name before the rename.
const LEGACY_STORE_HOME: &str = ".tungsten";

/// What first use must do about the old store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Migrate {
    /// Nothing to move: no old store.
    Nothing,
    /// Only the old store exists: move it across.
    Move,
    /// Both exist — an older `tungsten` recreated the old one after the
    /// move: the new one wins, and the old one is named in a note.
    NewShadowsOld,
}

/// The new store's root under `home`, and what to do about the old one.
#[must_use]
pub fn sidecar_root(home: &Path, new_exists: bool, old_exists: bool) -> (PathBuf, Migrate) {
    let root = home.join(STORE_HOME).join("sidecar");
    let migrate = match (new_exists, old_exists) {
        (_, false) => Migrate::Nothing,
        (false, true) => Migrate::Move,
        (true, true) => Migrate::NewShadowsOld,
    };
    (root, migrate)
}

/// How the move's `rename` ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveOutcome {
    Moved,
    /// `ENOENT` or `ENOTEMPTY`: a concurrent process moved it first. Not an
    /// error.
    AlreadyMigrated,
    /// Anything else: this run falls back to the old path, with a note.
    Failed(String),
}

/// Read a `rename` result as D4 step 3–4 do.
#[must_use]
pub fn classify_rename(rename_result: &io::Result<()>) -> MoveOutcome {
    match rename_result {
        Ok(()) => MoveOutcome::Moved,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::DirectoryNotEmpty
                    | io::ErrorKind::AlreadyExists
            ) =>
        {
            MoveOutcome::AlreadyMigrated
        }
        Err(e) => MoveOutcome::Failed(e.to_string()),
    }
}

/// The store root under `home`, moving the old store across once.
pub fn resolve_sidecar_root(home: &Path) -> PathBuf {
    let old = home.join(LEGACY_STORE_HOME).join("sidecar");
    let new_exists = home.join(STORE_HOME).join("sidecar").exists();
    let (root, migrate) = sidecar_root(home, new_exists, old.exists());
    match migrate {
        Migrate::Nothing => root,
        Migrate::NewShadowsOld => {
            eprintln!(
                "note: `{}` is ignored: the sidecar store is `{}`",
                old.display(),
                root.display()
            );
            root
        }
        Migrate::Move => {
            let created = fs::create_dir_all(home.join(STORE_HOME));
            let renamed = created.and_then(|()| fs::rename(&old, &root));
            store_after_move(classify_rename(&renamed), old, root)
        }
    }
}

/// D4 steps 3–4: the store this run uses once the move has ended.
fn store_after_move(outcome: MoveOutcome, old: PathBuf, root: PathBuf) -> PathBuf {
    match outcome {
        MoveOutcome::Moved | MoveOutcome::AlreadyMigrated => root,
        MoveOutcome::Failed(reason) => {
            eprintln!(
                "note: could not move `{}` to `{}` ({reason}); using the old store for this run",
                old.display(),
                root.display()
            );
            old
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 20.10.26i AC3: the decision over new-only, old-only, both and neither.
    #[test]
    fn sidecar_root_covers_all_four_cases() {
        let home = Path::new("/h");
        let new = PathBuf::from("/h/.albite/sidecar");
        assert_eq!(
            sidecar_root(home, true, false),
            (new.clone(), Migrate::Nothing)
        );
        assert_eq!(
            sidecar_root(home, false, true),
            (new.clone(), Migrate::Move)
        );
        assert_eq!(
            sidecar_root(home, true, true),
            (new.clone(), Migrate::NewShadowsOld)
        );
        assert_eq!(sidecar_root(home, false, false), (new, Migrate::Nothing));
    }

    // 20.10.26i AC3: a rename that lost the race reads as "already migrated";
    // any other failure falls back.
    #[test]
    fn a_lost_rename_race_is_already_migrated() {
        let err = |kind| Err(io::Error::from(kind));
        assert_eq!(classify_rename(&Ok(())), MoveOutcome::Moved);
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::DirectoryNotEmpty,
            io::ErrorKind::AlreadyExists,
        ] {
            assert_eq!(classify_rename(&err(kind)), MoveOutcome::AlreadyMigrated);
        }
        let denied = classify_rename(&err(io::ErrorKind::PermissionDenied));
        assert!(matches!(denied, MoveOutcome::Failed(_)), "{denied:?}");
    }

    // 20.10.26i D4: a failed move falls back to the old store for this run;
    // a moved or already-migrated one uses the new.
    #[test]
    fn a_failed_move_falls_back_to_the_old_store() {
        let (old, new) = (PathBuf::from("/h/old"), PathBuf::from("/h/new"));
        let failed = MoveOutcome::Failed("denied".into());
        assert_eq!(store_after_move(failed, old.clone(), new.clone()), old);
        assert_eq!(
            store_after_move(MoveOutcome::Moved, old.clone(), new.clone()),
            new
        );
        assert_eq!(
            store_after_move(MoveOutcome::AlreadyMigrated, old, new.clone()),
            new
        );
    }

    // 20.10.26i AC3: the store moves exactly once, its contents with it.
    #[test]
    fn the_store_moves_exactly_once() {
        let home = tempfile::tempdir().unwrap();
        let old = home.path().join(".tungsten/sidecar/abc");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("data.mdb"), "history").unwrap();

        let root = resolve_sidecar_root(home.path());
        assert_eq!(root, home.path().join(".albite/sidecar"));
        assert_eq!(
            fs::read_to_string(root.join("abc/data.mdb")).unwrap(),
            "history"
        );
        assert!(!home.path().join(".tungsten/sidecar").exists());

        // A second use finds nothing to move and keeps the same root.
        assert_eq!(resolve_sidecar_root(home.path()), root);
        assert!(root.join("abc/data.mdb").exists());
    }

    // 20.10.26i AC3: with both, the new store wins and the old is left alone.
    #[test]
    fn the_new_store_wins_when_both_exist() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".tungsten/sidecar/old")).unwrap();
        fs::create_dir_all(home.path().join(".albite/sidecar/new")).unwrap();
        let root = resolve_sidecar_root(home.path());
        assert!(root.join("new").exists());
        assert!(home.path().join(".tungsten/sidecar/old").exists());
    }
}
