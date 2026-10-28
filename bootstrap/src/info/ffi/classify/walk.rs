//! The walk: each crate parsed whole, from `lib.rs` through its own `mod`
//! declarations (ADR 5.10.26f D4).
//!
//! The recogniser finds each export's file; this parses the crate that file
//! sits in, so a helper the export calls is in the index wherever it lives.
//! Following `mod` declarations rather than listing a directory skips every
//! `#[cfg(test)]` module, so a test helper is never mistaken for production
//! reach. This is the classifier's only effect.

use std::path::{Path, PathBuf};

use super::extract::{cfg_of, is_test_only, SourceFile};

/// Parse a crate whole, from `<root>/lib.rs` through its `mod` declarations.
pub(crate) fn parse_crate(root: &Path) -> Result<Vec<SourceFile>, String> {
    let mut parsed = Vec::new();
    let mut pending = vec![(root.join("lib.rs"), Vec::<String>::new(), None)];
    while let Some((path, module, cfg)) = pending.pop() {
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("reading {}: {error}", path.display()))?;
        let file = path.display().to_string();
        let ast = syn::parse_file(&text).map_err(|error| format!("parsing {file}: {error}"))?;
        for child in declared_children(&ast.items, &path, &module, cfg.as_ref())? {
            pending.push(child);
        }
        parsed.push(SourceFile {
            file,
            module,
            cfg,
            ast,
        });
    }
    Ok(parsed)
}

type PendingFile = (PathBuf, Vec<String>, Option<String>);

/// The files a source file's `mod name;` declarations name, test modules
/// skipped. Inline modules are searched for nested declarations too.
fn declared_children(
    items: &[syn::Item],
    path: &Path,
    module: &[String],
    cfg: Option<&String>,
) -> Result<Vec<PendingFile>, String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let is_dir_owner = matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("lib.rs" | "mod.rs")
    );
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let dir = if is_dir_owner {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };
    children_in(items, &dir, parent, module, cfg)
}

/// `declared_children` once the directory a `mod name;` resolves under is known.
fn children_in(
    items: &[syn::Item],
    dir: &Path,
    parent: &Path,
    module: &[String],
    cfg: Option<&String>,
) -> Result<Vec<PendingFile>, String> {
    let mut children = Vec::new();
    for item in items {
        let syn::Item::Mod(declared) = item else {
            continue;
        };
        if is_test_only(&declared.attrs) {
            continue;
        }
        let name = declared.ident.to_string();
        let mut child_module = module.to_vec();
        child_module.push(name.clone());
        let child_cfg = cfg_of(&declared.attrs).or_else(|| cfg.cloned());
        if let Some((_, inner)) = &declared.content {
            let nested = dir.join(&name);
            let cfg = child_cfg.as_ref();
            children.extend(children_in(inner, &nested, parent, &child_module, cfg)?);
        } else {
            let file = child_file(dir, &name, &declared.attrs, parent)?;
            children.push((file, child_module, child_cfg));
        }
    }
    Ok(children)
}

fn child_file(
    dir: &Path,
    name: &str,
    attrs: &[syn::Attribute],
    parent: &Path,
) -> Result<PathBuf, String> {
    if let Some(explicit) = path_attr(attrs) {
        return Ok(parent.join(explicit));
    }
    let flat = dir.join(format!("{name}.rs"));
    if flat.is_file() {
        return Ok(flat);
    }
    let nested = dir.join(name).join("mod.rs");
    if nested.is_file() {
        return Ok(nested);
    }
    Err(format!(
        "module `{name}` has no file under {}",
        dir.display()
    ))
}

fn path_attr(attrs: &[syn::Attribute]) -> Option<String> {
    attrs
        .iter()
        .find(|attr| attr.path().is_ident("path"))
        .and_then(|attr| {
            let value = &attr.meta.require_name_value().ok()?.value;
            match value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                }) => Some(text.value()),
                _ => None,
            }
        })
}
