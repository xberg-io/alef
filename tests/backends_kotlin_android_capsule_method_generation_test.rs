use alef::backends::jni::JniBackend;
use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

const CAPSULE_CONFIG: &str = r#"
[workspace]
languages = ["kotlin_android", "jni", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]
version_from = "/nonexistent/Cargo.toml"

[crates.kotlin_android]
package = "dev.sample"

[crates.kotlin_android.capsule_types.Language]
host_type = "dev.runtime.Language"
construct_expr = "dev.runtime.Language({ptr})"
pointer_ownership = "borrowed_static"
host_destructor = "none"
abi_compatible = true

[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"

[crates.package_metadata]
repository = "https://github.com/example/demo"
license = "MIT"
"#;

fn capsule_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(CAPSULE_CONFIG).expect("capsule config should parse");
    config.resolve().expect("capsule config should resolve").remove(0)
}

fn unsafe_owned_capsule_config() -> ResolvedCrateConfig {
    let unsafe_config = CAPSULE_CONFIG
        .replace("pointer_ownership = \"borrowed_static\"\n", "")
        .replace("host_destructor = \"none\"\n", "")
        .replace("abi_compatible = true\n", "");
    let config: NewAlefConfig = toml::from_str(&unsafe_config).expect("unsafe capsule config should parse");
    config
        .resolve()
        .expect("unsafe capsule config should resolve")
        .remove(0)
}

