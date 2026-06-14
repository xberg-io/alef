use super::*;
use crate::core::ir::{MethodDef, ParamDef, PrimitiveType};

/// Build a `visible_type_names` set containing every `Named` type referenced
/// by the trait method's params or return type, so tests behave as if those
/// types are visible in the generated API.
fn all_named_visible(methods: &[MethodDef]) -> HashSet<&str> {
    fn collect<'a>(ty: &'a TypeRef, out: &mut HashSet<&'a str>) {
        match ty {
            TypeRef::Named(n) => {
                out.insert(n.as_str());
            }
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect(inner, out),
            TypeRef::Map(k, v) => {
                collect(k, out);
                collect(v, out);
            }
            _ => {}
        }
    }
    let mut set = HashSet::new();
    for m in methods {
        collect(&m.return_type, &mut set);
        for p in &m.params {
            collect(&p.ty, &mut set);
        }
    }
    set
}

fn make_method(name: &str, return_type: TypeRef, params: Vec<ParamDef>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: Some("Error".to_string()),
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
    }
}

fn make_trait(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_crate::{name}"),
        original_rust_path: format!("sample_crate::{name}"),
        fields: vec![],
        methods,
        is_opaque: true,
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
        version: Default::default(),
    }
}

#[test]
fn interface_emisample_package_and_lifecycle_when_super_trait() {
    let trait_def = make_trait("OcrBackend", vec![make_method("process", TypeRef::String, vec![])]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    assert!(files.interface_content.starts_with("package dev.sample_crate;"));
    assert!(files.interface_content.contains("public interface IOcrBackend"));
    assert!(files.interface_content.contains("String name();"));
    assert!(files.interface_content.contains("default void initialize()"));
    assert!(files.interface_content.contains("String process()"));
}

#[test]
fn interface_omits_lifecycle_when_no_super_trait() {
    let trait_def = make_trait("Filter", vec![make_method("apply", TypeRef::String, vec![])]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        false,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    assert!(!files.interface_content.contains("String name();"));
    assert!(files.interface_content.contains("String apply()"));
}

#[test]
fn bridge_class_has_register_helper_and_registry() {
    let trait_def = make_trait("OcrBackend", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    // No unregister/clear configured: neither method should appear
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    assert!(body.starts_with("package dev.sample_crate;"));
    assert!(body.contains("public final class OcrBackendBridge"));
    assert!(body.contains("public static void registerOcrBackend(final IOcrBackend impl)"));
    assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
    assert!(!body.contains("public static void clearAllOcrBackend()"));
    assert!(body.contains("ConcurrentHashMap<String, OcrBackendBridge>"));
    assert!(body.contains("OCR_BACKEND_BRIDGES = new ConcurrentHashMap<>()"));
    assert!(body.contains("KRZ_REGISTER_OCR_BACKEND"));
    assert!(body.contains("private void freeString(MemorySegment ptr)"));
    assert!(body.contains("FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)"));
}

#[test]
fn lifecycle_string_callbacks_use_status_and_out_error() {
    let trait_def = make_trait("OcrBackend", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();

    assert!(body.contains("int handleName(MemorySegment userData, MemorySegment outName, MemorySegment outError)"));
    assert!(
        body.contains("int handleVersion(MemorySegment userData, MemorySegment outVersion, MemorySegment outError)")
    );
    assert!(
        body.contains(
            "MethodType.methodType(int.class, MemorySegment.class, MemorySegment.class, MemorySegment.class)"
        )
    );
    assert!(body.contains(
        "FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)"
    ));
}

#[test]
fn gen_unregistration_fn_emits_method_when_configured() {
    let trait_def = make_trait("OcrBackend", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        Some("unregister_ocr_backend"),
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    assert!(body.contains("public static void unregisterOcrBackend(String name)"));
    assert!(body.contains("KRZ_UNREGISTER_OCR_BACKEND"));
    assert!(body.contains("OCR_BACKEND_BRIDGES.remove(name)"));
    assert!(!body.contains("public static void clearAllOcrBackend()"));
}

#[test]
fn gen_unregistration_fn_omits_method_when_none() {
    let trait_def = make_trait("OcrBackend", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
}

#[test]
fn gen_clear_fn_emits_method_when_configured() {
    let trait_def = make_trait("OcrBackend", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        None,
        Some("clear_ocr_backends"),
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    assert!(body.contains("public static void clearOcrBackends()"));
    assert!(body.contains("KRZ_CLEAR_OCR_BACKEND"));
    assert!(body.contains("OCR_BACKEND_BRIDGES.values().forEach(OcrBackendBridge::close)"));
    assert!(body.contains("OCR_BACKEND_BRIDGES.clear()"));
    assert!(!body.contains("public static void unregisterOcrBackend(String name)"));
}

#[test]
fn gen_clear_fn_omits_method_when_none() {
    let trait_def = make_trait("OcrBackend", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    assert!(!body.contains("public static void clearOcrBackends()"));
}

#[test]
fn both_unregister_and_clear_emitted_when_both_configured() {
    let trait_def = make_trait("OcrBackend", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        true,
        Some("unregister_ocr_backend"),
        Some("clear_ocr_backends"),
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    assert!(body.contains("public static void unregisterOcrBackend(String name)"));
    assert!(body.contains("public static void clearOcrBackends()"));
}

#[test]
fn java_param_name_sanitizes_keywords() {
    assert_eq!(java_param_name("default"), "default_");
    assert_eq!(java_param_name("config"), "config");
}

#[test]
fn bridge_class_does_not_json_quote_raw_string_results() {
    let trait_def = make_trait("Renderer", vec![make_method("render", TypeRef::String, vec![])]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        false,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();

    assert!(
        body.contains("MemorySegment jsonCs = arena.allocateFrom(result);"),
        "String callback results must be returned as raw UTF-8, got:\n{body}"
    );
    assert!(
        !body.contains("String json = JSON.writeValueAsString(result);"),
        "String callback results must not be JSON-quoted, got:\n{body}"
    );
}

#[test]
fn bridge_class_does_not_double_encode_excluded_named_json_results() {
    let trait_def = make_trait(
        "Renderer",
        vec![make_method(
            "render",
            TypeRef::Named("InternalDocument".to_string()),
            vec![],
        )],
    );
    let visible = HashSet::new();
    let excluded = HashSet::from(["InternalDocument".to_string()]);
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        false,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();

    assert!(
        body.contains("String result = impl.render();"),
        "excluded named return should surface as a raw JSON String, got:\n{body}"
    );
    assert!(
        body.contains("MemorySegment jsonCs = arena.allocateFrom(result);"),
        "excluded named JSON return must be passed through without writeValueAsString, got:\n{body}"
    );
}

#[test]
fn bridge_class_unmarshals_path_and_bytes() {
    let trait_def = make_trait(
        "OcrBackend",
        vec![make_method(
            "process_image",
            TypeRef::String,
            vec![
                ParamDef {
                    name: "image_bytes".to_string(),
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
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("OcrConfig".to_string()),
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
                },
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
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
                },
            ],
        )],
    );
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        false,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    assert!(body.contains("toArray(ValueLayout.JAVA_BYTE)"));
    assert!(body.contains("OcrConfig"));
    assert!(body.contains("Paths.get("));
}

/// Regression (#114): the Panama FFM handler signature for a Bytes parameter must include
/// a `long {name}Len` companion, and the unmarshal expression must use that length to
/// bound the MemorySegment read (`reinterpret(len)` not `reinterpret(Long.MAX_VALUE)`).
/// Without the companion parameter, embedded NUL bytes (0x00) in the payload cause the
/// callee to read past the end of the buffer.
#[test]
fn bridge_handler_bytes_param_includes_len_companion_and_bounded_reinterpret() {
    let trait_def = make_trait(
        "Processor",
        vec![make_method(
            "ingest",
            TypeRef::Unit,
            vec![crate::core::ir::ParamDef {
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
        )],
    );
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        false,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();

    // The handler method signature must carry the length companion.
    assert!(
        body.contains("long payloadLen"),
        "handler signature must include `long payloadLen` for Bytes param;\nactual:\n{body}"
    );
    // The unmarshal must use the bounded reinterpret(payloadLen), never Long.MAX_VALUE.
    assert!(
        body.contains("reinterpret(payloadLen)"),
        "Bytes unmarshal must use `reinterpret(payloadLen)`;\nactual:\n{body}"
    );
    assert!(
        !body.contains("Long.MAX_VALUE"),
        "Bytes unmarshal must not use `Long.MAX_VALUE` (unbounded read);\nactual:\n{body}"
    );
}

#[test]
fn bridge_handler_emits_primitive_param_as_java_primitive_not_memory_segment() {
    let trait_def = make_trait(
        "Logger",
        vec![make_method(
            "log",
            TypeRef::Unit,
            vec![
                ParamDef {
                    name: "level".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U8),
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
                },
                ParamDef {
                    name: "msg".to_string(),
                    ty: TypeRef::String,
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
                },
            ],
        )],
    );
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let files = gen_trait_bridge_files(
        &trait_def,
        "krz",
        "dev.sample_crate",
        false,
        None,
        None,
        &visible,
        &excluded,
        &[],
    );
    let body = files.bridge_content.as_str();
    // The handler signature should have `byte level`, not `MemorySegment level_in`
    assert!(body.contains(
        "private int handleLog(MemorySegment userData, byte level, MemorySegment msg_in, MemorySegment outError)"
    ));
}

#[test]
fn adapter_bridge_implements_hand_authored_interface_for_text_processor() {
    // Regression: `TextProcessorAdapter` must declare `implements ITextProcessor` so
    // consumer code can pass the adapter where the hand-authored interface is expected.
    let trait_def = make_trait("TextProcessor", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let content = gen_trait_adapter_bridge_file(&trait_def, "dev.sample_crate", &visible, &excluded, &[]);

    assert!(
        content.contains("public final class TextProcessorAdapter implements ITextProcessor"),
        "adapter must declare `implements ITextProcessor`;\nactual:\n{content}"
    );
}

#[test]
fn adapter_bridge_implements_hand_authored_interface_for_asset_loader() {
    // AssetLoader adapter must declare `implements IAssetLoader`.
    let trait_def = make_trait("AssetLoader", vec![]);
    let visible = all_named_visible(&trait_def.methods);
    let excluded = HashSet::new();
    let content = gen_trait_adapter_bridge_file(&trait_def, "dev.sample_crate", &visible, &excluded, &[]);

    assert!(
        content.contains("public final class AssetLoaderAdapter implements IAssetLoader"),
        "adapter must declare `implements IAssetLoader`;\nactual:\n{content}"
    );
}
