//! The self-hosted compiler's error-code **ranges**, as data (ADR 18.8.26b D4).
//!
//! Its scheme is range-based where the bootstrap's is flat, so a new gate does
//! not pick a next free number — it claims a block. That makes the ranges the
//! thing worth asserting: a block that overlapped another would silently give
//! two unrelated failures the same code, and nothing downstream would notice,
//! because every consumer looks a code up rather than classifying it.
//!
//! E0700–E0799 is the soundness block, subdivided by gate: **positivity holds
//! E0700–E0709, termination E0710–E0719** (ADR 19.8.26d D3), **affine
//! handles E0720–E0729** (ADR 7.10.26t D7), **regions E0730–E0739** (ADR
//! 9.10.26o D4) **and mutable bindings E0740–E0749** (ADR 13.10.26r D4),
//! leaving E0750–E0799 reserved for later soundness gates. The
//! subdivision is a
//! convention rather than a checked range — what *is* checked is that no code
//! outside a catalogued entry resolves, so a later gate finds its number free.

/// One contiguous block of the self-hosted numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CodeRange {
    pub(super) start: u32,
    pub(super) end: u32,
    pub(super) name: &'static str,
}

impl CodeRange {
    pub(super) const fn contains(&self, code: u32) -> bool {
        self.start <= code && code <= self.end
    }

    /// Whether two blocks share any code at all.
    pub(super) const fn overlaps(&self, other: &CodeRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// Every declared block, mirroring the header comment of
/// `src/compiler/elab/error/kinds/mod.tg`.
pub(super) const RANGES: &[CodeRange] = &[
    CodeRange {
        start: 1,
        end: 99,
        name: "Type errors",
    },
    // Holds the package codes, E0120-E0124, which keep the bootstrap's own
    // numbers (ADR 6.10.26j D4) and list under their own category.
    CodeRange {
        start: 100,
        end: 199,
        name: "Name resolution",
    },
    CodeRange {
        start: 200,
        end: 299,
        name: "Items",
    },
    CodeRange {
        start: 300,
        end: 399,
        name: "Patterns",
    },
    CodeRange {
        start: 400,
        end: 499,
        name: "Proofs",
    },
    CodeRange {
        start: 500,
        end: 599,
        name: "References",
    },
    CodeRange {
        start: 600,
        end: 699,
        name: "Entry point / control flow",
    },
    CodeRange {
        start: 700,
        end: 799,
        name: "Soundness gates",
    },
    CodeRange {
        start: 800,
        end: 899,
        name: "Syntax",
    },
    CodeRange {
        start: 900,
        end: 999,
        name: "Other / internal",
    },
];

/// The soundness block, and the half of it positivity claims.
pub(super) const SOUNDNESS: CodeRange = CodeRange {
    start: 700,
    end: 799,
    name: "Soundness gates",
};
pub(super) const POSITIVITY: CodeRange = CodeRange {
    start: 700,
    end: 709,
    name: "Strict positivity",
};
/// The block affine handles carve from the soundness reserve (ADR 7.10.26t D7).
pub(super) const HANDLES: CodeRange = CodeRange {
    start: 720,
    end: 729,
    name: "Affine handles",
};
/// The block regions carve from the soundness reserve (ADR 9.10.26o D4).
pub(super) const REGIONS: CodeRange = CodeRange {
    start: 730,
    end: 739,
    name: "Regions",
};

/// The block mutable bindings carve from the soundness reserve (ADR 13.10.26r D4).
pub(super) const MUTABLE_BINDINGS: CodeRange = CodeRange {
    start: 740,
    end: 749,
    name: "Mutable bindings",
};

/// Parse an `Ennnn` code to its number. `None` on anything else.
pub(super) fn code_number(code: &str) -> Option<u32> {
    code.strip_prefix('E')?.parse().ok()
}

/// The block `code` falls in, if any.
pub(super) fn range_of(code: u32) -> Option<&'static CodeRange> {
    RANGES.iter().find(|range| range.contains(code))
}

/// The catalogue the range table describes. Imported at module level rather
/// than inside `tests` so the path stays one `super` deep.
use super::entries::SELF_HOSTED_ERRORS;

#[cfg(test)]
mod tests {
    use super::*;

    /// AC7. The **whole block** must be disjoint from every other range, not
    /// merely resolvable at one code: a partial overlap would leave the
    /// successor's E0710 colliding with something already spoken for, and the
    /// collision would only surface when that successor was written.
    #[test]
    fn the_soundness_block_is_disjoint_from_every_other_range() {
        for range in RANGES {
            if range == &SOUNDNESS {
                continue;
            }
            assert!(
                !SOUNDNESS.overlaps(range),
                "E0700-E0799 overlaps `{}` (E{:04}-E{:04})",
                range.name,
                range.start,
                range.end
            );
        }
    }

    /// The same property for every pair, so a future block cannot be added
    /// overlapping one that is not the soundness one.
    #[test]
    fn no_two_ranges_overlap() {
        for (index, left) in RANGES.iter().enumerate() {
            for right in &RANGES[index + 1..] {
                assert!(
                    !left.overlaps(right),
                    "`{}` overlaps `{}`",
                    left.name,
                    right.name
                );
            }
        }
    }

    /// Positivity's half is inside the soundness block and leaves E0710 free
    /// for the termination mirror (D4).
    #[test]
    fn positivity_claims_the_first_ten_and_leaves_e0710_free() {
        assert!(SOUNDNESS.contains(POSITIVITY.start) && SOUNDNESS.contains(POSITIVITY.end));
        assert_eq!(
            POSITIVITY.end + 1,
            710,
            "the termination mirror starts here"
        );
        assert!(SOUNDNESS.contains(710));
    }

