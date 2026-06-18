#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jni_return_type_unit() {
        assert_eq!(jni_return_type(&TypeRef::Unit), "()");
    }

    #[test]
    fn jni_return_type_i64() {
        assert_eq!(jni_return_type(&TypeRef::Primitive(PrimitiveType::I64)), "jlong");
    }

    #[test]
    fn jni_return_type_string() {
        assert_eq!(jni_return_type(&TypeRef::String), "jstring");
    }

    #[test]
    fn jni_return_type_vec_u8() {
        assert_eq!(
            jni_return_type(&TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8)))),
            "jbyteArray"
        );
    }

    #[test]
    fn type_ref_to_core_path_uses_btree_for_btree_map() {
        let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        assert_eq!(
            type_ref_to_core_path_with_btree(&map, "core_crate", true),
            "std::collections::BTreeMap<String, String>"
        );
        assert_eq!(
            type_ref_to_core_path_with_btree(&map, "core_crate", false),
            "std::collections::HashMap<String, String>"
        );
    }

    #[test]
    fn bytes_call_arg_optional_ref_uses_as_deref() {
        // Option<&[u8]>: Option<Vec<u8>> does not coerce, must deref.
        assert_eq!(bytes_call_arg("document_bytes", true, true), "document_bytes.as_deref()");
        // Option<Vec<u8>>: owned, pass through.
        assert_eq!(bytes_call_arg("document_bytes", true, false), "document_bytes");
        // &[u8]: &Vec<u8> coerces.
        assert_eq!(bytes_call_arg("document_bytes", false, true), "&document_bytes");
        // Vec<u8>: owned, pass through.
        assert_eq!(bytes_call_arg("document_bytes", false, false), "document_bytes");
    }

    fn btree_fixture_config() -> crate::core::config::ResolvedCrateConfig {
        use crate::core::config::NewAlefConfig;
        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
        )
        .unwrap();
        raw.resolve().unwrap().remove(0)
    }

    fn api_with_functions(functions: Vec<crate::core::ir::FunctionDef>) -> crate::core::ir::ApiSurface {
        crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions,
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// `analyze_document(..., document_bytes: Option<&[u8]>)` must pass
    /// `document_bytes.as_deref()` (Option<Vec<u8>> -> Option<&[u8]>), not the owned
    /// `Option<Vec<u8>>` which fails with E0308.
    #[test]
    fn optional_byte_slice_param_uses_as_deref_at_call_site() {
        let func = crate::core::ir::FunctionDef {
            name: "analyze_document".into(),
            rust_path: "demo::analyze_document".into(),
            params: vec![crate::core::ir::ParamDef {
                name: "document_bytes".into(),
                ty: TypeRef::Bytes,
                optional: true,
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            error_type: Some("DemoError".into()),
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_functions(vec![func]), &btree_fixture_config());
        assert!(
            content.contains("document_bytes.as_deref()"),
            "optional &[u8] param must be passed via .as_deref(): {content}"
        );
        assert!(
            content.contains("core_crate::analyze_document(document_bytes.as_deref())"),
            "call site must pass document_bytes.as_deref(): {content}"
        );
    }

    /// `resolve(..., context: &BTreeMap<String, String>)` must deserialize into a
    /// `BTreeMap` (not `HashMap`) so the `&context` argument matches the core's
    /// `&BTreeMap<String, String>` slot (E0308 otherwise).
    #[test]
    fn btree_map_param_deserializes_into_btreemap() {
        let func = crate::core::ir::FunctionDef {
            name: "resolve".into(),
            rust_path: "demo::resolve".into(),
            params: vec![crate::core::ir::ParamDef {
                name: "context".into(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                optional: false,
                is_ref: true,
                map_is_btree: true,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            error_type: Some("DemoError".into()),
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_functions(vec![func]), &btree_fixture_config());
        assert!(
            content.contains("let context: std::collections::BTreeMap<String, String>"),
            "BTreeMap param must deserialize into BTreeMap: {content}"
        );
        assert!(
            !content.contains("let context: std::collections::HashMap<String, String>"),
            "BTreeMap param must NOT deserialize into HashMap: {content}"
        );
        assert!(
            content.contains("core_crate::resolve(&context)"),
            "call site must pass &context: {content}"
        );
    }

    /// The generated `throw_jni_error` helper must use `env.throw_new(...).is_err()`
    /// and fall back to `java/lang/RuntimeException` rather than silently discarding
    /// a failed throw (which would leave the Kotlin caller with no exception pending
    /// and a null/zero sentinel that looks like a valid return value).
    #[test]
    fn throw_jni_error_has_runtime_exception_fallback() {
        use crate::core::config::NewAlefConfig;
        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
        )
        .unwrap();
        let config = raw.resolve().unwrap().remove(0);
        let api = crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let content = emit_lib_rs(&api, &config);
        // The generated helper must NOT use `let _ = env.throw_new(...)` which
        // silently swallows a missing-class error.
        assert!(
            !content.contains("let _ = env.throw_new(ERROR_CLASS"),
            "throw_jni_error must not discard the throw_new result: {content}"
        );
        // It must check the result and fall back to RuntimeException.
        // (`ERROR_CLASS` / `msg` are now wrapped in `JNIString::from(...)` per
        // the jni 0.22 API; assert on the structural pattern instead of the
        // exact arg form.)
        assert!(
            content.contains("if env.throw_new(&class_jni, &msg_jni).is_err()"),
            "throw_jni_error must check throw_new result: {content}"
        );
        assert!(
            content.contains("jni::strings::JNIString::from(ERROR_CLASS)"),
            "throw_jni_error must wrap ERROR_CLASS in JNIString::from: {content}"
        );
        assert!(
            content.contains("java/lang/RuntimeException"),
            "throw_jni_error must fall back to RuntimeException: {content}"
        );
    }
}
