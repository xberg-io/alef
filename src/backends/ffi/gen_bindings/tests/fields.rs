use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

/// Regression test: Option<Option<Primitive>> (update-struct pattern) must generate
/// a getter that returns the primitive type — not *mut c_char — and collapses both
/// None cases to the primitive's zero sentinel.
#[test]
fn test_option_option_primitive_getter_returns_primitive_type() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "ConfigUpdate".to_string(),
            rust_path: "my_lib::ConfigUpdate".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "max_depth".to_string(),
                // field.ty = Optional(Primitive(Usize)), field.optional = true
                // represents Rust type Option<Option<usize>>
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
                optional: true,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: crate::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                original_type: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Return type must be `usize`, not `*mut std::ffi::c_char`
    assert!(
        lib.content.contains("-> usize"),
        "expected `-> usize` in getter but got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("-> *mut std::ffi::c_char"),
        "getter must not return *mut c_char for Option<Option<usize>>"
    );

    // Both None arms must return 0, not a pointer
    assert!(
        lib.content.contains("None => 0"),
        "expected `None => 0` sentinel in generated getter"
    );

    // The inner Some(inner_val) branch must dereference the usize
    assert!(
        lib.content.contains("*inner_val"),
        "expected `*inner_val` deref for inner primitive in generated getter"
    );
}

