//! Gleam binding generator backend for alef.
//!
//! Phase 1A skeleton: registers `GleamBackend` and exposes `BuildConfig`
//! with `BuildDependency::Rustler`. Gleam shims the Rustler-emitted Erlang
//! NIF via `@external(erlang, ..., ...)` declarations; real codegen lands
//! in Phase 1B.

mod gen_bindings;
mod type_map;

pub use gen_bindings::GleamBackend;
