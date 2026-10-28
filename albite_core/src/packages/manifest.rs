//! `albite.toml`: finding it (D1) and reading slice 1's schema (D2). The old
//! name, `tungsten.toml`, is still read this release, with a note (ADR
//! 20.10.26i D1).
//!
//! Both halves are pure — the lookup takes an existence probe, the parser a
//! string and the reserved words — so the rules are testable without a
//! filesystem (ADR 29.9.26g). The reserved words are the caller's: the
//! bootstrap passes its lexer's list, the self-host its own (ADR 6.10.26j D2).

use std::path::{Path, PathBuf};

use super::{ByteRange, PlanNote, PlanProblem, PlanProblemKind};

/// The manifest's file name.
pub const MANIFEST_FILE: &str = "albite.toml";

/// The manifest's name before the rename, read until 2.1 (ADR 20.10.26i D1).
pub const LEGACY_MANIFEST_FILE: &str = "tungsten.toml";

/// Which of the two names a directory's manifest is read under (ADR 20.10.26i D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestChoice {
    /// Only `albite.toml`.
    New,
    /// Only `tungsten.toml`: read, with a note to rename it.
    Legacy,
    /// Both: `albite.toml` is read, with a note that the old file is ignored.
    NewShadowsLegacy,
    /// Neither.
    None,
}

/// D1's decision for one directory, from which of the two files exist.
#[must_use]
pub fn choose_manifest(albite_exists: bool, tungsten_exists: bool) -> ManifestChoice {
    match (albite_exists, tungsten_exists) {
        (true, false) => ManifestChoice::New,
        (true, true) => ManifestChoice::NewShadowsLegacy,
        (false, true) => ManifestChoice::Legacy,
        (false, false) => ManifestChoice::None,
    }
}

/// The manifest `dir` holds, and the note its choice carries, if any.
#[must_use]
pub fn manifest_in(
    dir: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<(PathBuf, Option<PlanNote>)> {
    let (new, legacy) = (dir.join(MANIFEST_FILE), dir.join(LEGACY_MANIFEST_FILE));
    match choose_manifest(exists(&new), exists(&legacy)) {
        ManifestChoice::New => Some((new, None)),
        ManifestChoice::Legacy => {
            let note = PlanNote::legacy_name(&legacy);
            Some((legacy, Some(note)))
        }
        ManifestChoice::NewShadowsLegacy => Some((new, Some(PlanNote::shadowed(&legacy)))),
        ManifestChoice::None => None,
    }
}

/// A library's root module, beside its manifest (D3). Not `mod.tg`, which
/// sibling discovery would register a second time.
pub const LIBRARY_ROOT_FILE: &str = "lib.tg";

/// The note every Phase 2 shape carries.
const PHASE_2_NOTE: &str = "path dependencies only until Phase 2";

/// A parsed, validated manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Where it was read from — the file its diagnostics span into.
    pub path: PathBuf,
    /// Its text, for the source map.
    pub text: String,
    /// `[package] name`, a module identifier.
    pub name: String,
    /// `[package] version`: stored, never interpreted.
    pub version: Option<String>,
    /// `[dependencies]`, in key order: `toml::Table` is a `BTreeMap` without
    /// the `preserve_order` feature, so the graph walk is alphabetical per
    /// manifest (ADR 3.10.26m P0.1).
    pub dependencies: Vec<Dependency>,
}

/// One `name = { path = "…" }` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// The key — the name the library is mounted under.
    pub key: String,
    /// The `path` value, relative to the manifest.
    pub path: String,
    /// Where the key is written.
    pub key_span: ByteRange,
}

/// The manifest governing `entry` (D1): the manifest in the nearest
/// ancestor, starting at the entry's own directory, that holds a manifest or
/// a `main.tg`. A `main.tg` without a manifest stops the walk: a manifest
/// above a project root is never read. Each level takes either name, by
/// [`choose_manifest`] (ADR 20.10.26i D1).
#[must_use]
pub fn find_manifest(entry: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut dir = entry.parent()?;
    loop {
        if let Some((manifest, _)) = manifest_in(dir, &exists) {
            return Some(manifest);
        }
        if exists(&dir.join("main.tg")) {
            return None;
        }
        dir = dir.parent()?;
    }
}

/// Parse and validate a manifest's text (D2). Every refusal is an E0120
/// spanned into `path`, one per cause, all reported together. A name in
/// `reserved` is refused as a package or dependency name.
///
/// # Errors
/// Every refusal the text earns, each located in `path`.
pub fn parse_manifest(
    text: &str,
    path: &Path,
    reserved: &[&str],
) -> Result<Manifest, Vec<PlanProblem>> {
    let invalid = |span: ByteRange, reason: String| {
        PlanProblem::new(PlanProblemKind::ManifestInvalid { reason }, path, span)
    };
    let table: toml::Table = toml::from_str(text).map_err(|e| {
        let span = e.span().map_or(ByteRange::new(0, 0), |r| {
            ByteRange::new(r.start as u32, r.end.max(r.start + 1) as u32)
        });
        vec![invalid(
            span,
            format!("TOML syntax error: {}", e.message().trim()),
        )]
    })?;

    let mut errors = Vec::new();
    for key in table
        .keys()
        .filter(|k| *k != "package" && *k != "dependencies")
    {
        let span = header_span(text, key).unwrap_or_else(|| key_span(text, None, key));
        errors.push(invalid(span, format!("unknown table or key `{key}`")));
    }
    let package = read_package(text, &table, reserved, &mut errors, &invalid);
    let dependencies = read_dependencies(text, &table, reserved, &mut errors, &invalid);
    match package {
        Some((name, version)) if errors.is_empty() => Ok(Manifest {
            path: path.to_path_buf(),
            text: text.to_string(),
            name,
            version,
            dependencies,
        }),
        _ => Err(errors),
    }
}

