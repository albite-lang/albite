//! Diagnostic helpers for the compile command (dump-ir, dump-encoding, sorry check).

use std::fmt::Write as _;

use albite_bootstrap::driver::AdtTypes;
use albite_bootstrap::elaborate::TypeProvenance;

/// Pretty-print Core IR for matching definitions (--dump-ir).
pub(super) fn dump_core_ir(
    pattern: &str,
    defs: &[albite_bootstrap::elaborate::CoreDef],
    type_provenance: &TypeProvenance,
) {
    let names: Vec<&str> = pattern.split(',').map(str::trim).collect();
    let mut found = false;
    for def in defs {
        if names.iter().any(|n| *n == def.name || *n == "*") {
            found = true;
            let structural = format!("{}", def.ty);
            let semantic = format_semantic_type(&def.ty, type_provenance);
            eprintln!("┌─────────────────────────────────────────────────────────────┐");
            eprintln!("│  Definition: {:<47}│", def.name);
            if let Some(ref sem) = semantic {
                eprintln!("│  Type: {sem:<53}│");
                eprintln!("│        = {structural:<51}│");
            } else {
                eprintln!("│  Type: {structural:<53}│");
            }
            eprintln!("│{:61}│", "");
            eprintln!("│  Term: {:<53}│", format!("{}", def.term));
            let free = def.term.free_type_vars();
            if free.is_empty() {
                eprintln!("│  Free TyVars: ∅{:45}│", "");
            } else {
                eprintln!(
                    "│  Free TyVars: {:?}{:width$}│",
                    free,
                    "",
                    width = 45usize.saturating_sub(format!("{free:?}").len())
                );
            }
            eprintln!("└─────────────────────────────────────────────────────────────┘");
            eprintln!();
        }
    }
    if !found {
        eprintln!("[dump-ir] No definitions matched pattern: {pattern}");
    }
}

/// Print encoding breakdown for a named ADT (ADR 13.4.26c §4a).
pub(super) fn dump_adt_encoding(
    adt_name: &str,
    adt_types: &AdtTypes,
    type_provenance: &TypeProvenance,
) {
    let Some((params, constructors)) = adt_types.get(adt_name) else {
        eprintln!("[dump-encoding] ADT not found: {adt_name}");
        eprintln!("Available ADTs: {}", {
            let mut names: Vec<&str> = adt_types.keys().map(std::string::String::as_str).collect();
            names.sort_unstable();
            names.join(", ")
        });
        return;
    };

    let is_recursive = type_provenance
        .mu_origins
        .values()
        .any(|o| o.adt_name == adt_name);
    let mu_var = format!("α_{adt_name}");

    let type_params = if params.is_empty() {
        adt_name.to_string()
    } else {
        format!("{}<{}>", adt_name, params.join(", "))
    };

    let ctor_count = constructors.len();
    let strategy = match ctor_count {
        0 => "Void (0 constructors)".to_string(),
        1 => "single constructor (unwrapped)".to_string(),
        2 => "binary Sum (2 constructors)".to_string(),
        n => format!("flat Adt ({n} constructors)"),
    };

    eprintln!("┌──────────────────────────────────────────────────────────────────┐");
    eprintln!("│  ADT: {type_params:<60}│");
    eprintln!(
        "│  Recursive: {:<54}│",
        if is_recursive { "yes" } else { "no" }
    );
    eprintln!("│  Constructors: {ctor_count:<51}│");
    eprintln!("│{:66}│", "");
    eprintln!("│  Encoding Strategy: {strategy:<46}│");
    if is_recursive {
        eprintln!("│  Mu binder: {mu_var:<54}│");
    }
    eprintln!("│{:66}│", "");

    for (i, ctor) in constructors.iter().enumerate() {
        let fields_desc = if ctor.fields.is_empty() {
            "(none)".to_string()
        } else {
            ctor.fields
                .iter()
                .map(|f| format!("{f}"))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let encoded = if ctor.fields.is_empty() {
            "Unit".to_string()
        } else if ctor.fields.len() == 1 {
            format!("{}", ctor.fields[0])
        } else {
            let mut s = String::new();
            for (j, f) in ctor.fields.iter().enumerate() {
                if j > 0 {
                    s.push_str(" × ");
                }
                let _ = write!(s, "{f}");
            }
            s
        };

        eprintln!("│  Constructor {}: {:<52}│", i, ctor.name);
        eprintln!("│    Fields: {fields_desc:<43}→ {encoded:<10}│");
        if i < ctor_count - 1 {
            eprintln!("│{:66}│", "");
        }
    }

    eprintln!("└──────────────────────────────────────────────────────────────────┘");
}

/// Format a type using semantic ADT names from provenance where available (ADR 13.4.26c §3).
fn format_semantic_type(
    ty: &albite_core::types::Type,
    provenance: &TypeProvenance,
) -> Option<String> {
    use albite_core::types::Type;

    fn fmt(ty: &Type, provenance: &TypeProvenance) -> (String, bool) {
        match ty {
            Type::Mu(binder, _body) => {
                if let Some(origin) = provenance.mu_origins.get(binder) {
                    let sem = if origin.type_args.is_empty() {
                        origin.adt_name.clone()
                    } else {
                        let args: Vec<String> = origin
                            .type_args
                            .iter()
                            .map(|a| {
                                let (s, _) = fmt(a, provenance);
                                s
                            })
                            .collect();
                        format!("{}<{}>", origin.adt_name, args.join(", "))
                    };
                    (sem, true)
                } else {
                    (format!("{ty}"), false)
                }
            }
            Type::Arrow(t1, t2) => {
                let (s1, h1) = fmt(t1, provenance);
                let (s2, h2) = fmt(t2, provenance);
                let left = if matches!(t1.as_ref(), Type::Arrow(_, _)) {
                    format!("({s1})")
                } else {
                    s1
                };
                (format!("{left} -> {s2}"), h1 || h2)
            }
            Type::Product(t1, t2) => {
                let (s1, h1) = fmt(t1, provenance);
                let (s2, h2) = fmt(t2, provenance);
                (format!("({s1} × {s2})"), h1 || h2)
            }
            Type::Sum(t1, t2) => {
                let (s1, h1) = fmt(t1, provenance);
                let (s2, h2) = fmt(t2, provenance);
                (format!("({s1} + {s2})"), h1 || h2)
            }
            _ => (format!("{ty}"), false),
        }
    }

    let (result, has_semantic) = fmt(ty, provenance);
    if has_semantic {
        Some(result)
    } else {
        None
    }
}
