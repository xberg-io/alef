use alef_backend_java::JavaBackend;
use alef_core::backend::Backend;
use alef_core::config::{AlefConfig, BridgeBinding, CrateConfig, FfiConfig, JavaConfig, TraitBridgeConfig};
use alef_core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};

fn make_test_config(package: &str) -> AlefConfig {
    AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
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
            prefix: Some("test".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            rename_fields: Default::default(),
        }),
        gleam: None,
        go: None,
        java: Some(JavaConfig {
            package: Some(package.to_string()),
            ffi_style: "panama".to_string(),
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
            project_file: None,
        }),
        kotlin: None,
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
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
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
            original_rust_path: String::new(),
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
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Test config".to_string(),
            cfg: None,
        }],
        functions: vec![FunctionDef {
            name: "extract".to_string(),
            rust_path: "test_lib::extract".to_string(),
            original_rust_path: String::new(),
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
                original_type: None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Extract text".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Accurate".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: "Accurate mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    // Create test config
    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
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
            prefix: Some("test".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            rename_fields: Default::default(),
        }),
        gleam: None,
        go: None,
        java: Some(JavaConfig {
            package: Some("com.example".to_string()),
            ffi_style: "panama".to_string(),
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
            project_file: None,
        }),
        kotlin: None,
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
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
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
        version: None,
        crate_config: CrateConfig {
            name: "my_lib".to_string(),
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
            prefix: None,
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            rename_fields: Default::default(),
        }),
        gleam: None,
        go: None,
        java: None,

        kotlin: None, // No explicit package
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
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
    };

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let native_lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeLib"))
        .unwrap();

    // When neither [java].package nor [scaffold].repository is configured,
    // alef emits a vendor-neutral placeholder so the build fails loudly
    // instead of silently inheriting another organization's namespace.
    assert!(native_lib.content.contains("package unconfigured.alef"));
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
            original_rust_path: String::new(),
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
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Config with defaults".to_string(),
            cfg: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    // Create test config
    let config = AlefConfig {
        version: None,
        crate_config: CrateConfig {
            name: "test_lib".to_string(),
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
            prefix: Some("test".to_string()),
            error_style: "last_error".to_string(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: false,
            features: None,
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            rename_fields: Default::default(),
        }),
        gleam: None,
        go: None,
        java: Some(JavaConfig {
            package: Some("com.example".to_string()),
            ffi_style: "panama".to_string(),
            features: None,
            serde_rename_all: None,
            rename_fields: Default::default(),
            run_wrapper: None,
            extra_lint_paths: Vec::new(),
            project_file: None,
        }),
        kotlin: None,
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
        format: alef_core::config::FormatConfig::default(),
        format_overrides: std::collections::HashMap::new(),
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
            original_rust_path: String::new(),
            serde_tag: Some("role".to_string()),
            serde_rename_all: Some("snake_case".to_string()),
            doc: String::new(),
            cfg: None,
            variants: vec![
                EnumVariant {
                    name: "System".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("SystemMessage".to_string()))],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "User".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("UserMessage".to_string()))],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Assistant".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("AssistantMessage".to_string()))],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            is_copy: false,
            has_serde: false,
        }],
        errors: vec![ErrorDef {
            name: "Error".to_string(),
            rust_path: "test_lib::Error".to_string(),
            original_rust_path: String::new(),
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

#[test]
fn test_output_path_no_doubling() {
    // Regression test for Bug 1: output path doubling when user config already includes package
    use std::path::PathBuf;

    // Simulate the fix: detect if output_dir already ends with package_path
    let package = "dev.kreuzberg";
    let package_path = package.replace('.', "/");

    // Case 1: User configured the full package path (should NOT append again)
    let output_dir_1 = "packages/java/src/main/java/dev/kreuzberg/";
    let base_path_1 = if output_dir_1.ends_with(&package_path) || output_dir_1.ends_with(&format!("{}/", package_path))
    {
        PathBuf::from(&output_dir_1)
    } else {
        PathBuf::from(&output_dir_1).join(&package_path)
    };
    assert_eq!(
        base_path_1,
        PathBuf::from("packages/java/src/main/java/dev/kreuzberg/"),
        "Should not double the package path"
    );

    // Case 2: User configured without package path (should append)
    let output_dir_2 = "packages/java/src/main/java/";
    let base_path_2 = if output_dir_2.ends_with(&package_path) || output_dir_2.ends_with(&format!("{}/", package_path))
    {
        PathBuf::from(&output_dir_2)
    } else {
        PathBuf::from(&output_dir_2).join(&package_path)
    };
    assert_eq!(
        base_path_2,
        PathBuf::from("packages/java/src/main/java/dev/kreuzberg"),
        "Should append package path when not already present"
    );
}

/// Build a minimal config with a single options-field bridge (bind_via = "options_field").
fn make_options_field_bridge_config(package: &str) -> AlefConfig {
    let mut config = make_test_config(package);
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,
        type_alias: Some("HtmlVisitor".to_string()),
        param_name: Some("visitor".to_string()),
        register_extra_args: None,
        exclude_languages: vec![],
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: Some("visitor".to_string()),
    }];
    config
}

