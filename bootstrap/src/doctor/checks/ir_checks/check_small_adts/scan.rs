//! The per-module half of the small-ADT census (ADR 28.9.26f §2.1): a pure
//! text scan of one emitted `.ll` module into [`SmallAdtCounts`].
//!
//! "Blob" is the literal tagged-union type text `{ i32, [N x i8] }`, and `N` —
//! the payload bytes — keys every per-shape count. Every pattern here reads a
//! value name the codegen itself chooses (`sum_alloca`, `adt_alloca`,
//! `data_ptr`, `mu_alloc`, `mu_stack`, `indirect_buf`); a rename there empties
//! a category, which `--strict` reports as vacuity.

use std::collections::{BTreeMap, HashMap};

use crate::doctor::checks::ir_checks::textparse::{
    split_ir_functions, split_top_level_commas, IrFunc,
};

/// The five per-shape counts, for one payload size `N`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BlobShapeCounts {
    /// Blob-typed `define` parameters passed by value.
    pub by_value_params: u64,
    /// `define`s returning a blob by value.
    pub by_value_returns: u64,
    /// `define` parameters `sret({ i32, [N x i8] })` — a return through memory.
    pub sret_returns: u64,
    /// `%indirect_buf.<k> = alloca <blob>` — a parameter passed through memory.
    pub indirect_params: u64,
    /// Payload stores through a `%data_ptr<k>` into a blob under construction.
    pub constructions: u64,
}

impl BlobShapeCounts {
    fn add(&mut self, other: &BlobShapeCounts) {
        self.by_value_params += other.by_value_params;
        self.by_value_returns += other.by_value_returns;
        self.sret_returns += other.sret_returns;
        self.indirect_params += other.indirect_params;
        self.constructions += other.constructions;
    }

    /// Memory-buffer parameters: `sret` returns plus indirect params.
    fn buffer_sites(&self) -> u64 {
        self.sret_returns + self.indirect_params
    }
}

/// What the census counted — per module, or summed over a corpus.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SmallAdtCounts {
    /// `define`s seen (corpus size, not a category).
    pub defines: u64,
    /// Per-`N` counts, keyed by payload bytes.
    pub shapes: BTreeMap<u64, BlobShapeCounts>,
    /// Constructions storing a bare `ptr` into an `N = 8` blob — an upper
    /// bound on null-niche candidates.
    pub pointer_payload_constructions: u64,
    /// `@__albite_alloc(i64 <S>, i32 0)` calls: one per heap `fold` site.
    pub mu_alloc_sites: u64,
    /// `%mu_stack<k> = alloca <blob>`: folds escape analysis kept on the stack.
    pub mu_stack_sites: u64,
    /// `mu_alloc` sites whose stored value has an empty payload.
    pub nullary_mu_alloc_sites: u64,
    /// `%sum_alloca<k>` / `%adt_alloca<k> = alloca <blob>`.
    pub construction_allocas: u64,
    /// Pointer-fallback warnings in the emit log; `None` without `--emit-log`.
    pub pointer_fallbacks: Option<u64>,
}

/// A blob's size in bytes: the `i32` tag plus `N` payload bytes, rounded up
/// to the tag's 4-byte alignment.
pub fn blob_bytes(payload_bytes: u64) -> u64 {
    (4 + payload_bytes).div_ceil(4) * 4
}

impl SmallAdtCounts {
    /// Add another module's counts. `pointer_fallbacks` is a corpus-level
    /// figure read from the emit log, so it is not summed here.
    pub fn add(&mut self, other: &SmallAdtCounts) {
        self.defines += other.defines;
        for (payload_bytes, shape) in &other.shapes {
            self.shapes.entry(*payload_bytes).or_default().add(shape);
        }
        self.pointer_payload_constructions += other.pointer_payload_constructions;
        self.mu_alloc_sites += other.mu_alloc_sites;
        self.mu_stack_sites += other.mu_stack_sites;
        self.nullary_mu_alloc_sites += other.nullary_mu_alloc_sites;
        self.construction_allocas += other.construction_allocas;
    }

    /// True when any category counted anything — the reach pair's `tracked`.
    pub fn counted_anything(&self) -> bool {
        !self.shapes.is_empty()
            || self.mu_alloc_sites > 0
            || self.mu_stack_sites > 0
            || self.construction_allocas > 0
    }

    fn sum_where(
        &self,
        keep: impl Fn(u64) -> bool,
        field: impl Fn(&BlobShapeCounts) -> u64,
    ) -> u64 {
        self.shapes
            .iter()
            .filter(|(payload_bytes, _)| keep(blob_bytes(**payload_bytes)))
            .map(|(_, shape)| field(shape))
            .sum()
    }

    /// Σ `sret_returns` + Σ `indirect_params`.
    pub fn blob_buffer_sites(&self) -> u64 {
        self.sum_where(|_| true, BlobShapeCounts::buffer_sites)
    }

    /// Buffer sites whose blob fits one tagged word (≤ 8 bytes).
    pub fn small_blob_buffer_sites(&self) -> u64 {
        self.sum_where(|bytes| bytes <= 8, BlobShapeCounts::buffer_sites)
    }

