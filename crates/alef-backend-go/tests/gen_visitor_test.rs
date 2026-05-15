use alef_backend_go::gen_visitor::gen_visitor_file;

/// Smoke test: gen_visitor_file produces output with the expected prefix structure.
/// The exact C struct name depends on `vtable_trait_name` and `ffi_prefix`.
#[test]
fn test_visitor_file_emits_prefixed_struct() {
    // Minimal trait def with one method to exercise the generator.
    let trait_def = alef_core::ir::TypeDef {
        name: "HtmlVisitor".to_string(),
        rust_path: "html_to_markdown_rs::visitor::HtmlVisitor".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![alef_core::ir::MethodDef {
            name: "visit_text".to_string(),
            params: vec![alef_core::ir::ParamDef {
                name: "_text".to_string(),
                ty: alef_core::ir::TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                newtype_wrapper: None,
                is_ref: false,
                is_mut: false,
                original_type: None,
            }],
            return_type: alef_core::ir::TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Visit text nodes.".to_string(),
            receiver: Some(alef_core::ir::ReceiverKind::RefMut),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: true,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let output = gen_visitor_file(
        "mypkg",
        "htm",
        "my_lib.h",
        "../ffi",
        "..",
        "HtmlVisitor",
        "visitor",
        &trait_def,
    );
    // The cbindgen-derived C type embeds `{PREFIX}{PascalPrefix}{TraitName}VTable`.
    assert!(
        output.contains("VTable"),
        "expected VTable in output, got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(output.contains("HTM"), "expected upper-case prefix HTM in output");
}
