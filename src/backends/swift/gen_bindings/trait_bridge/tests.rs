use super::*;
use crate::core::config::BridgeBinding;

fn make_trait_def(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("testcrate::{}", name),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: true,
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
    }
}

fn make_bridge_cfg(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        exclude_functions: Vec::new(),
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
fn test_trait_bridge_protocol_generated() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend");
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let exclude_types = HashSet::new();
    let files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());

    assert_eq!(files.len(), 2);
    assert_eq!(files[0].0, "SwiftPluginBridge.swift");
    assert!(files[0].1.contains("protocol SwiftPluginBridge: AnyObject"));
    assert_eq!(files[1].0, "SwiftTextBackendBridge.swift");
    assert!(
        files[1]
            .1
            .contains("protocol SwiftTextBackendBridge: SwiftPluginBridge")
    );
}

#[test]
fn test_trait_bridge_excludes_swift_language() {
    let trait_def = make_trait_def("TextBackend");
    let mut bridge_cfg = make_bridge_cfg("TextBackend");
    bridge_cfg.exclude_languages = vec!["swift".to_string()];
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let exclude_types = HashSet::new();
    let files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());

    assert!(files.is_empty());
}

#[test]
fn test_trait_bridge_skips_non_function_param() {
    let trait_def = make_trait_def("TextBackend");
    let mut bridge_cfg = make_bridge_cfg("TextBackend");
    bridge_cfg.bind_via = BridgeBinding::OptionsField;
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let exclude_types = HashSet::new();
    let files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());

    assert!(files.is_empty());
}

#[test]
fn test_swift_type_mapping() {
    use crate::core::ir::PrimitiveType;
    let exclude_types = HashSet::new();
    assert_eq!(swift_type_name(&TypeRef::String, &exclude_types), "String");
    assert_eq!(swift_type_name(&TypeRef::Bytes, &exclude_types), "Data");
    assert_eq!(swift_type_name(&TypeRef::Unit, &exclude_types), "Void");
    assert_eq!(
        swift_type_name(&TypeRef::Primitive(PrimitiveType::I32), &exclude_types),
        "Int32"
    );
    assert_eq!(swift_type_name(&TypeRef::Duration, &exclude_types), "TimeInterval");
}

#[test]
fn test_swift_marshals_excluded_types_as_json() {
    let mut exclude_types = HashSet::new();
    exclude_types.insert("PrivatePayload".to_string());
    assert_eq!(
        swift_type_name(&TypeRef::Named("PrivatePayload".to_string()), &exclude_types),
        "String",
        "Excluded types should be marshalled as JSON strings"
    );
    assert_eq!(
        swift_type_name(&TypeRef::Named("VisibleResult".to_string()), &exclude_types),
        "VisibleResult",
        "Non-excluded types should keep their original names"
    );
}

/// alef-tasks #309: a `Map<_, Named>` protocol method previously declared `[String: String]`
/// whose values were themselves JSON payloads produced by `swift_shim_param_decode` /
/// `vec_element_crosses_as_string`'s sibling logic in `plugin_marshal.rs` -- JSON-encoding
/// that dictionary double-encodes every value. The whole `Map` must be one blob instead.
#[test]
fn test_swift_type_name_map_named_value_is_one_blob() {
    let mut exclude_types = HashSet::new();
    exclude_types.insert("SinkStats".to_string());
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Named("SinkStats".to_string())),
    );

    assert_eq!(
        swift_type_name(&ty, &exclude_types),
        "String",
        "Map<_, Named> must declare a single JSON String blob, not [String: String]"
    );
}

/// The one-blob rule for `Map` does not depend on the value being `Named`: swift-bridge has
/// no `Map`/`HashMap` bridging at all, so every `Map` is a blob, matching
/// `plugin_marshal::swift_shim_param_ffi_type`'s unconditional `Map(_, _) => "RustString"`.
#[test]
fn test_swift_type_name_map_primitive_value_is_also_one_blob() {
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U32)),
    );

    assert_eq!(swift_type_name(&ty, &HashSet::new()), "String");
}

/// alef-tasks #333: recursing normally would declare `[String]?` for `Optional<Vec<Named>>`,
/// matching the inbound (`extern "Swift"`) rule's per-element decision -- but this function
/// backs the *outbound* `{Trait}Box` FFI trampolines (`extern "Rust"`), and that boundary
/// cannot bridge `Option<Vec<T>>` at all (see the DTO-getter fix "bridge optional vectors
/// through JSON" for the identical limitation). The protocol must declare one blob, `String?`,
/// matching what `plugin_marshal::swift_shim_return_ffi_type` already declares (`RustString`)
/// and what `plugin_marshal::swift_shim_return_marshal` must now also produce.
#[test]
fn test_swift_type_name_optional_vec_named_is_one_blob() {
    let mut exclude_types = HashSet::new();
    exclude_types.insert("SinkStats".to_string());
    let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
        "SinkStats".to_string(),
    )))));

    assert_eq!(
        swift_type_name(&ty, &exclude_types),
        "String?",
        "Optional<Vec<Named>> must declare a single nilable JSON String blob, not [String]?"
    );
}