fn capsule_method_api(return_type: TypeRef) -> ApiSurface {
    let get_language = MethodDef {
        name: "get_language".into(),
        return_type,
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    };
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![
            TypeDef {
                name: "DefaultClient".into(),
                methods: vec![get_language],
                is_opaque: true,
                ..Default::default()
            },
            TypeDef {
                name: "Language".into(),
                is_opaque: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn capsule_function_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        functions: vec![
            FunctionDef {
                name: "language_sample".into(),
                rust_path: "demo::language_sample".into(),
                return_type: TypeRef::Named("Language".into()),
                ..Default::default()
            },
            FunctionDef {
                name: "optional_language_sample".into(),
                rust_path: "demo::optional_language_sample".into(),
                return_type: TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
                ..Default::default()
            },
        ],
        types: vec![TypeDef {
            name: "Language".into(),
            is_opaque: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn ffi_only_capsule_api() -> ApiSurface {
    let mut api = capsule_method_api(TypeRef::Named("Language".into()));
    api.functions.extend([
        FunctionDef {
            name: "language_sample".into(),
            rust_path: "demo::language_sample".into(),
            return_type: TypeRef::Named("Language".into()),
            ..Default::default()
        },
        FunctionDef {
            name: "optional_language_sample".into(),
            rust_path: "demo::optional_language_sample".into(),
            return_type: TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
            ..Default::default()
        },
    ]);
    api
}

fn ffi_only_capsule_method_api() -> ApiSurface {
    let mut api = capsule_method_api(TypeRef::Named("Language".into()));
    let client = api
        .types
        .iter_mut()
        .find(|type_def| type_def.name == "DefaultClient")
        .expect("fixture should contain DefaultClient");
    client.methods.push(MethodDef {
        name: "find_language".into(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Named("Language".into()))),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        ..Default::default()
    });
    api
}

fn file_name(file: &GeneratedFile) -> String {
    file.path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("generated file should have a UTF-8 name")
        .to_owned()
}

#[test]
fn should_generate_host_capsule_for_opaque_client_method_return() {
    let files = KotlinAndroidBackend
        .generate_bindings(
            &capsule_method_api(TypeRef::Named("Language".into())),
            &capsule_config(),
        )
        .expect("Kotlin Android bindings should generate");
    let client = files
        .iter()
        .find(|file| file_name(file) == "DefaultClient.kt")
        .expect("DefaultClient.kt should be generated");
    let signature = client.content.lines().find(|line| line.contains("fun getLanguage("));
    let construction = client
        .content
        .lines()
        .find(|line| line.trim_start().starts_with("return ") && line.contains("Language("));
    let language_files: Vec<_> = files
        .iter()
        .map(file_name)
        .filter(|name| name == "Language.kt")
        .collect();
    let free_bridge_files: Vec<_> = files
        .iter()
        .filter(|file| file.content.contains("nativeFreeLanguage"))
        .map(file_name)
        .collect();

    assert_eq!(
        (signature, construction, language_files, free_bridge_files),
        (
            Some("    fun getLanguage(): dev.runtime.Language {"),
            Some("        return dev.runtime.Language(capsulePtr)"),
            Vec::<String>::new(),
            Vec::<String>::new(),
        )
    );
}

#[test]
fn should_generate_nullable_host_capsule_for_optional_method_return() {
    let return_type = TypeRef::Optional(Box::new(TypeRef::Named("Language".into())));
    let files = KotlinAndroidBackend
        .generate_bindings(&capsule_method_api(return_type), &capsule_config())
        .expect("Kotlin Android bindings should generate");
    let client = files
        .iter()
        .find(|file| file_name(file) == "DefaultClient.kt")
        .expect("DefaultClient.kt should be generated");
    let signature = client.content.lines().find(|line| line.contains("fun getLanguage("));
    let construction = client
        .content
        .lines()
        .find(|line| line.trim_start().starts_with("return if (capsulePtr"));

    assert_eq!(
        (signature, construction),
        (
            Some("    fun getLanguage(): dev.runtime.Language? {"),
            Some("        return if (capsulePtr == 0L) null else dev.runtime.Language(capsulePtr)"),
        )
    );
}

#[test]
fn should_declare_configured_capsule_method_as_long_without_ir_type_definition() {
    let mut api = capsule_method_api(TypeRef::Named("Language".into()));
    api.types.retain(|type_def| type_def.name != "Language");
    let files = KotlinAndroidBackend
        .generate_bindings(&api, &capsule_config())
        .expect("Kotlin Android bindings should generate");
    let bridge = files
        .iter()
        .find(|file| file_name(file) == "DemoBridge.kt")
        .expect("DemoBridge.kt should be generated");
    let declaration = bridge
        .content
        .lines()
        .find(|line| line.contains("nativeDefaultClientGetLanguage"));

    assert_eq!(
        declaration,
        Some("    external fun nativeDefaultClientGetLanguage(handle: Long): Long")
    );
}

#[test]
fn should_generate_direct_and_optional_host_capsule_free_functions() {
    let api = capsule_function_api();
    let kotlin_files = KotlinAndroidBackend
        .generate_bindings(&api, &capsule_config())
        .expect("Kotlin Android capsule functions should generate");
    let facade = kotlin_files
        .iter()
        .find(|file| file.content.contains("fun languageSample("))
        .expect("module facade should contain capsule functions");
    let bridge = kotlin_files
        .iter()
        .find(|file| file_name(file) == "DemoBridge.kt")
        .expect("DemoBridge.kt should be generated");
    let jni_files = JniBackend
        .generate_bindings(&api, &capsule_config())
        .expect("JNI capsule functions should generate");
    let jni = &jni_files[0].content;

    assert!(facade.content.contains("fun languageSample(): dev.runtime.Language {"));
    assert!(
        facade
            .content
            .contains("fun optionalLanguageSample(): dev.runtime.Language? {")
    );
    assert!(
        facade
            .content
            .contains("return if (capsulePtr == 0L) null else dev.runtime.Language(capsulePtr)")
    );
    assert!(
        bridge
            .content
            .contains("external fun nativeOptionalLanguageSample(): Long")
    );
    assert!(jni.contains("v.into_raw() as jlong"));
    assert!(jni.contains("Some(inner) => inner.into_raw() as jlong"));
    assert!(!jni.contains("nativeFreeLanguage"));
}

#[test]
fn should_reject_host_capsule_without_matching_ffi_definition() {
    let mut config = capsule_config();
    config
        .ffi
        .as_mut()
        .expect("fixture should configure FFI")
        .capsule_types
        .clear();

    let error = KotlinAndroidBackend
        .generate_bindings(&capsule_method_api(TypeRef::Named("Language".into())), &config)
        .expect_err("host-only capsule configuration must fail closed");

    assert_eq!(
        error.to_string(),
        "kotlin_android capsule types require matching FFI capsule definitions: Language; \
         add each type under `[crates.ffi.capsule_types.<Type>]`"
    );
}

#[test]
fn should_generate_ordinary_owned_handle_for_ffi_only_capsule_definition() {
    let mut config = capsule_config();
    config
        .kotlin_android
        .as_mut()
        .expect("fixture should configure Kotlin Android")
        .capsule_types
        .clear();

    let api = ffi_only_capsule_api();
    let kotlin_files = KotlinAndroidBackend
        .generate_bindings(&api, &config)
        .expect("FFI-only capsule should use ordinary Kotlin handles");
    let client = kotlin_files
        .iter()
        .find(|file| file_name(file) == "DefaultClient.kt")
        .expect("DefaultClient.kt should be generated");
    let facade = kotlin_files
        .iter()
        .find(|file| file.content.contains("fun languageSample("))
        .expect("module facade should contain ordinary free function");
    let jni_files = JniBackend
        .generate_bindings(&api, &config)
        .expect("FFI-only capsule should use ordinary JNI handles");
    assert_ffi_only_owned_shape(&kotlin_files, &client.content, &facade.content, &jni_files[0].content);
}

#[test]
fn should_declare_destructor_for_ffi_only_capsule_returned_only_by_methods() {
    let mut config = capsule_config();
    config
        .kotlin_android
        .as_mut()
        .expect("fixture should configure Kotlin Android")
        .capsule_types
        .clear();
    let api = ffi_only_capsule_method_api();
    let files = KotlinAndroidBackend
        .generate_bindings(&api, &config)
        .expect("FFI-only method returns should use ordinary Kotlin handles");
    let bridge = files
        .iter()
        .find(|file| file_name(file) == "DemoBridge.kt")
        .expect("DemoBridge.kt should be generated");
    assert!(bridge.content.contains("external fun nativeFreeLanguage(handle: Long)"));
    assert!(
        bridge
            .content
            .contains("nativeDefaultClientGetLanguage(handle: Long): Long")
    );
    assert!(
        bridge
            .content
            .contains("nativeDefaultClientFindLanguage(handle: Long): Long")
    );
}

fn assert_ffi_only_owned_shape(kotlin_files: &[GeneratedFile], client: &str, facade: &str, jni: &str) {
    assert_ffi_only_direct_kotlin_shape(kotlin_files, client, facade);
    assert_ffi_only_optional_kotlin_shape(kotlin_files, facade);
    assert_ffi_only_jni_shape(jni);
}

fn assert_ffi_only_direct_kotlin_shape(kotlin_files: &[GeneratedFile], client: &str, facade: &str) {
    assert_eq!(
        (
            client.lines().find(|line| line.contains("fun getLanguage(")),
            client.contains("return Language(handle)"),
            facade.contains("fun languageSample(): Language"),
            facade.contains("Language(DemoBridge.nativeLanguageSample())"),
            kotlin_files.iter().any(|file| file_name(file) == "Language.kt"),
            kotlin_files
                .iter()
                .any(|file| file.content.contains("nativeFreeLanguage")),
        ),
        (Some("    fun getLanguage(): Language {"), true, true, true, true, true,)
    );
}

fn assert_ffi_only_optional_kotlin_shape(kotlin_files: &[GeneratedFile], facade: &str) {
    assert!(facade.contains("fun optionalLanguageSample(): Language?"), "{facade}");
    assert!(
        facade.contains("DemoBridge.nativeOptionalLanguageSample().takeIf { it != 0L }?.let(::Language)"),
        "{facade}"
    );
    assert!(kotlin_files.iter().any(|file| {
        file.content
            .contains("external fun nativeOptionalLanguageSample(): Long")
    }));
}

fn assert_ffi_only_jni_shape(jni: &str) {
    assert_eq!(jni.matches("Box::into_raw(Box::new(v)) as jlong").count(), 2);
    assert!(jni.contains("Some(inner) => Box::into_raw(Box::new(inner)) as jlong"));
    assert!(jni.contains("None => 0"));
    assert!(jni.contains("nativeFreeLanguage"));
    assert!(!jni.contains("v.into_raw() as jlong"));
}

#[test]
fn should_reject_owned_host_capsule_without_shared_native_runtime() {
    let error = KotlinAndroidBackend
        .generate_bindings(
            &capsule_method_api(TypeRef::Named("Language".into())),
            &unsafe_owned_capsule_config(),
        )
        .expect_err("owned host capsules require a shared native runtime");

    assert_eq!(
        error.to_string(),
        "capsule configuration in backend `kotlin_android` cannot safely wrap native pointers: \
         capsule type `Language`: `pointer_ownership = \"borrowed_static\"` is required, \
         `abi_compatible = true` is required, `host_destructor = \"none\"` or \
         `host_destructor = \"abi_noop\"` is required; declare a complete borrowed-static \
         ABI-compatible no-destructor contract for every listed capsule, or set \
         `[crates.kotlin_android].shares_native_runtime = true` when every host wrapper shares one \
         native runtime and ownership contract"
    );
}
