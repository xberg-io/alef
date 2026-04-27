use alef_backend_kotlin::KotlinBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, FfiConfig, KotlinConfig, KotlinTarget};
use alef_core::ir::{
    ApiSurface, CoreWrapper, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef,
    PrimitiveType, TypeDef, TypeRef,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

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

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,

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
    }
}

fn make_function(name: &str, params: Vec<ParamDef>, return_type: TypeRef, is_async: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        params,
        return_type,
        is_async,
        doc: String::new(),
        error_type: None,
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
    }
}

/// Build a multiplatform-target config.
fn make_mpp_config(crate_name: &str) -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: crate_name.to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
            auto_path_mappings: Default::default(),
            extra_dependencies: Default::default(),
            source_crates: vec![],
            error_type: None,
            error_constructor: None,
        },
        languages: vec![],
        exclude: Default::default(),
        include: Default::default(),
        output: Default::default(),
        python: None,
        node: None,
        ruby: None,
        php: None,
        elixir: None,
        wasm: None,
        ffi: Some(FfiConfig {
            prefix: Some("demo".to_string()),
            error_style: "last_error".to_string(),
            header_name: Some("demo.h".to_string()),
            lib_name: Some("demo_ffi".to_string()),
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: vec![],
            exclude_types: vec![],
            rename_fields: std::collections::HashMap::new(),
        }),
        gleam: None,
        go: None,
        java: None,
        kotlin: Some(KotlinConfig {
            package: Some("dev.kreuzberg.demo".to_string()),
            features: None,
            serde_rename_all: None,
            rename_fields: std::collections::HashMap::new(),
            exclude_functions: vec![],
            exclude_types: vec![],
            run_wrapper: None,
            extra_lint_paths: vec![],
            target: KotlinTarget::Multiplatform,
        }),
        dart: None,
        swift: None,
        csharp: None,
        r: None,
        zig: None,
        scaffold: None,
        readme: None,
        lint: None,
        update: None,
        test: None,
        setup: None,
        clean: None,
        build_commands: None,
        publish: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        e2e: None,
        trait_bridges: vec![],
        tools: alef_core::config::ToolsConfig::default(),
    format: ::alef_core::config::FormatConfig::default(),
    format_overrides: ::std::collections::HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// KMP target emits five files: commonMain .kt, jvmMain .kt, nativeMain .kt, .def, build.gradle.kts.
#[test]
fn mpp_emits_five_files() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 5, "expected 5 files (commonMain, jvmMain, nativeMain, def, gradle): {files:?}");
}

/// commonMain contains `expect object` with function signatures only (no bodies).
#[test]
fn mpp_common_contains_expect_object() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_function(
            "convert_html",
            vec![make_param("input", TypeRef::String)],
            TypeRef::String,
            false,
        )],
        enums: vec![],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let common = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("commonMain"))
        .expect("commonMain file missing");

    assert!(
        common.content.contains("expect object MyCrate"),
        "missing expect object: {}",
        common.content
    );
    // Expect signature — no body (no opening brace after the signature line).
    assert!(
        common.content.contains("fun convertHtml(input: String): String"),
        "missing expect signature: {}",
        common.content
    );
    // No Bridge call in commonMain.
    assert!(
        !common.content.contains("Bridge."),
        "commonMain must not contain Bridge calls: {}",
        common.content
    );
    // No memScoped in commonMain.
    assert!(
        !common.content.contains("memScoped"),
        "commonMain must not contain memScoped: {}",
        common.content
    );
}

/// jvmMain contains `actual object` with Bridge delegate calls.
#[test]
fn mpp_jvm_contains_actual_object_with_bridge() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_function(
            "greet",
            vec![make_param("name", TypeRef::String)],
            TypeRef::Primitive(PrimitiveType::I32),
            false,
        )],
        enums: vec![],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let jvm = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("jvmMain"))
        .expect("jvmMain file missing");

    assert!(
        jvm.content.contains("actual object MyCrate"),
        "missing actual object: {}",
        jvm.content
    );
    assert!(
        jvm.content.contains("Bridge.greet("),
        "missing Bridge call in jvmMain: {}",
        jvm.content
    );
}