    /// Buffer sites whose blob fits two registers (≤ 16 bytes). Informational.
    pub fn small16_blob_buffer_sites(&self) -> u64 {
        self.sum_where(|bytes| bytes <= 16, BlobShapeCounts::buffer_sites)
    }

    /// By-value params + returns of ≤ 8 bytes — already in registers.
    pub fn small_blob_by_value_sites(&self) -> u64 {
        self.sum_where(
            |bytes| bytes <= 8,
            |shape| shape.by_value_params + shape.by_value_returns,
        )
    }

    /// Σ `constructions`.
    pub fn blob_constructions(&self) -> u64 {
        self.sum_where(|_| true, |shape| shape.constructions)
    }

    fn shape(&mut self, payload_bytes: u64) -> &mut BlobShapeCounts {
        self.shapes.entry(payload_bytes).or_default()
    }
}

const BLOB_OPEN: &str = "{ i32, [";
const BLOB_CLOSE: &str = " x i8] }";

/// Parse a blob type at the start of `text`: its payload bytes and the rest.
pub fn parse_blob_prefix(text: &str) -> Option<(u64, &str)> {
    let rest = text.strip_prefix(BLOB_OPEN)?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let payload_bytes = rest[..digits].parse().ok()?;
    let rest = rest[digits..].strip_prefix(BLOB_CLOSE)?;
    Some((payload_bytes, rest))
}

/// The payload bytes when `text` is exactly one blob type (whitespace-trimmed).
fn parse_exact_blob(text: &str) -> Option<u64> {
    match parse_blob_prefix(text.trim()) {
        Some((payload_bytes, "")) => Some(payload_bytes),
        _ => None,
    }
}

/// Count the pointer-fallback warnings in a captured emit log (§2.1).
pub fn count_fallbacks(log: &str) -> u64 {
    log.lines()
        .filter(|line| {
            (line.contains("TyVar(") && line.contains("reached lower_type"))
                || (line.contains("Type::App(") && line.contains("encountered in codegen"))
                || line.contains("lower_type recursion depth")
        })
        .count() as u64
}

/// Scan one `.ll` module. Pure: no filesystem.
pub fn scan_module(text: &str) -> SmallAdtCounts {
    let mut counts = SmallAdtCounts::default();
    for func in split_ir_functions(text) {
        counts.defines += 1;
        scan_header(&func, &mut counts);
        FunctionFacts::collect(&func, &mut counts).count_nullary_folds(&mut counts);
    }
    counts
}

/// Count the `define` header's by-value blob return, by-value blob params and
/// `sret(<blob>)` params. `declare`s never reach here.
fn scan_header(func: &IrFunc<'_>, counts: &mut SmallAdtCounts) {
    let Some((before_name, params)) = split_header(func.header) else {
        return;
    };
    if let Some(start) = before_name.rfind(BLOB_OPEN) {
        if let Some(payload_bytes) = parse_exact_blob(&before_name[start..]) {
            counts.shape(payload_bytes).by_value_returns += 1;
        }
    }
    for param in split_top_level_commas(params) {
        let param = param.trim();
        if let Some(at) = param.find("sret(") {
            if let Some((payload_bytes, rest)) = parse_blob_prefix(&param[at + "sret(".len()..]) {
                if rest.starts_with(')') {
                    counts.shape(payload_bytes).sret_returns += 1;
                }
            }
        } else if let Some((payload_bytes, _)) = parse_blob_prefix(param) {
            counts.shape(payload_bytes).by_value_params += 1;
        }
    }
}

/// Split a `define` header into the text before the function name and the
/// parameter list between its parentheses.
fn split_header(header: &str) -> Option<(&str, &str)> {
    let at = header.find('@')?;
    let name_start = at + 1;
    // A quoted name may itself hold a `(`, so its parameter list opens exactly
    // one past the closing quote; a bare name ends at the first `(`.
    let open = match header[name_start..].strip_prefix('"') {
        Some(quoted) => name_start + 1 + quoted.find('"')? + 1,
        None => name_start + header[name_start..].find('(')?,
    };
    if !header[open..].starts_with('(') {
        return None;
    }
    let mut depth = 0i32;
    for (offset, c) in header[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&header[..at], &header[open + 1..open + offset]));
                }
            }
            _ => {}
        }
    }
    None
}

/// The value a construction alloca's payload slot received.
enum PayloadStore<'a> {
    /// `store {} zeroinitializer` — an empty constructor.
    Empty,
    /// A blob-typed SSA value: empty only if it was itself loaded from an
    /// alloca whose payload is empty (the nested binary-`Sum` shape).
    Blob(&'a str),
    /// Anything else.
    Other,
}

