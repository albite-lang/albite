//! The cache directory's two names (ADR 20.10.26i D3): the user's
//! `config.toml` survives the rename, and either name is a cache.

use crate::cache::*;
use std::fs;
use std::path::Path;

// 20.10.26i AC4: the config path prefers the new directory, falls back to the
// old one (flagged), and is absent with neither.
#[test]
fn cache_config_path_prefers_new_then_legacy() {
    let root = Path::new("/p");
    let only = |present: &'static [&'static str]| {
        move |p: &Path| present.iter().any(|f| Path::new(f) == p)
    };
    let both = only(&["/p/.albite/config.toml", "/p/.tungsten/config.toml"]);
    assert_eq!(
        cache_config_path(root, both),
        Some((Path::new("/p/.albite/config.toml").to_path_buf(), false))
    );
    assert_eq!(
        cache_config_path(root, only(&["/p/.tungsten/config.toml"])),
        Some((Path::new("/p/.tungsten/config.toml").to_path_buf(), true))
    );
    assert_eq!(cache_config_path(root, only(&[])), None);
}

// 20.10.26i AC4: an old `.tungsten/config.toml` is read when the new
// directory has none, and the new one wins once it exists.
#[test]
fn the_legacy_cache_config_is_read_when_the_new_one_is_absent() {
    let project_dir = tempfile::TempDir::new().unwrap();
    let root = project_dir.path();
    fs::create_dir_all(root.join(".tungsten")).unwrap();
    fs::write(root.join(".tungsten/config.toml"), "max_size_mb = 321\n").unwrap();

    let cache = BuildCache::new(root, false).unwrap();
    assert_eq!(cache.stats().unwrap().max_size_mb, 321);
    assert!(
        root.join(".albite/cache").exists(),
        "the cache itself is new-named"
    );

    fs::write(root.join(".albite/config.toml"), "max_size_mb = 654\n").unwrap();
    let cache = BuildCache::new(root, false).unwrap();
    assert_eq!(cache.stats().unwrap().max_size_mb, 654);
}

// 20.10.26i D3: either name is a cache directory; nothing else is.
#[test]
fn both_names_are_cache_directories() {
    assert!(is_cache_dir_name(".albite"));
    assert!(is_cache_dir_name(".tungsten"));
    assert!(!is_cache_dir_name("albite"));
    assert!(!is_cache_dir_name("target"));
}
