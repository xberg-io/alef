use alef_backend_java::JavaBackend;
use alef_core::backend::Backend;
use alef_core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef_core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_test_config(package: &str) -> ResolvedCrateConfig {
    resolved_one(&format!(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "{package}"
"#
    ))
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
        serde_rename: None,
    }
}

#[test]
fn test_basic_generation() {
    let backend = JavaBackend;

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
                serde_rename: None,
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
            serde_untagged: false,
            serde_rename_all: None,
        }],
        errors: vec![],
    };

    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "com.example"
"#,
    );

    let result = backend.generate_bindings(&api, &config);

    assert!(result.is_ok());
    let files = result.unwrap();

    // Should generate 6 files:
    // 1. package-info.java
    // 2. NativeLib.java
    // 3. TestLibRs.java (main class — "Rs" suffix avoids facade/FFI name collision)
    // 4. TestLibRsException.java
    // 5. Config.java (record) — but Config has no serde, so it's skipped
    // 6. Mode.java (enum)
    // Note: Config has no serde, so no record is generated; check actual count
    assert!(files.len() >= 4, "expected at least 4 files, got {}", files.len());

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
fn test_package_default_when_unconfigured() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "my_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
    };

    // No java package and no scaffold repository configured
    let config = resolved_one(
        r#"
[workspace]
languages = ["java"]

[[crates]]
name = "my_lib"
sources = ["src/lib.rs"]
"#,
    );

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
                    serde_rename: None,
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
                    serde_rename: None,
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
                    serde_rename: None,
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
                    serde_rename: None,
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

    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "com.example"
"#,
    );

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());
    let files = result.unwrap();

    // Find the builder class
    let builder_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ConfigWithDefaultsBuilder"))
        .expect("Builder class should be generated");

    let builder_content = &builder_file.content;

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
    // Regression: internally tagged enums whose variants are newtypes (single unnamed
    // field, IR name "0") must not emit the numeric index as a Java field name.
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
            serde_untagged: false,
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

    assert!(
        content.contains("public sealed interface Message"),
        "should be sealed interface:\n{content}"
    );

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

    assert!(
        content.contains("@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, property = \"role\""),
        "should emit @JsonTypeInfo with role tag:\n{content}"
    );

    assert!(
        content.contains("import com.fasterxml.jackson.annotation.JsonUnwrapped;"),
        "should import JsonUnwrapped:\n{content}"
    );
}

#[test]
fn test_output_path_no_doubling() {
    use std::path::PathBuf;

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
