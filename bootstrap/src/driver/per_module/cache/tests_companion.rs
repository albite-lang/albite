//! An attached sibling bypasses every cache tier (ADR 2.10.26j D6, AC 7).
//!
//! Each test seeds a real entry and runs a control that hits (or stages), so a
//! refusal is the companion guard and not a miss.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{elab_cache, stage_module_cache_write, try_cache_hit, try_full_output_hit};
use super::{BuildCtx, CachedModuleFullOutput, ParsedModule, FULL_OUTPUT_SCHEMA_VERSION};
use crate::ast::{SourceFile, Visibility};
use crate::cache::elab_cache::CachedModuleSignature;
use crate::cache::{BuildCache, COMPILER_VERSION};
use crate::driver::modules::{CompanionSplit, ModuleInfo, SourceMap};
use crate::elaborate::{ElabMode, ElabOutput, ModuleExports};
use crate::span::Span;

const EXPORTS_HASH: [u8; 32] = [9u8; 32];

/// A module at `dir/m.tg` with both tiers' entries seeded under its key.
fn seeded(dir: &Path) -> (Mutex<BuildCache>, ParsedModule) {
    let path = dir.join("m.tg");
    let source = "fn main() -> Nat { 0 }\n";
    std::fs::write(&path, source).unwrap();
    let cache = BuildCache::new(dir, false).unwrap();
    let content_hash = elab_cache::hash_file_content(source.as_bytes());
    let key = elab_cache::compute_module_cache_key(COMPILER_VERSION, &content_hash, &EXPORTS_HASH);
    let signature = CachedModuleSignature {
        delta_exports: ModuleExports::default(),
        warnings: Vec::new(),
        def_count: 1,
        termination: Default::default(),
    };
    cache.put_module_elab(&key, &signature).unwrap();
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
    (Mutex::new(cache), module)
}

/// A build context whose source map attaches a companion to `attached`, if any.
fn build_ctx<'a>(cache: &'a Mutex<BuildCache>, attached: Option<&Path>) -> BuildCtx<'a> {
    let mut source_map = SourceMap::single(PathBuf::from("m.tg"), String::new());
    if let Some(sibling) = attached {
        source_map.companions.push(CompanionSplit {
            sibling: sibling.to_path_buf(),
            base: 24,
            companion: sibling.with_file_name("m_test.tg"),
        });
    }
    BuildCtx {
        cache: Some(cache),
        module_info: ModuleInfo::default(),
        source_map,
    }
}

/// Run `probe` against the module, with and without its companion attached.
fn with_and_without<T>(probe: impl Fn(&ParsedModule, &BuildCtx<'_>) -> T) -> (T, T) {
    let dir = tempfile::tempdir().unwrap();
    let (cache, module) = seeded(dir.path());
    let plain = probe(&module, &build_ctx(&cache, None));
    let attached = probe(&module, &build_ctx(&cache, Some(&module.path)));
    (plain, attached)
}

fn flags() -> crate::driver::per_module::ElabCtxFlags {
    crate::driver::per_module::ElabCtxFlags {
        verbose: false,
        profiling: false,
        full_output_cache: true,
        signature_hits: true,
    }
}

// ADR 2.10.26j AC7 (D6): the signature tier is never read for an attached sibling.
#[test]
fn the_signature_tier_skips_an_attached_sibling() {
    let (plain, attached) = with_and_without(|module, build| {
        try_cache_hit(module, &EXPORTS_HASH, build, ElabMode::Check, true).is_some()
    });
    assert!(plain, "the control hits");
    assert!(!attached);
}

// ADR 2.10.26j AC7 (D6): nor the full-output tier.
#[test]
fn the_full_output_tier_skips_an_attached_sibling() {
    let (plain, attached) = with_and_without(|module, build| {
        let trace = crate::driver::output::TraceOptions::default();
        let ctx =
            crate::driver::per_module::build_elab_ctx(flags(), build, &trace, EXPORTS_HASH, None);
        try_full_output_hit(module, &ctx).is_some()
    });
    assert!(plain, "the control hits");
    assert!(!attached);
}

// ADR 2.10.26j AC7 (D6): and nothing is staged for writing — the write-side
// check, whose absence would store the companion's names under the sibling's
// bytes.
#[test]
fn no_cache_write_is_staged_for_an_attached_sibling() {
    let (plain, attached) = with_and_without(|module, build| {
        let trace = crate::driver::output::TraceOptions::default();
        let ctx =
            crate::driver::per_module::build_elab_ctx(flags(), build, &trace, EXPORTS_HASH, None);
        let empty = ModuleExports::default();
        stage_module_cache_write(module, &ctx, &ElabOutput::default(), (&empty, &empty)).is_some()
    });
    assert!(plain, "the control stages a write");
    assert!(!attached);
}
