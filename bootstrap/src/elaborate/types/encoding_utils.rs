//! Type encoding utilities — query and comparison helpers.
//!
//! These functions are used by normalization, type comparison, and codegen
//! but are separable from the core ADT/record encoding pipeline in `encoding.rs`.

use crate::elaborate::env::Constructor;
use crate::elaborate::Elaborator;
use albite_core::Type;

impl<'a> Elaborator<'a> {
    /// Check if an ADT is recursive (directly or via mutual recursion).
    ///
    /// An ADT is recursive if:
    /// 1. Any constructor field directly references the ADT by name, OR
    /// 2. The ADT is a member of a mutual recursion group (detected by SCC
    ///    in Recursion Grouping). Mutual recursion means the type participates in a
    ///    cycle through other types (e.g., MaybeTypeExpr → TypeExpr → ... →
    ///    MaybeTypeExpr). Such types require fold/unfold for their Mu encoding.
    pub(crate) fn adt_is_recursive(&self, name: &str, constructors: &[Constructor]) -> bool {
        // Check mutual recursion group membership first (fast HashMap lookup)
        let result = if self.mutual_recursion_groups.contains_key(name) {
            true
        } else {
            // Fall back to direct self-reference check
            constructors
                .iter()
                .any(|ctor| ctor.fields.iter().any(|field| field.references_name(name)))
        };

        // Debug-mode consistency check (ADR 21.4.26c): every call for a given ADT name
        // must agree on recursiveness. If they disagree, a caller is passing wrong data.
        #[cfg(debug_assertions)]
        {
            let mut map = self.recursiveness_decisions.borrow_mut();
            if let Some(&prev) = map.get(name) {
                debug_assert_eq!(
                    prev, result,
                    "adt_is_recursive disagreement for '{}': was {}, now {}",
                    name, prev, result
                );
            } else {
                map.insert(name.to_string(), result);
            }
        }

        result
    }

    /// Encode a record type as a right-nested product type.
    ///
    /// `{ f1: T1, f2: T2, f3: T3 }` → `T1 × (T2 × T3)`
    ///
    /// Single-field records are encoded as just the field type.
    pub(crate) fn encode_record_type(&self, fields: &[(String, Type)]) -> Type {
        if fields.is_empty() {
            Type::Unit
        } else if fields.len() == 1 {
            fields[0].1.clone()
        } else {
            let mut iter = fields.iter().rev();
            let (_, last_ty) = iter.next().unwrap();
            let mut product = last_ty.clone();
            for (_, ty) in iter {
                product = Type::product(ty.clone(), product);
            }
            product
        }
    }

    /// Check if two types are equal, using normalization and α-equivalence.
    ///
    /// Deliberately NOT on the `children`/`map_children` discipline
    /// (ADR 23.7.26b): the underlying `types_equal_alpha` walks *two* types
    /// in lockstep, not a single tree.
    pub(crate) fn types_equal(&self, a: &Type, b: &Type) -> bool {
        let a_norm = self.normalize_for_comparison(a);
        let b_norm = self.normalize_for_comparison(b);
        albite_core::types_equal_alpha(&a_norm, &b_norm)
    }

    /// Check if two terms are definitionally equal at a given type (ADR 21.5.26d).
    ///
    /// Normalizes both sides using `albite_core::eval::eval` and compares
    /// structurally. This is scoped to the existing normalizer's capabilities —
    /// no new reduction rules or proof search.
    ///
    /// **This is the δ-free fast path (ADR 28.9.26d §2.1).**
    /// `albite_core::eval::eval` is the *environment-free* evaluator, in which
    /// `Term::Global` is Stuck, so nothing is unfolded here. It must stay that
    /// way: this runs during Body Elaboration, before termination admission has
    /// decided which constants are `Total` (§1.2), so an unfold here could not
    /// consult the admission check that has to sit beside δ. A failure δ could
    /// change is deferred by `convert_or_defer` instead, and discharged after
    /// admission with only admitted `Total`, effect-free bodies in scope
    /// (`elaborate::conversion`). Do **not** move the admission check to
    /// `EvalEnv::lookup`: that evaluator is `run`/`test` execution, where
    /// partial constants are legitimate.
    ///
    /// When `eval` `==` fails, the WHNF comparison of ADR 29.9.26c runs with an
    /// **empty** global map, so it gains ι and the `+` rule but still no δ, and
    /// needs no admission. Both sides are renamed apart first (D2), so neither
    /// evaluator can capture a free variable.
    ///
    /// The body is `albite_core::conversion::convertible_without_delta`,
    /// which the self-host's `tg_terms_convertible` also calls (ADR 30.9.26g D2).
    pub(crate) fn terms_definitionally_equal(
        &self,
        t1: &albite_core::Term,
        t2: &albite_core::Term,
        _ty: &Type,
    ) -> bool {
        albite_core::conversion::convertible_without_delta(t1, t2)
    }

