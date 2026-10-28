//! The type names a payload mentions, for `copy_layout`'s mutual-cluster
//! resolution (ADR 13.10.26q D4). A `TyVar` is read with its `@` and `α_`
//! prefixes stripped, so a marker and a still-collecting name both resolve
//! through the ADT table by the name the source wrote. Split from
//! `layout.rs` when formatting took it over the size cap.

use albite_core::types::Type;

/// Push every type name `ty` mentions onto `names`, in a left-to-right walk.
pub(super) fn collect_names(ty: &Type, names: &mut Vec<String>) {
    match ty {
        Type::TyVar(var) => {
            let var = var.strip_prefix('@').unwrap_or(var);
            names.push(var.strip_prefix("α_").unwrap_or(var).to_string());
        }
        Type::App(name, args) => {
            names.push(name.clone());
            for arg in args {
                collect_names(arg, names);
            }
        }
        Type::Product(a, b) | Type::Sum(a, b) | Type::Sigma(_, a, b) => {
            collect_names(a, names);
            collect_names(b, names);
        }
        Type::Mu(_, body) | Type::Forall(_, body) => collect_names(body, names),
        Type::Adt(_, args, variants) => {
            for arg in args {
                collect_names(arg, names);
            }
            for (_, payload) in variants {
                collect_names(payload, names);
            }
        }
        _ => {}
    }
}
