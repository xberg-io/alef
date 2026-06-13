//! Service-API codegen for the Rustler (Elixir) backend.
//!
//! Generates two outputs per `ServiceDef`:
//!
//! 1. **`service.ex`** — Elixir module with a server-like class containing:
//!    - A constructor and configurator methods.
//!    - Registration decorator-style helpers that store callbacks.
//!    - A GenServer to dispatch trait_call messages to registered handlers.
//!    - A `run` entrypoint that marshals registrations to Rust.
//!
//! 2. **`service.rs`** — Rust rustler glue that:
//!    - Emits a message-passing handler bridge for each referenced `HandlerContractDef`.
//!    - Provides a `#[rustler::nif]` run function (with `schedule = "DirtyCpu"`) that
//!      receives registrations, builds the service, and drives entrypoints.
//!    - The bridge sends `{:trait_call, method, args_json, reply_id}` to the Elixir pid
//!      and awaits the response via a `complete_trait_call` NIF.
//!
//! All names are derived entirely from the `ApiSurface` IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

mod elixir;
mod helpers;
mod new_ir_stubs;
mod orchestration;
mod registration;
mod registration_nif;
mod rust;

#[cfg(test)]
mod tests;

pub use orchestration::generate;

#[cfg(test)]
use elixir::gen_service_ex;
#[cfg(test)]
use rust::gen_service_rs;