/// `[package]`: `name` required and an identifier, `version` an optional string.
fn read_package(
    text: &str,
    table: &toml::Table,
    reserved: &[&str],
    errors: &mut Vec<PlanProblem>,
    invalid: &impl Fn(ByteRange, String) -> PlanProblem,
) -> Option<(String, Option<String>)> {
    let Some(package) = table.get("package").and_then(toml::Value::as_table) else {
        let span = header_span(text, "package").unwrap_or(ByteRange::new(0, 0));
        errors.push(invalid(span, "missing `[package]` table".to_string()));
        return None;
    };
    for key in package.keys().filter(|k| *k != "name" && *k != "version") {
        let span = key_span(text, Some("package"), key);
        errors.push(invalid(span, format!("unknown key `{key}` in `[package]`")));
    }
    let version = package
        .get("version")
        .and_then(|v| v.as_str().map(str::to_string));
    if package.get("version").is_some() && version.is_none() {
        let span = key_span(text, Some("package"), "version");
        errors.push(invalid(span, "`version` must be a string".to_string()));
    }
    let header = header_span(text, "package").unwrap_or(ByteRange::new(0, 0));
    let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
        errors.push(invalid(header, "missing `[package] name`".to_string()));
        return None;
    };
    if let Some(problem) = identifier_problem(name, reserved) {
        let span = key_span(text, Some("package"), "name");
        errors.push(invalid(span, format!("package name `{name}` {problem}")));
        return None;
    }
    Some((name.to_string(), version))
}

/// `[dependencies]`: only `name = { path = "…" }`; every Phase 2 shape refused.
fn read_dependencies(
    text: &str,
    table: &toml::Table,
    reserved: &[&str],
    errors: &mut Vec<PlanProblem>,
    invalid: &impl Fn(ByteRange, String) -> PlanProblem,
) -> Vec<Dependency> {
    let Some(value) = table.get("dependencies") else {
        return Vec::new();
    };
    let Some(deps) = value.as_table() else {
        let span = key_span(text, None, "dependencies");
        errors.push(invalid(span, "`dependencies` must be a table".to_string()));
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, spec) in deps {
        let key_span = key_span(text, Some("dependencies"), key);
        match dependency_problem(key, spec, reserved) {
            Some(reason) => errors.push(invalid(key_span, reason)),
            None => out.push(Dependency {
                key: key.clone(),
                path: spec["path"].as_str().unwrap_or_default().to_string(),
                key_span,
            }),
        }
    }
    out
}

/// Why a dependency entry is refused, or `None` for a well-formed one.
fn dependency_problem(key: &str, spec: &toml::Value, reserved: &[&str]) -> Option<String> {
    if let Some(problem) = identifier_problem(key, reserved) {
        return Some(format!("dependency name `{key}` {problem}"));
    }
    let Some(fields) = spec.as_table() else {
        return Some(format!(
            "`{key}` is not `{{ path = \"…\" }}`: {PHASE_2_NOTE}"
        ));
    };
    if let Some(field) = ["git", "version", "registry"]
        .iter()
        .find(|f| fields.contains_key(**f))
    {
        return Some(format!("`{field}` in dependency `{key}`: {PHASE_2_NOTE}"));
    }
    if let Some(field) = fields.keys().find(|f| *f != "path") {
        return Some(format!("unknown key `{field}` in dependency `{key}`"));
    }
    match fields.get("path").map(toml::Value::as_str) {
        Some(Some(_)) => None,
        Some(None) => Some(format!("`path` of dependency `{key}` must be a string")),
        None => Some(format!("dependency `{key}` has no `path`")),
    }
}

/// Why `name` cannot be a module path segment, or `None` if it can: it must
/// be an identifier and none of the caller's `reserved` words.
#[must_use]
pub fn identifier_problem(name: &str, reserved: &[&str]) -> Option<&'static str> {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    if !first_ok || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Some("is not a module identifier (`[a-z_][a-z0-9_]*`)");
    }
    if reserved.contains(&name) {
        return Some("is a reserved word");
    }
    None
}

/// The span of a `[name]` table header line, if the text has one.
fn header_span(text: &str, name: &str) -> Option<ByteRange> {
    let header = format!("[{name}]");
    lines_with_offsets(text)
        .find(|(_, line)| line.trim() == header)
        .map(|(start, line)| trimmed_span(start, line))
}

/// The span of `key` as written — under `[table]` when given, else anywhere —
/// falling back to the table header, then to the file's start. Located by
/// text because `toml::Table` keeps no spans; a key's line is `key = …`.
fn key_span(text: &str, table: Option<&str>, key: &str) -> ByteRange {
    let header = table.map(|t| format!("[{t}]"));
    let mut in_table = header.is_none();
    for (start, line) in lines_with_offsets(text) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_table = header.as_deref().is_none_or(|h| trimmed.trim_end() == h);
            continue;
        }
        let rest = trimmed.strip_prefix(key).map(str::trim_start);
        if in_table && rest.is_some_and(|r| r.starts_with('=')) {
            let offset = start + (line.len() - trimmed.len());
            return ByteRange::new(offset as u32, (offset + key.len()) as u32);
        }
    }
    table
        .and_then(|t| header_span(text, t))
        .unwrap_or(ByteRange::new(0, 0))
}

/// Each line with its byte offset.
fn lines_with_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_inclusive('\n').scan(0usize, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line.trim_end_matches(['\n', '\r'])))
    })
}

/// A line's span without its surrounding whitespace.
fn trimmed_span(start: usize, line: &str) -> ByteRange {
    let lead = line.len() - line.trim_start().len();
    let s = start + lead;
    ByteRange::new(s as u32, (s + line.trim().len()) as u32)
}
