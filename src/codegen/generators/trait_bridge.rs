//! Shared trait bridge code generation.
//!
//! Generates wrapper structs that allow foreign language objects (Python, JS, etc.)
//! to implement Rust traits via FFI. Each backend implements [`TraitBridgeGenerator`]
//! to provide language-specific dispatch logic; the shared functions in this module
//! handle the structural boilerplate.

mod formatting;
mod generator;
mod lookup;
mod registration;
mod spec;
mod trait_impl;
mod wrapper;

pub use formatting::{
    bridge_param_type, format_param_type, format_param_type_with_lifetimes, format_return_type, format_type_ref, prim,
    to_camel_case, visitor_param_type,
};
pub use generator::{BridgeOutput, TraitBridgeGenerator, gen_bridge_all};
pub use lookup::{
    BridgeFieldMatch, bridge_handle_path, bridge_wrapper_name, find_bridge_field, find_bridge_param, find_trait_def,
    is_bridge_handle_type_ref, is_native_marshalled_struct, is_trait_bridge_managed_fn,
    native_marshalled_struct_params,
};
pub use registration::{
    gen_bridge_clear_fn, gen_bridge_registration_fn, gen_bridge_unregistration_fn, host_function_path,
};
pub use spec::{TraitBridgeSpec, visitor_callback_methods};
pub use trait_impl::gen_bridge_trait_impl;
pub use wrapper::{gen_bridge_debug_impl, gen_bridge_plugin_impl, gen_bridge_wrapper_struct};

#[cfg(test)]
mod tests;
