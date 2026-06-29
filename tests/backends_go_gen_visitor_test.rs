use alef::backends::go::gen_visitor::gen_visitor_file;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};

/// Smoke test: gen_visitor_file produces output with the expected callback structure.
/// The exact C callback struct name depends on `ffi_prefix`.
#[test]
fn test_visitor_file_emits_prefixed_struct() {
    // Minimal trait def with one method to exercise the generator.
    let trait_def = alef::core::ir::TypeDef {
        name: "SyntaxWalker".to_string(),
        rust_path: "sample_crate::visitor::SyntaxWalker".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![alef::core::ir::MethodDef {
            name: "visit_text".to_string(),
            params: vec![alef::core::ir::ParamDef {
                name: "_ctx".to_string(),
                ty: alef::core::ir::TypeRef::Named("SyntaxContext".to_string()),
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
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: alef::core::ir::TypeRef::Named("WalkDecision".to_string()),
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
            version: Default::default(),
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
        has_private_fields: false,
        version: Default::default(),
    };

    let output = gen_visitor_file(
        &visitor_metadata_api("SyntaxContext", "WalkDecision", "Continue"),
        "mypkg",
        "htm",
        "my_lib.h",
        "../ffi",
        "..",
        "SyntaxWalker",
        "visitor",
        &trait_def,
        &bridge_config(
            "SyntaxWalker",
            "ParseOptions",
            "visitor",
            "VisitorHandle",
            Some("SyntaxContext"),
            Some("WalkDecision"),
        ),
        &bridge_function("convert", "html", "options", "ParseOptions", "ParseOutput"),
    );
    assert!(
        output.contains("HTMHtmVisitorCallbacks"),
        "expected VisitorCallbacks in output, got:\n{}",
        &output[..output.find("import \"C\"").unwrap_or(output.len())]
    );
    assert!(
        output.contains("makeVisitorCallbacks"),
        "expected callback factory in output"
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
                param("_ctx", TypeRef::Named("SyntaxContext".to_string()), false),
                param("_text", TypeRef::String, false),
            ],
            return_type: TypeRef::Named("WalkDecision".to_string()),
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
            version: Default::default(),
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
        has_private_fields: false,
        version: Default::default(),
    };

    let output = gen_visitor_file(
        &visitor_metadata_api("SyntaxContext", "WalkDecision", "Continue"),
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
            Some("SyntaxContext"),
            Some("WalkDecision"),
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
            version: Default::default(),
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
        has_private_fields: false,
        version: Default::default(),
    };

    let output = gen_visitor_file(
        &ApiSurface::default(),
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
    assert!(!output.contains("type SyntaxContext struct"));
    assert!(!output.contains("type WalkDecision struct"));
}

#[test]
fn test_visitor_file_uses_ir_context_fields_and_result_enum_metadata() {
    let trait_def = trait_def(
        "SyntaxVisitor",
        vec![method(
            "visit_token",
            vec![
                param("_ctx", TypeRef::Named("ParseContext".to_string()), false),
                param("_token", TypeRef::String, false),
            ],
            TypeRef::Named("WalkOutcome".to_string()),
        )],
    );
    let mut api = ApiSurface::default();
    api.types.push(context_type(
        "ParseContext",
        vec![
            field("rule_name", TypeRef::String, false),
            field(
                "byte_offset",
                TypeRef::Primitive(alef::core::ir::PrimitiveType::Usize),
                false,
            ),
            field("source_path", TypeRef::String, true),
        ],
    ));
    api.enums.push(result_enum(
        "WalkOutcome",
        vec![
            variant("Proceed", None, false),
            default_variant("StopHere", None, false),
            variant("ReplaceWith", Some("replacement"), true),
            variant("Diagnostic", Some("message"), true),
        ],
        Some("snake_case"),
    ));

    let output = gen_visitor_file(
        &api,
        "parser",
        "prs",
        "parser.h",
        "../ffi",
        "..",
        "SyntaxVisitor",
        "visitor",
        &trait_def,
        &bridge_config(
            "SyntaxVisitor",
            "ParseOptions",
            "visitor",
            "SyntaxVisitorHandle",
            Some("ParseContext"),
            Some("WalkOutcome"),
        ),
        &bridge_function("parse", "source", "options", "ParseOptions", "ParseTree"),
    );

    assert!(output.contains("type ParseContext struct"));
    assert!(output.contains("RuleName string `json:\"rule_name\"`"));
    assert!(output.contains("ByteOffset uint `json:\"byte_offset\"`"));
    assert!(output.contains("SourcePath *string `json:\"source_path\"`"));
    assert!(output.contains("type WalkOutcome struct"));
    assert!(output.contains("func WalkOutcomeProceed() WalkOutcome"));
    assert!(output.contains("func WalkOutcomeReplaceWith(replacement string) WalkOutcome"));
    assert!(output.contains("return WalkOutcomeStopHere()"));
    assert!(!output.contains("return WalkOutcomeContinue()"));
    assert!(output.contains("return WalkOutcome{Code: 1}"));
    assert!(output.contains("return WalkOutcome{Code: 2, Value: &replacement}"));
    assert!(output.contains("cStr := C.CString(payload)"));
    assert!(output.contains("return 2"));
    assert!(!output.contains("PreserveHTML"));
    assert!(!output.contains("type SyntaxContext struct"));
    assert!(!output.contains("type WalkDecision struct"));
}

#[test]
fn test_visitor_file_fails_without_result_enum_metadata() {
    let trait_def = trait_def(
        "SyntaxVisitor",
        vec![method(
            "visit_token",
            vec![param("_ctx", TypeRef::Named("ParseContext".to_string()), false)],
            TypeRef::Named("WalkOutcome".to_string()),
        )],
    );
    let mut api = ApiSurface::default();
    api.types.push(context_type(
        "ParseContext",
        vec![field("rule_name", TypeRef::String, false)],
    ));

    let output = gen_visitor_file(
        &api,
        "parser",
        "prs",
        "parser.h",
        "../ffi",
        "..",
        "SyntaxVisitor",
        "visitor",
        &trait_def,
        &bridge_config(
            "SyntaxVisitor",
            "ParseOptions",
            "visitor",
            "SyntaxVisitorHandle",
            Some("ParseContext"),
            Some("WalkOutcome"),
        ),
        &bridge_function("parse", "source", "options", "ParseOptions", "ParseTree"),
    );

    assert!(output.is_empty());
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
        version: Default::default(),
    }
}

fn trait_def(name: &str, methods: Vec<alef::core::ir::MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
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
        has_private_fields: false,
        version: Default::default(),
    }
}

fn method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> alef::core::ir::MethodDef {
    alef::core::ir::MethodDef {
        name: name.to_string(),
        params,
        return_type,
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
        version: Default::default(),
    }
}

fn context_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    let mut type_def = trait_def(name, vec![]);
    type_def.is_trait = false;
    type_def.fields = fields;
    type_def
}

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn result_enum(name: &str, variants: Vec<EnumVariant>, serde_rename_all: Option<&str>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("sample::{name}"),
        original_rust_path: String::new(),
        variants,
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: serde_rename_all.map(str::to_string),
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
        has_default: false,
    }
}

