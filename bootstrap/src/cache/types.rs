//! Data types and schema constants for the build cache.
//!
//! Contains both cache entry types (CacheEntry, CacheManifest, etc.) and
//! schema versioning constants (CACHE_FORMAT_VERSION, COMPILER_VERSION, etc.).
//! Merged from types.rs + schema.rs to reduce directory item count.

use super::graph::DependencyGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

// --- Schema constants (formerly schema.rs) ---

/// Current cache format version - bump on incompatible changes to manifest format.
/// v3: the crates and runtime symbols are renamed for Albite (ADR 20.10.26g D4).
pub const CACHE_FORMAT_VERSION: u32 = 3;

/// Current IR schema version - bump when CoreDef/Type/Term format changes.
/// v2: `Type::Int` + the `Int` `Term` variants (ADR 14.9.26c).
/// v3: `Type::Pi`, the dependent function type (ADR 30.9.26d D11).
/// v4: `Type::Sigma`, the dependent pair (ADR 4.10.26h D10).
/// v5: the `Prop` variant removed from the middle of `Type`, shifting every later variant, and
/// `TypeDefKind::Family` appended (ADR 4.10.26i D11).
/// v6: the compared hash folds in the collision set, so a cached `CoreDef`'s
/// name cannot outlive the clash that scoped it (ADR 4.10.26j D7).
/// v7: `Type::PredApp`, `Type::ForallPred` and `Type::PredAbs`, the predicate
/// binders (ADR 6.10.26h D10).
/// v8: the runtime's link names are `__albite_*` (ADR 20.10.26g D4).
pub const IR_SCHEMA_VERSION: u32 = 8;

/// Computed hash of AST struct definitions.
/// Auto-invalidates cache when AST types change (fields added/removed/reordered, type changes).
/// This is computed from AST_SCHEMA_SIGNATURE below.
pub const AST_SCHEMA_HASH: [u8; 32] = compute_schema_hash(AST_SCHEMA_SIGNATURE);

/// String representation of AST schema for hashing.
/// Update this when AST struct definitions change - the hash will auto-invalidate caches.
/// Format: "StructName{field:Type,...};..." for each serialized struct.
pub const AST_SCHEMA_SIGNATURE: &str = concat!(
    "SourceFile{items:Vec<Item>,span:Span};",
    "FunctionDef{visibility:Visibility,name:Ident,type_params:Vec<TypeParam>,params:Vec<Param>,return_type:Option<TypeExpr>,body:Expr,span:Span};",
    "TypeDef{visibility:Visibility,name:Ident,type_params:Vec<TypeParam>,body:TypeBody,span:Span};",
    "TypeAlias{visibility:Visibility,name:Ident,type_params:Vec<TypeParam>,value_params:Vec<Param>,sort:Option<SortAnnot>,aliased:TypeExpr,span:Span};SortAnnot{Prop,Type};",
    // The real fields (ADR 2.10.26c D11 corrected a stale `kind`/`Option<Expr>` row).
    "TheoremDef{visibility:Visibility,name:Ident,type_params:Vec<TypeParam>,params:Vec<Param>,prop:TypeExpr,body:TheoremBody,span:Span};",
    "TheoremBody{Term(Expr),Tactics(TacticBlock)};TacticBlock{tactics:Vec<Tactic>,span:Span};",
    "Tactic{Intro(Ident,Span),Exact(Expr,Span),Apply(Expr,Span),Refl(Span),Rewrite(Vec<RewriteRule>,Span),Induction(Ident,Span),Cases(Box<Expr>,Vec<CasesArm>,Span),Simp(Vec<RewriteRule>,Span),Ring(Span),Exists(Expr,Span),CasesPair(Box<Expr>,Option<Ident>,Option<Ident>,Vec<Tactic>,Span),Omega(Span),Left(Span),Right(Span)};",
    "CasesArm{ctor:Ident,binders:Vec<Option<Ident>>,body:Vec<Tactic>,span:Span};",
    "RewriteRule{reverse:bool,rule:Expr,span:Span};",
    "AxiomDef{visibility:Visibility,name:Ident,type_params:Vec<TypeParam>,params:Vec<Param>,prop:TypeExpr,span:Span};",
    "ExternFnDef{visibility:Visibility,name:Ident,type_params:Vec<TypeParam>,params:Vec<Param>,return_type:TypeExpr,span:Span};",
    // v4: `LiteralPattern::NegInt` appended (ADR 18.9.26e).
    // v5: a theorem's body is a `TheoremBody`, a term or a `by` block (ADR 2.10.26c D11).
    // v6: `Tactic::Rewrite` and `RewriteRule` (ADR 2.10.26e D11).
    // v7: `Tactic::Induction` (ADR 3.10.26e D10).
    // v8: `Tactic::Cases` and `CasesArm` (ADR 3.10.26f D7).
    // v9: `Tactic::Simp` (ADR 4.10.26e D10).
    // v10: `Tactic::Ring` (ADR 4.10.26g D9).
    // v11: `TypeExpr::Exists` (ADR 4.10.26h D10).
    // v12: `TypeAlias.value_params` and `TypeAlias.sort` (ADR 4.10.26i D11).
    // v13: `TypeExpr::Term`, a term in a type argument (ADR 6.10.26d D7).
    // v14: `Tactic::Exists` and `Tactic::CasesPair` (ADR 6.10.26e D9).
    // v15: `Tactic::Omega` (ADR 6.10.26f D11).
    // v16: `TypeParam.bounds` is filled — a kind, `<P: Nat -> Prop>` (ADR 6.10.26h D1).
    // v17: `Tactic::Left` and `Tactic::Right` (ADR 7.10.26l D7).
    // v18: `TypeExpr::Pi`, a dependent function type (ADR 7.10.26n D8).
    "v18"  // Bump this suffix when changing schema but not struct layouts
);