/// Build a minimal `ApiSurface` with one struct that has a Named field,
/// controlling `is_clone` on the field's referenced type.
fn api_with_named_field(field_type: &str, is_clone: bool) -> ApiSurface {
    // The struct that holds the Named field
    let holder = TypeDef {
        name: "Holder".to_string(),
        rust_path: "my_lib::Holder".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "inner".to_string(),
            ty: TypeRef::Named(field_type.to_string()),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: crate::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            original_type: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
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
        version: Default::default(),
    };
    // The type referenced by the Named field
    let named_type = TypeDef {
        name: field_type.to_string(),
        rust_path: format!("my_lib::{field_type}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![holder, named_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Non-Clone opaque Named-type fields must not emit `.clone()` in the
/// generated field accessor — the accessor should use a raw pointer cast instead.
#[test]
fn test_named_field_non_clone_no_clone_call() {
    let api = api_with_named_field("LanguageRegistry", false);
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The field accessor for `inner` (a non-Clone opaque type) must not call .clone()
    assert!(
        !lib.content.contains(".clone()"),
        "non-Clone opaque Named field must not emit .clone() in accessor:\n{}",
        lib.content
    );
}

/// Clone-capable Named-type fields must still emit `.clone()` in the accessor.
#[test]
fn test_named_field_clone_capable_emits_clone() {
    let api = api_with_named_field("ConversionOptions", true);
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The field accessor for `inner` (a Clone type) must clone the value
    assert!(
        lib.content.contains(".clone()"),
        "Clone-capable Named field must emit .clone() in accessor:\n{}",
        lib.content
    );
}

#[test]
fn test_options_field_visitor_callbacks_use_configured_renderer_setter() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "syn"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "SyntaxWalker"
type_alias = "SyntaxWalkerHandle"
param_name = "renderer"
bind_via = "options_field"
options_type = "ParseOptions"
options_field = "renderer"
context_type = "SyntaxContext"
result_type = "WalkOutcome"
"#,
    );
    let mut api = sample_api();
    api.types.push(TypeDef {
        name: "SyntaxWalker".to_string(),
        rust_path: "my_lib::syntax::SyntaxWalker".to_string(),
        methods: vec![MethodDef {
            name: "visit_token".to_string(),
            params: vec![ParamDef {
                name: "context".to_string(),
                ty: TypeRef::Named("SyntaxContext".to_string()),
                is_ref: true,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("WalkOutcome".to_string()),
            receiver: Some(ReceiverKind::RefMut),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        is_trait: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "SyntaxContext".to_string(),
        rust_path: "my_lib::syntax::SyntaxContext".to_string(),
        fields: vec![FieldDef {
            name: "rule_name".to_string(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "ParseOptions".to_string(),
        rust_path: "my_lib::ParseOptions".to_string(),
        is_clone: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "ParseResult".to_string(),
        rust_path: "my_lib::ParseResult".to_string(),
        is_clone: true,
        is_return_type: true,
        ..TypeDef::default()
    });
    api.enums.push(EnumDef {
        name: "WalkOutcome".to_string(),
        rust_path: "my_lib::syntax::WalkOutcome".to_string(),
        variants: vec![
            EnumVariant {
                name: "Continue".to_string(),
                is_default: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Stop".to_string(),
                ..EnumVariant::default()
            },
        ],
        has_serde: true,
        has_default: false,
        ..EnumDef::default()
    });
    api.functions.push(FunctionDef {
        name: "parse".to_string(),
        rust_path: "my_lib::parse".to_string(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                is_ref: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ParseOptions".to_string()))),
                optional: true,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("ParseResult".to_string()),
        error_type: Some("ParseError".to_string()),
        ..FunctionDef::default()
    });
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("syn_options_set_renderer"),
        "options-field setter must derive from configured renderer field"
    );
    assert!(
        !lib.content.contains("syn_options_set_visitor_handle"),
        "options-field mode must not emit the legacy visitor_handle setter"
    );
    assert!(
        lib.content.contains("pub struct SynVisitorCallbacks"),
        "Java callback lifecycle support should remain available"
    );
    assert!(
        lib.content.contains("syn_visitor_create") && lib.content.contains("syn_visitor_free"),
        "visitor create/free symbols should remain available"
    );
    let convert_count = lib.content.matches("fn syn_parse(").count();
    assert_eq!(convert_count, 1, "syn_parse must appear exactly once");
    assert!(
        !lib.content.contains("syn_parse_with_visitor"),
        "options-field mode must not emit the legacy with_visitor wrapper"
    );
    // Regression for Zig (strict opaque pointer cast): when visitor_callbacks is enabled the
    // options-field setter must accept the *mut SynVisitor handle produced by
    // syn_visitor_create, NOT the *mut SynSyntaxWalkerBridge produced by the trait-bridge
    // path. C and Zig consumers both call syn_visitor_create followed by
    // syn_options_set_renderer; Zig refuses implicit casts between opaque pointer types.
    assert!(
        lib.content.contains("visitor: *mut SynVisitor"),
        "options-field setter must accept *mut SynVisitor when visitor_callbacks is enabled"
    );
    assert!(
        !lib.content.contains("visitor: *mut SynSyntaxWalkerBridge"),
        "options-field setter must not require the trait-bridge handle when visitor_callbacks is enabled"
    );
}

#[test]
fn test_options_field_bridge_generates_non_convert_function_from_ir() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "doc"

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "RenderHandle"
param_name = "renderer"
bind_via = "options_field"
options_type = "RenderSettings"
options_field = "renderer"
"#,
    );
    let mut api = visitor_api();
    api.types.push(TypeDef {
        name: "RenderSettings".to_string(),
        rust_path: "my_lib::RenderSettings".to_string(),
        fields: vec![],
        is_clone: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "RenderedDocument".to_string(),
        rust_path: "my_lib::RenderedDocument".to_string(),
        fields: vec![],
        is_clone: true,
        ..TypeDef::default()
    });
    api.functions.push(FunctionDef {
        name: "render_document".to_string(),
        rust_path: "my_lib::render_document".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                is_ref: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "settings".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("RenderSettings".to_string()))),
                optional: true,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("RenderedDocument".to_string()),
        is_async: false,
        error_type: Some("RenderError".to_string()),
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
    });
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn doc_render_document("),
        "must generate IR-derived symbol"
    );
    assert!(
        lib.content.contains("settings: *const my_lib::RenderSettings"),
        "must use configured options type"
    );
    assert!(
        lib.content.contains(") -> *mut my_lib::RenderedDocument"),
        "must use actual return type"
    );
    assert!(
        lib.content
            .contains("match my_lib::render_document(source_rs, settings_rs)"),
        "must call actual core function with actual parameters"
    );
    assert!(
        !lib.content.contains("my_lib::convert("),
        "must not hardcode conversion call"
    );
    assert!(
        !lib.content.contains("ConversionOptions") && !lib.content.contains("ConversionResult"),
        "must not leak conversion-shaped type names in generic wrapper"
    );
}
