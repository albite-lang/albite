//! The mode guard on `try_cache_hit` (ADR 4.7.26c). The signature-only
//! cache carries no bodies, so only `Check` may consume it; `Run`/`Test`
//! (which lower to `Compile`/`Test`) must skip it and re-elaborate. Moved out
//! of `mod.rs` by ADR 3.10.26m, whose reach fingerprint took that file to its
//! function and fan-out limits.

use super::{elab_cache, try_cache_hit, try_full_output_hit, BuildCtx, ParsedModule};
use super::{CachedModuleFullOutput, FULL_OUTPUT_SCHEMA_VERSION};
use crate::ast::{SourceFile, Visibility};
use crate::cache::elab_cache::CachedModuleSignature;
use crate::cache::BuildCache;
use crate::cache::COMPILER_VERSION;
use crate::driver::modules::{ModuleInfo, SourceMap};
use crate::elaborate::{ElabMode, ElabOutput, ModuleExports};
use crate::span::Span;
use std::sync::Mutex;

/// Populate a signature-cache entry for a real on-disk module and return the
/// pieces `try_cache_hit` needs: the cache, module, and exports hash.
fn seed_signature_entry(dir: &std::path::Path) -> (Mutex<BuildCache>, ParsedModule, [u8; 32]) {
    seed_signature_entry_for(dir, "fn main() -> Nat { 0 }\n")
}

/// [`seed_signature_entry`] over `source`, whose parsed items the module
/// carries, as the walk's `ParsedModule` does.
fn seed_signature_entry_for(
    dir: &std::path::Path,
    source: &str,
) -> (Mutex<BuildCache>, ParsedModule, [u8; 32]) {
    let path = dir.join("m.tg");
    let content = source.as_bytes();
    std::fs::write(&path, content).unwrap();

    let cache = BuildCache::new(dir, false).unwrap();
    let exports_hash = [7u8; 32];
    let content_hash = elab_cache::hash_file_content(content);
    let key = elab_cache::compute_module_cache_key(COMPILER_VERSION, &content_hash, &exports_hash);
    let signature = CachedModuleSignature {
        delta_exports: ModuleExports::default(),
        warnings: Vec::new(),
        def_count: 1,
        termination: Default::default(),
    };
    cache.put_module_elab(&key, &signature).unwrap();
    // The full-output tier's entry under the same key (ADR 12.5.26a).
    let full_key = elab_cache::compute_full_output_cache_key(&key, FULL_OUTPUT_SCHEMA_VERSION);
    let empty = ModuleExports::default();
    let full = CachedModuleFullOutput::from_output(&ElabOutput::default(), &empty, &empty);
    cache.put_module_full_output(&full_key, &full).unwrap();

    let module = ParsedModule {
        path,
        visibility: Visibility::Private,
        source_file: SourceFile {
            items: crate::parse(source).0.items,
            span: Span::default(),
        },
        submodules: Vec::new(),
        mount_name: None,
    };
    (Mutex::new(cache), module, exports_hash)
}

fn build_ctx(cache: &Mutex<BuildCache>) -> BuildCtx<'_> {
    BuildCtx {
        cache: Some(cache),
        module_info: ModuleInfo::default(),
        source_map: SourceMap::single(std::path::PathBuf::from("m.tg"), String::new()),
    }
}

#[test]
fn check_mode_consumes_signature_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, module, exports_hash) = seed_signature_entry(dir.path());
    let build = build_ctx(&cache);
    // Check needs no bodies → the signature hit is sound and taken.
    assert!(
        try_cache_hit(&module, &exports_hash, &build, ElabMode::Check, true).is_some(),
        "check mode must still hit the signature cache"
    );
}

/// ADR 28.9.26d AC 5: the cold retry (`signature_hits: false`) never
/// consults the signature tier, even in Check mode with a matching entry.
#[test]
fn signature_hits_off_skips_the_signature_tier() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, module, exports_hash) = seed_signature_entry(dir.path());
    let build = build_ctx(&cache);
    assert!(
        try_cache_hit(&module, &exports_hash, &build, ElabMode::Check, false).is_none(),
        "the cold retry must re-elaborate every module"
    );
}

#[test]
fn body_needing_modes_skip_signature_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, module, exports_hash) = seed_signature_entry(dir.path());
    let build = build_ctx(&cache);
    // Compile (run) and Test would inherit an empty def list from a bodyless
    // hit → they must skip the read even though a matching entry exists.
    for mode in [ElabMode::Compile, ElabMode::Test] {
        assert!(
            try_cache_hit(&module, &exports_hash, &build, mode, true).is_none(),
            "{mode:?} must not consume the bodyless signature cache",
        );
    }
}

const THEOREM: &str = "theorem t(): 2 == 2 { refl }\n";

// ADR 2.10.26d AC5 (D8): an entry written before that ADR, under a key
// that did not change, is never served for a theorem-bearing module.
#[test]
fn try_cache_hit_refuses_a_theorem_bearing_module() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, module, exports_hash) = seed_signature_entry_for(dir.path(), THEOREM);
    let build = build_ctx(&cache);
    assert!(try_cache_hit(&module, &exports_hash, &build, ElabMode::Check, true).is_none());
    let lemma = "lemma l(): 2 == 2 { refl }\n";
    let (cache, module, exports_hash) = seed_signature_entry_for(dir.path(), lemma);
    let build = build_ctx(&cache);
    assert!(try_cache_hit(&module, &exports_hash, &build, ElabMode::Check, true).is_none());
}

// ADR 2.10.26d AC5 (D8): the full-output tier too — with a theorem-free
// control that does hit, so the refusal is the guard and not a miss.
#[test]
fn try_full_output_hit_refuses_a_theorem_bearing_module() {
    let hit_for = |source: &str| {
        let dir = tempfile::tempdir().unwrap();
        let (cache, module, exports_hash) = seed_signature_entry_for(dir.path(), source);
        let build = build_ctx(&cache);
        let trace = crate::driver::output::TraceOptions::default();
        let flags = crate::driver::per_module::ElabCtxFlags {
            verbose: false,
            profiling: false,
            full_output_cache: true,
            signature_hits: true,
        };
        let ctx =
            crate::driver::per_module::build_elab_ctx(flags, &build, &trace, exports_hash, None);
        try_full_output_hit(&module, &ctx).is_some()
    };
    assert!(hit_for("fn main() -> Nat { 0 }\n"), "the control hits");
    assert!(!hit_for(THEOREM), "a theorem-bearing module never does");
}