/// Compute hash of the schema signature at compile time.
pub const fn compute_schema_hash(signature: &str) -> [u8; 32] {
    // Simple compile-time hash using FNV-1a style mixing
    // (SHA-256 isn't const-friendly, so we use a simpler deterministic hash)
    let bytes = signature.as_bytes();
    let mut hash = [0u8; 32];
    let mut i = 0;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0100_0000_01b3); // FNV prime
        i += 1;
    }
    // Spread the 64-bit hash across 32 bytes for compatibility with existing code
    hash[0] = (h >> 56) as u8;
    hash[1] = (h >> 48) as u8;
    hash[2] = (h >> 40) as u8;
    hash[3] = (h >> 32) as u8;
    hash[4] = (h >> 24) as u8;
    hash[5] = (h >> 16) as u8;
    hash[6] = (h >> 8) as u8;
    hash[7] = h as u8;
    // Fill rest with hash variations
    let mut j = 8;
    while j < 32 {
        hash[j] = hash[j - 8] ^ ((j as u8).wrapping_mul(0x9e));
        j += 1;
    }
    hash
}

/// Default max cache size in MB.
pub const DEFAULT_MAX_SIZE_MB: u64 = 500;

/// Compiler version string for cache invalidation.
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- Cache data types ---

/// A cached entry for a single source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// SHA-256 hash of the source file content.
    pub content_hash: [u8; 32],
    /// File modification time (for fast-path checking).
    pub mtime: Duration,
    /// Last time this entry was accessed (for LRU eviction).
    pub last_accessed: Duration,
    /// Path to the cached AST file (relative to cache dir).
    pub ast_path: PathBuf,
    /// Path to the cached IR file (relative to cache dir), if available.
    #[serde(default)]
    pub ir_path: Option<PathBuf>,
    /// Hash of type definitions used during elaboration (for IR invalidation).
    #[serde(default)]
    pub types_hash: Option<[u8; 32]>,
    /// IR schema version when IR was cached.
    #[serde(default)]
    pub ir_schema_version: Option<u32>,
}

/// The cache manifest tracking all cached files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheManifest {
    /// Cache format version.
    pub version: u32,
    /// Compiler version that created this cache.
    pub compiler_version: String,
    /// AST schema hash - auto-invalidates cache when AST struct definitions change.
    /// Computed from AST_SCHEMA_SIGNATURE at compile time.
    #[serde(default)]
    pub ast_schema_hash: [u8; 32],
    /// Map from source file path (absolute) to cache entry.
    pub entries: HashMap<PathBuf, CacheEntry>,
    /// Module dependency graph (for invalidation).
    #[serde(default)]
    pub dependency_graph: DependencyGraph,
}

impl Default for CacheManifest {
    fn default() -> Self {
        Self {
            version: CACHE_FORMAT_VERSION,
            compiler_version: COMPILER_VERSION.to_string(),
            ast_schema_hash: AST_SCHEMA_HASH,
            entries: HashMap::new(),
            dependency_graph: DependencyGraph::new(),
        }
    }
}

/// Cache configuration loaded from `.albite/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Maximum cache size in megabytes.
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u64,
}

fn default_max_size_mb() -> u64 {
    DEFAULT_MAX_SIZE_MB
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size_mb: DEFAULT_MAX_SIZE_MB,
        }
    }
}

/// Cache statistics.
#[derive(Debug)]
pub struct CacheStats {
    /// Total size of cached files in bytes.
    pub size_bytes: u64,
    /// Number of cached entries.
    pub entry_count: usize,
    /// Maximum configured size in MB.
    pub max_size_mb: u64,
    /// Oldest entry's last access time.
    pub oldest_accessed: Option<Duration>,
    /// Newest entry's last access time.
    pub newest_accessed: Option<Duration>,
}

/// Statistics from a prune operation.
#[derive(Debug)]
pub struct PruneStats {
    /// Number of entries removed.
    pub removed_count: usize,
    /// Bytes freed.
    pub freed_bytes: u64,
    /// New cache size in bytes.
    pub new_size_bytes: u64,
}
