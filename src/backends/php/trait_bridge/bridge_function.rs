use crate::core::config::TraitBridgeConfig;

/// Return an explicit compiler error for PHP trait-bridge function generation.
pub fn gen_bridge_function(
    _func: &crate::core::ir::FunctionDef,
    _bridge_param_idx: usize,
    _bridge_cfg: &TraitBridgeConfig,
    _mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    _opaque_types: &ahash::AHashSet<String>,
    _core_import: &str,
    _handle_path: &str,
) -> String {
    super::disabled_code()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_bridge_function_generation_fails_closed_for_every_index() {
        let function = crate::core::ir::FunctionDef {
            name: "run".to_string(),
            params: vec![crate::core::ir::ParamDef {
                name: "handler".to_string(),
                ty: crate::core::ir::TypeRef::Named("Handler".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let bridge = TraitBridgeConfig {
            trait_name: "Handler".to_string(),
            ..Default::default()
        };

        for index in [0, usize::MAX] {
            let output = gen_bridge_function(
                &function,
                index,
                &bridge,
                &crate::codegen::type_mapper::IdentityMapper,
                &ahash::AHashSet::new(),
                "core",
                "HandlerRef",
            );
            assert!(output.contains("compile_error!"));
            for forbidden in ["Zval", "unsafe impl Send", "unsafe impl Sync", "Arc<dyn", "Arc::new"] {
                assert!(!output.contains(forbidden), "unsafe token `{forbidden}` in: {output}");
            }
        }
    }
}
