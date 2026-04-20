use alef_backend_java::JavaBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, CrateConfig, FfiConfig, JavaConfig};
use alef_core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};

fn make_test_config(package: &str) -> AlefConfig {
    AlefConfig {
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
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
            prefix: Some("test".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
        }),
        go: None,
        java: Some(JavaConfig {
            package: Some(package.to_string()),
            ffi_style: "panama".to_string(),
            features: None,
            serde_rename_all: None,
        }),
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
    }
}

fn make_newtype_field(ty: TypeRef) -> FieldDef {
    FieldDef {
        name: "0".to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef_core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
        newtype_wrapper: None,
    }
}

#[test]
fn test_basic_generation() {
    let backend = JavaBackend;

    // Create test API surface
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            fields: vec![FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: false,
                default: None,
                doc: "Timeout in seconds".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: alef_core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                newtype_wrapper: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            doc: "Test config".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "extract".to_string(),
            rust_path: "test_lib::extract".to_string(),
            params: vec![ParamDef {
                name: "path".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text".to_string(),
            cfg: None,
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    // Create test config
    let config = AlefConfig {
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
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
            prefix: Some("test".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
        }),
        go: None,
        java: Some(JavaConfig {
            package: Some("com.example".to_string()),
            ffi_style: "panama".to_string(),
            features: None,
            serde_rename_all: None,
        }),
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
    };

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok());
    let files = result.unwrap();

    // Should generate 6 files:
    // 1. NativeLib.java
    // 2. TestLibRs.java (main class — "Rs" suffix avoids facade/FFI name collision)
    // 3. TestLibRsException.java
    // 4. Config.java (record)
    // 5. Mode.java (enum)
    // 6. Additional generated file (e.g. loader or helper)
    assert_eq!(files.len(), 6);

    // Check NativeLib.java
    let native_lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeLib"))
        .unwrap();
    assert!(native_lib.content.contains("class NativeLib"));
    assert!(native_lib.content.contains("TEST_EXTRACT"));
    assert!(native_lib.content.contains("MethodHandle"));

    // Check main class (PascalCase + "Rs" suffix)
    let main_class = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .unwrap();
    assert!(main_class.content.contains("public final class TestLibRs"));
    assert!(main_class.content.contains("public static String extract"));
    assert!(main_class.content.contains("throws TestLibRsException"));

    // Check exception
    let exception = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Exception"))
        .unwrap();
    assert!(
        exception
            .content
            .contains("public class TestLibRsException extends Exception")
    );
    assert!(exception.content.contains("private final int code"));

    // Check enum
    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Mode"))
        .unwrap();
    assert!(enum_file.content.contains("public enum Mode"));
    assert!(enum_file.content.contains("Fast"));
    assert!(enum_file.content.contains("Accurate"));
}

#[test]
fn test_capabilities() {
    let backend = JavaBackend;
    let caps = backend.capabilities();

    assert!(caps.supports_async);
    assert!(caps.supports_classes);
    assert!(caps.supports_enums);
    assert!(caps.supports_option);
    assert!(caps.supports_result);
    assert!(!caps.supports_callbacks);
    assert!(!caps.supports_streaming);
}

#[test]
fn test_package_default() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "my_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    let config = AlefConfig {
        crate_config: CrateConfig {
            name: "my_lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
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
            prefix: None,
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
        }),
        go: None,
        java: None, // No explicit package
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
    };

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let native_lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeLib"))
        .unwrap();

    // Should use default package
    assert!(native_lib.content.contains("package dev.kreuzberg"));
}

