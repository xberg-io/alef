use super::{
    TraitBridgeGenerator, TraitBridgeSpec, gen_bridge_clear_fn, gen_bridge_debug_impl, gen_bridge_plugin_impl,
    gen_bridge_registration_fn, gen_bridge_trait_impl, gen_bridge_unregistration_fn, gen_bridge_wrapper_struct,
};

/// Result of trait bridge generation: imports (to be added via `builder.add_import`)
/// and the code body (to be added via `builder.add_item`).
pub struct BridgeOutput {
    /// Import paths (e.g., `"std::sync::Arc"`) — callers should add via `builder.add_import()`.
    pub imports: Vec<String>,
    /// The generated code (struct, impls, registration fn).
    pub code: String,
}

/// Generate the complete trait bridge code block: struct, impls, and
/// optionally a registration function.
///
/// Returns [`BridgeOutput`] with imports separated from code so callers can
/// route imports through `builder.add_import()` (which deduplicates).
pub fn gen_bridge_all(spec: &TraitBridgeSpec, generator: &dyn TraitBridgeGenerator) -> BridgeOutput {
    let imports = generator.bridge_imports();
    let mut out = String::with_capacity(4096);

    out.push_str(&gen_bridge_wrapper_struct(spec, generator));
    out.push_str("\n\n");

    out.push_str(&gen_bridge_debug_impl(spec));
    out.push_str("\n\n");

    out.push_str(&generator.gen_constructor(spec));
    out.push_str("\n\n");

    if let Some(plugin_impl) = gen_bridge_plugin_impl(spec, generator) {
        out.push_str(&plugin_impl);
        out.push_str("\n\n");
    }

    out.push_str(&gen_bridge_trait_impl(spec, generator));

    if let Some(reg_fn_code) = gen_bridge_registration_fn(spec, generator) {
        out.push_str("\n\n");
        out.push_str(&reg_fn_code);
    }

    if let Some(unreg_fn_code) = gen_bridge_unregistration_fn(spec, generator) {
        out.push_str("\n\n");
        out.push_str(&unreg_fn_code);
    }

    if let Some(clear_fn_code) = gen_bridge_clear_fn(spec, generator) {
        out.push_str("\n\n");
        out.push_str(&clear_fn_code);
    }

    BridgeOutput { imports, code: out }
}
