//! Extended typing rules: strings, arithmetic, booleans, refs, ADTs, and the
//! proof-mode dependent case split (ADR 3.10.26o).
//!
//! These functions implement typing rules for extended term forms,
//! called from the main `type_of` match in `rules.rs`.

mod adt;
mod arithmetic;
mod control_flow;
mod dependent_case;
mod predicate;
mod recursion;
mod refs;
mod sigma;
mod strings;

pub(super) use adt::{type_of_adt_construct, type_of_adt_match};
pub(super) use arithmetic::{
    type_of_bool_binop, type_of_bool_not, type_of_int_bin, type_of_int_neg, type_of_int_to_nat,
    type_of_nat_binop, type_of_nat_cmp, type_of_nat_to_int,
};
pub(super) use control_flow::type_of_return;
#[cfg(test)]
pub(super) use dependent_case::SPLITS_CHECKED;
pub(super) use dependent_case::{is_case_split, type_of_dependent_case};
pub(crate) use predicate::{check_signature_wf, type_of_pred_annot, type_of_pred_tyapp};
pub(super) use recursion::{type_of_fix, type_of_fold, type_of_subst, type_of_unfold};
pub(super) use refs::{type_of_ref_get, type_of_ref_set};
pub(super) use sigma::{as_pair, check_sigma_binders, snd_instance, type_of_sigma_pair};
pub(super) use strings::{
    type_of_str_char_at, type_of_str_concat, type_of_str_eq, type_of_str_len, type_of_str_substring,
};