/// The one-blob rule for `Optional<Vec<Named>>` depends on the element actually needing JSON
/// bridging (i.e. being excluded). An `Optional<Vec<Named>>` whose element is NOT excluded
/// keeps recursing normally, matching `Vec<Named>`'s own visible-type recursion.
#[test]
fn test_swift_type_name_optional_vec_visible_named_still_recurses() {
    let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
        "SinkStats".to_string(),
    )))));

    assert_eq!(
        swift_type_name(&ty, &HashSet::new()),
        "[SinkStats]?",
        "a Vec<Named> element not in exclude_types keeps its native array recursion"
    );
}

#[test]
fn test_bridge_policy_derives_named_types_from_trait_methods() {
    use crate::core::ir::{MethodDef, ParamDef};

    let mut trait_def = make_trait_def("Processor");
    trait_def.methods.push(MethodDef {
        name: "process".to_string(),
        params: vec![ParamDef {
            name: "payload".to_string(),
            ty: TypeRef::Named("PrivatePayload".to_string()),
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Named("VisibleResult".to_string()),
        is_async: false,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: None,
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let policy = excluded_named_type_bridge_policy(&trait_def, &HashSet::new());

    assert!(policy.contains("PrivatePayload"));
    assert!(policy.contains("VisibleResult"));
    assert!(!policy.contains("UnmentionedResult"));
    assert!(!policy.contains("UnmentionedPayload"));
}

/// Verify that gen_single_trait_bridge_file does NOT emit a `public func register*`
/// function. Registration is handled by `emit_trait_bridge_forwarders` in `<Binding>.swift`
/// which uses the Box-based type (`SwiftDocumentExtractorBox`) that the RustBridge
/// `register_*` entry point actually expects. A duplicate here would pass the
/// incompatible `Adapter` type, causing a compile error.
#[test]
fn test_no_register_fn_in_trait_bridge_file() {
    let trait_def = make_trait_def("DocumentExtractor");
    let bridge_cfg = make_bridge_cfg("DocumentExtractor");
    let bridges = vec![("DocumentExtractor".to_string(), &bridge_cfg, &trait_def)];
    let exclude_types = HashSet::new();
    let files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());

    assert_eq!(files.len(), 2);
    let content = &files[1].1;

    assert!(
        content.contains("protocol SwiftDocumentExtractorBridge: SwiftPluginBridge"),
        "protocol must be emitted with SwiftPluginBridge inheritance, got:\n{content}"
    );
    assert!(content.contains("may be invoked concurrently and must synchronize mutable state"));
    assert!(
        content.contains("final class SwiftDocumentExtractorAdapter"),
        "adapter class must be emitted, got:\n{content}"
    );

    assert!(
        !content.contains("public func registerDocumentExtractor("),
        "register function must NOT be emitted in the bridge file (would use wrong Adapter type), got:\n{content}"
    );
}

/// Verify that when an excluded type appears in a trait method
/// signature, the protocol accepts the native type but the adapter marshals it as JSON String.
#[test]
fn test_excluded_type_in_method_becomes_string() {
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind};

    let mut trait_def = make_trait_def("DocumentExtractor");
    trait_def.methods.push(MethodDef {
        name: "extract_bytes".to_string(),
        params: vec![ParamDef {
            name: "content".to_string(),
            ty: TypeRef::Bytes,
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Named("PrivatePayload".to_string()),
        is_async: false,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let bridge_cfg = make_bridge_cfg("DocumentExtractor");
    let bridges = vec![("DocumentExtractor".to_string(), &bridge_cfg, &trait_def)];

    let mut exclude_types = HashSet::new();
    exclude_types.insert("PrivatePayload".to_string());

    let files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());
    assert_eq!(files.len(), 2);
    let content = &files[1].1;

    assert!(
        content.contains("func extractBytes(content: Data) throws -> String"),
        "protocol method must marshal excluded type to String, got:\n{content}"
    );

    assert!(
        content.contains("func extractBytesCall(content: Data) throws -> String"),
        "adapter method must marshal to String, got:\n{content}"
    );

    assert!(
        content.contains("marshal_encode_excluded"),
        "marshal_encode_excluded helper must be present, got:\n{content}"
    );
}

#[test]
fn test_bridge_registration_overloads_file() {
    let trait_def = make_trait_def("MyLib");
    let bridge_cfg = make_bridge_cfg("MyLib");
    let bridges = vec![("MyLib".to_string(), &bridge_cfg, &trait_def)];

    let result = gen_bridge_registration_overloads_file(&bridges);
    assert!(result.is_some(), "should generate file");

    let (filename, content) = result.unwrap();
    assert_eq!(filename, "BridgeRegistrationOverloads.swift");

    assert!(
        content.contains("// MARK: - Unregister name: label overloads"),
        "missing unregister overload section"
    );
    assert!(
        content.contains("public func unregisterMyLib(name: String) throws"),
        "missing unregister overload"
    );
    assert!(
        content.contains("try RustBridge.unregisterMyLib(name)"),
        "unregister label overload must delegate to the RustBridge function, not itself"
    );
    assert!(
        !content.contains("try unregisterMyLib(name)\n"),
        "unregister label overload must not recursively call itself"
    );

    assert!(
        content.contains("// MARK: - Bridge → Box register overloads"),
        "missing register overload section"
    );
    assert!(
        content.contains("public func registerMyLib(_ bridge: any SwiftMyLibBridge) throws"),
        "missing register overload"
    );

    // NOTE: adapter class, lifecycle stub methods (`name()`, `version()`,
}

#[test]
fn test_bridge_registration_overloads_empty_when_no_bridges() {
    let bridges: Vec<(String, &TraitBridgeConfig, &TypeDef)> = vec![];
    let result = gen_bridge_registration_overloads_file(&bridges);
    assert!(result.is_none(), "should not generate file when no bridges");
}

#[test]
fn test_bridge_registration_overloads_skips_excluded_language() {
    let trait_def = make_trait_def("MyLib");
    let mut bridge_cfg = make_bridge_cfg("MyLib");
    bridge_cfg.exclude_languages = vec!["swift".to_string()];
    let bridges = vec![("MyLib".to_string(), &bridge_cfg, &trait_def)];

    let result = gen_bridge_registration_overloads_file(&bridges);
    assert!(result.is_none(), "should skip bridges excluded from swift");
}

// NOTE: previously asserted that async trait methods produced async stubs in

#[test]
fn test_pascal_case_conversion() {
    assert_eq!(trait_bridge_pascal_name("my_lib"), "MyLib");
    assert_eq!(trait_bridge_pascal_name("text_backend"), "TextBackend");
    assert_eq!(trait_bridge_pascal_name("test"), "Test");
    assert_eq!(trait_bridge_pascal_name("a"), "A");
}

/// Regression test for B3: String return conversion in trait adapter success body.
/// When a method returns String, the RustString FFI result must be wrapped in String(...).
/// When a method returns Vec<String>, each element must be converted via .map { String($0) }.
#[test]
fn test_b3_string_return_conversion_trait_adapter() {
    use crate::core::ir::{MethodDef, ReceiverKind};

    let mut trait_def = make_trait_def("TextExtractor");

    trait_def.methods.push(MethodDef {
        name: "extract_text".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    trait_def.methods.push(MethodDef {
        name: "split_text".to_string(),
        params: vec![],
        return_type: TypeRef::Vec(Box::new(TypeRef::String)),
        is_async: false,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let bridge_cfg = make_bridge_cfg("TextExtractor");
    let bridges = vec![("TextExtractor".to_string(), &bridge_cfg, &trait_def)];
    let exclude_types = HashSet::new();

    let files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());
    assert_eq!(files.len(), 2);
    let content = &files[1].1;

    assert!(
        content.contains("marshal_ok_result(String(result))"),
        "String return must be wrapped in String(...) converter, got:\n{content}"
    );

    assert!(
        content.contains("marshal_ok_result(result.map { String($0) })"),
        "Vec<String> return must map each element, got:\n{content}"
    );
}

#[test]
fn text_processor_protocol_inherits_swift_plugin_bridge() {
    let trait_def = make_trait_def("TextProcessor");
    let bridge_cfg = make_bridge_cfg("TextProcessor");
    let bridges = vec![("TextProcessor".to_string(), &bridge_cfg, &trait_def)];
    let exclude_types = HashSet::new();
    let files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());

    let protocol_file = files
        .iter()
        .find(|(name, _)| name == "SwiftTextProcessorBridge.swift")
        .expect("SwiftTextProcessorBridge.swift must be emitted");

    assert!(
        protocol_file
            .1
            .contains("protocol SwiftTextProcessorBridge: SwiftPluginBridge"),
        "TextProcessor protocol must inherit SwiftPluginBridge;\nactual:\n{}",
        protocol_file.1
    );
}

/// The regression this guards against: a doc snippet declares `class Foo: Swift{Trait}Bridge`
/// in a file that only `import <Module>`s (never `import RustBridge` -- see
/// `crate::e2e::codegen::swift::snippet`, which renders exactly one `import {{ module }}` line
/// and no other). `gen_trait_bridge_files` emits the protocol into `Sources/RustBridge/`, so
/// that conformance only resolves if `gen_bridge_registration_overloads_file` -- which is
/// written into `Sources/<Module>/` -- re-exports the identical protocol name.
///
/// This is deliberately a relationship assertion, not a literal string pinned on one side: the
/// protocol name is read back out of `gen_trait_bridge_files`' own emitted filename (the
/// production authority for what the protocol is actually called) and only then checked against
/// the typealias `gen_bridge_registration_overloads_file` emits. A future rename that updates
/// one call site's naming logic but not the other must fail here, whereas two independent
/// literal-string tests could each keep passing while quietly drifting apart.
#[test]
fn umbrella_module_reexports_every_protocol_the_rustbridge_target_declares() {
    let trait_def = make_trait_def("EmbeddingBackend");
    let bridge_cfg = make_bridge_cfg("EmbeddingBackend");
    let bridges = vec![("EmbeddingBackend".to_string(), &bridge_cfg, &trait_def)];
    let exclude_types = HashSet::new();

    let rust_bridge_files = gen_trait_bridge_files(&bridges, &exclude_types, &HashSet::new());
    let protocol_filename = rust_bridge_files
        .iter()
        .map(|(name, _)| name.as_str())
        .find(|name| *name != "SwiftPluginBridge.swift")
        .expect("a trait-specific protocol file must be emitted alongside SwiftPluginBridge.swift");
    let protocol_name = protocol_filename
        .strip_suffix(".swift")
        .expect("RustBridge trait-bridge files are always named `<Protocol>.swift`");

    let (_, overloads_content) =
        gen_bridge_registration_overloads_file(&bridges).expect("registration overloads file must be emitted");

    assert_eq!(
        protocol_name, "SwiftEmbeddingBackendBridge",
        "sanity check on the naming helper both sides must agree on"
    );
    assert!(
        overloads_content.contains(&format!(
            "public typealias {protocol_name} = RustBridge.{protocol_name}"
        )),
        "Sources/<Module>/BridgeRegistrationOverloads.swift must re-export `{protocol_name}` \
         (declared in Sources/RustBridge/{protocol_filename}) so a snippet that only imports the \
         umbrella module can still conform to it; got:\n{overloads_content}"
    );
}

/// Companion to the relationship test above, proving the re-export is per-bridge rather than a
/// single hard-coded alias: two distinct trait bridges configured together must each get their
/// own re-export, keyed off their own protocol name.
#[test]
fn umbrella_module_reexports_multiple_distinct_bridge_protocols() {
    let extractor_def = make_trait_def("DocumentExtractor");
    let extractor_cfg = make_bridge_cfg("DocumentExtractor");
    let renderer_def = make_trait_def("Renderer");
    let renderer_cfg = make_bridge_cfg("Renderer");
    let bridges = vec![
        ("DocumentExtractor".to_string(), &extractor_cfg, &extractor_def),
        ("Renderer".to_string(), &renderer_cfg, &renderer_def),
    ];

    let (_, content) =
        gen_bridge_registration_overloads_file(&bridges).expect("registration overloads file must be emitted");

    assert!(
        content.contains("public typealias SwiftDocumentExtractorBridge = RustBridge.SwiftDocumentExtractorBridge"),
        "got:\n{content}"
    );
    assert!(
        content.contains("public typealias SwiftRendererBridge = RustBridge.SwiftRendererBridge"),
        "got:\n{content}"
    );
}

/// Negative control for the two re-export tests above: a bridge excluded from swift must not
/// leak a typealias for a protocol that was never emitted for swift in the first place.
#[test]
fn excluded_bridge_gets_no_protocol_reexport() {
    let trait_def = make_trait_def("MyLib");
    let mut bridge_cfg = make_bridge_cfg("MyLib");
    bridge_cfg.exclude_languages = vec!["swift".to_string()];
    let bridges = vec![("MyLib".to_string(), &bridge_cfg, &trait_def)];

    let result = gen_bridge_registration_overloads_file(&bridges);
    assert!(
        result.is_none(),
        "an all-excluded bridge list must not emit a file at all, let alone a typealias"
    );
}
