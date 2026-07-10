use alef::backends::swift::gen_rust_crate::trait_bridge::{
    emit_extern_block_for_trait_bridge, emit_trait_bridge_wrapper,
};
use alef::core::ir::{MethodDef, TypeDef, TypeRef};
use std::collections::HashSet;

fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{}", name),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn make_method(name: &str, return_type: TypeRef, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn test_swift_trait_bridge_vec_phantom_symbols() {
    let trait_names = vec!["DocumentExtractor", "TextBackend", "PostProcessor", "Renderer"];

    for trait_name in trait_names {
        let trait_def = make_trait_def(trait_name, vec![make_method("process", TypeRef::String, false)]);

        let visible_types = HashSet::new();

        let extern_block = emit_extern_block_for_trait_bridge(&trait_def, &visible_types);

        let phantom_fn_name = format!("alef_phantom_vec_{}", heck::AsSnakeCase(trait_name));
        let phantom_decl = format!("fn {}() -> Vec<{}Box>;", phantom_fn_name, trait_name);
        assert!(
            extern_block.contains(&phantom_decl),
            "Extern block for {} missing phantom declaration: {}",
            trait_name,
            phantom_decl
        );

        let wrapper = emit_trait_bridge_wrapper(
            &trait_def,
            "sample_crate",
            &HashSet::new(),
            &HashSet::new(),
            &std::collections::HashMap::new(),
        );

        let phantom_impl = format!("pub fn {}() -> Vec<{}Box>", phantom_fn_name, trait_name);
        assert!(
            wrapper.contains(&phantom_impl),
            "Wrapper for {} missing phantom implementation: {}",
            trait_name,
            phantom_impl
        );

        let expected_symbols = vec![
            format!("__swift_bridge__$Vec_{}Box$new", trait_name),
            format!("__swift_bridge__$Vec_{}Box$drop", trait_name),
            format!("__swift_bridge__$Vec_{}Box$len", trait_name),
            format!("__swift_bridge__$Vec_{}Box$pop", trait_name),
            format!("__swift_bridge__$Vec_{}Box$push", trait_name),
            format!("__swift_bridge__$Vec_{}Box$get", trait_name),
            format!("__swift_bridge__$Vec_{}Box$get_mut", trait_name),
            format!("__swift_bridge__$Vec_{}Box$as_ptr", trait_name),
        ];

        for _symbol in &expected_symbols {}
    }
}

#[test]
fn test_swift_renderer_trait_bridge_vec_symbols_specifically() {
    let renderer_trait = make_trait_def("Renderer", vec![make_method("render", TypeRef::String, false)]);

    let visible_types = HashSet::new();

    let extern_block = emit_extern_block_for_trait_bridge(&renderer_trait, &visible_types);
    let wrapper = emit_trait_bridge_wrapper(
        &renderer_trait,
        "sample_crate",
        &HashSet::new(),
        &HashSet::new(),
        &std::collections::HashMap::new(),
    );

    assert!(
        extern_block.contains("fn alef_phantom_vec_renderer() -> Vec<RendererBox>;"),
        "RendererBox phantom declaration missing from extern block"
    );
    assert!(
        wrapper.contains("pub fn alef_phantom_vec_renderer() -> Vec<RendererBox>"),
        "RendererBox phantom implementation missing from wrapper"
    );
}
