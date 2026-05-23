//! alef — polyglot binding generator.
//!
//! Top-level module re-exports for the consolidated `alef` crate.
//! Each module corresponds to one of the former workspace member crates
//! (alef-core, alef-codegen, ...). See README and CHANGELOG (v0.18.0)
//! for the consolidation rationale.

pub mod adapters;
pub mod backends;
pub mod cli;
pub mod codegen;
pub mod core;
pub mod docs;
pub mod e2e;
pub mod extract;
pub mod publish;
pub mod readme;
pub mod scaffold;
pub mod snippets;
