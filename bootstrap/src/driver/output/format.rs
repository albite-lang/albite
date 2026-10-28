//! Human-readable rendering of values and types (split from `driver/output.rs`
//! by ADR 4.10.26h, which added the `exists` arm and took that file past its cap).

use crate::config::MAX_TYPE_DISPLAY_DEPTH;
use albite_core::eval::term_to_nat;
use albite_core::{Term, Type};

use crate::driver::type_registry::{lookup_type_name, TYPE_PATTERN_REGISTRY};

/// Format a term as a human-readable value.
pub fn format_value(term: &Term) -> String {
    match term {
        Term::Zero => "0".to_string(),
        Term::NatLit(n) => n.to_string(),
        Term::IntLit(i) => i.to_string(),
        Term::Succ(n) => {
            if let Some(n) = term_to_nat(term) {
                n.to_string()
            } else {
                format!("succ({})", format_value(n))
            }
        }
        Term::True => "true".to_string(),
        Term::False => "false".to_string(),
        Term::Unit => "()".to_string(),
        Term::Pair(a, b) => format!("({}, {})", format_value(a), format_value(b)),
        Term::Inl(_, t) => format!("inl({})", format_value(t)),
        Term::Inr(_, t) => format!("inr({})", format_value(t)),
        Term::Lambda(_, _, _) | Term::Fix(_, _, _) => "<function>".to_string(),
        Term::TyAbs(_, _) => "<polymorphic function>".to_string(),
        Term::Refl(_, _) => "refl".to_string(),
        Term::Sorry => "sorry".to_string(),
        Term::StringLit(s) => format!("\"{}\"", s),
        Term::Fold(_, t) => format_value(t),
        _ => format!("{:?}", term),
    }
}

/// Format a type as a human-readable string.
pub fn format_type(ty: &Type) -> String {
    format_type_with_depth(ty, usize::MAX)
}

/// Format a type for display in error messages with depth-limited truncation.
///
/// Uses MAX_TYPE_DISPLAY_DEPTH to control how deeply nested types are expanded.
/// Types exceeding the depth limit are truncated with "..." to keep error
/// messages readable.
///
/// For non-parameterized user-defined types (ADTs, Records, Aliases), attempts
/// to show the original type name instead of the Core encoding.
pub fn format_type_for_display(ty: &Type) -> String {
    // First, try to find a user-defined type name for this encoding.
    // This handles non-parameterized types like `Color`, `Direction`, etc.
    if let Some(name) = lookup_type_name(ty) {
        return name;
    }

    format_type_with_depth(ty, MAX_TYPE_DISPLAY_DEPTH)
}

/// Format a type with explicit depth limit.
///
/// When depth reaches 0, nested structures are shown as "...".
fn format_type_with_depth(ty: &Type, depth: usize) -> String {
    if depth == 0 {
        return "...".to_string();
    }

    // Base types (no recursion needed)
    if let Some(s) = format_base_type(ty) {
        return s;
    }

    match ty {
        // Binary compound types
        Type::Arrow(a, b) => {
            let a_str = if matches!(**a, Type::Arrow(_, _) | Type::Pi(..)) {
                format!("({})", format_type_with_depth(a, depth - 1))
            } else {
                format_type_with_depth(a, depth - 1)
            };
            format!("{} -> {}", a_str, format_type_with_depth(b, depth - 1))
        }
        // A dependent function names its binder (ADR 30.9.26d)
        Type::Pi(x, a, b) => format!(
            "({x}: {}) -> {}",
            format_type_with_depth(a, depth - 1),
            format_type_with_depth(b, depth - 1)
        ),
        // A dependent pair reads as its source spelling (ADR 4.10.26h)
        Type::Sigma(x, a, b) => format!(
            "exists {x}: {}. {}",
            format_type_with_depth(a, depth - 1),
            format_type_with_depth(b, depth - 1)
        ),
        Type::Product(a, b) | Type::Sum(a, b) => {
            let op = if matches!(ty, Type::Product(..)) {
                " × "
            } else {
                " + "
            };
            format!(
                "({}{}{})",
                format_type_with_depth(a, depth - 1),
                op,
                format_type_with_depth(b, depth - 1)
            )
        }

        // Binding forms
        Type::Forall(name, body) => {
            format!("∀ {}. {}", name, format_type_with_depth(body, depth - 1))
        }
        Type::Mu(name, body) => format_mu_type(name, body, depth),

        // Parameterized wrappers
        Type::Ptr(inner) | Type::Ref(inner) => {
            let tag = if matches!(ty, Type::Ptr(_)) {
                "Ptr"
            } else {
                "Ref"
            };
            format!("{}<{}>", tag, format_type_with_depth(inner, depth - 1))
        }

        // Equality proofs
        Type::Eq(inner_ty, a, b) => format_eq_with_depth(inner_ty, a, b, depth),

        // Type applications / ADT — both use name<args> format
        Type::App(name, args) | Type::Adt(name, args, _) => {
            format_app_with_depth(name, args, depth)
        }

        // Predicate binders (ADR 6.10.26h): a kind reads as its source spelling
        Type::ForallPred(name, domains, body) => {
            let domains: Vec<String> = domains
                .iter()
                .map(|d| format_type_with_depth(d, depth - 1))
                .collect();
            format!(
                "∀ {name}: {} -> Prop. {}",
                domains.join(" -> "),
                format_type_with_depth(body, depth - 1)
            )
        }
        Type::PredApp(..) | Type::PredAbs(..) => ty.to_string(),

        // Base types already handled above
        _ => unreachable!(),
    }
}

