//! Ruby (Magnus) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Ruby objects via Magnus `respond_to` checks and `funcall`.

mod bridge_functions;
mod bridge_generator;
mod options_field;
mod owned_params;
mod visitor_bridge;

pub use crate::codegen::generators::trait_bridge::find_bridge_param;
pub use bridge_functions::gen_bridge_function;
pub use bridge_generator::gen_trait_bridge;
pub use options_field::{find_options_field_binding, gen_options_field_bridge_function};

use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, TypeDef};

/// The `exclude_languages` spellings that name this target: the language (`"ruby"`) and the
/// backend (`"magnus"`). Both are honoured so a consumer who names either one gets the same
/// answer from every site that asks. ~keep
pub const TARGET_SPELLINGS: [&str; 2] = ["ruby", "magnus"];

/// The trait a bridge wraps, when Ruby emits that bridge at all.
///
/// `gen_module_init` binds `define_module_function("<register_fn>", …)` to the `pub fn` that
/// `gen_trait_bridge` writes, and `gen_trait_bridge` runs only when the trait resolves in the
/// `ApiSurface`. Asking here keeps the `#[magnus::init]` body from binding a function no pass
/// generated. ~keep
pub fn active_bridge_trait<'a>(bridge: &TraitBridgeConfig, api: &'a ApiSurface) -> Option<&'a TypeDef> {
    crate::codegen::generators::trait_bridge::active_bridge_trait_def(bridge, api, &TARGET_SPELLINGS)
}

#[cfg(test)]
mod tests {
    #[test]
    fn visitor_bridge_uses_configured_context_and_result_metadata() {
        let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
        let code = super::gen_trait_bridge(
            &trait_type,
            &bridge,
            "sample_core",
            "SampleError",
            "SampleError::Message { message: {msg} }",
            &api,
        )
        .expect("visitor bridge should generate");

        crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&code);
        assert!(code.contains("\"display_name\""));
    }
}
