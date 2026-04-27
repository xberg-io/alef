//! Swift binding generator backend for alef.
//!
//! Phase 2A skeleton: registers `SwiftBackend` targeting Apple platforms
//! (macOS, iOS, tvOS, watchOS, visionOS). Linux Swift uses the same backend
//! with a separate CI matrix; no platform-specific codegen is needed here.
//! Real codegen (swift-bridge wiring, type generation) lands in Phase 2B.

mod gen_bindings;
pub mod gen_rust_crate;
mod type_map;

pub use gen_bindings::SwiftBackend;