/// Format base (leaf) types that require no recursion.
fn format_base_type(ty: &Type) -> Option<String> {
    if let Some(name) = ty.primitive_name() {
        return Some(name.to_string());
    }
    match ty {
        Type::Error => Some("<error>".to_string()),
        Type::TyVar(name) => Some(name.strip_prefix('@').unwrap_or(name).to_string()),
        _ => None,
    }
}

fn format_eq_with_depth(inner_ty: &Type, a: &Term, b: &Term, depth: usize) -> String {
    if depth <= 1 {
        format!("Eq<{}, ...>", format_type_with_depth(inner_ty, depth - 1))
    } else {
        format!(
            "Eq<{}, {:?}, {:?}>",
            format_type_with_depth(inner_ty, depth - 1),
            a,
            b
        )
    }
}

fn format_app_with_depth(name: &str, args: &[Type], depth: usize) -> String {
    if args.is_empty() {
        name.to_string()
    } else {
        let args_str: Vec<String> = args
            .iter()
            .map(|a| format_type_with_depth(a, depth - 1))
            .collect();
        format!("{}<{}>", name, args_str.join(", "))
    }
}

fn format_mu_type(name: &str, body: &Type, depth: usize) -> String {
    // Try to extract a readable type name from our encoding convention.
    // We use names like "α_List" or "α_Tree" for recursive types.
    if let Some(type_name) = name.strip_prefix("α_") {
        let has_params = TYPE_PATTERN_REGISTRY.with(|registry| {
            registry
                .borrow()
                .iter()
                .find(|p| p.name == type_name)
                .map(|p| !p.params.is_empty())
                .unwrap_or(false)
        });

        if has_params {
            format!("{}<...>", type_name)
        } else {
            type_name.to_string()
        }
    } else {
        format!("μ{}. {}", name, format_type_with_depth(body, depth - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use albite_core::types::Type;

    #[test]
    fn test_format_value_renders_nat_representations_as_numerals() {
        // Both Nat spellings must print as plain numerals (ADR 21.7.26e:
        // literals > 64 are NatLit, so run output would otherwise show
        // `NatLit(99)` — the golden regression that motivated this).
        assert_eq!(format_value(&Term::NatLit(99)), "99");
        // 14.9.26c: an `Int` result prints signed, not as its debug form.
        assert_eq!(format_value(&Term::IntLit(-42)), "-42");
        assert_eq!(format_value(&Term::nat(3)), "3");
        assert_eq!(format_value(&Term::Zero), "0");
        // A mixed chain (Succ over NatLit) can arise when evaluation wraps a
        // literal — it must still render as one numeral.
        assert_eq!(format_value(&Term::succ(Term::NatLit(9))), "10");
    }

    #[test]
    fn test_format_base_type_returns_some_for_base_types() {
        assert_eq!(format_base_type(&Type::Bool), Some("Bool".to_string()));
        assert_eq!(format_base_type(&Type::Nat), Some("Nat".to_string()));
        assert_eq!(format_base_type(&Type::Int), Some("Int".to_string()));
        assert_eq!(format_base_type(&Type::Unit), Some("Unit".to_string()));
        assert_eq!(format_base_type(&Type::String), Some("String".to_string()));
        assert_eq!(format_base_type(&Type::Error), Some("<error>".to_string()));
    }

    #[test]
    fn test_format_base_type_returns_none_for_compound_types() {
        assert_eq!(format_base_type(&Type::arrow(Type::Nat, Type::Bool)), None);
        assert_eq!(
            format_base_type(&Type::product(Type::Nat, Type::Bool)),
            None
        );
    }

    #[test]
    fn test_format_base_type_strips_at_prefix() {
        assert_eq!(
            format_base_type(&Type::TyVar("@T".to_string())),
            Some("T".to_string())
        );
        assert_eq!(
            format_base_type(&Type::TyVar("T".to_string())),
            Some("T".to_string())
        );
    }

    #[test]
    fn test_format_type_delegates_to_depth() {
        // format_type should produce the same output as format_type_with_depth(_, MAX)
        let ty = Type::arrow(Type::Nat, Type::Bool);
        assert_eq!(format_type(&ty), "Nat -> Bool");
    }

    #[test]
    fn test_format_type_arrow_precedence() {
        let ty = Type::arrow(Type::arrow(Type::Nat, Type::Bool), Type::Unit);
        assert_eq!(format_type(&ty), "(Nat -> Bool) -> Unit");
    }

    // 4.10.26h: a dependent pair names its binder, and is never shown as a product.
    #[test]
    fn test_format_type_names_a_sigma_binder() {
        let body = Type::eq(Type::Nat, Term::var("k"), Term::Zero);
        let ty = Type::sigma("k", Type::Nat, body);
        assert!(format_type(&ty).starts_with("exists k: Nat. Eq<Nat, "));
        assert!(!format_type(&ty).contains('×'));
    }

    /// ADR 6.10.26h: a predicate binder reads as its source spelling, and its
    /// domains and body elide one level below it, as an arrow's children do.
    #[test]
    fn a_predicate_binder_elides_below_its_own_depth() {
        let nested = || Type::arrow(Type::arrow(Type::Nat, Type::Nat), Type::Nat);
        let binder = Type::forall_pred("P", vec![nested()], nested());
        assert_eq!(
            format_type_for_display(&binder),
            "∀ P: (... -> ...) -> Nat -> Prop. (... -> ...) -> Nat"
        );
        let app = Type::pred_app("P", vec![Term::Zero]);
        assert_eq!(format_type(&app), app.to_string());
    }
}
