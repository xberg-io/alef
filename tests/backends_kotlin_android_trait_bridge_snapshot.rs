use alef::backends::jni::JniBackend;
use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

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
    }
}

fn make_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        receiver: None,
        error_type: None,
        doc: String::new(),
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

/// Create a mock API with two traits: a sync trait and an async trait.
fn make_trait_api() -> ApiSurface {
    let ocr_trait = TypeDef {
        name: "OcrBackend".to_string(),
        rust_path: "kreuzberg::OcrBackend".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![
            make_method("name", vec![], TypeRef::String, false),
            make_method(
                "recognize",
                vec![
                    make_param(
                        "image_bytes",
                        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
                    ),
                    make_param("language", TypeRef::String),
                ],
                TypeRef::String,
                true,
            ),
        ],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: "OCR backend trait.".to_string(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec!["Plugin".to_string()],
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let processor_trait = TypeDef {
        name: "PostProcessor".to_string(),
        rust_path: "kreuzberg::PostProcessor".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![make_method(
            "process",
            vec![make_param("text", TypeRef::String)],
            TypeRef::String,
            false,
        )],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: "Post-processing trait.".to_string(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec!["Plugin".to_string()],
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    ApiSurface {
        crate_name: "kreuzberg".into(),
        version: "0.1.0".into(),
        types: vec![ocr_trait, processor_trait],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    }
}

#[test]
fn test_jni_trait_register_shim_generation() {
    let toml = r#"
[crates.demo]
name = "demo"
package = "dev.kreuzberg"
kotlin_android.package = "dev.kreuzberg"
kotlin_ffi_style = "jni"

[[crates.demo.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "Plugin"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
clear_fn = "clear_ocr_backends"
"#;
    let config = resolved_one(toml);
    let api = make_trait_api();

    // Generate Kotlin Android files
    let kotlin_android_backend = KotlinAndroidBackend;
    let kotlin_files = kotlin_android_backend.generate_bindings(&api, &config).unwrap();
    let bridge_file = kotlin_files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Bridge.kt"));
    assert!(bridge_file.is_some(), "Bridge.kt should be generated");

    let bridge_content = &bridge_file.unwrap().content;
    // Check that trait bridge external funs are in the Bridge
    assert!(
        bridge_content.contains("external fun nativeRegisterOcrBackend"),
        "Should generate nativeRegisterOcrBackend"
    );
    assert!(
        bridge_content.contains("external fun nativeUnregisterOcrBackend"),
        "Should generate nativeUnregisterOcrBackend"
    );
    assert!(
        bridge_content.contains("external fun nativeClearOcrBackends"),
        "Should generate nativeClearOcrBackends"
    );

    // Check that IOcrBackend interface is generated
    let iocr_file = kotlin_files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("IOcrBackend.kt"));
    assert!(iocr_file.is_some(), "IOcrBackend.kt should be generated");
    let iocr_content = &iocr_file.unwrap().content;
    assert!(
        iocr_content.contains("interface IOcrBackend"),
        "Should have IOcrBackend interface"
    );
    assert!(
        iocr_content.contains("suspend fun recognize"),
        "Should have async recognize method"
    );

    // Generate JNI shim
    let jni_backend = JniBackend;
    let jni_files = jni_backend.generate_bindings(&api, &config).unwrap();
    assert_eq!(jni_files.len(), 1, "Should generate one JNI file");
    let jni_content = &jni_files[0].content;

    // Check that trait register shim is generated
    assert!(
        jni_content.contains("Java_dev_kreuzberg_KreuzbergBridge_nativeRegisterOcrBackend"),
        "Should generate JNI register shim symbol"
    );
    assert!(
        jni_content.contains("jni_call_string_method"),
        "Should use jni_call_string_method helper"
    );
    assert!(jni_content.contains("NewGlobalRef"), "Should create global reference");
    assert!(
        jni_content.contains("register_ocr_backend"),
        "Should call the host register function"
    );

    // Check that unregister and clear are also present
    assert!(
        jni_content.contains("Java_dev_kreuzberg_KreuzbergBridge_nativeUnregisterOcrBackend"),
        "Should generate JNI unregister shim"
    );
    assert!(
        jni_content.contains("Java_dev_kreuzberg_KreuzbergBridge_nativeClearOcrBackends"),
        "Should generate JNI clear shim"
    );
}

#[test]
fn test_jni_trait_bridge_multiple_traits() {
    let toml = r#"
[crates.demo]
name = "demo"
package = "dev.kreuzberg"
kotlin_android.package = "dev.kreuzberg"
kotlin_ffi_style = "jni"

[[crates.demo.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "Plugin"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"

[[crates.demo.trait_bridges]]
trait_name = "PostProcessor"
super_trait = "Plugin"
register_fn = "register_post_processor"
clear_fn = "clear_post_processors"
"#;
    let config = resolved_one(toml);
    let api = make_trait_api();

    let jni_backend = JniBackend;
    let jni_files = jni_backend.generate_bindings(&api, &config).unwrap();
    let jni_content = &jni_files[0].content;

    // Both traits should have registration support
    assert!(
        jni_content.contains("nativeRegisterOcrBackend"),
        "Should register OcrBackend"
    );
    assert!(
        jni_content.contains("nativeRegisterPostProcessor"),
        "Should register PostProcessor"
    );
    assert!(
        jni_content.contains("register_post_processor"),
        "Should call host register_post_processor"
    );
    assert!(
        jni_content.contains("nativeClearPostProcessors"),
        "Should clear PostProcessor"
    );
}

#[test]
fn test_jni_trait_bridge_excluded_language() {
    let toml = r#"
[crates.demo]
name = "demo"
package = "dev.kreuzberg"
kotlin_android.package = "dev.kreuzberg"
kotlin_ffi_style = "jni"

[[crates.demo.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "Plugin"
register_fn = "register_ocr_backend"
exclude_languages = ["kotlin_android"]
"#;
    let config = resolved_one(toml);
    let api = make_trait_api();

    let jni_backend = JniBackend;
    let jni_files = jni_backend.generate_bindings(&api, &config).unwrap();
    let jni_content = &jni_files[0].content;

    // Should not generate shim for excluded language
    assert!(
        !jni_content.contains("nativeRegisterOcrBackend"),
        "Should not register excluded trait"
    );
}
