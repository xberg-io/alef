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
        assert_eq!(
            bytes_call_arg("document_bytes", true, true),
            "document_bytes.as_deref()"
        );
        assert_eq!(bytes_call_arg("document_bytes", true, false), "document_bytes");
        assert_eq!(bytes_call_arg("document_bytes", false, true), "&document_bytes");
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
            unresolved_modules: Vec::new(),
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

    /// A free function resolved into a *sibling* workspace crate (rust_path
    /// `<sibling_crate>::<fn>`, where the sibling crate is not the umbrella crate)
    /// must be reached through the umbrella facade by item path
    /// (`core_crate::<fn>`), mirroring how opaque types are referenced. Prefixing
    /// the origin crate — `core_crate::<sibling_crate>::<fn>` — does not resolve
    /// (E0433: cannot find `<sibling_crate>` in `core_crate`).
    #[test]
    fn sibling_crate_function_is_reached_through_umbrella_facade() {
        let func = crate::core::ir::FunctionDef {
            name: "schema_query_only".into(),
            rust_path: "demo_graphql::schema_query_only".into(),
            params: vec![],
            return_type: TypeRef::String,
            error_type: None,
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_functions(vec![func]), &btree_fixture_config());
        assert!(
            content.contains("core_crate::schema_query_only("),
            "sibling-crate fn must be called as core_crate::schema_query_only(): {content}"
        );
        assert!(
            !content.contains("core_crate::demo_graphql::"),
            "sibling-crate fn must NOT be prefixed with the origin crate: {content}"
        );
    }

    #[test]
    fn disabled_feature_uses_available_fallback_without_referencing_gated_module() {
        let gated = crate::core::ir::FunctionDef {
            name: "decode_sample".into(),
            rust_path: "demo::decoder::decode_sample".into(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"decoder\"".into()),
            ..Default::default()
        };
        let fallback = crate::core::ir::FunctionDef {
            name: "decode_sample".into(),
            rust_path: "demo::decode_sample".into(),
            return_type: TypeRef::String,
            cfg: Some("all(feature = \"mobile\", not(feature = \"decoder\"))".into()),
            ..Default::default()
        };
        let gated_only = crate::core::ir::FunctionDef {
            name: "decoder_details".into(),
            rust_path: "demo::decoder::decoder_details".into(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"decoder\"".into()),
            ..Default::default()
        };
        let mut config = btree_fixture_config();
        config
            .kotlin_android
            .as_mut()
            .expect("fixture has Android config")
            .features = Some(vec!["mobile".into()]);

        let content = emit_lib_rs(&api_with_functions(vec![gated, fallback, gated_only]), &config);

        assert!(
            content.contains("core_crate::decode_sample()"),
            "available fallback must be emitted: {content}"
        );
        assert!(
            !content.contains("core_crate::decoder::decode_sample") && !content.contains("decoder_details"),
            "disabled feature functions must not reference the absent core module: {content}"
        );
    }

    #[test]
    fn target_feature_overrides_gate_real_and_fallback_functions() {
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
features = ["decoder"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["mobile"]
"#,
        )
        .expect("fixture config parses");
        let config = raw.resolve().expect("fixture config resolves").remove(0);
        let function = |rust_path: &str, cfg: &str| crate::core::ir::FunctionDef {
            name: "decode_sample".into(),
            rust_path: rust_path.into(),
            return_type: TypeRef::String,
            cfg: Some(cfg.into()),
            ..Default::default()
        };
        let gated_only = crate::core::ir::FunctionDef {
            name: "decoder_details".into(),
            rust_path: "demo::decoder::decoder_details".into(),
            return_type: TypeRef::String,
            cfg: Some("feature = \"decoder\"".into()),
            ..Default::default()
        };

        let content = emit_lib_rs(
            &api_with_functions(vec![
                function("demo::decoder::decode_sample", "feature = \"decoder\""),
                function(
                    "demo::decode_sample",
                    "all(feature = \"mobile\", not(feature = \"decoder\"))",
                ),
                gated_only,
            ]),
            &config,
        );

        assert!(
            content.contains("#[cfg(not(any(target_os = \"android\")))]\n#[unsafe(no_mangle)]"),
            "desktop-only implementations must be target gated: {content}"
        );
        assert!(
            content.contains("#[cfg(target_os = \"android\")]\n#[unsafe(no_mangle)]"),
            "the enabled Android fallback must be emitted behind its target gate: {content}"
        );
        assert!(content.contains("core_crate::decode_sample()"));
        assert!(content.contains("core_crate::decoder::decoder_details()"));
    }

    /// Mirrors a real crate shape: a function defined inside a `pub mod` and ALSO re-exported
    /// by name at the crate root (`pub use the_module::{score_pair, ...}`), e.g. a real crate's
    /// `scoring` module. The extractor preserves both `FunctionDef` entries — one with
    /// a module-nested `rust_path`, one with a crate-root `rust_path` — and since their raw
    /// `cfg` is identical (unconditional here), `dedup_same_name_functions`'s cfg-must-differ
    /// merge guard leaves both alone. With `[[crates.jni.target_dep_overrides]]` configured
    /// (as a real jni consumer's alef.toml is), `emit_lib_rs` used to source `visible_functions`
    /// straight from the raw, un-deduped surface, so both entries resolved to the identical
    /// `#[cfg(not(any(target_os = "android")))]` gate and were emitted as two
    /// `#[unsafe(no_mangle)]` definitions of the same native symbol — E0428.
    #[test]
    fn duplicate_ir_entries_with_matching_target_predicate_emit_one_native_symbol() {
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

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["mobile"]
"#,
        )
        .expect("fixture config parses");
        let config = raw.resolve().expect("fixture config resolves").remove(0);

        let module_definition_entry = crate::core::ir::FunctionDef {
            name: "score_pair".into(),
            rust_path: "demo::scoring::score_pair".into(),
            return_type: TypeRef::String,
            cfg: None,
            ..Default::default()
        };
        let crate_root_reexport_entry = crate::core::ir::FunctionDef {
            name: "score_pair".into(),
            rust_path: "demo::score_pair".into(),
            return_type: TypeRef::String,
            cfg: None,
            ..Default::default()
        };

        let content = emit_lib_rs(
            &api_with_functions(vec![module_definition_entry, crate_root_reexport_entry]),
            &config,
        );

        let native_fn_count = content.matches("nativeScorePair").count();
        assert_eq!(
            native_fn_count, 1,
            "the module-definition and crate-root-reexport entries must collapse to one native symbol: {content}"
        );
    }

    #[test]
    fn streaming_kotlin_declarations_have_matching_jni_exports() {
        use crate::core::config::extras::{AdapterConfig, AdapterParam, AdapterPattern};
        use crate::core::ir::TypeDef;

        let mut config = btree_fixture_config();
        config.adapters.push(AdapterConfig {
            name: "stream_items".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: "demo::Engine::stream_items".to_string(),
            params: vec![AdapterParam {
                name: "request".to_string(),
                ty: "demo::StreamRequest".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: None,
            owner_type: Some("Engine".to_string()),
            item_type: Some("StreamItem".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        });
        let api = crate::core::ir::ApiSurface {
            crate_name: "demo".to_string(),
            types: vec![TypeDef {
                name: "Engine".to_string(),
                rust_path: "demo::Engine".to_string(),
                is_opaque: true,
                methods: Vec::new(),
                ..TypeDef::default()
            }],
            ..crate::core::ir::ApiSurface::default()
        };
        let kotlin = crate::backends::kotlin::emit_jni_bridge_object(&api, &config).content;
        let rust = emit_lib_rs(&api, &config);

        let expected_methods = ["Start", "Next", "Free"].map(|suffix| format!("nativeEngineStreamItems{suffix}"));
        for method in &expected_methods {
            assert!(kotlin.contains(&format!("external fun {method}(")), "{kotlin}");
            assert!(
                rust.contains(method.as_str()),
                "missing JNI export for {method}: {rust}"
            );
        }
        for kotlin_method in kotlin
            .lines()
            .filter_map(|line| line.trim().strip_prefix("external fun nativeEngineStreamItems"))
            .filter_map(|tail| {
                tail.split_once('(')
                    .map(|(suffix, _)| format!("nativeEngineStreamItems{suffix}"))
            })
        {
            assert!(
                rust.contains(&kotlin_method),
                "Kotlin declaration lacks Rust export: {kotlin_method}"
            );
        }
        for rust_method in rust
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .filter(|token| token.starts_with("nativeEngineStreamItems"))
        {
            assert!(
                kotlin.contains(&format!("external fun {rust_method}(")),
                "Rust export lacks Kotlin declaration: {rust_method}"
            );
        }
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
            unresolved_modules: Vec::new(),
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
        assert!(
            !content.contains("let _ = env.throw_new(ERROR_CLASS"),
            "throw_jni_error must not discard the throw_new result: {content}"
        );
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

    /// Build an `ApiSurface` whose single opaque client type carries `methods`,
    /// so `emit_lib_rs` routes them through `emit_method_shim` (the request-map
    /// multi-param path) rather than the per-param free-function path.
    fn api_with_client_methods(methods: Vec<crate::core::ir::MethodDef>) -> crate::core::ir::ApiSurface {
        let client = crate::core::ir::TypeDef {
            name: "Loader".into(),
            rust_path: "demo::Loader".into(),
            is_opaque: true,
            methods,
            ..Default::default()
        };
        crate::core::ir::ApiSurface {
            unresolved_modules: Vec::new(),
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![client],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// Multi-param method `parse_preset(path: &str, raw: &[u8])` is decoded from the
    /// request map. The `&[u8]` param must bind `let raw: Vec<u8>` (not the generic
    /// `serde_json::Value` catch-all) and be passed as `&raw` so `&Vec<u8>` derefs to
    /// `&[u8]` (E0308 otherwise: `expected &[u8], found &Value`).
    #[test]
    fn request_map_byte_slice_param_binds_vec_u8_not_json_value() {
        let method = crate::core::ir::MethodDef {
            name: "parse_preset".into(),
            params: vec![
                crate::core::ir::ParamDef {
                    name: "path".into(),
                    ty: TypeRef::String,
                    is_ref: true,
                    ..Default::default()
                },
                crate::core::ir::ParamDef {
                    name: "raw".into(),
                    ty: TypeRef::Bytes,
                    is_ref: true,
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Named("Preset".into()),
            error_type: Some("LoadError".into()),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_client_methods(vec![method]), &btree_fixture_config());
        assert!(
            content.contains("let raw: Vec<u8> = match req_map.get(\"raw\")"),
            "request-map &[u8] param must bind Vec<u8>: {content}"
        );
        assert!(
            !content.contains("let raw: serde_json::Value"),
            "request-map &[u8] param must NOT bind serde_json::Value: {content}"
        );
        assert!(
            content.contains("client.parse_preset(&path, &raw)"),
            "call site must pass &path and &raw: {content}"
        );
    }

    /// Multi-param method `load_at(path: &Path, raw: &[u8])`: a `&Path` param in the
    /// request-map path must deserialize as `String` then convert to `PathBuf` (so
    /// `&path` derefs `&PathBuf` → `&Path`), never bind the `serde_json::Value`
    /// catch-all (E0277: `Value` does not impl `AsRef<Path>`).
    #[test]
    fn request_map_path_param_binds_pathbuf_not_json_value() {
        let method = crate::core::ir::MethodDef {
            name: "load_at".into(),
            params: vec![
                crate::core::ir::ParamDef {
                    name: "path".into(),
                    ty: TypeRef::Path,
                    is_ref: true,
                    ..Default::default()
                },
                crate::core::ir::ParamDef {
                    name: "raw".into(),
                    ty: TypeRef::Bytes,
                    is_ref: true,
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Named("Preset".into()),
            error_type: Some("LoadError".into()),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_client_methods(vec![method]), &btree_fixture_config());
        assert!(
            content.contains("let path = std::path::PathBuf::from(path);"),
            "request-map &Path param must convert to PathBuf: {content}"
        );
        assert!(
            !content.contains("let path: serde_json::Value"),
            "request-map &Path param must NOT bind serde_json::Value: {content}"
        );
        assert!(
            content.contains("client.load_at(&path, &raw)"),
            "call site must pass &path and &raw: {content}"
        );
    }

    /// A client type listed in `[crates.kotlin_android].exclude_types` (or the shared
    /// `[crates.ffi].exclude_types`) must not have any JNI shims emitted. The
    /// kotlin_android binding backend already drops the Kotlin class via
    /// `effective_exclude_types`; without the matching filter here the JNI side emits
    /// orphan `#[no_mangle]` shims and re-exposes a type every other FFI-derived
    /// binding hides (e.g. the test-only client). The exclusion must be *targeted*:
    /// a sibling client that is not excluded keeps its shims.
    #[test]
    fn excluded_client_type_emits_no_shims_but_keeps_others() {
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
exclude_types = ["Loader"]
"#,
        )
        .unwrap();
        let config = raw.resolve().unwrap().remove(0);
        let method = |name: &str| crate::core::ir::MethodDef {
            name: name.into(),
            params: vec![crate::core::ir::ParamDef {
                name: "path".into(),
                ty: TypeRef::String,
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            error_type: Some("LoadError".into()),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        };
        let client = |name: &str, m: crate::core::ir::MethodDef| crate::core::ir::TypeDef {
            name: name.into(),
            rust_path: format!("demo::{name}"),
            is_opaque: true,
            methods: vec![m],
            ..Default::default()
        };
        let api = crate::core::ir::ApiSurface {
            unresolved_modules: Vec::new(),
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![
                client("Loader", method("excluded_call")),
                client("Keeper", method("kept_call")),
            ],
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
        assert!(
            !content.contains("excluded_call"),
            "excluded client type must not emit method shims: {content}"
        );
        assert!(
            !content.contains("FreeLoader") && !content.contains("nativeFreeLoader"),
            "excluded client type must not emit a destructor shim: {content}"
        );
        assert!(
            content.contains("client.kept_call"),
            "a non-excluded sibling client must keep its shims: {content}"
        );
    }

    #[test]
    fn synchronous_function_body_is_panic_contained() {
        let function = crate::core::ir::FunctionDef {
            name: "normalize".into(),
            rust_path: "demo::normalize".into(),
            params: vec![crate::core::ir::ParamDef {
                name: "input".into(),
                ty: TypeRef::String,
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_functions(vec![function]), &btree_fixture_config());
        syn::parse_file(&content).expect("generated JNI crate must parse as Rust");
        let function_body = content
            .split("Java_dev_sample_1crate_DemoBridge_nativeNormalize")
            .nth(1)
            .expect("nativeNormalize shim");

        assert!(function_body.contains("run_or_throw(env, |env|"), "{function_body}");
        let boundary = function_body.find("run_or_throw(env, |env|").expect("panic boundary");
        let core_call = function_body.find("core_crate::normalize").expect("core call");
        assert!(
            boundary < core_call,
            "panic boundary must precede the core call: {function_body}"
        );
    }

    #[test]
    fn capsule_language_returns_raw_grammar_pointer_without_box_destructor() {
        let config = capsule_fixture_config();
        let function = crate::core::ir::FunctionDef {
            name: "language_sample".into(),
            rust_path: "sample::language_sample".into(),
            return_type: TypeRef::Named("Language".into()),
            ..Default::default()
        };
        let mut api = api_with_functions(vec![function]);
        api.types.push(crate::core::ir::TypeDef {
            name: "Language".into(),
            is_opaque: true,
            ..Default::default()
        });

        let content = emit_lib_rs(&api, &config);

        assert!(
            content.contains("v.into_raw() as jlong"),
            "capsule return must mirror the C FFI raw-pointer transfer: {content}"
        );
        assert!(
            !content.contains("Box::into_raw(Box::new(v))"),
            "capsule must not box Language: {content}"
        );
        assert!(
            !content.contains("nativeFreeLanguage"),
            "host runtime owns the raw language pointer: {content}"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    fn capsule_fixture_config() -> crate::core::config::ResolvedCrateConfig {
        let raw: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni", "ffi"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
[crates.kotlin_android]
package = "dev.sample"
[crates.kotlin_android.capsule_types.Language]
host_type = "dev.runtime.Language"
construct_expr = "dev.runtime.Language({ptr})"
[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
"#,
        )
        .expect("valid config");
        raw.resolve().expect("resolved config").remove(0)
    }

    #[test]
    fn capsule_client_methods_return_raw_grammar_pointers_without_boxes() {
        let api = crate::core::ir::ApiSurface {
            types: vec![
                crate::core::ir::TypeDef {
                    name: "LanguageRegistry".into(),
                    is_opaque: true,
                    methods: vec![
                        capsule_method("get_language", TypeRef::Named("Language".into()), true),
                        capsule_method(
                            "find_language",
                            TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
                            false,
                        ),
                    ],
                    ..Default::default()
                },
                crate::core::ir::TypeDef {
                    name: "Language".into(),
                    is_opaque: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let content = emit_lib_rs(&api, &capsule_fixture_config());

        assert!(
            content.contains("v.into_raw() as jlong"),
            "direct method capsule must return its raw grammar pointer: {content}"
        );
        assert!(
            content.contains("Some(inner) => inner.into_raw() as jlong"),
            "optional method capsule must map Some to the raw grammar pointer: {content}"
        );
        assert!(
            !content.contains("Box::into_raw(Box::new(v))"),
            "capsule must not box Language: {content}"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    #[test]
    fn capsule_client_methods_own_borrowed_and_cow_values_before_raw_transfer() {
        let mut borrowed = capsule_method("borrowed_language", TypeRef::Named("Language".into()), true);
        borrowed.returns_ref = true;
        let mut optional_borrowed = capsule_method(
            "optional_borrowed_language",
            TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
            false,
        );
        optional_borrowed.returns_ref = true;
        let mut cow = capsule_method("cow_language", TypeRef::Named("Language".into()), false);
        cow.returns_ref = true;
        cow.returns_cow = true;
        let mut optional_cow = capsule_method(
            "optional_cow_language",
            TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
            false,
        );
        optional_cow.returns_ref = true;
        optional_cow.returns_cow = true;
        let content = emit_lib_rs(
            &api_with_client_methods(vec![borrowed, optional_borrowed, cow, optional_cow]),
            &capsule_fixture_config(),
        );

        assert!(
            content.contains("v.clone().into_raw() as jlong"),
            "borrowed capsule must be cloned before ownership transfer: {content}"
        );
        assert!(
            content.contains("Some(inner) => inner.clone().into_raw() as jlong"),
            "optional borrowed capsule must clone Some before ownership transfer: {content}"
        );
        assert!(
            content.contains("v.into_owned().into_raw() as jlong"),
            "Cow capsule must become owned before ownership transfer: {content}"
        );
        assert!(
            content.contains("Some(inner) => inner.into_owned().into_raw() as jlong"),
            "optional Cow capsule must own Some before ownership transfer: {content}"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    /// `into_raw()` already yields `*const {into_raw_type}`, so an intermediate `as *const T`
    /// is a same-type cast that fails a consumer's `clippy::unnecessary_cast` under `-D warnings`.
    /// The `as jlong` stays: any pointer casts to an integer, so it remains correct for a capsule
    /// whose `into_raw()` returns some other pointer type. ~keep
    #[test]
    fn capsule_returns_transfer_the_pointer_without_a_redundant_cast() {
        let plain = capsule_method("plain_language", TypeRef::Named("Language".into()), true);
        let optional_plain = capsule_method(
            "optional_plain_language",
            TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
            false,
        );
        let mut borrowed = capsule_method("borrowed_language", TypeRef::Named("Language".into()), true);
        borrowed.returns_ref = true;
        let mut optional_borrowed = capsule_method(
            "optional_borrowed_language",
            TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
            false,
        );
        optional_borrowed.returns_ref = true;
        let mut cow = capsule_method("cow_language", TypeRef::Named("Language".into()), false);
        cow.returns_ref = true;
        cow.returns_cow = true;
        let mut optional_cow = capsule_method(
            "optional_cow_language",
            TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
            false,
        );
        optional_cow.returns_ref = true;
        optional_cow.returns_cow = true;

        let content = emit_lib_rs(
            &api_with_client_methods(vec![
                plain,
                optional_plain,
                borrowed,
                optional_borrowed,
                cow,
                optional_cow,
            ]),
            &capsule_fixture_config(),
        );

        let transfers: Vec<&str> = content
            .lines()
            .filter(|line| line.contains(".into_raw()") && line.contains("as jlong"))
            .collect();
        assert_eq!(
            transfers.len(),
            6,
            "every capsule variant must emit exactly one raw-pointer transfer: {content}"
        );
        for transfer in transfers {
            assert!(
                !transfer.contains("as *const"),
                "capsule transfer must not cast the pointer to its own type before `as jlong`: {transfer}"
            );
        }
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    fn capsule_method(name: &str, return_type: TypeRef, fallible: bool) -> crate::core::ir::MethodDef {
        crate::core::ir::MethodDef {
            name: name.into(),
            return_type,
            error_type: fallible.then(|| "LanguageError".into()),
            ..Default::default()
        }
    }

    #[test]
    fn opaque_receiver_is_rejected_before_reference_construction() {
        let method = crate::core::ir::MethodDef {
            name: "status".into(),
            return_type: TypeRef::String,
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_client_methods(vec![method]), &btree_fixture_config());
        syn::parse_file(&content).expect("generated JNI crate must parse as Rust");
        let method_body = content
            .split("nativeLoaderStatus")
            .nth(1)
            .expect("nativeLoaderStatus shim");
        let zero_check = method_body.find("if handle == 0").expect("zero-handle check");
        let reference = method_body.find("&*(handle as *const").expect("receiver reference");

        assert!(method_body.contains("run_or_throw(env, |env|"), "{method_body}");
        assert!(
            zero_check < reference,
            "zero check must precede reference construction: {method_body}"
        );
    }

    fn config_with_trait_bridge(trait_name: &str) -> crate::core::config::ResolvedCrateConfig {
        use crate::core::config::NewAlefConfig;
        let source = format!(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"

[[crates.trait_bridges]]
trait_name = "{trait_name}"
register_fn = "register_backend"
"#
        );
        let raw: NewAlefConfig = toml::from_str(&source).unwrap();
        raw.resolve().unwrap().remove(0)
    }

    fn api_with_types(types: Vec<crate::core::ir::TypeDef>) -> crate::core::ir::ApiSurface {
        crate::core::ir::ApiSurface {
            unresolved_modules: Vec::new(),
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types,
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// Regression pin for the trait-registration no-op shim: when `register_fn` is
    /// configured but the trait cannot be resolved in the API surface (or resolves
    /// with no own methods), generation must fail loudly rather than emit a
    /// `nativeRegister*` shim that accepts the JNI call and silently never invokes
    /// `register_fn`. No shim text is produced because the call never returns a value.
    #[test]
    #[should_panic(expected = "cannot bridge it")]
    fn jni_trait_register_shim_panics_when_trait_not_resolvable() {
        let config = config_with_trait_bridge("OcrBackend");
        let api = api_with_types(vec![]);
        emit_lib_rs(&api, &config);
    }

    /// Sibling positive control: when the configured trait IS resolvable with a
    /// non-empty method set, generation succeeds and routes through the real
    /// `gen_plugin_trait_bridge` generator, which actually calls `register_fn`.
    #[test]
    fn jni_trait_register_shim_uses_real_bridge_when_trait_resolvable() {
        let method = crate::core::ir::MethodDef {
            name: "process".into(),
            params: vec![],
            return_type: TypeRef::Unit,
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        };
        let trait_def = crate::core::ir::TypeDef {
            name: "OcrBackend".into(),
            rust_path: "demo::OcrBackend".into(),
            is_trait: true,
            methods: vec![method],
            ..Default::default()
        };
        let config = config_with_trait_bridge("OcrBackend");
        let api = api_with_types(vec![trait_def]);

        let content = emit_lib_rs(&api, &config);

        assert!(
            content.contains("nativeRegisterOcrBackend"),
            "expected a real nativeRegister shim symbol: {content}"
        );
        assert!(
            content.contains("core_crate::register_backend(backend"),
            "expected the real bridge to actually call register_fn: {content}"
        );
    }
}