/// nativeMain contains `actual object` with memScoped bodies.
#[test]
fn mpp_native_contains_actual_object_with_mem_scoped() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![make_function(
            "convert_html",
            vec![make_param("input", TypeRef::String)],
            TypeRef::String,
            false,
        )],
        enums: vec![],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let native = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("nativeMain"))
        .expect("nativeMain file missing");

    assert!(
        native.content.contains("actual object MyCrate"),
        "missing actual object in nativeMain: {}",
        native.content
    );
    assert!(
        native.content.contains("memScoped {"),
        "missing memScoped in nativeMain: {}",
        native.content
    );
    assert!(
        native.content.contains("demo_convert_html("),
        "missing C call in nativeMain: {}",
        native.content
    );
}

/// DTOs (data classes) appear in commonMain only — not duplicated in jvmMain or nativeMain.
#[test]
fn mpp_dtos_only_in_common_main() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![make_type(
            "ConversionOptions",
            vec![make_field("max_width", TypeRef::Primitive(PrimitiveType::U32), false)],
        )],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let common = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("commonMain"))
        .expect("commonMain file missing");
    let jvm = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("jvmMain"))
        .expect("jvmMain file missing");
    let native = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("nativeMain"))
        .expect("nativeMain file missing");

    assert!(
        common.content.contains("data class ConversionOptions("),
        "DTO missing from commonMain: {}",
        common.content
    );
    assert!(
        !jvm.content.contains("data class ConversionOptions"),
        "DTO must not be duplicated in jvmMain: {}",
        jvm.content
    );
    assert!(
        !native.content.contains("data class ConversionOptions"),
        "DTO must not be duplicated in nativeMain: {}",
        native.content
    );
}

/// .def file references the correct header, prefix, and library name.
#[test]
fn mpp_def_file_has_correct_fields() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let def = files
        .iter()
        .find(|f| f.path.extension().is_some_and(|e| e == "def"))
        .expect(".def file missing");

    assert!(def.content.contains("headers = demo.h"), "missing header: {}", def.content);
    assert!(def.content.contains("headerFilter = demo_*"), "missing headerFilter: {}", def.content);
    assert!(def.content.contains("-ldemo_ffi"), "missing linker flag: {}", def.content);
}

/// build.gradle.kts contains the multiplatform plugin and jvm() + linuxX64 + macosArm64 targets.
#[test]
fn mpp_gradle_uses_multiplatform_plugin() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let gradle = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|n| n == "build.gradle.kts"))
        .expect("build.gradle.kts missing");

    assert!(
        gradle.content.contains(r#"kotlin("multiplatform")"#),
        "missing multiplatform plugin: {}",
        gradle.content
    );
    assert!(gradle.content.contains("jvm()"), "missing jvm target: {}", gradle.content);
    assert!(gradle.content.contains("linuxX64"), "missing linuxX64 target: {}", gradle.content);
    assert!(gradle.content.contains("macosArm64"), "missing macosArm64 target: {}", gradle.content);
}

/// Sealed enum with payload fields works in commonMain (pure Kotlin, no platform deps).
#[test]
fn mpp_sealed_enum_in_common_main() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "demo::OutputFormat".to_string(),
            original_rust_path: String::new(),
            doc: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Markdown".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
                EnumVariant {
                    name: "PlainText".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                is_tuple: false,
                },
            ],
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,

            is_copy: false,
            has_serde: false,
        }],
        errors: vec![],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let common = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("commonMain"))
        .expect("commonMain file missing");

    assert!(
        common.content.contains("enum class OutputFormat"),
        "missing enum in commonMain: {}",
        common.content
    );
}

/// Error sealed class appears in commonMain only.
#[test]
fn mpp_error_sealed_class_in_common_main() {
    let api = ApiSurface {
        crate_name: "my-crate".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "ApiError".into(),
            rust_path: "demo::ApiError".into(),
            original_rust_path: String::new(),
            doc: String::new(),
            variants: vec![ErrorVariant {
                name: "NotFound".into(),
                message_template: Some("not found".into()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                doc: String::new(),
            }],
        }],
    };
    let config = make_mpp_config("my-crate");
    let files = KotlinBackend.generate_bindings(&api, &config).unwrap();

    let common = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("commonMain"))
        .expect("commonMain file missing");
    let jvm = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("jvmMain"))
        .expect("jvmMain file missing");

    assert!(
        common.content.contains("sealed class ApiError"),
        "error class missing from commonMain: {}",
        common.content
    );
    assert!(
        !jvm.content.contains("sealed class ApiError"),
        "error class must not be duplicated in jvmMain: {}",
        jvm.content
    );
}
