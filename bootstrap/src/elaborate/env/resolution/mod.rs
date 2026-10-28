//! Path resolution, canonical lookup, and visibility checking for Env.
//!
//! - `resolve`: Qualified path resolution for types, values, constructors
//! - `canonical`: Canonical module resolution following re-export chains
//! - `visibility`: Visibility checking for modules and items

mod canonical;
mod core_names;
mod module_items;
mod qualified;
mod resolve;
mod visibility;

pub use canonical::CanonicalResolutionError;
pub use core_names::CoreNames;
pub use qualified::qualified_value_path;
