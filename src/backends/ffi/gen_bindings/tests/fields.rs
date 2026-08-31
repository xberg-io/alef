use super::super::FfiBackend;
use super::super::types::{gen_field_accessor, gen_field_presence_accessor};
use super::common::*;
use crate::backends::ffi::type_map::optional_leaf_needs_presence_signal;
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
                version: Default::default(),
                name: "max_depth".to_string(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
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
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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

    assert!(
        lib.content.contains("-> usize"),
        "expected `-> usize` in getter but got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("-> *mut std::ffi::c_char"),
        "getter must not return *mut c_char for Option<Option<usize>>"
    );

    assert!(
        lib.content.contains("None => 0"),
        "expected `None => 0` sentinel in generated getter"
    );

    assert!(
        lib.content.contains("*inner_val"),
        "expected `*inner_val` deref for inner primitive in generated getter"
    );
}

/// Build a minimal `ApiSurface` with one struct that has a Named field,
/// controlling `is_clone` on the field's referenced type.
fn api_with_named_field(field_type: &str, is_clone: bool) -> ApiSurface {
    let holder = TypeDef {
        name: "Holder".to_string(),
        rust_path: "my_lib::Holder".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            version: Default::default(),
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
            serde_with: None,
            serde_skip_serializing_if: false,
            serde_skip: false,
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
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

#[test]
fn test_named_field_non_clone_does_not_return_borrow_as_owned() {
    let api = api_with_named_field("LanguageRegistry", false);
    let config = sample_config();
    let backend = FfiBackend;

    let error = backend
        .generate_bindings(&api, &config)
        .expect_err("non-Clone named fields cannot produce owned handles");
    assert!(
        error
            .to_string()
            .contains("non-Copy, non-Clone type `LanguageRegistry`")
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

    assert!(
        lib.content.contains(".clone()"),
        "Clone-capable Named field must emit .clone() in accessor:\n{}",
        lib.content
    );
}

#[test]
fn test_optional_trait_bridge_handle_getter_clones_owned_handle() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[[crates.trait_bridges]]
trait_name = "DocumentVisitor"
type_alias = "VisitorHandle"
bind_via = "options_field"
options_type = "RenderOptions"
options_field = "visitor"

"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "RenderOptions".to_string(),
            rust_path: "my_lib::RenderOptions".to_string(),
            fields: vec![FieldDef {
                name: "visitor".to_string(),
                ty: TypeRef::Named("VisitorHandle".to_string()),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = FfiBackend
        .generate_bindings(&api, &config)
        .expect("trait bridge handle getter");
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    let accessor = lib
        .content
        .split("fn sample_render_options_visitor")
        .nth(1)
        .expect("visitor accessor");

    assert!(accessor.contains("Some(val) =>"), "{accessor}");
    assert!(accessor.contains("insert_handle(val.clone())"), "{accessor}");
    assert!(!accessor.contains("Some(val) => {\n            0"), "{accessor}");
}

/// Regression test for the dead-getter defect: a field carrying a full `#[serde(skip)]`
/// (`FieldDef::serde_skip`) on a non-opaque, serde-derived type can only ever hold
/// `Default::default()` -- no caller-supplied JSON payload can populate it, since `_from_json`
/// (deserialize) is the only generic FFI constructor such a type gets, and this config has no
/// `options_field` trait-bridge setter naming `ConversionOptionsUpdate` at all. Its getter must
/// not be emitted: emitting one would document an "owned handle the caller must free" contract
/// (see `field_accessor_ownership_lines`) that no caller-supplied input can ever satisfy.
///
/// The sibling `ConversionOptions.visitor` field carries the IDENTICAL `serde_skip: true` but
/// IS reachable, through an `options_field` trait bridge whose `param_name` ("watcher")
/// deliberately differs from its `options_field` ("visitor") -- the bridge must be matched by
/// resolved field name, not by param name, per `TraitBridgeConfig::resolved_options_field`.
/// Asserting presence here, not just absence for the dead field, is required: a fix that
/// over-suppresses (e.g. hiding every `serde_skip` field regardless of a matching bridge) would
/// pass the absence half of this test while silently breaking a getter real callers depend on.
#[test]
fn test_serde_skip_field_getter_suppressed_unless_options_bridge_reaches_it() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "htm"

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "watcher"
bind_via = "options_field"
options_type = "ConversionOptions"
options_field = "visitor"
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "ConversionOptionsUpdate".to_string(),
                rust_path: "my_lib::ConversionOptionsUpdate".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "visitor".to_string(),
                    ty: TypeRef::Named("VisitorHandle".to_string()),
                    optional: true,
                    serde_skip: true,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "ConversionOptions".to_string(),
                rust_path: "my_lib::ConversionOptions".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "visitor".to_string(),
                    ty: TypeRef::Named("VisitorHandle".to_string()),
                    optional: true,
                    serde_skip: true,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    };

    let files = FfiBackend
        .generate_bindings(&api, &config)
        .expect("serde-skip options-bridge reachability");
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("fn htm_conversion_options_update_visitor"),
        "ConversionOptionsUpdate.visitor has no from_json path and no matching options-bridge \
         entry -- its getter must not be emitted:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("fn htm_conversion_options_visitor"),
        "ConversionOptions.visitor IS reachable via the configured options-field bridge \
         (resolved field name \"visitor\", even though param_name is \"watcher\") -- \
         suppressing its getter would be an over-suppression regression:\n{}",
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
            cfg: None,
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
    assert!(
        lib.content
            .contains("fn syn_options_set_renderer(options: AlefHandle, visitor: AlefHandle)"),
        "options-field setter must use the public scalar managed-handle ABI"
    );
    assert!(
        !lib.content.contains("visitor: *mut SynSyntaxWalkerBridge"),
        "options-field setter must not require the trait-bridge handle when visitor_callbacks is enabled"
    );
    assert!(
        lib.content.contains("options: AlefHandle") && lib.content.contains(") -> AlefHandle"),
        "options-field wrapper parameters and results must use scalar managed handles"
    );
    assert!(
        lib.content.contains("with_handle::<my_lib::ParseOptions")
            && lib.content.contains("with_handle_mut::<SynVisitor")
            && lib.content.contains("insert_handle(result)"),
        "options-field wrapper must resolve every managed value through the handle registry"
    );
    syn::parse_file(&lib.content).expect("scalar options-field bridge output must parse as Rust");
    assert!(
        !lib.content.contains("SynSyntaxWalkerBridge"),
        "legacy visitor callbacks must not ship an unattached generic bridge with an independent destructor"
    );
    assert!(!lib.content.contains("syn_syntax_walker_bridge_new"));
    assert!(!lib.content.contains("syn_syntax_walker_bridge_free"));
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
        lib.content.contains("settings: AlefHandle"),
        "must carry the configured options type through the managed handle ABI"
    );
    assert!(
        lib.content.contains(") -> AlefHandle"),
        "must carry the actual return type through the managed handle ABI"
    );
    assert!(
        lib.content.contains("with_handle::<my_lib::RenderSettings") && lib.content.contains("insert_handle(result)"),
        "must resolve options and register results through the handle registry"
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
    // ~keep Every failure path of a bridge returning `AlefHandle` must yield the scalar
    // sentinel 0. `catch_ffi_panic(0, ..)` and the terminal arms were migrated to the scalar
    // ABI, but the null-parameter guard and the UTF-8 guard still emitted
    // `std::ptr::null_mut()` -- a `*mut` where a `u64` is expected, so the generated crate did
    // not compile at all (E0308). It reached h2m's committed ffi crate that way.
    //
    // `rfind`, not `find`: the backend emits a "Not implemented" stub for this symbol BEFORE
    // the real bridge, and the stub body contains no sentinel at all. Anchoring on the first
    // match slices the stub and the check passes no matter what the bridge emits -- verified,
    // that is exactly how the first version of this assertion passed against the bug it was
    // written to catch. The positive assertions below keep an incorrectly anchored slice loud.
    let definition = "pub unsafe extern \"C\" fn doc_render_document(";
    let start = lib
        .content
        .rfind(definition)
        .expect("options-field bridge definition must exist");
    let after = &lib.content[start..];
    let bridge_body = after.split_once("\npub ").map_or(after, |(body, _)| body);
    assert!(
        bridge_body.contains("is_null()") && bridge_body.contains("catch_ffi_panic(0"),
        "slice must cover the real bridge body, or the sentinel check below is vacuous: {bridge_body}"
    );
    assert!(
        !bridge_body.contains("null_mut"),
        "AlefHandle bridge must return the scalar sentinel on every failure path: {bridge_body}"
    );

    syn::parse_file(&lib.content).expect("generic scalar options-field bridge output must parse as Rust");
}

/// Regression: a field marked `binding_excluded` (e.g. a global `[crates.exclude].fields`
/// entry hiding a pipeline-invariant field of a foreign `source_crate` type) must NOT get a
/// generated FFI accessor. Previously the FFI backend filtered only on `sanitized`, so an
/// excluded field still emitted a getter — and a name-colliding foreign type (h2m
/// `OutputFormat` vs host `OutputFormat`) made that getter fail to compile.
#[test]
fn test_binding_excluded_field_emits_no_accessor() {
    let backend = FfiBackend;
    let config = sample_config();

    let baseline = backend
        .generate_bindings(&sample_api(), &config)
        .unwrap()
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    assert!(
        baseline.contains("_verbose("),
        "baseline should emit a `verbose` accessor"
    );

    let mut api = sample_api();
    let verbose = api.types[0].fields.iter_mut().find(|f| f.name == "verbose").unwrap();
    verbose.binding_excluded = true;
    verbose.binding_exclusion_reason = Some("exclude.fields".to_string());

    let excluded = backend
        .generate_bindings(&api, &config)
        .unwrap()
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    assert!(
        !excluded.contains("_verbose("),
        "excluded field must not emit an accessor, got:\n{excluded}"
    );
    assert!(
        excluded.contains("_name("),
        "sibling non-excluded fields must still emit accessors"
    );
}

/// `optional_leaf_needs_presence_signal` decides which leaf types collapse `None` into the
/// same sentinel a legitimate `Some` can also produce -- the exact defect a `has_<field>`
/// companion getter exists to fix. Table-driven over every primitive plus `Duration`
/// (ambiguous: the C ABI has no null for a number, so both need a presence companion) and every
/// pointer/handle-shaped leaf (already distinguishable via a real null pointer or a
/// reserved-zero handle -- `insert_handle` never allocates handle `0` -- so no companion is
/// needed). Also covers the nested `Option<Option<Primitive>>` update-struct shape, whose outer
/// getter still emits one sentinel for both `None` and `Some(None)`. ~keep
#[test]
fn presence_accessor_predicate_covers_every_optional_leaf_shape() {
    let ambiguous_cases = [
        ("f32", TypeRef::Primitive(PrimitiveType::F32)),
        ("f64", TypeRef::Primitive(PrimitiveType::F64)),
        ("i32", TypeRef::Primitive(PrimitiveType::I32)),
        ("u64", TypeRef::Primitive(PrimitiveType::U64)),
        ("bool", TypeRef::Primitive(PrimitiveType::Bool)),
        ("usize", TypeRef::Primitive(PrimitiveType::Usize)),
        ("Duration", TypeRef::Duration),
        (
            "nested Option<Option<Primitive>>",
            TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
        ),
    ];
    for (label, ty) in ambiguous_cases {
        assert!(
            optional_leaf_needs_presence_signal(&ty),
            "{label} getter has no null representation and must require a presence companion"
        );
    }

    let distinguishable_cases = [
        ("String", TypeRef::String),
        ("Named", TypeRef::Named("Child".to_string())),
        ("Bytes", TypeRef::Bytes),
        ("Path", TypeRef::Path),
        ("Json", TypeRef::Json),
        ("Char", TypeRef::Char),
    ];
    for (label, ty) in distinguishable_cases {
        assert!(
            !optional_leaf_needs_presence_signal(&ty),
            "{label} getter already returns a real null/reserved-zero sentinel and must not get a presence companion"
        );
    }
}

/// Wiring test: `gen_lib_rs`, invoked through the real `Backend::generate_bindings` entry point
/// (not a direct helper call), must emit a `has_<field>` companion for every optional field whose
/// leaf type is ambiguous, and must NOT emit one for a field that already has a real null. This
/// proves the gate in `lib_rs.rs`'s field loop actually reaches every shape it claims to, for
/// every scalar kind named in the consumer's report -- not just floats. ~keep
#[test]
fn generated_bindings_emit_presence_companion_for_every_ambiguous_optional_field_only() {
    let fields = vec![
        FieldDef {
            name: "margin_fraction".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::F64),
            optional: true,
            ..FieldDef::default()
        },
        FieldDef {
            name: "retry_count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::I32),
            optional: true,
            ..FieldDef::default()
        },
        FieldDef {
            name: "max_bytes".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U64),
            optional: true,
            ..FieldDef::default()
        },
        FieldDef {
            name: "enabled".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            optional: true,
            ..FieldDef::default()
        },
        FieldDef {
            name: "timeout".to_string(),
            ty: TypeRef::Duration,
            optional: true,
            ..FieldDef::default()
        },
        FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            optional: true,
            ..FieldDef::default()
        },
    ];
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "SampleConfig".to_string(),
            rust_path: "my_lib::SampleConfig".to_string(),
            fields,
            is_clone: true,
            has_serde: true,
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "smp"
"#,
    );
    let backend = FfiBackend;
    let lib = backend
        .generate_bindings(&api, &config)
        .unwrap()
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .unwrap()
        .content;

    for field_name in ["margin_fraction", "retry_count", "max_bytes", "enabled", "timeout"] {
        let has_fn_signature = format!("fn smp_sample_config_has_{field_name}(handle: AlefHandle) -> i32");
        assert!(
            lib.contains(&has_fn_signature),
            "expected presence companion `{has_fn_signature}` for ambiguous optional field, got:\n{lib}"
        );
        assert!(
            lib.contains(&format!("obj.{field_name}.is_some() as i32")),
            "presence companion for `{field_name}` must check obj.{field_name}.is_some(), got:\n{lib}"
        );
    }

    assert!(
        !lib.contains("smp_sample_config_has_label"),
        "String field already returns a real null on None and must not get a presence companion, got:\n{lib}"
    );
}

