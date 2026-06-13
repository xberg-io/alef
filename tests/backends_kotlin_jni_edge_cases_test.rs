use alef::backends::jni::JniBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::core::backend::Backend;
use alef::core::config::workspace::{ClientConstructorConfig, ConstructorParam};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

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

fn make_jni_config_no_streaming() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.kotlin]
package = "dev.sample_crate"
ffi_style = "jni"
"#,
    )
}

fn make_pairing_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin", "kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.sample_crate"
ffi_style = "jni"

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
    )
}

fn make_jni_api_with_client_and_function() -> ApiSurface {
    let foo_function = FunctionDef {
        name: "foo".into(),
        rust_path: "demo::foo".into(),
        original_rust_path: String::new(),
        params: vec![make_param("value", TypeRef::Primitive(PrimitiveType::I32))],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
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
    };
    let client_method = MethodDef {
        name: "do_thing".into(),
        params: vec![make_param("input", TypeRef::String)],
        return_type: TypeRef::String,
        is_async: true,
        is_static: false,
        error_type: None,
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
    };
    let client_type = TypeDef {
        name: "DefaultClient".into(),
        rust_path: "demo::DefaultClient".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![client_method],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
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
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![client_type],
        functions: vec![foo_function],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    }
}

#[test]
fn kotlin_jni_pairing_sentinel() {
    let api = make_jni_api_with_client_and_function();
    let config = make_pairing_config();
    let package = "dev.sample_crate";
    let bridge_class = alef::core::jni::bridge_class_name("demo");

    let kotlin_files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let bridge_file = kotlin_files
        .iter()
        .find(|f| {
            f.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("Bridge.kt"))
                .unwrap_or(false)
        })
        .expect("DemoBridge.kt must be emitted by KotlinBackend");

    let mut kotlin_native_names: std::collections::BTreeSet<String> = extract_kotlin_native_names(&bridge_file.content);

    let bridge_name = alef::core::jni::bridge_class_name("demo");
    let free_call_prefix = format!("{bridge_name}.native");
    for file in &kotlin_files {
        if let Some(name) = file.path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".kt") && !name.ends_with("Bridge.kt") {
                for line in file.content.lines() {
                    let mut search = line;
                    while let Some(pos) = search.find(&free_call_prefix) {
                        let rest = &search[pos + free_call_prefix.len()..];
                        if let Some(paren) = rest.find('(') {
                            let method_name = format!("native{}", &rest[..paren]);
                            if method_name.starts_with("nativeFree") {
                                kotlin_native_names.insert(method_name);
                            }
                        }
                        search = &search[pos + 1..];
                    }
                }
            }
        }
    }

    let rust_files = JniBackend.generate_bindings(&api, &config).unwrap();
    let rust_java_symbols: std::collections::BTreeSet<String> = extract_rust_java_symbols(&rust_files[0].content);

    let kotlin_expected_symbols: std::collections::BTreeSet<String> = kotlin_native_names
        .iter()
        .map(|name| alef::core::jni::jni_symbol(package, &bridge_class, name))
        .collect();

    let missing_in_rust: Vec<_> = kotlin_expected_symbols.difference(&rust_java_symbols).collect();
    let extra_in_rust: Vec<_> = rust_java_symbols.difference(&kotlin_expected_symbols).collect();

    assert!(
        missing_in_rust.is_empty() && extra_in_rust.is_empty(),
        "JNI symbol pairing drift detected!\n\
         Kotlin declared but Rust missing: {missing_in_rust:?}\n\
         Rust emitted but Kotlin missing: {extra_in_rust:?}\n\
         \nKotlin `external fun` names: {kotlin_native_names:?}\n\
         Rust `Java_*` symbols: {rust_java_symbols:?}"
    );
}

fn extract_kotlin_native_names(content: &str) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("external fun ") {
            if let Some(paren) = rest.find('(') {
                let name = rest[..paren].trim().to_string();
                if !name.is_empty() {
                    names.insert(name);
                }
            }
        }
    }
    names
}

fn extract_rust_java_symbols(content: &str) -> std::collections::BTreeSet<String> {
    let mut syms = std::collections::BTreeSet::new();
    let marker = "pub unsafe extern \"system\" fn ";
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(marker) {
            if let Some(paren) = rest.find('(') {
                let sym = rest[..paren].trim().to_string();
                if sym.starts_with("Java_") {
                    syms.insert(sym);
                }
            }
        }
    }
    syms
}

#[test]
fn client_constructors_emits_bridge_extern_and_factory_method() {
    let api = make_jni_api_with_client_and_function();
    let mut config = make_jni_config_no_streaming();
    config.client_constructors.insert(
        "DefaultClient".to_string(),
        ClientConstructorConfig {
            params: vec![ConstructorParam {
                name: "api_key".to_string(),
                ty: "*const std::ffi::c_char".to_string(),
            }],
            body: "{source_path}::new(api_key)".to_string(),
            error_type: Some("DemoError".to_string()),
        },
    );

    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    let bridge_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Bridge.kt"))
        .expect("Bridge.kt must be emitted");
    let client_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("DefaultClient.kt"))
        .expect("DefaultClient.kt must be emitted");

    let bridge = &bridge_file.content;
    assert!(
        bridge.contains("external fun nativeNewDefaultClient(apiKey: String): Long"),
        "Bridge must declare nativeNewDefaultClient; got:\n{bridge}"
    );
    assert!(
        bridge.contains("@Throws(DemoBridgeException::class)"),
        "Bridge nativeNew must have @Throws annotation; got:\n{bridge}"
    );

    let client = &client_file.content;
    assert!(
        client.contains("fun create(apiKey: String): DefaultClient"),
        "DefaultClient must have create factory method; got:\n{client}"
    );
    assert!(
        client.contains("DemoBridge.nativeNewDefaultClient(apiKey)"),
        "factory method must call DemoBridge.nativeNewDefaultClient; got:\n{client}"
    );
}