    /// Every catalogued code lands in exactly one block. This is what makes
    /// the ranges a *description* of the catalogue rather than a parallel
    /// table that can drift from it.
    #[test]
    fn every_catalogued_code_falls_in_exactly_one_range() {
        for entry in SELF_HOSTED_ERRORS.iter() {
            let number = code_number(entry.code)
                .unwrap_or_else(|| panic!("`{}` is not an Ennnn code", entry.code));
            let matches = RANGES.iter().filter(|range| range.contains(number)).count();
            assert_eq!(matches, 1, "`{}` falls in {matches} range(s)", entry.code);
        }
    }

    /// The mirror's own code is catalogued, in the block it claims — the
    /// failure this would have caught is a code that elaborates and explains
    /// as "unknown".
    #[test]
    fn the_positivity_code_is_catalogued_in_its_own_range() {
        let entry = SELF_HOSTED_ERRORS
            .iter()
            .find(|entry| entry.code == "E0700")
            .expect("E0700 is catalogued");
        assert_eq!(entry.name, "ErrNonStrictlyPositive");
        assert!(POSITIVITY.contains(code_number(entry.code).expect("Ennnn")));
        assert_eq!(
            range_of(700).map(|range| range.name),
            Some("Soundness gates")
        );
    }

    /// 7.10.26t AC5: the handle block sits inside the soundness reserve, past
    /// termination's, and both handle codes are catalogued in it, each naming
    /// its bootstrap counterpart.
    #[test]
    fn the_handle_codes_are_catalogued_in_their_own_block() {
        assert!(SOUNDNESS.contains(HANDLES.start) && SOUNDNESS.contains(HANDLES.end));
        assert!(!POSITIVITY.overlaps(&HANDLES));
        assert_eq!(HANDLES.start, 720, "termination keeps E0710-E0719");
        for (code, name, counterpart) in [
            ("E0720", "ErrHandleUsedAfterConsumed", "E0092"),
            ("E0721", "ErrHandleCapturedByClosure", "E0093"),
        ] {
            let entry = SELF_HOSTED_ERRORS
                .iter()
                .find(|entry| entry.code == code)
                .expect("each handle code is catalogued");
            assert_eq!(entry.name, name);
            assert!(HANDLES.contains(code_number(code).expect("Ennnn")));
            assert!(
                entry.description.contains(counterpart),
                "{code} names {counterpart}"
            );
        }
    }

    /// 9.10.26o AC3: the region block sits inside the soundness reserve, past
    /// the handles', and its three codes are catalogued in it, each naming its
    /// bootstrap counterpart.
    #[test]
    fn the_region_codes_are_catalogued_in_their_own_block() {
        assert!(SOUNDNESS.contains(REGIONS.start) && SOUNDNESS.contains(REGIONS.end));
        assert!(!HANDLES.overlaps(&REGIONS));
        for (code, name, counterpart) in [
            ("E0730", "ErrRegionNotFirstOrder", "E0094"),
            ("E0731", "ErrRegionReachesOutside", "E0095"),
            ("E0732", "ErrRegionInProof", "E0096"),
        ] {
            let entry = SELF_HOSTED_ERRORS
                .iter()
                .find(|entry| entry.code == code)
                .expect("each region code is catalogued");
            assert_eq!(entry.name, name);
            assert!(REGIONS.contains(code_number(code).expect("Ennnn")));
            assert!(
                entry.description.contains(counterpart),
                "{code} names {counterpart}"
            );
        }
    }

    /// A code outside every block resolves to nothing rather than to the
    /// nearest one. The probe was E0850 until ADR 30.9.26h gave E0800-E0899
    /// to Syntax; E1000 is past the last block.
    #[test]
    fn a_code_in_no_range_resolves_to_none() {
        assert_eq!(range_of(1000), None);
        assert_eq!(range_of(0), None);
        assert_eq!(range_of(850).map(|range| range.name), Some("Syntax"));
        assert_eq!(code_number("nonsense"), None);
        assert_eq!(code_number("E0700"), Some(700));
    }

    /// 13.10.26r AC5: the mutable-binding block sits inside the soundness
    /// reserve, past the regions', and its three codes are catalogued in it,
    /// each naming its bootstrap counterpart.
    #[test]
    fn the_mutable_binding_codes_are_catalogued_in_their_own_block() {
        assert!(
            SOUNDNESS.contains(MUTABLE_BINDINGS.start) && SOUNDNESS.contains(MUTABLE_BINDINGS.end)
        );
        assert!(!REGIONS.overlaps(&MUTABLE_BINDINGS));
        for (code, name, counterpart) in [
            ("E0740", "ErrAssignmentRefused", "E0104"),
            ("E0741", "ErrMutableCaptured", "E0105"),
            ("E0742", "ErrMutableAssignedInRegion", "E0106"),
        ] {
            let entry = SELF_HOSTED_ERRORS
                .iter()
                .find(|entry| entry.code == code)
                .expect("each mutable-binding code is catalogued");
            assert_eq!(entry.name, name);
            assert!(MUTABLE_BINDINGS.contains(code_number(code).expect("Ennnn")));
            assert!(
                entry.description.contains(counterpart),
                "{code} names {counterpart}"
            );
        }
    }
}
