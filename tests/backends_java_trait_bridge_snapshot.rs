//! Snapshot tests for Java Panama FFM trait bridge codegen.
//!
//! Tests verify that trait interface + bridge class emission generates correctly
//! with upcall stubs, registry, and register/unregister/clear methods.

#[cfg(test)]
mod tests {
    use alef::backends::java::gen_bindings::trait_bridge;
    use alef::core::ir::{MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
    use std::collections::HashSet;

    fn make_method(name: &str, return_type: TypeRef, params: Vec<ParamDef>) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type,
            is_async: false,
            is_static: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            receiver: Some(alef::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn make_trait(name: &str, methods: Vec<MethodDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("kreuzberg::{name}"),
            original_rust_path: format!("kreuzberg::{name}"),
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
        }
    }

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

    #[test]
    fn test_java_trait_bridge_async_sync_methods_with_super_trait() {
        let trait_def = make_trait(
            "OcrBackend",
            vec![make_method(
                "process",
                TypeRef::String,
                vec![ParamDef {
                    name: "image".to_string(),
                    ty: TypeRef::Bytes,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let files = trait_bridge::gen_trait_bridge_files(
            &trait_def,
            "krz",
            "dev.kreuzberg",
            true,
            Some("unregister_ocr_backend"),
            Some("clear_ocr_backends"),
            &visible,
        );

        // Verify interface has lifecycle methods
        assert!(files.interface_content.contains("package dev.kreuzberg;"));
        assert!(files.interface_content.contains("public interface IOcrBackend"));
        assert!(files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("String version();"));
        assert!(files.interface_content.contains("void initialize()"));
        assert!(files.interface_content.contains("void shutdown()"));
        assert!(files.interface_content.contains("String process(byte[])"));

        // Verify bridge class has:
        // - Registry field
        let bridge = &files.bridge_content;
        assert!(bridge.contains("public final class OcrBackendBridge"));
        assert!(bridge.contains("ConcurrentHashMap<String, OcrBackendBridge>"));
        assert!(bridge.contains("OCR_BACKEND_BRIDGES = new ConcurrentHashMap<>()"));

        // - Upcall stubs for lifecycle + methods
        assert!(bridge.contains("LINKER.upcallStub"));
        assert!(bridge.contains("handleName"));
        assert!(bridge.contains("handleVersion"));
        assert!(bridge.contains("handleInitialize"));
        assert!(bridge.contains("handleShutdown"));
        assert!(bridge.contains("handleProcess"));

        // - Register method that calls the FFI function
        assert!(bridge.contains("public static void registerOcrBackend(final IOcrBackend impl)"));
        assert!(bridge.contains("NativeLib.KRZ_REGISTER_OCR_BACKEND.invoke("));
        assert!(bridge.contains("impl.name()"));
        assert!(bridge.contains("bridge.vtableSegment()"));

        // - Unregister method
        assert!(bridge.contains("public static void unregisterOcrBackend(String name)"));
        assert!(bridge.contains("NativeLib.KRZ_UNREGISTER_OCR_BACKEND.invoke("));
        assert!(bridge.contains("OCR_BACKEND_BRIDGES.remove(name)"));

        // - Clear method
        assert!(bridge.contains("public static void clearAllOcrBackend()"));
        assert!(bridge.contains("NativeLib.KRZ_CLEAR_OCR_BACKEND.invoke("));
        assert!(bridge.contains("OCR_BACKEND_BRIDGES.values().forEach(OcrBackendBridge::close)"));
        assert!(bridge.contains("OCR_BACKEND_BRIDGES.clear()"));
    }

    #[test]
    fn test_java_trait_bridge_no_super_trait_requires_explicit_name() {
        let trait_def = make_trait(
            "Validator",
            vec![make_method(
                "validate",
                TypeRef::String,
                vec![ParamDef {
                    name: "input".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let files =
            trait_bridge::gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", false, None, None, &visible);

        // No super trait means:
        // - Interface has no lifecycle methods
        assert!(!files.interface_content.contains("String name();"));
        assert!(files.interface_content.contains("String validate(String)"));

        // - Register method requires explicit name param
        let bridge = &files.bridge_content;
        assert!(bridge.contains("public static void registerValidator(final IValidator impl, String name)"));
        assert!(!bridge.contains("impl.name()"));
        assert!(bridge.contains("var nameCs = nameArena.allocateFrom(name);"));
    }

    #[test]
    fn test_java_trait_bridge_marshalls_json_for_complex_types() {
        let trait_def = make_trait(
            "Extractor",
            vec![make_method(
                "extract",
                TypeRef::Named("ExtractionResult".to_string()),
                vec![ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("ExtractionConfig".to_string()),
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                }],
            )],
        );
        let visible = all_named_visible(&trait_def.methods);
        let files =
            trait_bridge::gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", false, None, None, &visible);

        let bridge = &files.bridge_content;
        // Complex types (structs) are JSON-marshalled via Jackson
        assert!(bridge.contains("JSON.readValue"));
        assert!(bridge.contains("JSON.writeValueAsString"));
        // Returns are written to outResult MemorySegment
        assert!(bridge.contains("outResult.set(ValueLayout.ADDRESS, 0, jsonCs)"));
    }

    #[test]
    fn test_java_trait_bridge_error_handling() {
        let trait_def = make_trait("Processor", vec![make_method("process", TypeRef::String, vec![])]);
        let visible = all_named_visible(&trait_def.methods);
        let files =
            trait_bridge::gen_trait_bridge_files(&trait_def, "krz", "dev.kreuzberg", false, None, None, &visible);

        let bridge = &files.bridge_content;
        // Error handling writes error message to outError MemorySegment
        assert!(bridge.contains("writeError(outError, e)"));
        assert!(bridge.contains("arena.allocateFrom(e.getClass().getSimpleName()"));
        // Handlers return 0 on success, 1 on error
        assert!(bridge.contains("return 0;"));
        assert!(bridge.contains("return 1;"));
    }
}
