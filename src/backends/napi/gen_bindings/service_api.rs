//! Service-API codegen for the NAPI-RS (Node.js/TypeScript) backend.
//!
//! Generates Rust napi glue and idiomatic TypeScript service wrappers for
//! [`crate::core::ir::ServiceDef`] entries.

mod assembly;
mod helpers;
mod new_ir_stubs;
mod rust_glue;
mod typescript;

pub use assembly::generate;
pub(super) use rust_glue::gen_service_rs;
pub(super) use typescript::gen_service_ts;

#[cfg(test)]
#[path = "service_api/tests.rs"]
mod tests;