/// Compiles and runs the ACTUAL generated `has_<field>` companion and its sibling getter
/// together, proving at runtime -- not just by string-matching the rendered source -- that
/// `None` and a zero-valued `Some` are distinguishable for every scalar leaf named in the
/// consumer's report (f64, i32, u64, bool, Duration). Round-trips BOTH directions: the getter's
/// own return value collapses `None` and `Some(zero)` to the identical sentinel (asserted first,
/// to prove the underlying ambiguity the presence companion exists to resolve), while the
/// companion distinguishes them via `is_some()` in both the absent (`0`) and present (`1`) case.
/// ~keep
#[test]
fn presence_companion_distinguishes_none_from_zero_valued_some_at_runtime() {
    let field = |name: &str, ty: TypeRef| FieldDef {
        name: name.to_string(),
        ty,
        optional: true,
        ..FieldDef::default()
    };
    let fields = vec![
        field("margin_fraction", TypeRef::Primitive(PrimitiveType::F64)),
        field("retry_count", TypeRef::Primitive(PrimitiveType::I32)),
        field("max_bytes", TypeRef::Primitive(PrimitiveType::U64)),
        field("enabled", TypeRef::Primitive(PrimitiveType::Bool)),
        field("timeout", TypeRef::Duration),
    ];
    let typ = TypeDef {
        name: "SampleConfig".to_string(),
        rust_path: "sample_core::SampleConfig".to_string(),
        fields: fields.clone(),
        ..TypeDef::default()
    };

    let mut generated = String::new();
    for f in &fields {
        generated.push_str(
            &gen_field_accessor(
                &typ,
                f,
                "smp",
                "sample_core",
                &ahash::AHashMap::new(),
                &ahash::AHashSet::new(),
                &ahash::AHashSet::new(),
                &std::collections::HashMap::new(),
            )
            .expect("field accessor"),
        );
        generated.push('\n');
        generated.push_str(&gen_field_presence_accessor(&typ, f, "smp", "sample_core"));
        generated.push('\n');
    }

    let last_error = crate::backends::ffi::template_env::render(
        "last_error.jinja",
        minijinja::context! {
            prefix => "smp",
            builtin_prefix => "",
            error_code_impls => Vec::<String>::new(),
            has_error_code_impls => false,
            taxonomy => Vec::<String>::new(),
            no_error_code => 0,
            conversion_error_code => 1,
            unknown_error_code => 2,
            panic_error_code => 3,
            invalid_handle_error_code => 4,
        },
    );
    // `insert_serialized_handle` pulls in `serde`/`serde_json`, unneeded by this harness (none of
    // these fields are serialized handles) and unavailable to a bare `rustc` invocation with no
    // Cargo dependency graph. Strip it, matching the same excision `handle_registry.rs`'s own
    // compile-and-run regression test uses for the identical reason.
    let mut handle_registry =
        crate::backends::ffi::template_env::render("handle_registry.rs.jinja", minijinja::context! {});
    let serialized_start = handle_registry
        .find("struct SerializedHandle")
        .expect("serialized helper start");
    let core_registry_resume = handle_registry[serialized_start..]
        .find("fn with_handle")
        .map(|offset| serialized_start + offset)
        .expect("core registry helpers resume");
    handle_registry.replace_range(serialized_start..core_registry_resume, "");

    let source = format!(
        r#"
use std::cell::RefCell;
use std::ffi::{{c_char, CString}};

mod sample_core {{
    #[derive(Clone, Default)]
    pub struct SampleConfig {{
        pub margin_fraction: Option<f64>,
        pub retry_count: Option<i32>,
        pub max_bytes: Option<u64>,
        pub enabled: Option<bool>,
        pub timeout: Option<std::time::Duration>,
    }}
}}

{last_error}
{handle_registry}
{generated}

fn main() {{
  unsafe {{
    let absent = insert_handle(sample_core::SampleConfig::default()).expect("insert absent");
    let present_zero = insert_handle(sample_core::SampleConfig {{
        margin_fraction: Some(0.0),
        retry_count: Some(0),
        max_bytes: Some(0),
        enabled: Some(false),
        timeout: Some(std::time::Duration::ZERO),
    }})
    .expect("insert present-zero");

    // Direction 1: the getter alone cannot tell `None` from `Some(zero)` -- both collapse to
    // the same sentinel. This is the defect being guarded against, asserted here so a future
    // regression that removes the presence companion's *reason to exist* is still caught.
    assert_eq!(smp_sample_config_margin_fraction(absent), 0.0);
    assert_eq!(smp_sample_config_margin_fraction(present_zero), 0.0);
    assert_eq!(smp_sample_config_retry_count(absent), 0);
    assert_eq!(smp_sample_config_retry_count(present_zero), 0);
    assert_eq!(smp_sample_config_max_bytes(absent), 0);
    assert_eq!(smp_sample_config_max_bytes(present_zero), 0);
    assert_eq!(smp_sample_config_enabled(absent), 0);
    assert_eq!(smp_sample_config_enabled(present_zero), 0);
    assert_eq!(smp_sample_config_timeout(absent), 0);
    assert_eq!(smp_sample_config_timeout(present_zero), 0);

    // Direction 2: the presence companion distinguishes them, in both the absent (0) and
    // present (1) case, for every scalar leaf.
    assert_eq!(smp_sample_config_has_margin_fraction(absent), 0);
    assert_eq!(smp_sample_config_has_margin_fraction(present_zero), 1);
    assert_eq!(smp_sample_config_has_retry_count(absent), 0);
    assert_eq!(smp_sample_config_has_retry_count(present_zero), 1);
    assert_eq!(smp_sample_config_has_max_bytes(absent), 0);
    assert_eq!(smp_sample_config_has_max_bytes(present_zero), 1);
    assert_eq!(smp_sample_config_has_enabled(absent), 0);
    assert_eq!(smp_sample_config_has_enabled(present_zero), 1);
    assert_eq!(smp_sample_config_has_timeout(absent), 0);
    assert_eq!(smp_sample_config_has_timeout(present_zero), 1);

    // An invalid handle reports -1 on the presence channel too, distinct from both 0 and 1.
    assert_eq!(smp_sample_config_has_margin_fraction(0), -1);
  }}
}}
"#
    );

    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("presence_companion.rs");
    let binary_path = directory.path().join("presence-companion-test");
    std::fs::write(&source_path, &source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(
        compile.status.success(),
        "{}\n---source---\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(&binary_path)
        .current_dir(directory.path())
        .output()
        .expect("run compiled harness");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
}
