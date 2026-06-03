use alef::backends::go::gen_visitor::gen_visitor_file;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::{FunctionDef, ParamDef, TypeRef};

/// Smoke test: gen_visitor_file produces output with the expected prefix structure.
/// The exact C struct name depends on `vtable_trait_name` and `ffi_prefix`.
#[test]
fn test_visitor_file_emits_prefixed_struct() {
    // Minimal trait def with one method to exercise the generator.
    let trait_def = alef::core::ir::TypeDef {
        name: "HtmlVisitor".to_string(),
        rust_path: "sample_markdown_rs::visitor::HtmlVisitor".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![alef::core::ir::MethodDef {
            name: "visit_text".to_string(),
            params: vec![alef::core::ir::ParamDef {
                name: "_ctx".to_string(),
                ty: alef::core::ir::TypeRef::Named("NodeContext".to_string()),
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                newtype_wrapper: None,
                is_ref: false,
                is_mut: false,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
            }],
            return_type: alef::core::ir::TypeRef::Named("VisitResult".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Visit text nodes.".to_string(),
            receiver: Some(alef::core::ir::ReceiverKind::RefMut),
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
        is_variant_wrapper: false,
        has_lifetime_params: false,
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
        &bridge_config(
            "HtmlVisitor",
            "ConversionOptions",
            "visitor",
            "VisitorHandle",
            Some("NodeContext"),
            Some("VisitResult"),
        ),
        &bridge_function("convert", "html", "options", "ConversionOptions", "ConversionResult"),
    );
    // The cbindgen-derived C type embeds `{PREFIX}{PascalPrefix}{TraitName}VTable`.
    assert!(
        output.contains("VTable"),
        "expected VTable in output, got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(output.contains("HTM"), "expected upper-case prefix HTM in output");
}

#[test]
fn test_visitor_file_uses_configured_function_options_field_and_result() {
    let trait_def = alef::core::ir::TypeDef {
        name: "Renderer".to_string(),
        rust_path: "sample::Renderer".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![alef::core::ir::MethodDef {
            name: "visit_text".to_string(),
            params: vec![
                param("_ctx", TypeRef::Named("NodeContext".to_string()), false),
                param("_text", TypeRef::String, false),
            ],
            return_type: TypeRef::Named("VisitResult".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(alef::core::ir::ReceiverKind::RefMut),
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
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };

    let output = gen_visitor_file(
        "mypkg",
        "krz",
        "my_lib.h",
        "../ffi",
        "..",
        "Renderer",
        "renderer",
        &trait_def,
        &bridge_config(
            "Renderer",
            "RenderOptions",
            "renderer",
            "RendererHandle",
            Some("NodeContext"),
            Some("VisitResult"),
        ),
        &bridge_function("render", "document", "settings", "RenderOptions", "RenderOutput"),
    );

    assert!(output.contains(
        "func renderWithVisitorHelper(document string, settings *RenderOptions, visitor Visitor) (*RenderOutput, error)"
    ));
    assert!(output.contains("var cOptions *C.KRZRenderOptions"));
    assert!(output.contains("cOptions = C.krz_render_options_from_json(optionsJSON)"));
    assert!(output.contains("C.krz_options_set_renderer(cOptions"));
    assert!(output.contains("ptr := C.krz_render(cDocument, cOptions)"));
    assert!(output.contains("defer C.krz_render_output_free(ptr)"));
    assert!(output.contains("jsonPtr := C.krz_render_output_to_json(ptr)"));
    assert!(!output.contains("convertWithVisitorHelper"));
}

#[test]
fn test_generic_trait_without_compat_callback_types_does_not_emit_fixed_helpers() {
    let trait_def = alef::core::ir::TypeDef {
        name: "Renderer".to_string(),
        rust_path: "sample::Renderer".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![alef::core::ir::MethodDef {
            name: "render".to_string(),
            params: vec![param("_input", TypeRef::String, false)],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(alef::core::ir::ReceiverKind::RefMut),
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
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };

    let output = gen_visitor_file(
        "mypkg",
        "krz",
        "my_lib.h",
        "../ffi",
        "..",
        "Renderer",
        "renderer",
        &trait_def,
        &bridge_config("Renderer", "RenderOptions", "renderer", "RendererHandle", None, None),
        &bridge_function("render", "document", "settings", "RenderOptions", "RenderOutput"),
    );

    assert!(output.is_empty());
    assert!(!output.contains("type NodeContext struct"));
    assert!(!output.contains("type VisitResult struct"));
}

fn bridge_config(
    trait_name: &str,
    options_type: &str,
    options_field: &str,
    type_alias: &str,
    context_type: Option<&str>,
    result_type: Option<&str>,
) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        type_alias: Some(type_alias.to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some(options_type.to_string()),
        options_field: Some(options_field.to_string()),
        context_type: context_type.map(str::to_string),
        result_type: result_type.map(str::to_string),
        ..TraitBridgeConfig::default()
    }
}

fn bridge_function(
    name: &str,
    input_name: &str,
    options_name: &str,
    options_type: &str,
    return_type: &str,
) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("sample::{name}"),
        original_rust_path: String::new(),
        params: vec![
            param(input_name, TypeRef::String, false),
            param(
                options_name,
                TypeRef::Optional(Box::new(TypeRef::Named(options_type.to_string()))),
                true,
            ),
        ],
        return_type: TypeRef::Named(return_type.to_string()),
        is_async: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        sanitized: false,
        typed_default: None,
        newtype_wrapper: None,
        is_ref: false,
        is_mut: false,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
    }
}
