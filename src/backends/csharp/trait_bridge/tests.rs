use super::*;

fn make_trait_def(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{}", name),
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

fn make_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        param_name: None,
        type_alias: None,
        exclude_languages: vec![],
        super_trait: super_trait.map(|s| s.to_string()),
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        register_extra_args: None,
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
        ffi_skip_methods: Vec::new(),
    }
}

#[test]
fn test_interface_contains_lifecycle_when_super_trait_set() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", Some("Plugin"));
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public interface ITextBackend"));
    assert!(content.contains("string Name { get; }"));
    assert!(content.contains("string Version { get; }"));
    assert!(content.contains("void Initialize();"));
    assert!(content.contains("void Shutdown();"));
}

#[test]
fn test_interface_omits_lifecycle_when_super_trait_empty() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public interface ITextBackend"));
    assert!(!content.contains("string Name { get; }"));
}

#[test]
fn test_bridge_class_exists() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public sealed class TextBackendBridge : IDisposable"));
    assert!(content.contains("private delegate void FreeStringFn(IntPtr ptr);"));
    assert!(content.contains("FreeStringCallback"));
    assert!(content.contains("Marshal.FreeCoTaskMem(ptr);"));
}

#[test]
fn test_bool_callback_param_uses_int_boundary_type() {
    let mut trait_def = make_trait_def("Checker");
    trait_def.methods = vec![crate::core::ir::MethodDef {
        name: "check".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "enabled".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
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
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }];
    let bridge_cfg = make_bridge_cfg("Checker", None);
    let bridges = vec![("Checker".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["Checker"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("private delegate int CheckFn(IntPtr userData, int enabled);"));
}

#[test]
fn test_registry_no_super_trait_requires_explicit_name_param() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static class TextBackendRegistry"));
    assert!(content.contains("public static IntPtr Register(ITextBackend impl, string name)"));
    assert!(!content.contains("public static void Unregister(string name)"));
    assert!(!content.contains("impl.Name"));
}

#[test]
fn test_registry_with_super_trait_reads_name_from_impl() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", Some("Plugin"));
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static class TextBackendRegistry"));
    assert!(content.contains("public static IntPtr Register(ITextBackend impl)"));
    assert!(!content.contains("Register(ITextBackend impl, string name)"));
    assert!(content.contains("impl.Name"));
}

#[test]
fn test_exclude_languages_skips_csharp() {
    let trait_def = make_trait_def("TextBackend");
    let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
    bridge_cfg.exclude_languages = vec!["csharp".to_string()];
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(!content.contains("interface ITextBackend"));
    assert!(!content.contains("class TextBackendBridge"));
}

#[test]
fn test_native_methods_declarations_without_unregister() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("RegisterTextBackend"));
    assert!(!content.contains("UnregisterTextBackend"));
    assert!(content.contains("[DllImport"));
    assert!(content.contains("sample_crate_register_text_backend"));
    assert!(!content.contains("sample_crate_unregister_text_backend"));
}

#[test]
fn test_native_methods_declarations_with_configured_unregister() {
    let trait_def = make_trait_def("TextBackend");
    let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
    bridge_cfg.register_fn = Some("sample_crate_register_text_backend".to_string());
    bridge_cfg.unregister_fn = Some("sample_crate_unregister_text_backend".to_string());
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("RegisterTextBackend"));
    assert!(content.contains("UnregisterTextBackend"));
    assert!(content.contains("[DllImport"));
    assert!(content.contains("sample_crate_register_text_backend"));
    assert!(content.contains("sample_crate_unregister_text_backend"));
}

#[test]
fn test_native_methods_register_unregister_use_derived_ffi_symbol_not_alias() {
    let trait_def = make_trait_def("Renderer");
    let mut bridge_cfg = make_bridge_cfg("Renderer", None);
    bridge_cfg.register_fn = Some("register_renderer".to_string());
    bridge_cfg.unregister_fn = Some("unregister_renderer".to_string());
    bridge_cfg.clear_fn = Some("clear_renderers".to_string());
    let bridges = vec![("Renderer".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["Renderer"].into_iter().collect();
    let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("EntryPoint = \"sample_crate_register_renderer\""));
    assert!(content.contains("EntryPoint = \"sample_crate_unregister_renderer\""));
    assert!(content.contains("EntryPoint = \"sample_crate_clear_renderer\""));
    assert!(!content.contains("EntryPoint = \"register_renderer\""));
    assert!(!content.contains("EntryPoint = \"unregister_renderer\""));
    assert!(!content.contains("EntryPoint = \"clear_renderers\""));
    assert!(!content.contains("sample_crate_clear_renderers"));
}

#[test]
fn test_native_methods_clear_uses_derived_ffi_symbol_not_alias() {
    let trait_def = make_trait_def("TextBackend");
    let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
    bridge_cfg.clear_fn = Some("clear_text_backends".to_string());
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("EntryPoint = \"sample_crate_clear_text_backend\""));
    assert!(!content.contains("sample_crate_clear_text_backends"));
    assert!(!content.contains("EntryPoint = \"clear_text_backends\""));
    assert!(content.contains("ClearTextBackend("));
}

#[test]
fn test_native_methods_omits_clear_when_not_configured() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let content = gen_native_methods_trait_bridges("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(!content.contains("sample_crate_clear_text_backend"));
    assert!(!content.contains("ClearTextBackend("));
}

#[test]
fn test_registry_emits_clear_when_configured() {
    let trait_def = make_trait_def("TextBackend");
    let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
    bridge_cfg.clear_fn = Some("clear_text_backends".to_string());
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static void Clear()"));
    assert!(content.contains("NativeMethods.ClearTextBackend(out var outError)"));
}

