//! Build cache module.
//!
//! This module provides caching for parsed ASTs and elaborated IR to speed up
//! incremental compilation. It includes:
//!
//! - Content-based cache invalidation (SHA-256 hashes)
//! - Schema versioning for safe cache upgrades
//! - Dependency graphs for cascade invalidation
//! - LRU eviction when the cache exceeds its size limit
//!
//! # Usage
//!
//! ```ignore
//! use tungsten::cache::BuildCache;
//!
//! let mut cache = BuildCache::new(project_root, verbose)?;
//!
//! // Try to get cached AST
//! if let Some(ast) = cache.get(&path, &content) {
//!     // Cache hit - use cached AST
//! } else {
//!     // Cache miss - parse and cache
//!     let ast = parse(&content)?;
//!     cache.put(&path, &content, &ast)?;
//! }
//! ```

mod build;
pub mod elab_cache;
mod graph;
mod ir_cache;
mod management;
mod types;

pub use build::BuildCache;
pub use elab_cache::{
    compute_full_output_cache_key, compute_module_cache_key, hash_exports, hash_file_content,
    CachedModuleFullOutput, CachedModuleSignature, ElabCacheStats, FULL_OUTPUT_SCHEMA_VERSION,
};
pub use graph::DependencyGraph;
pub use ir_cache::collision_keyed_hash;
pub use types::{
    compute_schema_hash, CacheConfig, CacheEntry, CacheManifest, CacheStats, PruneStats,
    AST_SCHEMA_HASH, AST_SCHEMA_SIGNATURE, CACHE_FORMAT_VERSION, COMPILER_VERSION,
    DEFAULT_MAX_SIZE_MB, IR_SCHEMA_VERSION,
};

use std::path::{Path, PathBuf};

/// A project's cache directory, beside its entry file (ADR 20.10.26i D3).
pub const CACHE_DIR: &str = ".albite";

/// The cache directory's name before the rename. Never read as a cache — its
/// contents are rebuilt — except for the user-authored `config.toml`, and
/// removed by `cache clean` beside the new one until 2.1.
pub const LEGACY_CACHE_DIR: &str = ".tungsten";

/// Whether a directory named `name` is a project cache, under either name.
#[must_use]
pub fn is_cache_dir_name(name: &str) -> bool {
    name == CACHE_DIR || name == LEGACY_CACHE_DIR
}

/// The `config.toml` a project's cache reads (D3): `.albite/config.toml`,
/// else `.tungsten/config.toml` — `true` when it is the old one, which earns
/// a note — else none.
#[must_use]
pub fn cache_config_path(
    project_root: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<(PathBuf, bool)> {
    let new = project_root.join(CACHE_DIR).join("config.toml");
    let legacy = project_root.join(LEGACY_CACHE_DIR).join("config.toml");
    if exists(&new) {
        Some((new, false))
    } else if exists(&legacy) {
        Some((legacy, true))
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
