//! Statement edges: a name cited in a definition's TYPE is a use (ADR 6.10.26r).
//!
//! A theorem's statement is its type, so a helper cited only there —
//! `f(n) == n`, or a family argument `Le<f(n), n>` — has no occurrence in any
//! term. [`OccurrenceGraph`] walks terms only, and it stays that way: it is the
//! termination checker's graph, and a type edge there would change which
//! definitions it groups as mutually recursive. So the union happens here, at
//! the diagnostic layer, and `--callers` and `audit-dead-definitions` both
//! build their adjacency through [`adjacency_with_statements`] and nothing else.
//!
//! The type half is [`globals_in_type`] — the set the `#[partial]` taint rule
//! already treats as a dependency — so a statement cannot count as a use for
//! taint and as no use here.

use albite_core::terms::termination::{globals_in_type, Adjacency, OccurrenceGraph};

use crate::elaborate::CoreDef;

/// Union each definition's statement names into its adjacency row.
///
/// `statement_names` maps a definition to the globals its type mentions. Only
/// names that are themselves rows of `adjacency` become edges — an extern or a
/// builtin is not a node, exactly as [`OccurrenceGraph::build`] treats a term
/// occurrence — and a definition never gains an edge to itself: a theorem
/// whose type names it is not thereby self-recursive.
#[must_use]
pub(crate) fn with_statement_edges(
    mut adjacency: Adjacency,
    statement_names: &Adjacency,
) -> Adjacency {
    for (name, cited) in statement_names {
        let edges: Vec<String> = cited
            .iter()
            .filter(|target| *target != name && adjacency.contains_key(*target))
            .cloned()
            .collect();
        if let Some(row) = adjacency.get_mut(name) {
            row.extend(edges);
        }
    }
    adjacency
}

/// The adjacency both reports read: term occurrences plus statement citations.
#[must_use]
pub fn adjacency_with_statements(defs: &[CoreDef]) -> Adjacency {
    let term_adjacency =
        OccurrenceGraph::build(defs.iter().map(|def| (def.name.as_str(), &def.term.term)))
            .adjacency();
    let statement_names: Adjacency = defs
        .iter()
        .map(|def| (def.name.clone(), globals_in_type(&def.ty)))
        .collect();
    with_statement_edges(term_adjacency, &statement_names)
}

/// ADR 6.10.26r § 1's fixture pair: `f` is cited only in theorem `t`'s
/// statement. `def.rs`'s `--callers` test spells the same pair inline: it is
/// compiled into the binary, where this `cfg(test)` constant does not exist.
#[cfg(test)]
pub(crate) const STATEMENT_ONLY_EQUATION: &str =
    "fn f(n: Nat) -> Nat { n }\npub theorem t(n: Nat, h: f(n) == n) -> f(n) == n { h }\n";
#[cfg(test)]
pub(crate) const STATEMENT_ONLY_FAMILY_ARGUMENT: &str =
    "pub type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n\
     fn f(n: Nat) -> Nat { n }\n\
     pub theorem t(n: Nat, h: Le<f(n), n>) -> Le<f(n), n> { h }\n";

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use albite_core::terms::Term;
    use albite_core::Type;

    use super::*;

    fn set(of: &[&str]) -> BTreeSet<String> {
        of.iter().map(|n| (*n).to_string()).collect()
    }

    fn rows(of: &[(&str, &[&str])]) -> Adjacency {
        of.iter()
            .map(|(name, targets)| ((*name).to_string(), set(targets)))
            .collect()
    }

    /// ADR 6.10.26r P0: the core already answers the type half for the `==`
    /// shape, so the helper has something to union.
    #[test]
    fn globals_in_type_reads_a_call_inside_an_equation() {
        let statement = Type::Eq(
            Box::new(Type::Nat),
            Box::new(Term::App(
                Box::new(Term::Global("f".to_string())),
                Box::new(Term::Var("n".to_string())),
            )),
            Box::new(Term::Var("n".to_string())),
        );
        assert_eq!(globals_in_type(&statement), set(&["f"]));
    }

    // 6.10.26r AC1
    #[test]
    fn a_name_cited_only_in_a_statement_becomes_an_edge() {
        let terms = rows(&[("f", &[]), ("t", &[])]);
        let unioned = with_statement_edges(terms, &rows(&[("t", &["f"])]));
        assert_eq!(unioned["t"], set(&["f"]));
        assert_eq!(unioned["f"], set(&[]), "only the citing row grows");
    }

    // 6.10.26r AC1
    #[test]
    fn a_statement_that_cites_nothing_leaves_its_row_unchanged() {
        let terms = rows(&[("f", &[]), ("g", &["f"])]);
        let unioned = with_statement_edges(terms.clone(), &rows(&[("g", &[]), ("f", &[])]));
        assert_eq!(unioned, terms);
    }

    // 6.10.26r AC1
    #[test]
    fn a_statement_that_names_its_own_definition_adds_no_self_edge() {
        let terms = rows(&[("t", &[]), ("f", &[])]);
        let unioned = with_statement_edges(terms, &rows(&[("t", &["t", "f"])]));
        assert_eq!(unioned["t"], set(&["f"]));
    }

    #[test]
    fn a_cited_name_that_is_no_definition_is_not_an_edge() {
        // An extern or builtin in a statement is not a node, as in a term.
        let terms = rows(&[("t", &[])]);
        let unioned = with_statement_edges(terms, &rows(&[("t", &["tg_extern"])]));
        assert_eq!(unioned["t"], set(&[]));
    }

    #[test]
    fn statement_edges_add_to_term_edges_rather_than_replacing_them() {
        let terms = rows(&[("f", &[]), ("g", &[]), ("t", &["g"])]);
        let unioned = with_statement_edges(terms, &rows(&[("t", &["f"])]));
        assert_eq!(unioned["t"], set(&["f", "g"]));
    }

    #[test]
    fn a_statement_row_for_an_unknown_definition_adds_no_row() {
        let terms = rows(&[("f", &[])]);
        let unioned = with_statement_edges(terms.clone(), &rows(&[("ghost", &["f"])]));
        assert_eq!(unioned, terms);
    }
}