    /// Encode ADT constructors to the canonical comparison shape (for
    /// normalization/pattern matching), without type-parameter substitution.
    ///
    /// Uses the SAME builders as the canonical stored encoder (ADR 21.7.26e
    /// wall 1): left-nested field products via `ctor_fields_product`, and the
    /// ADR 2.2.26 sum policy via `build_adt_sum_body` (2 ctors → `Sum`,
    /// 3+ → `Adt`). This was the third parallel encoder copy — its
    /// right-nested products silently failed `types_pattern_match` for
    /// ≥3-field constructors, and its right-nested `Sum` chains could
    /// false-match a 3+-constructor ADT against a nested sum type argument.
    pub(crate) fn encode_adt_constructors_to_sum(
        &self,
        adt_name: &str,
        params: &[String],
        constructors: &[Constructor],
    ) -> Type {
        let constructor_types: Vec<Type> = constructors
            .iter()
            .map(|ctor| super::encoding::ctor_fields_product(ctor.fields.clone()))
            .collect();
        let param_args: Vec<Type> = params
            .iter()
            .map(|param| Type::TyVar(param.clone()))
            .collect();
        super::encoding::build_adt_sum_body(constructor_types, constructors, adt_name, &param_args)
    }
}

#[cfg(test)]
mod tests {
    // `Type::references_name`'s tests moved to the core with the walker
    // (ADR 6.10.26k D6, `albite_core/src/types/tyvar_ops_tests.rs`).
    use albite_core::Type;

    /// ADR 6.10.26k AC5: the bootstrap's alias refusal still fires through the
    /// core walker, and a non-recursive alias still elaborates.
    #[test]
    fn a_self_referencing_alias_is_refused_through_the_core_walker() {
        let errors = crate::elaborate::tests::elab_err("type Foo = Foo\n");
        assert!(
            errors.iter().any(|e| e.kind.code() == "E0060"),
            "{errors:?}"
        );
        crate::elaborate::tests::elab_ok("type Wrapped = Nat\nfn f(w: Wrapped) -> Nat { w }\n");
    }

    /// ADR 28.9.26d D1: the check run *before admission exists* unfolds
    /// nothing. δ happens only at the post-admission discharge — see
    /// `elaborate::conversion::tests` — so this fast path must stay δ-free.
    ///
    /// Asserted in both polarities on purpose. The `false` case is the invariant
    /// (a global does not reduce to its body); the `true` case is what stops the
    /// test passing vacuously — if `terms_definitionally_equal` answered `false`
    /// for everything, the first assertion alone would still look like proof.
    ///
    /// 29.9.26c AC5: Phase 1 is unmoved — the three original assertions stand,
    /// and the fourth shows the fast path gained ι and the `+` rule, not δ.
    #[test]
    fn elaboration_time_conversion_does_not_unfold_a_global() {
        use albite_core::{Context, Term};

        let mut ctx = Context::new();
        let elab = crate::elaborate::Elaborator::new(&mut ctx);
        let global = Term::Global("two".to_string());
        let body = Term::NatLit(2);

        assert!(
            !elab.terms_definitionally_equal(&global, &body, &Type::Nat),
            "a global must stay Stuck through conversion, never δ-reduce to its body"
        );
        assert!(
            elab.terms_definitionally_equal(&global, &global.clone(), &Type::Nat),
            "the same global is still definitionally equal to itself"
        );
        let beta_redex = Term::app(
            Term::lambda("x", Type::Nat, Term::var("x")),
            Term::NatLit(2),
        );
        assert!(
            elab.terms_definitionally_equal(&beta_redex, &Term::NatLit(2), &Type::Nat),
            "reduction that does not need a global still happens — it is δ that is absent, not evaluation"
        );
        // ADR 29.9.26c §2.6: the fast path gained ι and D4's `+` rule, not δ.
        let k_plus_zero = Term::nat_add(Term::var("k"), Term::Zero);
        assert!(
            elab.terms_definitionally_equal(&k_plus_zero, &Term::var("k"), &Type::Nat),
            "`k + 0` reduces to `k` on its right operand, with no global involved"
        );
    }

    /// 30.9.26g AC3: the bootstrap's fast path IS the core function the
    /// self-host's FFI calls — one answer per pair, accepted or refused.
    #[test]
    fn the_fast_path_is_the_core_function() {
        use albite_core::conversion::convertible_without_delta;
        use albite_core::{Context, Term};

        let mut ctx = Context::new();
        let elab = crate::elaborate::Elaborator::new(&mut ctx);
        let k = || Term::var("k");
        let pairs = [
            (Term::nat_add(k(), Term::NatLit(0)), k(), true),
            (
                Term::nat_add(Term::NatLit(2), Term::NatLit(2)),
                Term::NatLit(4),
                true,
            ),
            (Term::nat_mul(k(), Term::NatLit(0)), Term::NatLit(0), true),
            (Term::nat_add(Term::NatLit(0), k()), k(), false),
            (Term::nat_mul(k(), Term::NatLit(1)), k(), false),
            (Term::app(Term::global("same"), k()), k(), false),
        ];
        for (left, right, expected) in pairs {
            let core = convertible_without_delta(&left, &right);
            assert_eq!(core, expected, "{left} ≡ {right}");
            assert_eq!(
                elab.terms_definitionally_equal(&left, &right, &Type::Nat),
                core,
                "{left} ≡ {right}"
            );
        }
    }
}
