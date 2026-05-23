//! Re-exports of reserved keyword lists and field-name escaping from `alef-core`.
//!
//! The canonical definitions live in `crate::core::keywords`. This module re-exports
//! them so that `alef-codegen` consumers can use `crate::codegen::keywords::*` without
//! a direct dependency on `alef-core`.

pub use crate::core::keywords::*;