/// Per-function SSA facts for the nullary-fold rule. Tracked by SSA name,
/// never by proximity: a `{}` store on the line before a fold proves nothing.
#[derive(Default)]
struct FunctionFacts<'a> {
    /// `%data_ptr<k>` → (the blob alloca it points into, `N`).
    payload_slots: HashMap<&'a str, (&'a str, u64)>,
    /// Blob alloca → what its payload slot received.
    payloads: HashMap<&'a str, PayloadStore<'a>>,
    /// Blob SSA value → the alloca it was loaded from.
    loaded_from: HashMap<&'a str, &'a str>,
    /// Values stored into a `%mu_alloc<k>` heap cell.
    folded_values: Vec<&'a str>,
}

impl<'a> FunctionFacts<'a> {
    fn collect(func: &IrFunc<'a>, counts: &mut SmallAdtCounts) -> Self {
        let mut facts = Self::default();
        for line in &func.body {
            let line = line.trim();
            if line.contains("@__albite_alloc(i64 ") && line.ends_with(", i32 0)") {
                counts.mu_alloc_sites += 1;
            } else if let Some((name, rhs)) = split_assignment(line) {
                facts.note_definition(name, rhs, counts);
            } else if let Some(store) = line.strip_prefix("store ") {
                facts.note_store(store, counts);
            }
        }
        facts
    }

    fn note_definition(&mut self, name: &'a str, rhs: &'a str, counts: &mut SmallAdtCounts) {
        if let Some(allocated) = rhs.strip_prefix("alloca ") {
            let Some((payload_bytes, _)) = parse_blob_prefix(allocated) else {
                return;
            };
            if name.starts_with("%indirect_buf.") {
                counts.shape(payload_bytes).indirect_params += 1;
            } else if name.starts_with("%mu_stack") {
                counts.mu_stack_sites += 1;
            } else if name.starts_with("%sum_alloca") || name.starts_with("%adt_alloca") {
                counts.construction_allocas += 1;
            }
        } else if let Some(gep) = rhs.strip_prefix("getelementptr inbounds ") {
            if !name.starts_with("%data_ptr") {
                return;
            }
            let Some((payload_bytes, rest)) = parse_blob_prefix(gep) else {
                return;
            };
            let Some(base) = rest
                .strip_prefix(", ptr ")
                .and_then(|r| r.strip_suffix(", i32 0, i32 1"))
            else {
                return;
            };
            self.payload_slots.insert(name, (base, payload_bytes));
        } else if let Some(loaded) = rhs.strip_prefix("load ") {
            let Some((_, rest)) = parse_blob_prefix(loaded) else {
                return;
            };
            if let Some(source) = rest
                .strip_prefix(", ptr ")
                .and_then(|r| r.split(',').next())
            {
                self.loaded_from.insert(name, source);
            }
        }
    }

    /// `store <T> <value>, ptr <dest>, align N` — the text after `store `.
    fn note_store(&mut self, store: &'a str, counts: &mut SmallAdtCounts) {
        let Some((typed_value, dest)) = store.split_once(", ptr ") else {
            return;
        };
        let dest = dest.split(',').next().unwrap_or("").trim();
        let Some((value_type, value)) = typed_value.rsplit_once(' ') else {
            return;
        };
        if let Some(&(alloca, payload_bytes)) = self.payload_slots.get(dest) {
            counts.shape(payload_bytes).constructions += 1;
            if payload_bytes == 8 && value_type == "ptr" {
                counts.pointer_payload_constructions += 1;
            }
            let payload = if value_type == "{}" {
                PayloadStore::Empty
            } else if parse_exact_blob(value_type).is_some() {
                PayloadStore::Blob(value)
            } else {
                PayloadStore::Other
            };
            self.payloads.insert(alloca, payload);
        } else if is_mu_alloc_cell(dest) && parse_exact_blob(value_type).is_some() {
            self.folded_values.push(value);
        }
    }

    fn count_nullary_folds(&self, counts: &mut SmallAdtCounts) {
        let nullary = self
            .folded_values
            .iter()
            .filter(|folded| self.value_is_empty(folded, 0))
            .count();
        counts.nullary_mu_alloc_sites += nullary as u64;
    }

    /// A value has an empty payload when it was loaded from an alloca whose
    /// payload store was `{}`, or — transitively — another such value. The
    /// depth bound only guards a malformed cycle; real nesting is shallow.
    fn value_is_empty(&self, blob_ssa_name: &str, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        let Some(alloca) = self.loaded_from.get(blob_ssa_name) else {
            return false;
        };
        match self.payloads.get(alloca) {
            Some(PayloadStore::Empty) => true,
            Some(PayloadStore::Blob(inner)) => self.value_is_empty(inner, depth + 1),
            _ => false,
        }
    }
}

/// `%name = <rhs>` → (`%name`, `<rhs>`).
fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let (name, rhs) = line.split_once(" = ")?;
    name.starts_with('%').then_some((name, rhs))
}

/// `%mu_alloc` or `%mu_alloc<digits>` — the fold's heap cell, whichever of the
/// two emitted shapes produced it (the profiled call, or the arena `phi`).
fn is_mu_alloc_cell(dest: &str) -> bool {
    dest.strip_prefix("%mu_alloc")
        .is_some_and(|suffix| suffix.chars().all(|c| c.is_ascii_digit()))
}