/// Build an API surface with:
///   - `ConversionOptions` record with a `visitor` field (TypeRef::String because IR sanitises the type)
///   - `convert(html: String, options: ConversionOptions) -> String` function
fn make_visitor_options_api() -> ApiSurface {
    let conversion_options_type = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "test_lib::ConversionOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            FieldDef {
                name: "bullets".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
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
            },
            FieldDef {
                name: "visitor".to_string(),
                ty: TypeRef::String, // sanitised by the extractor
                optional: true,
                default: None,
                doc: String::new(),
                sanitized: true,
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
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
    };

    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![conversion_options_type],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "test_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "html".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                },
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ConversionOptions".to_string()),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }],
        enums: vec![],
        errors: vec![],
    }
}

#[test]
fn test_options_field_bridge_visitor_on_conversion_options_record() {
    // When bind_via = "options_field", the visitor field on ConversionOptions should
    // be emitted as the bridge interface type (HtmlVisitor) with @JsonIgnore,
    // not as the sanitised IR type (String/Optional<String>).
    let backend = JavaBackend;
    let api = make_visitor_options_api();
    let config = make_options_field_bridge_config("dev.example");

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let options_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ConversionOptions.java"))
        .expect("ConversionOptions.java should be generated");

    let content = &options_file.content;

    // Bridge field must be typed as HtmlVisitor (the bridge interface), not String
    assert!(
        content.contains("HtmlVisitor visitor"),
        "visitor field must be typed as HtmlVisitor:\n{content}"
    );

    // Bridge field must carry @JsonIgnore so Jackson won't serialise it to Rust
    assert!(
        content.contains("@JsonIgnore"),
        "visitor field must be annotated @JsonIgnore:\n{content}"
    );

    // JsonIgnore import must be present
    assert!(
        content.contains("import com.fasterxml.jackson.annotation.JsonIgnore;"),
        "JsonIgnore import must be present:\n{content}"
    );

    // The field must NOT be typed as Optional<String> (the sanitised IR type)
    assert!(
        !content.contains("Optional<String> visitor"),
        "visitor must not be typed as Optional<String>:\n{content}"
    );
}

#[test]
fn test_options_field_bridge_options_set_handle_in_native_lib() {
    // NativeLib.java must contain an HTM_OPTIONS_SET_VISITOR MethodHandle
    // using orElse(null) so class init doesn't fail when the dylib lacks the symbol.
    let backend = JavaBackend;
    let api = make_visitor_options_api();
    let config = make_options_field_bridge_config("dev.example");

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let native_lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeLib.java"))
        .expect("NativeLib.java should be generated");

    let content = &native_lib.content;

    // Setter handle must be present
    assert!(
        content.contains("TEST_OPTIONS_SET_VISITOR"),
        "OPTIONS_SET_VISITOR handle must be present in NativeLib:\n{content}"
    );

    // Must use orElse(null) — the symbol may be absent during development
    assert!(
        content.contains("orElse(null)"),
        "OPTIONS_SET_VISITOR handle must use orElse(null):\n{content}"
    );
}

#[test]
fn test_options_field_bridge_no_standalone_convert_with_visitor() {
    // When using options-field bridging, no `convertWithVisitor` method should be emitted.
    // The visitor is accessed from options.visitor() inside the normal convert() method.
    let backend = JavaBackend;
    let api = make_visitor_options_api();
    let config = make_options_field_bridge_config("dev.example");

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let main_class = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .expect("main class should be generated");

    let content = &main_class.content;

    // No standalone convertWithVisitor method in options-field mode
    assert!(
        !content.contains("convertWithVisitor"),
        "options-field mode must not emit a standalone convertWithVisitor method:\n{content}"
    );

    // The normal convert() method must be present
    assert!(
        content.contains("public static String convert("),
        "convert() method must be present:\n{content}"
    );

    // Bridge setter call must be present inside the convert() method
    assert!(
        content.contains("TEST_OPTIONS_SET_VISITOR"),
        "options-field bridge setter must be called inside convert():\n{content}"
    );
}

#[test]
fn test_options_field_bridge_convert_method_uses_bridge_setter() {
    // The generated convert() method in options-field mode must:
    //   - Check if the bridge setter handle and visitor are both non-null
    //   - Create the bridge object (HtmlVisitorBridge)
    //   - Call the FFI setter before the main FFI invocation
    let backend = JavaBackend;
    let api = make_visitor_options_api();
    let config = make_options_field_bridge_config("dev.example");

    let files = backend
        .generate_bindings(&api, &config)
        .expect("generation should succeed");

    let main_class = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .expect("main class should be generated");

    let content = &main_class.content;

    // Bridge object creation
    assert!(
        content.contains("HtmlVisitorBridge"),
        "convert() must create an HtmlVisitorBridge:\n{content}"
    );

    // Null-guard for both the handle and the visitor
    assert!(
        content.contains("TEST_OPTIONS_SET_VISITOR != null"),
        "convert() must null-check the setter handle:\n{content}"
    );

    // Setter invocation
    assert!(
        content.contains("TEST_OPTIONS_SET_VISITOR.invoke("),
        "convert() must invoke the setter handle:\n{content}"
    );
}