#[test]
fn test_optional_field_defaults_in_builder() {
    let backend = JavaBackend;

    // Create test API with optional fields that have defaults
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ConfigWithDefaults".to_string(),
            rust_path: "test_lib::ConfigWithDefaults".to_string(),
            fields: vec![
                FieldDef {
                    name: "list_indent_width".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
                    optional: true,
                    default: Some("0".to_string()),
                    doc: "Optional list indent".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                },
                FieldDef {
                    name: "bullets".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    default: Some("\"*\"".to_string()),
                    doc: "Optional bullets".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                },
                FieldDef {
                    name: "escape_asterisks".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                    optional: true,
                    default: Some("true".to_string()),
                    doc: "Optional escape flag".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                },
                FieldDef {
                    name: "timeout_ms".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                    optional: true,
                    default: None,
                    doc: "Optional timeout without default".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            doc: "Config with defaults".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    // Create test config
    let config = AlefConfig {
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
            sources: vec![],
            version_from: "Cargo.toml".to_string(),
            core_import: None,
            workspace_root: None,
            skip_core_import: false,
            features: vec![],
            path_mappings: std::collections::HashMap::new(),
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
            prefix: Some("test".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
        }),
        go: None,
        java: Some(JavaConfig {
            package: Some("com.example".to_string()),
            ffi_style: "panama".to_string(),
            features: None,
            serde_rename_all: None,
        }),
        csharp: None,
        r: None,
        scaffold: None,
        readme: None,
        lint: None,
        custom_files: None,
        adapters: vec![],
        custom_modules: alef_core::config::CustomModulesConfig::default(),
        custom_registrations: alef_core::config::CustomRegistrationsConfig::default(),
        opaque_types: std::collections::HashMap::new(),
        generate: alef_core::config::GenerateConfig::default(),
        generate_overrides: std::collections::HashMap::new(),
        dto: Default::default(),
        sync: None,
        test: None,
        e2e: None,
    };

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());
    let files = result.unwrap();

    // Find the builder class
    let builder_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ConfigWithDefaultsBuilder"))
        .expect("Builder class should be generated");

    let builder_content = &builder_file.content;

    // Verify Optional fields use Optional.of() or Optional.empty()
    assert!(
        builder_content.contains("Optional<Long> listIndentWidth = Optional.of(0L)"),
        "Optional Long field with default should use Optional.of(0L), got:\n{}",
        builder_content
    );

    assert!(
        builder_content.contains("Optional<String> bullets = Optional.of(\"*\")"),
        "Optional String field with default should use Optional.of(\"*\"), got:\n{}",
        builder_content
    );

    assert!(
        builder_content.contains("Optional<Boolean> escapeAsterisks = Optional.of(true)"),
        "Optional Boolean field with default should use Optional.of(true), got:\n{}",
        builder_content
    );

    assert!(
        builder_content.contains("Optional<Long> timeoutMs = Optional.empty()"),
        "Optional field without default should use Optional.empty(), got:\n{}",
        builder_content
    );

    // Verify no raw values in Optional fields (these would be the WRONG behavior)
    assert!(
        !builder_content.contains("Optional<Long> listIndentWidth = 0;"),
        "Should not have raw value in Optional field"
    );
    assert!(
        !builder_content.contains("Optional<String> bullets = \"\";"),
        "Should not have raw value in Optional field"
    );
    assert!(
        !builder_content.contains("Optional<Boolean> escapeAsterisks = false;"),
        "Should not have raw value in Optional field"
    );
}

#[test]
fn test_tagged_union_newtype_variants_produce_valid_java() {
    // Regression test: internally tagged enums whose variants are newtypes (single unnamed
    // field, IR name "0") must not emit the numeric index as a Java field name — "0" is not
    // a valid Java identifier.  The codegen must instead emit `@JsonUnwrapped <Type> value`.
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::Message".to_string(),
            serde_tag: Some("role".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            doc: String::new(),
            cfg: None,
            variants: vec![
                EnumVariant {
                    name: "System".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("SystemMessage".to_string()))],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "User".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("UserMessage".to_string()))],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Assistant".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("AssistantMessage".to_string()))],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
        }],
        errors: vec![ErrorDef {
            name: "Error".to_string(),
            rust_path: "test_lib::Error".to_string(),
            variants: vec![],
            doc: String::new(),
        }],
    };

    let files = backend
        .generate_bindings(&api, &make_test_config("dev.example"))
        .expect("generation should succeed");

    let message_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Message.java"))
        .expect("Message.java should be generated");

    let content = &message_file.content;

    // Must be a sealed interface (internally tagged)
    assert!(
        content.contains("public sealed interface Message"),
        "should be sealed interface:\n{content}"
    );

    // @JsonUnwrapped must appear for each newtype variant; numeric "0" must not be used as a field name
    assert!(
        content.contains("@JsonUnwrapped"),
        "should use @JsonUnwrapped for newtype fields:\n{content}"
    );
    assert!(
        !content.contains("\"0\""),
        "numeric tuple index must not appear as a Java field name or @JsonProperty value:\n{content}"
    );
    assert!(
        !content.contains(" 0)"),
        "numeric field name \"0\" must not appear as Java identifier:\n{content}"
    );

    // Each variant record should declare a `value` field
    assert!(
        content.contains("SystemMessage value"),
        "System variant should have `value` field:\n{content}"
    );
    assert!(
        content.contains("UserMessage value"),
        "User variant should have `value` field:\n{content}"
    );
    assert!(
        content.contains("AssistantMessage value"),
        "Assistant variant should have `value` field:\n{content}"
    );

    // Jackson annotations for the discriminator must be present
    assert!(
        content.contains("@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, property = \"role\""),
        "should emit @JsonTypeInfo with role tag:\n{content}"
    );

    // @JsonUnwrapped import must be present
    assert!(
        content.contains("import com.fasterxml.jackson.annotation.JsonUnwrapped;"),
        "should import JsonUnwrapped:\n{content}"
    );
}
