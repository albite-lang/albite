//! Region detection (ADR 9.10.26o D3).
//!
//! `contains_region` answers whether a term holds a `region { … }` anywhere,
//! lambda bodies included: a closure built inside a definition still opens
//! its region when it runs, so the program needs the scoped arena mode.

use crate::terms::Term;

impl Term {
    /// Whether `self` contains a `Term::Region` node at any depth.
    #[must_use]
    pub fn contains_region(&self) -> bool {
        if matches!(self, Term::Region(..)) {
            return true;
        }
        let mut found = false;
        self.for_each_subterm(|sub| {
            if !found {
                found = sub.contains_region();
            }
        });
        found
    }
}

#[cfg(test)]
mod tests {
    use crate::terms::Term;
    use crate::types::Type;

    // 9.10.26o AC6: contains_region finds a region at the root, under a
    // binder, and reports none on a region-free term.
    #[test]
    fn contains_region_finds_a_root_region() {
        assert!(Term::region(
            crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
            Term::nat_lit(1)
        )
        .contains_region());
    }

    #[test]
    fn contains_region_descends_through_binders_and_spines() {
        let nested = Term::lambda(
            "x",
            Type::Nat,
            Term::nat_add(
                Term::var("x"),
                Term::region(
                    crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                    Term::var("x"),
                ),
            ),
        );
        assert!(nested.contains_region());
    }

    #[test]
    fn contains_region_is_false_on_a_region_free_term() {
        let plain = Term::lambda(
            "x",
            Type::Nat,
            Term::nat_add(Term::var("x"), Term::early_return(Term::var("x"))),
        );
        assert!(!plain.contains_region());
        assert!(!Term::Unit.contains_region());
    }
}

#[cfg(test)]
mod semantics_tests {
    use crate::conversion::convertible_without_delta;
    use crate::eval::eval;
    use crate::terms::Term;

    // 9.10.26o D3: a region evaluates as its body, a `return` inside unwinds
    // through it, and conversion erases it.
    #[test]
    fn a_region_evaluates_and_converts_as_its_body() {
        let sum = Term::nat_add(Term::nat_lit(2), Term::nat_lit(3));
        assert_eq!(
            eval(&Term::region(
                crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                sum.clone()
            )),
            eval(&sum)
        );
        assert_eq!(
            eval(&Term::region(
                crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                Term::early_return(Term::nat_lit(4))
            )),
            Term::nat_lit(4)
        );
        assert!(convertible_without_delta(
            &Term::region(
                crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                sum.clone()
            ),
            &sum
        ));
        assert!(!convertible_without_delta(
            &Term::region(
                crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                sum
            ),
            &Term::nat_lit(6)
        ));
    }
}
