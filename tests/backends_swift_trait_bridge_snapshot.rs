use alef::backends::swift::gen_bindings::trait_bridge::gen_trait_bridge_files;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::{MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
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

fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        param_name: None,
        type_alias: None,
        exclude_languages: vec![],
        super_trait: None,
        registry_getter: None,
        register_fn: Some(format!("register{}", trait_name)),
        unregister_fn: None,
        clear_fn: None,
        register_extra_args: None,
        bind_via: BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }
}

#[test]
fn test_trait_bridge_sync_method() {
    let trait_def = make_trait_def(
        "DocumentExtractor",
        vec![make_method(
            "extract",
            vec![
                make_param("bytes", TypeRef::Bytes),
                make_param("mime_type", TypeRef::String),
            ],
            TypeRef::String,
            false,
        )],
    );

    let bridge_cfg = make_bridge_cfg("DocumentExtractor");
    let bridges = vec![("DocumentExtractor".to_string(), &bridge_cfg, &trait_def)];
    let files = gen_trait_bridge_files(
        &bridges,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    // Two files now: the `SwiftPluginBridge.swift` super-protocol and the
    // per-trait bridge file (commit 23a58ff9e — drop async from trait bridge).
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, "SwiftPluginBridge.swift");
    let (filename, content) = &files[1];

    assert_eq!(filename, "SwiftDocumentExtractorBridge.swift");
    assert!(content.contains("protocol SwiftDocumentExtractorBridge"));
    assert!(content.contains("func extract"));
    // Registration function is emitted by `emit_trait_bridge_forwarders` in the
    // binding-level module file, not here — see commit 896eca93e.
    assert!(!content.contains("public func registerDocumentExtractor"));
}

#[test]
fn test_trait_bridge_async_method() {
    let trait_def = make_trait_def(
        "OcrBackend",
        vec![make_method(
            "recognize",
            vec![make_param("image_bytes", TypeRef::Bytes)],
            TypeRef::String,
            true,
        )],
    );

    let bridge_cfg = make_bridge_cfg("OcrBackend");
    let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
    let files = gen_trait_bridge_files(
        &bridges,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, "SwiftPluginBridge.swift");
    let (filename, content) = &files[1];

    assert_eq!(filename, "SwiftOcrBackendBridge.swift");
    assert!(content.contains("protocol SwiftOcrBackendBridge"));
    // Per commit 23a58ff9e ("drop async from trait bridge"), async trait methods
    // now emit as plain `throws` in the Swift protocol — host implementations
    // bridge async/non-async at their own boundary.
    assert!(content.contains("throws"));
    assert!(content.contains("func recognize"));
}

#[test]
fn test_trait_bridge_multiple_methods() {
    let trait_def = make_trait_def(
        "PostProcessor",
        vec![
            make_method(
                "process",
                vec![make_param("text", TypeRef::String)],
                TypeRef::String,
                false,
            ),
            make_method(
                "validate",
                vec![make_param("text", TypeRef::String)],
                TypeRef::String,
                false,
            ),
        ],
    );

    let bridge_cfg = make_bridge_cfg("PostProcessor");
    let bridges = vec![("PostProcessor".to_string(), &bridge_cfg, &trait_def)];
    let files = gen_trait_bridge_files(
        &bridges,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, "SwiftPluginBridge.swift");
    let (filename, content) = &files[1];

    assert_eq!(filename, "SwiftPostProcessorBridge.swift");
    assert!(content.contains("func process"));
    assert!(content.contains("func validate"));
    assert!(content.matches("func").count() >= 2); // At least process and validate
}

#[test]
fn test_trait_bridge_excludes_swift() {
    let trait_def = make_trait_def(
        "OcrBackend",
        vec![make_method("recognize", vec![], TypeRef::String, false)],
    );

    let mut bridge_cfg = make_bridge_cfg("OcrBackend");
    bridge_cfg.exclude_languages = vec!["swift".to_string()];

    let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
    let files = gen_trait_bridge_files(
        &bridges,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert!(files.is_empty());
}

#[test]
fn test_trait_bridge_skips_options_field() {
    let trait_def = make_trait_def(
        "OcrBackend",
        vec![make_method("recognize", vec![], TypeRef::String, false)],
    );

    let mut bridge_cfg = make_bridge_cfg("OcrBackend");
    bridge_cfg.bind_via = BridgeBinding::OptionsField;

    let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
    let files = gen_trait_bridge_files(
        &bridges,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    // OptionsField bridges are handled by inbound plugin codegen, not outbound
    assert!(files.is_empty());
}

#[test]
fn test_trait_bridge_primitive_params() {
    let trait_def = make_trait_def(
        "Renderer",
        vec![make_method(
            "render",
            vec![
                make_param("count", TypeRef::Primitive(PrimitiveType::I32)),
                make_param("enabled", TypeRef::Primitive(PrimitiveType::Bool)),
            ],
            TypeRef::String,
            false,
        )],
    );

    let bridge_cfg = make_bridge_cfg("Renderer");
    let bridges = vec![("Renderer".to_string(), &bridge_cfg, &trait_def)];
    let files = gen_trait_bridge_files(
        &bridges,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );

    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, "SwiftPluginBridge.swift");
    let (_filename, content) = &files[1];

    // Check that primitive types are properly declared in method signature
    assert!(content.contains("count: Int32"));
    assert!(content.contains("enabled: Bool"));
}

#[test]
fn test_trait_bridge_excluded_type_return() {
    let trait_def = make_trait_def(
        "OcrBackend",
        vec![make_method(
            "process",
            vec![make_param("image_bytes", TypeRef::Bytes)],
            TypeRef::Named("ExtractionResult".to_string()),
            true,
        )],
    );

    let bridge_cfg = make_bridge_cfg("OcrBackend");
    let mut exclude_types = std::collections::HashSet::new();
    exclude_types.insert("ExtractionResult".to_string());

    let bridges = vec![("OcrBackend".to_string(), &bridge_cfg, &trait_def)];
    let files = gen_trait_bridge_files(&bridges, &exclude_types, &std::collections::HashSet::new());

    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, "SwiftPluginBridge.swift");
    let (_filename, content) = &files[1];

    // Protocol marshals an excluded return type as a JSON String — the native
    // struct is not visible to the Swift side, so the conformer returns JSON.
    // Per commit 23a58ff9e the async keyword is no longer emitted; the trait
    // method shape is now plain `throws`.
    assert!(content.contains("func process(imageBytes: Data) throws -> String"));

    // Adapter method should return String (JSON envelope)
    assert!(content.contains("func processCall(imageBytes: Data) throws -> String"));

    // The marshal_encode_excluded helper should be present
    assert!(content.contains("marshal_encode_excluded"));
    assert!(content.contains("func marshal_encode_excluded<T: Encodable>"));

    // Verify that the body uses the new helper to encode excluded types
    assert!(content.contains("try marshal_encode_excluded(result)"));
}