#[test]
fn test_registry_omits_clear_when_not_configured() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(!content.contains("NativeMethods.ClearTextBackend("));
}

#[test]
fn test_registry_emits_unregister_when_configured() {
    let trait_def = make_trait_def("TextBackend");
    let mut bridge_cfg = make_bridge_cfg("TextBackend", None);
    bridge_cfg.unregister_fn = Some("sample_crate_unregister_text_backend".to_string());
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static class TextBackendRegistry"));
    assert!(content.contains("public static void Unregister(string name)"));
    assert!(content.contains("NativeMethods.UnregisterTextBackend(name, out var outError)"));
}

#[test]
fn test_registry_omits_unregister_when_not_configured() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public static class TextBackendRegistry"));
    assert!(!content.contains("public static void Unregister(string name)"));
    assert!(!content.contains("NativeMethods.UnregisterTextBackend"));
}

/// Regression (#114): the `[UnmanagedFunctionPointer]` delegate type for a Bytes parameter
/// must include `UIntPtr {name}Len` immediately after the `IntPtr {name}` field.
/// The callback marshalling must use `Marshal.Copy(ptr, dst, 0, len)` rather than reading
/// bytes as a NUL-terminated JSON string, which silently truncates payloads containing 0x00.
#[test]
fn test_bridge_delegate_bytes_param_includes_len_companion() {
    let mut trait_def = make_trait_def("Processor");
    trait_def.methods.push(crate::core::ir::MethodDef {
        name: "ingest".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "payload".to_string(),
            ty: TypeRef::Bytes,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
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
    let bridge_cfg = make_bridge_cfg("Processor", None);
    let bridges = vec![("Processor".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["Processor"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(
        content.contains("UIntPtr payloadLen"),
        "delegate signature must include `UIntPtr payloadLen` for Bytes param;\nactual:\n{content}"
    );
    assert!(
        content.contains("Marshal.Copy(payload"),
        "callback must use Marshal.Copy for Bytes param;\nactual:\n{content}"
    );
    assert!(
        !content.contains("MarshalBytesFromIntPtr"),
        "callback must not use MarshalBytesFromIntPtr;\nactual:\n{content}"
    );
}

#[test]
fn test_bridge_class_has_register_static_method_with_super_trait() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", Some("Plugin"));
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public sealed class TextBackendBridge : IDisposable"));
    assert!(content.contains("public static IntPtr Register(ITextBackend impl)"));
    assert!(content.contains("var name = impl.Name;"));
}

#[test]
fn test_bridge_class_has_register_static_method_without_super_trait() {
    let trait_def = make_trait_def("TextBackend");
    let bridge_cfg = make_bridge_cfg("TextBackend", None);
    let bridges = vec![("TextBackend".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["TextBackend"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("public sealed class TextBackendBridge : IDisposable"));
    assert!(content.contains("public static IntPtr Register(ITextBackend impl, string name)"));
}

/// Regression: enum return types are visible in the interface, so the interface
/// declares the actual enum type. The callback receives the enum and must serialize it
/// using .ToFfiJson() extension method.
#[test]
fn test_trait_method_enum_return_uses_toffijson_serialization() {
    let mut trait_def = make_trait_def("PostProcessor");
    trait_def.methods.push(crate::core::ir::MethodDef {
        name: "processing_stage".to_string(),
        params: vec![],
        return_type: TypeRef::Named("ProcessingStage".to_string()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
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
    let bridge_cfg = make_bridge_cfg("PostProcessor", Some("Plugin"));
    let bridges = vec![("PostProcessor".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = vec!["PostProcessor", "ProcessingStage"].into_iter().collect();
    let (_filename, content) = gen_trait_bridges_file("SampleCrate", "sample_crate", &bridges, &visible_types);

    assert!(content.contains("ProcessingStage ProcessingStage { get; }"));
    assert!(content.contains("methodResult.ToFfiJson()"));
    assert!(!content.contains("ToJsonString(methodResult)"));
}

#[test]
fn bridge_adapter_implements_hand_authored_interface_for_text_processor() {
    let trait_def = make_trait_def("TextProcessor");
    let bridge_cfg = make_bridge_cfg("TextProcessor", Some("Plugin"));
    let bridges = vec![("TextProcessor".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = HashSet::new();
    let result = gen_bridge_adapters_file("SampleCrate", &bridges, &visible_types);
    let (filename, content) = result.expect("gen_bridge_adapters_file should return Some for non-empty bridges");

    assert_eq!(filename, "BridgeAdapters.cs");
    assert!(
        content.contains("sealed class _TextProcessorBridgeAdapter : ITextProcessor"),
        "adapter must declare conformance to ITextProcessor;\nactual:\n{content}"
    );
}

#[test]
fn bridge_adapter_delegates_to_inner_impl_for_asset_loader() {
    let trait_def = make_trait_def("AssetLoader");
    let bridge_cfg = make_bridge_cfg("AssetLoader", Some("Plugin"));
    let bridges = vec![("AssetLoader".to_string(), &bridge_cfg, &trait_def)];
    let visible_types: HashSet<&str> = HashSet::new();
    let result = gen_bridge_adapters_file("SampleCrate", &bridges, &visible_types);
    let (filename, content) = result.expect("gen_bridge_adapters_file must return Some");

    assert_eq!(filename, "BridgeAdapters.cs");
    assert!(
        content.contains("sealed class _AssetLoaderBridgeAdapter : IAssetLoader"),
        "adapter must declare conformance to IAssetLoader;\nactual:\n{content}"
    );
}
