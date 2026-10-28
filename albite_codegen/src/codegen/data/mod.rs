//! Data type compilation
//!
//! Compilation of Tungsten data types to LLVM IR:
//! - `primitives`: Basic types (bool, nat, unit)
//! - `products`: Product types (pairs, tuples)
//! - `sums`: Sum types and case expressions
//! - `adt`: Algebraic data types (flat enum representation)
//! - `mu_types`: Recursive type helpers (fold/unfold)
//! - `ops`: Data type operations (bool, nat, string, ref)
//! - `alloc`: the shared heap-allocation helpers and the `AllocClass` tags
//!   they feed to the runtime's allocation symbols (ADR 14.9.26b, 7.10.26s)
//! - `region_copy`: the static layouts a region's copy-out walks (ADR
//!   13.10.26q)

pub(crate) mod adt;
pub(crate) mod alloc;
pub(crate) mod mu_types;
pub(crate) mod ops;
pub(crate) mod primitives;
pub(crate) mod products;
pub(crate) mod region_copy;
pub(crate) mod sums;

pub(crate) use alloc::{
    AllocClass, ALLOC_SYMBOL, ARENA_INIT_REGIONS_SYMBOL, ARENA_INIT_SYMBOL, ARENA_MODE_GLOBAL,
    MALLOC_SYMBOL,
};
