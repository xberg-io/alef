//! E2E test generation configuration re-exports.
//!
//! The canonical config types live in `crate::core::config::e2e` so they can be
//! deserialized as part of `[[crates]]` entries. This module re-exports them for
//! convenience within the `alef-e2e` crate.

pub use crate::core::config::e2e::*;
