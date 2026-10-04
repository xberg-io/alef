#[cfg(test)]
mod transparent_string_tests {
    use super::*;
    use crate::core::ir::{NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata};

    fn config() -> crate::core::config::ResolvedCrateConfig {
        let raw: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.demo"
"#,
        )
        .expect("fixture config parses");
        raw.resolve().expect("fixture config resolves").remove(0)
    }

    fn wrapper(containers: Vec<NewtypeContainer>) -> String {
        wrapper_with_path("demo::SecretString", containers)
    }

    fn wrapper_with_path(path: &str, containers: Vec<NewtypeContainer>) -> String {
        wrapper_with_constructor(path, "from", containers)
    }

    fn wrapper_with_constructor(path: &str, constructor: &str, containers: Vec<NewtypeContainer>) -> String {
        NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            path,
            constructor,
            "into_inner",
            containers,
        )])
    }

    fn secret_param(optional: bool, is_ref: bool) -> ParamDef {
        ParamDef {
            name: "value".to_string(),
            ty: TypeRef::String,
            optional,
            is_ref,
            newtype_wrapper: Some(wrapper(if optional {
                vec![NewtypeContainer::Optional]
            } else {
                Vec::new()
            })),
            ..Default::default()
        }
    }

    fn secret_function(name: &str, optional: bool, is_ref: bool) -> crate::core::ir::FunctionDef {
        crate::core::ir::FunctionDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            params: vec![secret_param(optional, is_ref)],
            return_type: if optional {
                TypeRef::Optional(Box::new(TypeRef::String))
            } else {
                TypeRef::String
            },
            return_newtype_wrapper: Some(wrapper(if optional {
                vec![NewtypeContainer::Optional]
            } else {
                Vec::new()
            })),
            ..Default::default()
        }
    }

    fn api_with_functions(functions: Vec<crate::core::ir::FunctionDef>) -> ApiSurface {
        ApiSurface {
            crate_name: "demo".to_string(),
            functions,
            ..Default::default()
        }
    }

    #[test]
    fn free_functions_apply_explicit_wrapper_operations_for_all_call_shapes() {
        let mut async_secret = secret_function("async_secret", false, false);
        async_secret.is_async = true;
        let mut fallible_secret = secret_function("fallible_secret", false, false);
        fallible_secret.error_type = Some("SecretError".to_string());
        let content = emit_lib_rs(
            &api_with_functions(vec![
                secret_function("echo_secret", false, false),
                secret_function("borrow_secret", false, true),
                secret_function("optional_secret", true, false),
                secret_function("borrow_optional_secret", true, true),
                async_secret,
                fallible_secret,
            ]),
            &config(),
        );

        assert!(
            content.contains("core_crate::echo_secret(core_crate::SecretString::from(value))"),
            "owned input must construct the wrapper: {content}"
        );
        assert!(
            content.contains("let value_newtype = core_crate::SecretString::from(value);")
                && content.contains("core_crate::borrow_secret(&value_newtype)"),
            "borrowed input must keep an owned wrapper alive for the call: {content}"
        );
        assert!(
            content.contains("core_crate::optional_secret((if value.is_empty() { None } else { Some(value) }).map(core_crate::SecretString::from))"),
            "optional input must convert only Some: {content}"
        );
        assert!(
            content.contains("let value_newtype = (if value.is_empty() { None } else { Some(value) }).map(core_crate::SecretString::from);")
                && content.contains("core_crate::borrow_optional_secret(value_newtype.as_ref())"),
            "optional borrowed input must convert before borrowing: {content}"
        );
        assert!(
            !content.contains("map(|value| core_crate::SecretString::from(value))"),
            "root optional wrappers must not emit Clippy's redundant-closure shape: {content}"
        );
        assert!(
            content.contains("runtime().block_on(core_crate::async_secret(core_crate::SecretString::from(value)))"),
            "async input must construct the wrapper before awaiting: {content}"
        );
        assert!(
            content.contains("core_crate::fallible_secret(core_crate::SecretString::from(value))"),
            "fallible input must construct the wrapper before matching Result: {content}"
        );
        assert_eq!(
            content.matches("let v = (v).into_inner();").count(),
            4,
            "owned, borrowed-input, async, and Result-ok returns must use the explicit extractor: {content}"
        );
        assert!(
            content.contains("let v = (v).map(|value| (value).into_inner());"),
            "optional return must convert only Some: {content}"
        );
        assert!(
            !content.contains("v.to_string()"),
            "wrapper return must not require Display"
        );
        assert!(
            !content.contains("v.as_ref()"),
            "wrapper return must not require AsRef<str>"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    #[test]
    fn optional_param_uses_the_configured_constructor_as_a_function_item() {
        let function = crate::core::ir::FunctionDef {
            name: "optional_secret".to_string(),
            rust_path: "demo::optional_secret".to_string(),
            params: vec![ParamDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                optional: true,
                newtype_wrapper: Some(wrapper_with_constructor(
                    "demo::SecretString",
                    "new_secret",
                    vec![NewtypeContainer::Optional],
                )),
                ..Default::default()
            }],
            ..Default::default()
        };

        let content = emit_lib_rs(&api_with_functions(vec![function]), &config());

        assert!(
            content.contains(".map(core_crate::SecretString::new_secret)"),
            "configured constructor must be emitted directly: {content}"
        );
        assert!(
            !content.contains("map(|value| core_crate::SecretString::new_secret(value))"),
            "configured constructor must not be wrapped in a redundant closure: {content}"
        );
    }

    #[test]
    fn optional_map_function_converts_wrapped_values_in_both_directions() {
        let map_type = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let metadata = wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::MapValue]);
        let function = crate::core::ir::FunctionDef {
            name: "optional_secret_map".to_string(),
            rust_path: "demo::optional_secret_map".to_string(),
            params: vec![ParamDef {
                name: "value".to_string(),
                ty: map_type.clone(),
                optional: true,
                newtype_wrapper: Some(metadata.clone()),
                ..Default::default()
            }],
            return_type: TypeRef::Optional(Box::new(map_type)),
            return_newtype_wrapper: Some(metadata),
            ..Default::default()
        };

        let content = emit_lib_rs(&api_with_functions(vec![function]), &config());

        assert!(
            content.contains("core_crate::optional_secret_map((value).map(|value| (value).into_iter().map(|(key, value)| (key, core_crate::SecretString::from(value))).collect()))"),
            "optional map input must wrap every value: {content}"
        );
        assert!(
            content.contains("let v = (v).map(|value| (value).into_iter().map(|(key, value)| (key, (value).into_inner())).collect::<std::collections::HashMap<_, _>>());"),
            "optional map return must unwrap every value: {content}"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    #[test]
    fn instance_methods_apply_explicit_wrapper_operations() {
        let mut replace = crate::core::ir::MethodDef {
            name: "replace_secret".to_string(),
            params: vec![secret_param(false, false)],
            return_type: TypeRef::String,
            return_newtype_wrapper: Some(wrapper(Vec::new())),
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            ..Default::default()
        };
        replace.error_type = Some("SecretError".to_string());
        let optional_map_type = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let optional_map_wrapper = wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::MapValue]);
        let optional_map = crate::core::ir::MethodDef {
            name: "optional_secret_map".to_string(),
            params: vec![ParamDef {
                name: "value".to_string(),
                ty: optional_map_type.clone(),
                optional: true,
                is_ref: true,
                newtype_wrapper: Some(optional_map_wrapper.clone()),
                ..Default::default()
            }],
            return_type: TypeRef::Optional(Box::new(optional_map_type)),
            return_newtype_wrapper: Some(optional_map_wrapper),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            ..Default::default()
        };
        let api = ApiSurface {
            crate_name: "demo".to_string(),
            types: vec![crate::core::ir::TypeDef {
                name: "Vault".to_string(),
                rust_path: "demo::Vault".to_string(),
                is_opaque: true,
                methods: vec![replace, optional_map],
                ..Default::default()
            }],
            ..Default::default()
        };

        let content = emit_lib_rs(&api, &config());

        assert!(
            content.contains("client.replace_secret(core_crate::SecretString::from(value))"),
            "method input must construct the wrapper: {content}"
        );
        assert!(
            content.contains("let v = (v).into_inner();"),
            "method Result-ok return must use the explicit extractor: {content}"
        );
        assert!(
            content.contains("client.optional_secret_map(value_newtype.as_ref())"),
            "optional borrowed method input must convert before borrowing: {content}"
        );
        assert!(
            content.contains("let v = (v).map(|value| (value).into_iter().map(|(key, value)| (key, (value).into_inner())).collect::<std::collections::HashMap<_, _>>());"),
            "optional method return must unwrap every value: {content}"
        );
        assert!(
            !content.contains("v.to_string()"),
            "wrapper return must not require Display"
        );
        assert!(
            !content.contains("v.as_ref()"),
            "wrapper return must not require AsRef<str>"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    #[test]
    fn value_methods_apply_explicit_wrapper_operations() {
        let method = crate::core::ir::MethodDef {
            name: "borrow_secret".to_string(),
            params: vec![secret_param(false, true)],
            return_type: TypeRef::String,
            return_newtype_wrapper: Some(wrapper(Vec::new())),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            ..Default::default()
        };
        let symbol = "Java_dev_demo_DemoBridge_nativeSegmentBorrowSecret";
        let mut content = emit_jni_lib_header(&ApiSurface::default(), &config(), "dev.demo");
        emit_value_method_shim(&mut content, symbol, "Segment", &method);

        assert!(
            content.contains(symbol),
            "value-method shim must be present, not only JNI boilerplate: {content}"
        );
        assert!(
            content.contains("let value_newtype = core_crate::SecretString::from(value);")
                && content.contains("client.borrow_secret(&value_newtype)"),
            "value-method input must convert before borrowing: {content}"
        );
        assert!(
            content.contains("let v = (v).into_inner();"),
            "value-method return must use the explicit extractor: {content}"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    #[test]
    fn value_method_optional_borrowed_map_converts_wrapped_values() {
        let map_type = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        let metadata = wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::MapValue]);
        let method = crate::core::ir::MethodDef {
            name: "optional_secret_map".to_string(),
            params: vec![ParamDef {
                name: "value".to_string(),
                ty: map_type.clone(),
                optional: true,
                is_ref: true,
                newtype_wrapper: Some(metadata.clone()),
                ..Default::default()
            }],
            return_type: TypeRef::Optional(Box::new(map_type)),
            return_newtype_wrapper: Some(metadata),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            ..Default::default()
        };
        let mut content = emit_jni_lib_header(&ApiSurface::default(), &config(), "dev.demo");
        emit_value_method_shim(
            &mut content,
            "Java_dev_demo_DemoBridge_nativeSegmentOptionalSecretMap",
            "Segment",
            &method,
        );

        assert!(
            content.contains(
                "let value: Option<std::collections::HashMap<String, String>> = match req_map.get(\"value\")"
            ),
            "value-method optional map must deserialize as Option<HashMap>: {content}"
        );
        assert!(
            content.contains("let value_newtype = (value).map(|value| (value).into_iter().map(|(key, value)| (key, core_crate::SecretString::from(value))).collect());")
                && content.contains("client.optional_secret_map(value_newtype.as_ref())"),
            "value-method optional map must convert before borrowing: {content}"
        );
        assert!(
            content.contains("let v = (v).map(|value| (value).into_iter().map(|(key, value)| (key, (value).into_inner())).collect::<std::collections::HashMap<_, _>>());"),
            "value-method optional map return must unwrap every value: {content}"
        );
        syn::parse_file(&content).expect("generated JNI crate parses");
    }

    #[test]
    fn nested_wrapper_paths_keep_the_module_suffix_under_the_core_alias() {
        let encoded = wrapper_with_path("demo::auth::SecretString", Vec::new());
        let remapped = jni_newtype_wrapper(&encoded);
        let decoded = NewtypeWrapper::decode(&remapped).expect("remapped metadata decodes");

        assert_eq!(decoded.explicit_paths()[0].rust_path, "core_crate::auth::SecretString");
    }
}