fn variant(name: &str, payload_field: Option<&str>, is_tuple: bool) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields: payload_field
            .map(|field_name| vec![field(field_name, TypeRef::String, false)])
            .unwrap_or_default(),
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        is_tuple,
        binding_excluded: false,
        binding_exclusion_reason: None,
        originally_had_data_fields: payload_field.is_some(),
        cfg: None,
        version: Default::default(),
    }
}

fn default_variant(name: &str, payload_field: Option<&str>, is_tuple: bool) -> EnumVariant {
    EnumVariant {
        is_default: true,
        ..variant(name, payload_field, is_tuple)
    }
}

fn visitor_metadata_api(context_name: &str, result_name: &str, default_name: &str) -> ApiSurface {
    let mut api = ApiSurface::default();
    api.types.push(context_type(
        context_name,
        vec![
            field("tag_name", TypeRef::String, false),
            field("depth", TypeRef::Primitive(alef::core::ir::PrimitiveType::Usize), false),
        ],
    ));
    api.enums.push(result_enum(
        result_name,
        vec![
            if default_name == "Continue" {
                default_variant("Continue", None, false)
            } else {
                variant("Continue", None, false)
            },
            variant("Skip", None, false),
            variant("Custom", Some("value"), true),
        ],
        None,
    ));
    api
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
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}
