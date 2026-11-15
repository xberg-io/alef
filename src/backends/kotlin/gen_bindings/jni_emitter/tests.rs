// `#[cfg(test)]` module is the final item in the flattened module (the other `include!`d files
// contribute production items, which must not follow a test module — `clippy::items_after_test_module`).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{KotlinAndroidConfig, TraitBridgeConfig};
    use std::collections::HashMap;

    /// `output_for("kotlin_android")` answers the Gradle project root -- where
    /// `build.gradle.kts` lives -- not a source directory. Joining a filename onto it
    /// directly wrote JNI-mode `.kt` files straight into the project root instead of under
    /// the Gradle Android source-set layout `src/main/kotlin/<pkg>/`.
    #[test]
    fn jni_output_path_writes_under_the_kotlin_source_set_not_the_project_root() {
        let config = ResolvedCrateConfig {
            name: "toolkit".to_string(),
            kotlin_android: Some(KotlinAndroidConfig::default()),
            output_paths: HashMap::from([("kotlin_android".to_string(), PathBuf::from("sdk/android"))]),
            ..ResolvedCrateConfig::default()
        };

        let path = jni_output_path(&config, "DemoBridge.kt");

        assert_eq!(
            path,
            PathBuf::from("sdk/android/src/main/kotlin/toolkit/DemoBridge.kt"),
            "expected the file under the Kotlin source set inside the project root, got {}",
            path.display()
        );
    }

    #[test]
    fn jni_bridge_object_treats_android_trait_lifecycle_functions_as_managed() {
        let config = ResolvedCrateConfig {
            kotlin_android: Some(KotlinAndroidConfig::default()),
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "Renderer".to_string(),
                register_fn: Some("register_renderer".to_string()),
                unregister_fn: Some("unregister_renderer".to_string()),
                clear_fn: Some("clear_renderers".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        assert!(trait_bridge_manages_jni_function("register_renderer", &config));
        assert!(trait_bridge_manages_jni_function("unregister_renderer", &config));
        assert!(trait_bridge_manages_jni_function("clear_renderers", &config));
        assert!(!trait_bridge_manages_jni_function("list_renderers", &config));
    }

    /// Regression: the `close()` free-fn call must pascal-case an acronym owner
    /// (e.g. `GraphQLRouteConfig` -> `nativeFreeGraphQlRouteConfig`) so it resolves the
    /// bridge external-fun declaration and the Rust JNI export, both of which pascal-case
    /// the owner. Using the class name verbatim produced `nativeFreeGraphQLRouteConfig`,
    /// an unresolved reference that failed `compileReleaseKotlin`.
    #[test]
    fn jni_client_close_pascal_cases_acronym_owner_free_name() {
        use crate::core::ir::{MethodDef, TypeDef, TypeRef};

        let mut api = ApiSurface::default();
        api.types.push(TypeDef {
            name: "GraphQLRouteConfig".to_owned(),
            rust_path: "my_crate::GraphQLRouteConfig".to_owned(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "path".to_owned(),
                is_static: false,
                return_type: TypeRef::String,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        });

        let config = ResolvedCrateConfig {
            kotlin_android: Some(KotlinAndroidConfig::default()),
            ..ResolvedCrateConfig::default()
        };

        let file = emit_jni_client_class(&api, &config, Some("dev.example")).expect("client class must be emitted");

        assert!(
            file.content.contains("nativeFreeGraphQlRouteConfig"),
            "close() must call the pascal-cased free name matching the bridge decl + Rust export; got:\n{}",
            file.content
        );
        assert!(
            !file.content.contains("nativeFreeGraphQLRouteConfig"),
            "close() must not emit the verbatim (miscased) acronym free name; got:\n{}",
            file.content
        );
    }

    #[test]
    fn jni_client_serializes_native_calls_and_makes_close_idempotent() {
        use crate::core::ir::{MethodDef, TypeDef, TypeRef};

        let api = ApiSurface {
            types: vec![TypeDef {
                name: "ClientHandle".to_owned(),
                rust_path: "sample::ClientHandle".to_owned(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "status".to_owned(),
                    is_static: false,
                    return_type: TypeRef::String,
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            kotlin_android: Some(KotlinAndroidConfig::default()),
            ..ResolvedCrateConfig::default()
        };

        let file = emit_jni_client_class(&api, &config, Some("dev.example")).expect("client class must be emitted");

        assert!(file.content.contains("private val handleLock = Any()"));
        assert!(
            file.content
                .contains("check(nativeHandle != 0L) { \"ClientHandle is closed\" }")
        );
        assert!(file.content.contains("withHandle { handle ->"));
        assert!(file.content.contains("nativeClientHandleStatus(handle)"));
        assert!(file.content.contains("override fun close() = synchronized(handleLock)"));
        assert!(file.content.contains("if (nativeHandle != 0L)"));
        assert!(file.content.contains("nativeHandle = 0L"));
    }

    #[test]
    fn jni_bridge_omits_sanitized_instance_method_declarations() {
        use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};

        let api = ApiSurface {
            types: vec![TypeDef {
                name: "SampleClient".to_owned(),
                rust_path: "sample::SampleClient".to_owned(),
                is_opaque: true,
                methods: vec![
                    MethodDef {
                        name: "chat".to_owned(),
                        receiver: Some(ReceiverKind::Ref),
                        cfg: None,
                        return_type: TypeRef::String,
                        ..MethodDef::default()
                    },
                    MethodDef {
                        name: "chat_stream".to_owned(),
                        receiver: Some(ReceiverKind::Ref),
                        cfg: None,
                        return_type: TypeRef::String,
                        sanitized: true,
                        ..MethodDef::default()
                    },
                ],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample".to_owned(),
            kotlin_android: Some(KotlinAndroidConfig::default()),
            ..ResolvedCrateConfig::default()
        };

        let content = emit_jni_bridge_object(&api, &config).content;

        assert!(content.contains("external fun nativeSampleClientChat("));
        assert!(!content.contains("external fun nativeSampleClientChatStream("));
    }

    #[test]
    fn jni_bridge_only_declares_destructors_for_returned_opaque_handles() {
        use crate::core::ir::{FunctionDef, TypeDef, TypeRef};

        let opaque = |name: &str| TypeDef {
            name: name.to_owned(),
            rust_path: format!("sample::{name}"),
            is_opaque: true,
            ..TypeDef::default()
        };
        let api = ApiSurface {
            types: vec![opaque("ReturnedHandle"), opaque("InternalHandle")],
            functions: vec![FunctionDef {
                name: "create_handle".to_owned(),
                rust_path: "sample::create_handle".to_owned(),
                return_type: TypeRef::Named("ReturnedHandle".to_owned()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample".to_owned(),
            kotlin_android: Some(KotlinAndroidConfig::default()),
            ..ResolvedCrateConfig::default()
        };

        let content = emit_jni_bridge_object(&api, &config).content;

        assert!(content.contains("external fun nativeFreeReturnedHandle(handle: Long)"));
        assert!(!content.contains("nativeFreeInternalHandle"));
    }

    #[test]
    fn jni_wrapper_borrows_a_tree_handle_outside_the_json_request() {
        let method = crate::core::ir::MethodDef {
            params: vec![
                crate::core::ir::ParamDef {
                    name: "source".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
                crate::core::ir::ParamDef {
                    name: "old_tree".into(),
                    ty: TypeRef::Named("Tree".into()),
                    is_ref: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let opaque_names = std::collections::HashSet::from(["Tree"]);
        assert_eq!(
            method_jni_params(&method, &opaque_names),
            "handle: Long, requestJson: String, oldTreeHandle: Long"
        );
        let call = build_bridge_call(&method, "Bridge", "nativeParse", &opaque_names);
        assert_eq!(
            call,
            "synchronized(Bridge.borrowedHandleLock) { oldTree.withHandle { `alef$borrowed0` -> withHandle { `alef$receiver` -> Bridge.nativeParse(`alef$receiver`, MAPPER.writeValueAsString(mapOf(\"source\" to source)), `alef$borrowed0`) } } }"
        );
    }
}
