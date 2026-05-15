use alef_backend_kotlin_android::KotlinAndroidBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef,
    PrimitiveType, TypeDef, TypeRef,
};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
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

fn make_basic_api() -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "demo::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("value", TypeRef::Primitive(PrimitiveType::I32), false),
                make_field("label", TypeRef::String, false),
                make_field("tag", TypeRef::Optional(Box::new(TypeRef::String)), true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "A demo configuration struct.".to_string(),
            cfg: None,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        }],
        functions: vec![FunctionDef {
            name: "process".into(),
            rust_path: "demo::process".into(),
            original_rust_path: String::new(),
            params: vec![
                make_param("input", TypeRef::String),
                make_param("count", TypeRef::Primitive(PrimitiveType::U32)),
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("DemoError".to_string()),
            doc: "Process input and return a result.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "demo::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Active state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Inactive state.".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing status.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
        }],
        errors: vec![ErrorDef {
            name: "DemoError".to_string(),
            rust_path: "demo::DemoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    message_template: Some("invalid input provided".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    doc: "Input validation failed.".to_string(),
                },
                ErrorVariant {
                    name: "ProcessingFailed".to_string(),
                    message_template: Some("processing failed".to_string()),
                    fields: vec![],
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    doc: "Processing encountered an error.".to_string(),
                },
            ],
            doc: "Errors emitted by demo operations.".to_string(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
    }
}

fn make_basic_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.kreuzberg"

[crates.kotlin_android]
package = "dev.kreuzberg"
namespace = "dev.kreuzberg"
artifact_id = "demo-android"
group_id = "dev.kreuzberg"
"#,
    )
}

#[test]
fn snapshot_basic() {
    let api = make_basic_api();
    let config = make_basic_config();
    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    assert!(!files.is_empty(), "Backend must emit at least one file");
    for file in &files {
        insta::assert_snapshot!(
            format!(
                "snapshot_basic__{}",
                file.path.display().to_string().replace(['/', '\\'], "__")
            ),
            &file.content
        );
    }
}

/// Regression: when `[crates.output].kotlin_android` points at the Kotlin
/// source destination (`src/main/kotlin/<pkg>/`), build metadata files
/// (build.gradle.kts, settings.gradle.kts, AndroidManifest.xml,
/// consumer/proguard rules, .gitignore, jniLibs/) MUST
/// be emitted at the derived project root — not nested inside the
/// source destination. No Java files are emitted (pure-Kotlin JNI AAR).
#[test]
fn build_metadata_goes_to_project_root_when_output_points_at_kotlin_source() {
    let api = make_basic_api();
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "java", "ffi"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.java]
package = "dev.kreuzberg"

[crates.kotlin_android]
package = "dev.kreuzberg.demo.android"
namespace = "dev.kreuzberg.demo.android"
artifact_id = "demo-android"
group_id = "dev.kreuzberg"

[crates.output]
kotlin_android = "packages/kotlin-android/src/main/kotlin/dev/kreuzberg/demo/android/"
"#,
    );

    let files = KotlinAndroidBackend.generate_bindings(&api, &config).unwrap();
    let paths: Vec<String> = files.iter().map(|f| f.path.display().to_string()).collect();

    let expect_at = |needle: &str| {
        assert!(
            paths.iter().any(|p| p == needle),
            "expected an emitted file at {needle:?}; got:\n{paths:#?}"
        );
    };
    let expect_none_at_prefix = |bad_prefix: &str, file_suffix: &str| {
        let nested = format!("{bad_prefix}/{file_suffix}");
        assert!(
            !paths.iter().any(|p| p == &nested),
            "did not expect {nested:?} (build metadata nested inside Kotlin source dir);\
             \nall paths:\n{paths:#?}"
        );
    };

    // Project root: packages/kotlin-android
    expect_at("packages/kotlin-android/build.gradle.kts");
    expect_at("packages/kotlin-android/settings.gradle.kts");
    expect_at("packages/kotlin-android/consumer-rules.pro");
    expect_at("packages/kotlin-android/proguard-rules.pro");
    expect_at("packages/kotlin-android/.gitignore");
    expect_at("packages/kotlin-android/src/main/AndroidManifest.xml");
    expect_at("packages/kotlin-android/src/main/jniLibs/arm64-v8a/.gitkeep");
    expect_at("packages/kotlin-android/src/main/jniLibs/x86_64/.gitkeep");

    // No Java files — the AAR is pure-Kotlin JNI.
    let java_files: Vec<_> = paths.iter().filter(|p| p.ends_with(".java")).collect();
    assert!(
        java_files.is_empty(),
        "kotlin-android must not emit Java files; got: {java_files:?}"
    );

    // JNI Bridge + module object at the configured Kotlin source path.
    expect_at("packages/kotlin-android/src/main/kotlin/dev/kreuzberg/demo/android/DemoBridge.kt");
    expect_at("packages/kotlin-android/src/main/kotlin/dev/kreuzberg/demo/android/Demo.kt");

    // Negative assertions: nothing should be emitted under the source
    // destination as if it were the project root.
    let kotlin_src = "packages/kotlin-android/src/main/kotlin/dev/kreuzberg/demo/android";
    expect_none_at_prefix(kotlin_src, "build.gradle.kts");
    expect_none_at_prefix(kotlin_src, "settings.gradle.kts");
    expect_none_at_prefix(kotlin_src, ".gitignore");
    expect_none_at_prefix(kotlin_src, "consumer-rules.pro");
    expect_none_at_prefix(kotlin_src, "proguard-rules.pro");
    expect_none_at_prefix(kotlin_src, "src/main/AndroidManifest.xml");
}
