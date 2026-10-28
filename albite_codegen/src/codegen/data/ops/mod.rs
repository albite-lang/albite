//! Data type operations
//!
//! Codegen for operations on specific data types:
//! - `bool_ops`: Boolean logic operations
//! - `nat_ops`: Natural number arithmetic and comparisons
//! - `int_ops`: Signed integer arithmetic (checked, trapping) and comparisons
//! - `strings`: String operations
//! - `refs`: Mutable references
//! - `bytes`: The checked byte buffer's `tg_bytes_*` intrinsics, lowered inline

pub(crate) mod bool_ops;
pub(crate) mod bytes;
pub(crate) mod int_ops;
pub(crate) mod nat_ops;
pub(crate) mod refs;
pub(crate) mod strings;
