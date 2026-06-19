use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, TypeDef, TypeRef,
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

fn make_test_config_with_builder_always(package: &str) -> ResolvedCrateConfig {
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

[crates.java.dto]
builder = "always"
"#
    ))
}

fn make_test_config_with_trait_bridge(package: &str) -> ResolvedCrateConfig {
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

[[crates.trait_bridges]]
trait_name = "Renderer"
register_fn = "register_renderer"
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
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

#[test]
fn trait_bridge_string_return_is_not_json_quoted() {
    let renderer = TypeDef {
        name: "Renderer".to_string(),
        rust_path: "test_lib::Renderer".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "render".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: Some("TestError".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
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
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![renderer],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = JavaBackend
        .generate_bindings(&api, &make_test_config_with_trait_bridge("com.example"))
        .unwrap();
    let bridge = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("RendererBridge.java"))
        .expect("RendererBridge.java")
        .content
        .as_str();

    assert!(
        bridge.contains("MemorySegment jsonCs = arena.allocateFrom(result);"),
        "Java trait callback string returns must pass raw UTF-8 through: {bridge}"
    );
    assert!(
        !bridge.contains("String json = JSON.writeValueAsString(result);"),
        "Java trait callback string returns must not be JSON-quoted: {bridge}"
    );
}

#[test]
fn trait_bridge_register_downcall_passes_vtable_address() {
    let renderer = TypeDef {
        name: "Renderer".to_string(),
        rust_path: "test_lib::Renderer".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "render".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: Some("TestError".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
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
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![renderer],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = JavaBackend
        .generate_bindings(&api, &make_test_config_with_trait_bridge("com.example"))
        .unwrap();
    let native_lib = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("NativeLib.java"))
        .expect("NativeLib.java")
        .content
        .as_str();
    let bridge = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("RendererBridge.java"))
        .expect("RendererBridge.java")
        .content
        .as_str();

    assert!(
        !native_lib.contains("import java.lang.foreign.MemoryLayout;"),
        "NativeLib must not import MemoryLayout for pointer-based vtable registration, got:\n{native_lib}"
    );
    assert!(
        native_lib.contains("FunctionDescriptor.of(ValueLayout.JAVA_INT,\n            ValueLayout.ADDRESS"),
        "register downcall must pass the vtable as an address, got:\n{native_lib}"
    );
    assert!(
        bridge.contains(
            "NativeLib.TEST_REGISTER_RENDERER.invoke(nameCs, bridge.vtableSegment(), MemorySegment.NULL, outErr)"
        ),
        "register helper should pass the vtable MemorySegment value, got:\n{bridge}"
    );
}

#[test]
fn bool_function_uses_i32_ffi_layout_and_boolean_wrapper_result() {
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "is_ready".to_string(),
            rust_path: "test_lib::is_ready".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "enabled".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
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
            }],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = JavaBackend
        .generate_bindings(&api, &make_test_config("com.example"))
        .unwrap();
    let native_lib = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("NativeLib.java"))
        .expect("NativeLib.java")
        .content
        .as_str();
    let main_class = files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("TestLibRs.java"))
        .expect("TestLibRs.java")
        .content
        .as_str();

    assert!(
        native_lib.contains("FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.JAVA_INT)"),
        "bool FFI params and returns must use i32 layouts, got:\n{native_lib}"
    );
    assert!(!native_lib.contains("ValueLayout.JAVA_BOOLEAN"));
    assert!(
        main_class.contains("var primitiveResult = (int) NativeLib.TEST_IS_READY.invoke((enabled ? 1 : 0));"),
        "wrapper must receive the raw i32 bool result, got:\n{main_class}"
    );
    assert!(
        main_class.contains("return primitiveResult != 0;"),
        "safe wrapper must convert i32 to boolean, got:\n{main_class}"
    );
}

#[test]
fn string_return_uses_len_companion_and_bounded_decode() {
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "describe".to_string(),
            rust_path: "test_lib::describe".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "topic".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("TestError".to_string()),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = JavaBackend
        .generate_bindings(&api, &make_test_config("com.example"))
        .unwrap();
    let native_lib = files
        .iter()
        .find(|f| f.path.ends_with("NativeLib.java"))
        .expect("NativeLib.java")
        .content
        .as_str();
    let main = files
        .iter()
        .find(|f| f.content.contains("long resultLen = (long)"))
        .expect("Java FFI facade with bounded string decode")
        .content
        .as_str();

    assert!(
        native_lib.contains("TEST_DESCRIBE_LEN"),
        "NativeLib must bind the _len companion: {native_lib}"
    );
    assert!(
        main.contains("long resultLen = (long) NativeLib.TEST_DESCRIBE_LEN.invoke(ctopic);"),
        "Java wrapper must call _len with the same args: {main}"
    );
    assert!(
        main.contains("String str = readCString(resultPtr, resultLen);"),
        "Java wrapper must decode through bounded helper: {main}"
    );
    assert!(
        main.contains("return ptr.reinterpret(byteLen + 1).getString(0);"),
        "readCString must bound the segment before decoding: {main}"
    );
    assert!(
        !main.contains("resultPtr.reinterpret(Long.MAX_VALUE).getString(0)"),
        "string return path must not decode with Long.MAX_VALUE: {main}"
    );
}

#[test]
fn named_param_from_json_is_checked_before_primary_call() {
    let config_type = TypeDef {
        name: "Config".to_string(),
        rust_path: "test_lib::Config".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }],
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
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![config_type],
        functions: vec![FunctionDef {
            name: "configure".to_string(),
            rust_path: "test_lib::configure".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "config".to_string(),
                ty: TypeRef::Named("Config".to_string()),
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: Some("TestError".to_string()),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = JavaBackend
        .generate_bindings(&api, &make_test_config("com.example"))
        .unwrap();
    let main = files
        .iter()
        .find(|f| f.content.contains("NativeLib.TEST_CONFIGURE.invoke"))
        .expect("Java FFI facade with configure")
        .content
        .as_str();
    let check_pos = main
        .find("if (cconfigJson != null && cconfig.equals(MemorySegment.NULL))")
        .unwrap();
    let call_pos = main.find("NativeLib.TEST_CONFIGURE.invoke(cconfig)").unwrap();

    assert!(
        check_pos < call_pos,
        "_from_json failure must be checked before primary call: {main}"
    );
    assert!(
        main[check_pos..call_pos].contains("checkLastError();"),
        "_from_json null check must preserve and throw the real last_error: {main}"
    );
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
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
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![EnumDef {
            name: "Mode".to_string(),
            rust_path: "test_lib::Mode".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Fast".to_string(),
                    fields: vec![],
                    doc: "Fast mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Accurate".to_string(),
                    fields: vec![],
                    doc: "Accurate mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
fn test_ffi_excluded_types_are_not_generated_for_panama() {
    let backend = JavaBackend;
    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"
exclude_types = ["HiddenHandle"]

[crates.java]
package = "dev.example"
"#,
    );
    let hidden_type = TypeDef {
        name: "HiddenHandle".to_string(),
        rust_path: "test_lib::HiddenHandle".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Hidden FFI handle.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let visible_type = TypeDef {
        name: "VisibleHandle".to_string(),
        rust_path: "test_lib::VisibleHandle".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "hidden".to_string(),
            params: vec![],
            return_type: TypeRef::Named("HiddenHandle".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Returns the hidden handle.".to_string(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            trait_source: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: "Visible FFI handle.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![hidden_type, visible_type],
        functions: vec![FunctionDef {
            name: "hidden_handle".to_string(),
            rust_path: "test_lib::hidden_handle".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("HiddenHandle".to_string()),
            is_async: false,
            error_type: None,
            doc: "Returns the hidden handle.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(!files.iter().any(|file| file.path.ends_with("HiddenHandle.java")));
    assert!(files.iter().any(|file| file.path.ends_with("VisibleHandle.java")));
    for file in &files {
        assert!(!file.content.contains("TEST_HIDDEN_HANDLE"));
        assert!(!file.content.contains("TEST_VISIBLE_HANDLE_HIDDEN"));
    }
}

#[test]
fn test_duplicate_error_variant_exception_classes_are_emitted_once() {
    let backend = JavaBackend;
    let config = make_test_config("dev.example");
    let duplicate_variant = ErrorVariant {
        name: "DepthLimitExceeded".to_string(),
        message_template: Some("depth limit exceeded".to_string()),
        fields: vec![],
        has_source: false,
        has_from: false,
        is_unit: true,
        is_tuple: false,
        doc: "Depth limit exceeded.".to_string(),
    };
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![
            ErrorDef {
                name: "GraphQLError".to_string(),
                rust_path: "test_lib::GraphQLError".to_string(),
                original_rust_path: String::new(),
                variants: vec![duplicate_variant.clone()],
                doc: "GraphQL errors.".to_string(),
                methods: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            ErrorDef {
                name: "SchemaError".to_string(),
                rust_path: "test_lib::SchemaError".to_string(),
                original_rust_path: String::new(),
                variants: vec![duplicate_variant],
                doc: "Schema errors.".to_string(),
                methods: vec![],
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let duplicate_count = files
        .iter()
        .filter(|file| file.path.ends_with("DepthLimitExceededException.java"))
        .count();

    assert_eq!(duplicate_count, 1);
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
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "field5".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    optional: false,
                    default: Some("false".to_string()),
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "field6".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::I32),
                    optional: false,
                    default: Some("0".to_string()),
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "field7".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: Some("\"\"".to_string()),
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "field8".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::F64),
                    optional: false,
                    default: Some("0.0".to_string()),
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

    // The builder is now a nested static class inside the record file —
    // no separate *Builder.java file should exist.
    assert!(
        !files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("ConfigWithDefaultsBuilder")),
        "No standalone *Builder.java file should be generated; builder is nested inside the record"
    );

    // The record file itself must contain the nested Builder class.
    let record_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("ConfigWithDefaults.java"))
        .expect("Record file ConfigWithDefaults.java should be generated");

    let content = &record_file.content;

    // Verify the nested builder class header
    assert!(
        content.contains("public static final class Builder"),
        "Record should contain nested 'public static final class Builder', got:\n{}",
        content
    );

    // @JsonDeserialize must reference the nested class, not a sibling top-level class
    assert!(
        content.contains("@JsonDeserialize(builder = ConfigWithDefaults.Builder.class)"),
        "@JsonDeserialize should reference ConfigWithDefaults.Builder.class, got:\n{}",
        content
    );

    // builder() factory must return Builder (not ConfigWithDefaultsBuilder)
    assert!(
        content.contains("public static Builder builder()"),
        "factory method should return Builder, got:\n{}",
        content
    );

    assert!(
        content.contains("Optional<Long> listIndentWidth = Optional.of(0L)"),
        "Optional Long field with default should use Optional.of(0L), got:\n{}",
        content
    );

    assert!(
        content.contains("Optional<String> bullets = Optional.of(\"*\")"),
        "Optional String field with default should use Optional.of(\"*\"), got:\n{}",
        content
    );

    assert!(
        content.contains("Optional<Boolean> escapeAsterisks = Optional.of(true)"),
        "Optional Boolean field with default should use Optional.of(true), got:\n{}",
        content
    );

    assert!(
        content.contains("Optional<Long> timeoutMs = Optional.empty()"),
        "Optional field without default should use Optional.empty(), got:\n{}",
        content
    );

    assert!(
        !content.contains("Optional<Long> listIndentWidth = 0;"),
        "Should not have raw value in Optional field"
    );
    assert!(
        !content.contains("Optional<String> bullets = \"\";"),
        "Should not have raw value in Optional field"
    );
    assert!(
        !content.contains("Optional<Boolean> escapeAsterisks = false;"),
        "Should not have raw value in Optional field"
    );
}

/// Regression: builder is inlined as a nested class — no `*Builder.java` file should be emitted.
#[test]
fn test_no_standalone_builder_java_file_emitted() {
    let backend = JavaBackend;

    // Create a type with 8 fields to trigger auto builder emission (>= BUILDER_AUTO_THRESHOLD)
    let mut fields = vec![];
    for i in 1..=8 {
        fields.push(FieldDef {
            name: format!("field{}", i),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            optional: false,
            default: Some("true".to_string()),
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: Some(alef::core::ir::DefaultValue::BoolLiteral(true)),
            core_wrapper: alef::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        });
    }

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "MyOptions".to_string(),
            rust_path: "test_lib::MyOptions".to_string(),
            original_rust_path: String::new(),
            fields,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("com.example");
    let files = backend.generate_bindings(&api, &config).unwrap();

    // No standalone MyOptionsBuilder.java must exist
    assert!(
        !files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("MyOptionsBuilder")),
        "No standalone MyOptionsBuilder.java should be emitted; builder is nested inside MyOptions.java"
    );

    // The record file must exist and contain the nested Builder
    let record = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("MyOptions.java"))
        .expect("MyOptions.java must be generated");

    assert!(
        record.content.contains("public static final class Builder"),
        "MyOptions.java must contain nested Builder class"
    );
    assert!(
        record
            .content
            .contains("@JsonDeserialize(builder = MyOptions.Builder.class)"),
        "@JsonDeserialize must reference MyOptions.Builder.class"
    );
    assert!(
        record.content.contains("public static Builder builder()"),
        "factory must return Builder (not MyOptionsBuilder)"
    );
    assert!(
        record.content.contains("@JsonPOJOBuilder(withPrefix = \"with\""),
        "nested Builder must carry @JsonPOJOBuilder"
    );
}

/// Regression: a non-optional `#[serde(default)]` boolean field that defaults to `true`
/// must restore that default in the record's compact constructor. Boxed `@Nullable Boolean`
/// fields arrive as `null` when JSON omits them, so without the null-check the accessor would
/// return `null` instead of `true` (mirrors the boxed-numeric default handling and Kotlin's
/// `= true`). Primitive bool fields stay skipped — covered separately.
#[test]
fn test_serde_default_boxed_boolean_true_restored_in_compact_ctor() {
    let backend = JavaBackend;

    let fields = vec![FieldDef {
        name: "deny_private".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        default: Some("/* serde(default) */".to_string()),
        doc: String::new(),
        sanitized: false,
        is_boxed: true,
        type_rust_path: None,
        cfg: None,
        typed_default: Some(alef::core::ir::DefaultValue::BoolLiteral(true)),
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }];

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SsrfPolicy".to_string(),
            rust_path: "test_lib::SsrfPolicy".to_string(),
            original_rust_path: String::new(),
            fields,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("com.example");
    let files = backend.generate_bindings(&api, &config).unwrap();
    let record = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("SsrfPolicy.java"))
        .expect("SsrfPolicy.java must be generated");

    assert!(
        record
            .content
            .contains("if (denyPrivate == null) { denyPrivate = true; }"),
        "compact constructor must restore the serde-default `true` for boxed Boolean fields, got:\n{}",
        record.content
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
            methods: vec![],
            doc: String::new(),
            cfg: None,
            variants: vec![
                EnumVariant {
                    name: "System".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("SystemMessage".to_string()))],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "User".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("UserMessage".to_string()))],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Assistant".to_string(),
                    fields: vec![make_newtype_field(TypeRef::Named("AssistantMessage".to_string()))],
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            is_copy: false,
            has_serde: false,
            has_default: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![ErrorDef {
            name: "Error".to_string(),
            rust_path: "test_lib::Error".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

    // Newtype-variant tagged unions now use a custom Jackson deserializer
    // (StdDeserializer) instead of @JsonUnwrapped because the latter does not
    // round-trip cleanly with sealed interfaces. Verify the deserializer is
    // wired up via @JsonDeserialize and that its body reads/strips the tag.
    assert!(
        content.contains("@JsonDeserialize(using = MessageDeserializer.class)"),
        "should wire MessageDeserializer via @JsonDeserialize:\n{content}"
    );
    assert!(
        content.contains("class MessageDeserializer extends StdDeserializer<Message>"),
        "should emit a custom StdDeserializer for the sealed interface:\n{content}"
    );
    assert!(
        content.contains("node.get(\"role\")"),
        "deserializer should read the `role` discriminator:\n{content}"
    );
    assert!(
        content.contains("node.remove(\"role\")"),
        "deserializer should strip the tag before delegating to the variant type:\n{content}"
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
}

#[test]
fn test_output_path_no_doubling() {
    use std::path::PathBuf;

    let package = "dev.sample_crate";
    let package_path = package.replace('.', "/");

    // Case 1: User configured the full package path (should NOT append again)
    let output_dir_1 = "packages/java/src/main/java/dev/sample_crate/";
    let base_path_1 = if output_dir_1.ends_with(&package_path) || output_dir_1.ends_with(&format!("{}/", package_path))
    {
        PathBuf::from(&output_dir_1)
    } else {
        PathBuf::from(&output_dir_1).join(&package_path)
    };
    assert_eq!(
        base_path_1,
        PathBuf::from("packages/java/src/main/java/dev/sample_crate/"),
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
        PathBuf::from("packages/java/src/main/java/dev/sample_crate"),
        "Should append package path when not already present"
    );
}

/// Streaming-adapter emission: when a `[[crates.adapters]]` entry has
/// pattern = "streaming" and owner_type = an opaque handle, the Java backend
/// must emit (a) the three FFI iterator-handle MethodHandles in NativeLib and
/// (b) a public `chatStream(req)` instance method on the opaque handle that
/// returns `Stream<ChatCompletionChunk>` driven by those handles.
#[test]
fn test_streaming_adapter_emits_stream_method_on_opaque_handle() {
    use alef::core::ir::{MethodDef, ReceiverKind};

    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "tl"

[crates.java]
package = "com.example.test"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatCompletionChunk"
error_type = "TestError"
request_type = "test_lib::ChatCompletionRequest"

[[crates.adapters.params]]
name = "req"
type = "ChatCompletionRequest"
"#,
    );

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "DefaultClient".to_string(),
                rust_path: "test_lib::DefaultClient".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "chat_stream".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: true,
                    is_static: false,
                    error_type: Some("TestError".to_string()),
                    doc: String::new(),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    receiver: Some(ReceiverKind::Ref),
                    trait_source: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ChatCompletionRequest".to_string(),
                rust_path: "test_lib::ChatCompletionRequest".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "ChatCompletionChunk".to_string(),
                rust_path: "test_lib::ChatCompletionChunk".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = JavaBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    // 1. NativeLib must include the three iterator-handle MethodHandles.
    let native_lib = files
        .iter()
        .find(|f| f.path.ends_with("NativeLib.java"))
        .expect("NativeLib.java must be generated");
    for needle in [
        "TL_DEFAULT_CLIENT_CHAT_STREAM_START",
        "tl_default_client_chat_stream_start",
        "TL_DEFAULT_CLIENT_CHAT_STREAM_NEXT",
        "tl_default_client_chat_stream_next",
        "TL_DEFAULT_CLIENT_CHAT_STREAM_FREE",
        "tl_default_client_chat_stream_free",
        "TL_CHAT_COMPLETION_REQUEST_FROM_JSON",
        "TL_CHAT_COMPLETION_CHUNK_TO_JSON",
    ] {
        assert!(
            native_lib.content.contains(needle),
            "NativeLib must contain `{needle}`. Got:\n{}",
            &native_lib.content[..native_lib.content.len().min(2000)]
        );
    }

    // 2. DefaultClient.java must expose a public `chatStream(...)` returning Stream<ChatCompletionChunk>.
    let client = files
        .iter()
        .find(|f| f.path.ends_with("DefaultClient.java"))
        .expect("DefaultClient.java must be generated");
    assert!(
        client
            .content
            .contains("public java.util.stream.Stream<ChatCompletionChunk> chatStream(final ChatCompletionRequest"),
        "DefaultClient must emit `chatStream` returning Stream<ChatCompletionChunk>. Got:\n{}",
        client.content
    );
    assert!(
        !client.content.contains("public Iterator<ChatCompletionChunk>"),
        "DefaultClient must NOT use bare Iterator<> return type for streaming methods"
    );
    assert!(
        !client.content.contains("import java.util.stream.Stream;"),
        "DefaultClient must NOT import java.util.stream.Stream (template uses FQN; bare import triggers Checkstyle UnusedImports). Got:\n{}",
        client.content
    );
    assert!(
        client.content.contains("java.util.stream.StreamSupport.stream("),
        "DefaultClient must bridge via fully-qualified java.util.stream.StreamSupport.stream(...). Got:\n{}",
        client.content
    );
    // Iteration body must call all three FFI handles.
    for needle in [
        "TL_DEFAULT_CLIENT_CHAT_STREAM_START.invoke",
        "TL_DEFAULT_CLIENT_CHAT_STREAM_NEXT.invoke",
        "TL_DEFAULT_CLIENT_CHAT_STREAM_FREE.invoke",
        "TL_CHAT_COMPLETION_CHUNK_TO_JSON.invoke",
        "TL_CHAT_COMPLETION_CHUNK_FREE.invoke",
    ] {
        assert!(
            client.content.contains(needle),
            "DefaultClient body must invoke `{needle}`"
        );
    }
}

#[test]
fn test_bytes_parameter_expansion_in_ffi_descriptor_and_invoke() {
    // Regression test for SIGBUS bug: Bytes parameters must expand to (pointer, length)
    // in both the FunctionDescriptor AND the invoke() call arguments.
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
            original_rust_path: String::new(),
            // Rust signature: fn(*const u8, usize, *const c_char) -> i32
            // This mimics sample_crate_extract_bytes signature
            params: vec![
                ParamDef {
                    name: "content".to_string(),
                    ty: TypeRef::Bytes,
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
                },
                ParamDef {
                    name: "file_type".to_string(),
                    ty: TypeRef::String,
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
                },
            ],
            return_type: TypeRef::Primitive(PrimitiveType::I32),
            is_async: false,
            error_type: Some("Error".to_string()),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
package = "com.test"
"#,
    );

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());
    let files = result.unwrap();

    // Check NativeLib.java for descriptor
    let native_lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeLib"))
        .unwrap();

    // Descriptor must have 4 params: ADDRESS (content ptr), JAVA_LONG (content len), ADDRESS (file_type ptr), no return
    // Since return is i32 (primitive), descriptor should be:
    // FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
    assert!(
        native_lib.content.contains("FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)"),
        "FunctionDescriptor must have 4 params: int return, ptr, len, string ptr. Got:\n{}",
        native_lib.content
    );

    // Check main class for invoke call
    let main_class = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .unwrap();

    // The invoke call must pass ALL 3 arguments (ptr, len, string), not just 2
    // Expected pattern: TEST_PROCESS.invoke(ccontent, ccontentLen, cfileType)
    assert!(
        main_class.content.contains(".invoke(ccontent, ccontentLen, cfileType"),
        "invoke() call must pass (ptr, len, string) for bytes parameter. Got:\n{}",
        main_class.content
    );
}

#[test]
fn test_dto_emits_as_record_with_fields_only() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SimpleDto".to_string(),
            rust_path: "test_lib::SimpleDto".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "name".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: "Name field".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "count".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::I32),
                    optional: false,
                    default: None,
                    doc: "Count field".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
            ],
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
            doc: "A simple DTO".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("com.test");
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());
    let files = result.unwrap();

    let dto_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("SimpleDto.java"))
        .expect("SimpleDto.java should be generated");

    // Verify it's emitted as a record, not a sealed class
    assert!(
        dto_file.content.contains("public record SimpleDto("),
        "Fields-only DTO should be emitted as record, not sealed class. Got:\n{}",
        dto_file.content
    );

    // Verify record parameters are present
    assert!(
        dto_file.content.contains("String name") && dto_file.content.contains("int count"),
        "Record should contain field parameters. Got:\n{}",
        dto_file.content
    );
}

#[test]
fn test_opaque_handle_type_remains_class() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "OpaqueHandle".to_string(),
            rust_path: "test_lib::OpaqueHandle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "An opaque FFI handle".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("com.test");
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());
    let files = result.unwrap();

    let handle_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("OpaqueHandle.java"))
        .expect("OpaqueHandle.java should be generated");

    // Opaque handles should emit as classes (not records), with AutoCloseable for resource management
    assert!(
        handle_file.content.contains("public class OpaqueHandle")
            && handle_file.content.contains("implements AutoCloseable"),
        "Opaque handle type should be emitted as class implementing AutoCloseable. Got:\n{}",
        handle_file.content
    );
}

#[test]
fn test_sum_type_sealed_interface_with_record_variants() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "AuthConfig".to_string(),
            rust_path: "test_lib::AuthConfig".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Basic".to_string(),
                    fields: vec![
                        FieldDef {
                            name: "username".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef::core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
                            original_type: None,
                        },
                        FieldDef {
                            name: "password".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            doc: String::new(),
                            sanitized: false,
                            is_boxed: false,
                            type_rust_path: None,
                            cfg: None,
                            typed_default: None,
                            core_wrapper: alef::core::ir::CoreWrapper::None,
                            vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                            newtype_wrapper: None,
                            serde_rename: None,
                            serde_flatten: false,
                            binding_excluded: false,
                            binding_exclusion_reason: None,
                            original_type: None,
                        },
                    ],
                    doc: "Basic auth".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Bearer".to_string(),
                    fields: vec![FieldDef {
                        name: "token".to_string(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        doc: String::new(),
                        sanitized: false,
                        is_boxed: false,
                        type_rust_path: None,
                        cfg: None,
                        typed_default: None,
                        core_wrapper: alef::core::ir::CoreWrapper::None,
                        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                        newtype_wrapper: None,
                        serde_rename: None,
                        serde_flatten: false,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        original_type: None,
                    }],
                    doc: "Bearer token auth".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Authentication configuration".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: Some("type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("com.test");
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());
    let files = result.unwrap();

    let enum_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("AuthConfig.java"))
        .expect("AuthConfig.java should be generated");

    // Sum types should emit as sealed interface
    assert!(
        enum_file.content.contains("public sealed interface AuthConfig"),
        "Sum type should emit as sealed interface. Got:\n{}",
        enum_file.content
    );

    // Variant records should use record syntax
    assert!(
        enum_file.content.contains("record Basic(") || enum_file.content.contains("record Bearer("),
        "Sealed interface variants should be emitted as records. Got:\n{}",
        enum_file.content
    );
}

/// Regression: streaming method signature must use `Stream<T>`, never bare `Iterator<T>`.
#[test]
fn test_streaming_method_returns_stream_not_iterator() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "stream_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sl"

[crates.java]
package = "com.example.stream"

[[crates.adapters]]
name = "events"
pattern = "streaming"
core_path = "events"
owner_type = "EventSource"
item_type = "Event"
error_type = "StreamError"
request_type = "stream_lib::EventRequest"

[[crates.adapters.params]]
name = "req"
type = "EventRequest"
"#,
    );

    let api = ApiSurface {
        crate_name: "stream_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "EventSource".to_string(),
                rust_path: "stream_lib::EventSource".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "EventRequest".to_string(),
                rust_path: "stream_lib::EventRequest".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "Event".to_string(),
                rust_path: "stream_lib::Event".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = JavaBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    let source = files
        .iter()
        .find(|f| f.path.ends_with("EventSource.java"))
        .expect("EventSource.java must be generated");

    // (a) Stream< appears in streaming method signature (as FQN); no bare Iterator< return type
    assert!(
        source.content.contains("java.util.stream.Stream<"),
        "streaming method must return java.util.stream.Stream<T> (FQN). Got:\n{}",
        source.content
    );
    assert!(
        !source.content.contains("public Iterator<"),
        "streaming method must NOT return bare Iterator<T>. Got:\n{}",
        source.content
    );

    // StreamSupport bridge is present via FQN
    assert!(
        source.content.contains("java.util.stream.StreamSupport.stream("),
        "streaming bridge must use java.util.stream.StreamSupport.stream(). Got:\n{}",
        source.content
    );

    // Stream must NOT be imported — template uses fully-qualified names throughout,
    // so a bare import would trigger Checkstyle's UnusedImports rule.
    assert!(
        !source.content.contains("import java.util.stream.Stream;"),
        "must NOT import java.util.stream.Stream (template uses FQN). Got:\n{}",
        source.content
    );
}

/// Regression: tagged enum with data variants emits `sealed interface` with each
/// variant as a `record` implementing the sealed interface.
#[test]
fn test_tagged_enum_emits_sealed_interface_with_record_variants() {
    let backend = JavaBackend;

    let make_field = |name: &str, ty: TypeRef| FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Shape".to_string(),
            rust_path: "test_lib::Shape".to_string(),
            original_rust_path: String::new(),
            serde_tag: Some("kind".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            methods: vec![],
            doc: "A geometric shape".to_string(),
            cfg: None,
            variants: vec![
                EnumVariant {
                    name: "Circle".to_string(),
                    fields: vec![make_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
                    doc: "A circle".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Rectangle".to_string(),
                    fields: vec![
                        make_field("width", TypeRef::Primitive(PrimitiveType::F64)),
                        make_field("height", TypeRef::Primitive(PrimitiveType::F64)),
                    ],
                    doc: "A rectangle".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            is_copy: false,
            has_serde: false,
            has_default: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend
        .generate_bindings(&api, &make_test_config("com.example"))
        .expect("generation should succeed");

    let shape_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Shape.java"))
        .expect("Shape.java should be generated");

    let content = &shape_file.content;

    // (b) tagged enum emits `sealed interface`; variants listed in file
    assert!(
        content.contains("public sealed interface Shape"),
        "tagged enum must emit as sealed interface. Got:\n{content}"
    );
    assert!(
        content.contains("Circle") && content.contains("Rectangle"),
        "sealed interface file must contain all variant names. Got:\n{content}"
    );

    // (c) each variant is a `record` implementing the sealed interface
    assert!(
        content.contains("record Circle("),
        "Circle variant must be emitted as a record. Got:\n{content}"
    );
    assert!(
        content.contains("record Rectangle("),
        "Rectangle variant must be emitted as a record. Got:\n{content}"
    );
    assert!(
        content.contains("implements Shape"),
        "variant records must implement the sealed interface. Got:\n{content}"
    );
}

/// Regression: plain product DTOs (no tagged enum) emit as `public record`, not sealed class.
#[test]
fn test_plain_dto_emits_as_record_not_sealed_class() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ModelInfo".to_string(),
            rust_path: "test_lib::ModelInfo".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "id".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: "Model identifier".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "context_length".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::I64),
                    optional: false,
                    default: None,
                    doc: "Max context length".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
            ],
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
            doc: "LLM model metadata".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend
        .generate_bindings(&api, &make_test_config("com.example"))
        .expect("generation should succeed");

    let dto_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("ModelInfo.java"))
        .expect("ModelInfo.java should be generated");

    let content = &dto_file.content;

    // (c) plain product DTOs are records, not sealed classes
    assert!(
        content.contains("public record ModelInfo("),
        "plain product DTO must be a record. Got:\n{content}"
    );
    assert!(
        !content.contains("sealed interface"),
        "plain DTO must not emit as sealed interface. Got:\n{content}"
    );
}

#[test]
fn test_option_params_and_returns_emit_nullable_annotations() {
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "User".to_string(),
                rust_path: "test_lib::User".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    name: "id".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U64),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "Client".to_string(),
                rust_path: "test_lib::Client".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "display_name".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    is_static: false,
                    params: vec![],
                    return_type: TypeRef::Optional(Box::new(TypeRef::String)),
                    is_async: false,
                    doc: String::new(),
                    error_type: Some("Error".to_string()),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    has_default_impl: false,
                    version: Default::default(),
                }],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![
            FunctionDef {
                name: "extract_file".to_string(),
                rust_path: "test_lib::extract_file".to_string(),
                original_rust_path: String::new(),
                params: vec![
                    ParamDef {
                        name: "path".to_string(),
                        ty: TypeRef::Path,
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
                    },
                    ParamDef {
                        name: "mime_type".to_string(),
                        ty: TypeRef::String,
                        optional: true,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            FunctionDef {
                name: "find_user".to_string(),
                rust_path: "test_lib::find_user".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "id".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U64),
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
                }],
                return_type: TypeRef::Optional(Box::new(TypeRef::Named("User".to_string()))),
                is_async: false,
                error_type: Some("Error".to_string()),
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
            },
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("dev.test");
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generation failed: {:?}", result.err());

    let files = result.unwrap();

    let facade = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .expect("TestLibRs.java facade should be generated");

    let content = &facade.content;

    // (1) extract_file has optional mime_type parameter — should be @Nullable String
    assert!(
        content.contains("@Nullable String mimeType"),
        "Optional String parameter should be @Nullable. Got:\n{}",
        content
    );

    // (2) extract_file has required path parameter — should NOT be @Nullable Path
    assert!(
        content.contains("final java.nio.file.Path path"),
        "Non-optional Path parameter should not be annotated. Got:\n{}",
        content
    );
    assert!(
        !content.contains("@Nullable java.nio.file.Path path"),
        "Non-optional Path should not have @Nullable. Got:\n{}",
        content
    );

    // (3) find_user returns Option<User> — returns are represented as Optional<T>.
    assert!(
        content.contains("public static Optional<User> findUser(final long id)"),
        "Optional return type should be Optional<T>. Got:\n{}",
        content
    );

    let client = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Client.java"))
        .expect("Client.java should be generated");
    assert!(
        client.content.contains("public Optional<String> displayName()"),
        "Optional method return type should be Optional<T>. Got:\n{}",
        client.content
    );
    assert!(
        client.content.contains("import java.util.Optional;"),
        "Optional opaque method return should import Optional. Got:\n{}",
        client.content
    );

    // (4) Import should be present
    assert!(
        content.contains("import org.jspecify.annotations.Nullable;"),
        "Should import @Nullable annotation. Got:\n{}",
        content
    );
}

/// Regression: streaming method template uses fully-qualified `java.util.stream.Stream<T>` and
/// `java.util.stream.StreamSupport.stream(...)` in the method body. Adding
/// `import java.util.stream.Stream;` is therefore redundant and triggers Checkstyle's
/// `UnusedImports` rule (observed in sample-llm DefaultClient.java:12 after regeneration).
/// This test asserts the import is absent for opaque-handle classes that own streaming adapters.
#[test]
fn test_no_stream_import_emitted_for_streaming_opaque_handle() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "stream_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sl"

[crates.java]
package = "com.example.streamfix"

[[crates.adapters]]
name = "events"
pattern = "streaming"
core_path = "events"
owner_type = "EventSource"
item_type = "Event"
error_type = "StreamError"
request_type = "stream_lib::EventRequest"

[[crates.adapters.params]]
name = "req"
type = "EventRequest"
"#,
    );

    let api = ApiSurface {
        crate_name: "stream_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "EventSource".to_string(),
                rust_path: "stream_lib::EventSource".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "EventRequest".to_string(),
                rust_path: "stream_lib::EventRequest".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "Event".to_string(),
                rust_path: "stream_lib::Event".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = JavaBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    let source = files
        .iter()
        .find(|f| f.path.ends_with("EventSource.java"))
        .expect("EventSource.java must be generated");

    // The streaming body template uses java.util.stream.Stream<T> as a FQN, so
    // a bare `import java.util.stream.Stream;` would be unused and Checkstyle-flagged.
    assert!(
        !source.content.contains("import java.util.stream.Stream;"),
        "EventSource.java must NOT import java.util.stream.Stream; \
         template uses FQN — bare import triggers Checkstyle UnusedImports. Got:\n{}",
        source.content
    );

    // The streaming body must still emit the FQN return type and StreamSupport bridge.
    assert!(
        source.content.contains("java.util.stream.Stream<"),
        "Streaming method must use java.util.stream.Stream<T> FQN in signature. Got:\n{}",
        source.content
    );
    assert!(
        source.content.contains("java.util.stream.StreamSupport.stream("),
        "Streaming bridge must use java.util.stream.StreamSupport.stream() FQN. Got:\n{}",
        source.content
    );
}

// ---------------------------------------------------------------------------
// iter-9 Stream B: Java facade must unwrap `Optional<T>` returned from the
// raw class through `.orElse(null)` so the declared `@Nullable T` signature
// type-checks under javac.  Also covers the Optional-Named return case where
// the body of the raw FFI class must wrap the readValue() result in
// `Optional.of(...)` so the declared `Optional<NamedDto>` signature matches
// what the body actually returns.
// ---------------------------------------------------------------------------

#[test]
fn facade_unwraps_optional_string_return_via_or_else_null() {
    let backend = JavaBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "detect_language".to_string(),
            rust_path: "test_lib::detect_language".to_string(),
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
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Optional(Box::new(TypeRef::String)),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("dev.test");
    let files = backend
        .generate_public_api(&api, &config)
        .expect("public api generation");
    let facade = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("TestLib.java"))
        .expect("facade TestLib.java must be emitted by generate_public_api");
    let content = &facade.content;

    assert!(
        content.contains("public static @Nullable String detectLanguage"),
        "facade must declare @Nullable String detectLanguage, got:\n{content}"
    );
    assert!(
        content.contains(".detectLanguage(path).orElse(null);"),
        "facade must unwrap the bridge's Optional<String> via .orElse(null), got:\n{content}"
    );
}

#[test]
fn optional_named_method_body_wraps_via_optional_of() {
    // Regression for sample_language_pack Node.parent() / Node.child() / Parser.parse():
    // when an instance method on an opaque type returns `Optional<NamedDto>`,
    // the body must build the value through `Optional.of(STREAM_MAPPER...)`
    // — never return a bare NamedDto (which fails javac's type inference).
    let backend = JavaBackend;
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "DemoItem".to_string(),
                rust_path: "test_lib::DemoItem".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: true,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "DemoHandle".to_string(),
                rust_path: "test_lib::DemoHandle".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "maybe_item".to_string(),
                    receiver: Some(ReceiverKind::Ref),
                    is_static: false,
                    params: vec![],
                    return_type: TypeRef::Optional(Box::new(TypeRef::Named("DemoItem".to_string()))),
                    is_async: false,
                    doc: String::new(),
                    error_type: Some("Error".to_string()),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    has_default_impl: false,
                    version: Default::default(),
                }],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("dev.test");
    let files = backend.generate_bindings(&api, &config).expect("generation");
    let handle = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("DemoHandle.java"))
        .expect("DemoHandle.java must be emitted");
    let content = &handle.content;

    assert!(
        content.contains("public Optional<DemoItem> maybeItem("),
        "maybeItem must declare Optional<DemoItem> return, got:\n{content}"
    );
    // The emitter generates `java.util.Optional.of(...)` (fully qualified);
    // accept either the qualified or unqualified spelling so we stay robust
    // to future import-tidying in the line-wrapper.
    let has_wrapped_of = content
        .contains("return java.util.Optional.of(STREAM_MAPPER.readValue(json, DemoItem.class));")
        || content.contains("return Optional.of(STREAM_MAPPER.readValue(json, DemoItem.class));");
    assert!(
        has_wrapped_of,
        "Optional<DemoItem> body must wrap readValue in Optional.of(...), got:\n{content}"
    );
    assert!(
        content.contains("return java.util.Optional.empty();") || content.contains("return Optional.empty();"),
        "null-handle branch must return Optional.empty() (not bare null), got:\n{content}"
    );
    // Regression boundary: the body must not return the bare `STREAM_MAPPER.readValue(...)`
    // (which is what triggered the type-mismatch error pre-fix).
    assert!(
        !content.contains("return STREAM_MAPPER.readValue(json, DemoItem.class);"),
        "Optional<DemoItem> body must not return a bare DemoItem, got:\n{content}"
    );
}

#[test]
fn builder_optional_fields_use_nullable_not_optional_in_setters() {
    let backend = JavaBackend;

    // Create a DTO with an optional field to test builder setter signatures.
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    name: "enabled".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Bool),
                    optional: false,
                    default: Some("false".to_string()),
                    doc: "Enable feature".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                },
                FieldDef {
                    name: "description".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    default: None,
                    doc: "Optional description".to_string(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
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
            doc: "Configuration object".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config_with_builder_always("dev.test");
    let files = backend.generate_bindings(&api, &config).expect("generation");
    let config_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Config.java"))
        .expect("Config.java must be emitted");
    let content = &config_file.content;

    // Builder setter for optional field must use @Nullable, not Optional<String>.
    assert!(
        content.contains("public Builder withDescription(final @Nullable String value)"),
        "Builder setter for optional String field must use @Nullable String, got:\n{content}"
    );
    // Ensure we're not using Optional<String> in the setter signature (common Rust leak).
    assert!(
        !content.contains("public Builder withDescription(final Optional<String>"),
        "Builder setter must NOT use Optional<String> in signature, got:\n{content}"
    );
    // @Nullable must be imported from jspecify.
    assert!(
        content.contains("import org.jspecify.annotations.Nullable;"),
        "@Nullable must be imported, got:\n{content}"
    );
}

#[test]
fn json_util_centralizes_from_json_deserialization() {
    let backend = JavaBackend;

    // Create a minimal DTO to test JsonUtil emission
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SimpleDto".to_string(),
            rust_path: "test::SimpleDto".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: "Some value".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
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
            doc: "Simple DTO".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("dev.test");
    let files = backend.generate_bindings(&api, &config).expect("generation");

    // Check that JsonUtil is emitted
    let json_util = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("JsonUtil.java"))
        .expect("JsonUtil.java must be emitted");
    let util_content = &json_util.content;

    // Verify JsonUtil structure
    assert!(
        util_content.contains("public final class JsonUtil"),
        "JsonUtil class must be public final, got:\n{util_content}"
    );
    assert!(
        util_content.contains("public static <T> T fromJson(final String json, final Class<T> targetClass)"),
        "JsonUtil must have fromJson generic method, got:\n{util_content}"
    );

    // Check that per-DTO fromJson is removed
    let dto_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("SimpleDto.java"))
        .expect("SimpleDto.java must be emitted");
    let dto_content = &dto_file.content;

    assert!(
        !dto_content.contains("public static SimpleDto fromJson(String json)"),
        "Per-DTO fromJson must be removed (use JsonUtil instead), got:\n{dto_content}"
    );
}

#[test]
fn javadoc_sanitizes_rust_syntax() {
    let backend = JavaBackend;

    // Create an opaque handle type to test documentation sanitization
    // (opaque handles always emit full javadoc, unlike record field docs)
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ConfigHandle".to_string(),
            rust_path: "test::ConfigHandle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: r#"Configuration handle for processing.

This handle manages the `OutputFormat::None` output format internally.
Callers should never call `.unwrap()` directly — use the provided builder
pattern. When calling `Option::expect()` in Rust code, ensure proper error
handling. The underlying `self.format` field stores the configuration.

Related: `ConversionOptions::output_format` and `Result::unwrap_or()`."#
                .to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("dev.test");
    let files = backend.generate_bindings(&api, &config).expect("generation");
    let dto_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("ConfigHandle.java"))
        .expect("ConfigHandle.java must be emitted");
    let content = &dto_file.content;

    // Rust :: should be converted to . (Java package style)
    assert!(
        content.contains("{@code OutputFormat.None}") || content.contains("OutputFormat.None"),
        "Rust :: should become . in Javadoc, got:\n{content}"
    );
    assert!(
        !content.contains("OutputFormat::None"),
        "Rust :: should not appear in generated code, got:\n{content}"
    );

    // .unwrap() / .expect() should be sanitized (removed)
    assert!(
        !content.contains(".unwrap()"),
        ".unwrap() Rust idiom must be removed, got:\n{content}"
    );
    assert!(
        !content.contains(".expect("),
        ".expect() Rust idiom must be removed, got:\n{content}"
    );

    // Verify the key idioms are gone
    assert!(
        !content.contains("Result::unwrap_or()"),
        "Rust Result::unwrap_or() must become Result.orElse(), got:\n{content}"
    );
}

#[test]
fn test_trait_bridge_clear_fn_generates_correct_error_handling() {
    let backend = JavaBackend;

    // Create a simple API with a unit-return function that should be handled as a clear_fn
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "clear_validators".to_string(),
            rust_path: "test_lib::clear_validators".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Clear all validators".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

# Configure trait bridge for test
[[crates.trait_bridges]]
trait_name = "Validator"
clear_fn = "clear_validators"

[crates.java]
package = "com.example"
"#,
    );

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generation failed: {:?}", result);
    let files = result.unwrap();

    // Check main class generates clear_validators method
    let main_class = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .expect("TestLibRs.java must be emitted");
    let content = &main_class.content;

    // Verify the method exists
    assert!(
        content.contains("public static void clearValidators()"),
        "clearValidators method must exist, got:\n{content}"
    );

    // Verify error handling: should allocate outErr, invoke with it, check result code
    assert!(
        content.contains("var outErr = arena.allocate(ValueLayout.ADDRESS)"),
        "Should allocate outErr buffer, got:\n{content}"
    );

    // Verify the invocation passes outErr as an argument
    assert!(
        content.contains("outErr)"),
        "Should pass outErr to FFI invocation, got:\n{content}"
    );

    // Verify error code checking
    assert!(
        content.contains("if (primitiveResult != 0)"),
        "Should check primitiveResult != 0 for error, got:\n{content}"
    );

    // Verify error message extraction from the out-error pointer
    assert!(
        content.contains("outErr.get(ValueLayout.ADDRESS, 0)"),
        "Should read error pointer from outErr, got:\n{content}"
    );

    // Verify exception throwing on error
    assert!(
        content.contains("throw new TestLibRsException"),
        "Should throw exception on error, got:\n{content}"
    );

    // Verify it uses the correct handle constant (singular)
    assert!(
        content.contains("TEST_CLEAR_VALIDATOR"),
        "Should use singular TEST_CLEAR_VALIDATOR handle, got:\n{content}"
    );
}

#[test]
fn options_field_visitor_uses_trait_bridge_config_not_convert_literals() {
    let backend = JavaBackend;

    let field = |name: &str, ty: TypeRef| FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: alef::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    };
    let record = |name: &str, fields: Vec<FieldDef>| TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            record("VisitContext", vec![field("path", TypeRef::String)]),
            record(
                "WorkConfig",
                vec![
                    field("hook", TypeRef::Named("CallbackHandle".to_string())),
                    field("mode", TypeRef::String),
                ],
            ),
            record("WorkResult", vec![field("text", TypeRef::String)]),
            TypeDef {
                name: "Callback".to_string(),
                rust_path: "test_lib::Callback".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "inspect".to_string(),
                    params: vec![
                        ParamDef {
                            name: "context".to_string(),
                            ty: TypeRef::Named("VisitContext".to_string()),
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: alef::core::ir::CoreWrapper::None,
                        },
                        ParamDef {
                            name: "label".to_string(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            sanitized: false,
                            typed_default: None,
                            is_ref: true,
                            is_mut: false,
                            newtype_wrapper: None,
                            original_type: None,
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: alef::core::ir::CoreWrapper::None,
                        },
                    ],
                    return_type: TypeRef::Named("FlowDecision".to_string()),
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    receiver: Some(ReceiverKind::RefMut),
                    sanitized: false,
                    trait_source: None,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: true,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: false,
                is_clone: false,
                is_copy: false,
                is_trait: true,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![FunctionDef {
            name: "process_html".to_string(),
            rust_path: "test_lib::process_html".to_string(),
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
                    map_is_ahash: false,
                    map_key_is_cow: false,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("WorkConfig".to_string()),
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
                },
            ],
            return_type: TypeRef::Named("WorkResult".to_string()),
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
        }],
        enums: vec![EnumDef {
            name: "FlowDecision".to_string(),
            rust_path: "test_lib::FlowDecision".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Proceed".to_string(),
                    is_default: true,
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "DropNode".to_string(),
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "ReplaceWith".to_string(),
                    fields: vec![field("value", TypeRef::String)],
                    is_tuple: true,
                    ..EnumVariant::default()
                },
            ],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

[[crates.trait_bridges]]
trait_name = "Callback"
type_alias = "CallbackHandle"
bind_via = "options_field"
options_type = "WorkConfig"
options_field = "hook"
context_type = "VisitContext"
result_type = "FlowDecision"
"#,
    );

    let files = backend
        .generate_bindings(&api, &config)
        .expect("java generation must succeed");
    let main = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .expect("raw FFI class must be emitted");
    let main_content = &main.content;
    assert!(main_content.contains("if (config != null && config.hook() != null)"));
    assert!(main_content.contains("return processHtmlWithVisitorInternal(html, config);"));
    assert!(main_content.contains("private static WorkResult processHtmlWithVisitorInternal"));
    assert!(main_content.contains("new VisitorBridge(config.hook())"));
    assert!(main_content.contains("NativeLib.TEST_PROCESS_HTML"));
    assert!(main_content.contains("chtml"));
    assert!(main_content.contains("cconfig"));
    assert!(main_content.contains("NativeLib.TEST_WORK_CONFIG_FREE"));
    assert!(main_content.contains("NativeLib.TEST_WORK_RESULT_TO_JSON"));
    assert!(main_content.contains("NativeLib.TEST_WORK_RESULT_FREE"));
    assert!(main_content.contains("return MAPPER.readValue(json, WorkResult.class);"));
    assert!(!main_content.contains("convertWithVisitorInternal"));
    assert!(!main_content.contains("ConversionOptions"));
    assert!(!main_content.contains("ConversionResult"));

    let visitor = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("Callback.java"))
        .expect("visitor interface must be emitted");
    assert!(visitor.content.contains("public interface Callback"));
    assert!(visitor.content.contains("FlowDecision inspect"));
    assert!(visitor.content.contains("return new FlowDecision.Proceed();"));

    let result = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("FlowDecision.java"))
        .expect("visitor result enum must be emitted");
    assert!(result.content.contains("public sealed interface FlowDecision"));
    assert!(result.content.contains("record Proceed()"));
    assert!(result.content.contains("record ReplaceWith(String value)"));

    let options = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("WorkConfig.java"))
        .expect("options record must be emitted");
    assert!(options.content.contains("@JsonIgnore Callback hook"));
    assert!(options.content.contains("withHook(final Callback value)"));
    assert!(!options.content.contains("Visitor hook"));
}

#[test]
fn test_facade_no_java_lang_imports() {
    // BLK-12: Regression test that verifies no `java.lang.*` types are explicitly imported
    // in generated Java facades. These types are auto-imported by the JLS, and checkstyle's
    // UnusedImports rule will reject any explicit import.
    let backend = JavaBackend;

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "value".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: "Test value".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: alef::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_test_config("com.example");
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generation failed: {:?}", result);
    let files = result.unwrap();

    // Find the main facade class
    let facade_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("TestLibRs.java"))
        .expect("TestLibRs.java (facade) must be emitted");
    let content = &facade_file.content;

    // BLK-12: Verify that there is NO explicit `import java.lang.X;` statements
    // Iterable, String, Object, etc. are auto-imported by the JLS and must never
    // be explicitly imported, or checkstyle's UnusedImports rule will reject them.
    assert!(
        !content.contains("import java.lang."),
        "Facade must NOT contain any 'import java.lang.*' (auto-imported by JLS), got:\n{content}"
    );

    // Sanity checks: verify other non-auto-imported types ARE imported correctly
    // If the facade uses List or Optional, those should still be imported (from java.util)
    if content.contains("List<") {
        assert!(
            content.contains("import java.util.List;"),
            "Facade must import List from java.util, got:\n{content}"
        );
    }
    if content.contains("Optional<") {
        assert!(
            content.contains("import java.util.Optional;"),
            "Facade must import Optional from java.util, got:\n{content}"
        );
    }
}

/// Regression test: streaming adapter item types must have _to_json handles in NativeLib,
/// even if the type has has_serde=false in the IR (due to cfg gating).
/// Without the fix, this would cause: "cannot find symbol: variable KCRAWL_CRAWL_EVENT_TO_JSON"
#[test]
fn test_streaming_adapter_item_to_json_handle_emitted_unconditionally() {
    let backend = JavaBackend;

    let config = resolved_one(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "crawl_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "kcrawl"

[crates.java]
package = "dev.kreuzberg.kreuzcrawl"

[[crates.adapters]]
name = "crawl"
pattern = "streaming"
core_path = "crawl"
owner_type = "CrawlEngine"
item_type = "CrawlEvent"
request_type = "crawl_lib::CrawlRequest"

[[crates.adapters.params]]
name = "request"
type = "CrawlRequest"
"#,
    );

    // CrawlEvent has has_serde=false (simulating cfg-gated serde derive)
    let api = ApiSurface {
        crate_name: "crawl_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "CrawlEngine".to_string(),
                rust_path: "crawl_lib::CrawlEngine".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "crawl".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: true,
                    is_static: false,
                    error_type: None,
                    doc: String::new(),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    receiver: Some(ReceiverKind::Ref),
                    trait_source: None,
                    has_default_impl: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: true,
                is_clone: false,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "CrawlEvent".to_string(),
                rust_path: "crawl_lib::CrawlEvent".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false, // Simulates cfg-gated serde derive
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "CrawlRequest".to_string(),
                rust_path: "crawl_lib::CrawlRequest".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generation failed: {:?}", result);
    let files = result.unwrap();

    // Check NativeLib.java for _to_json handle for CrawlEvent (stream item type)
    let native_lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("NativeLib"))
        .expect("NativeLib.java must be emitted");

    // Even though CrawlEvent has has_serde=false, because it's a streaming item type,
    // NativeLib must emit KCRAWL_CRAWL_EVENT_TO_JSON handle (referenced by streaming_iterator_method)
    assert!(
        native_lib.content.contains("KCRAWL_CRAWL_EVENT_TO_JSON"),
        "NativeLib must emit KCRAWL_CRAWL_EVENT_TO_JSON MethodHandle for streaming item type CrawlEvent, even with has_serde=false. Got:\n{}",
        native_lib.content
    );

    // Also verify _from_json is emitted for the request type
    assert!(
        native_lib.content.contains("KCRAWL_CRAWL_REQUEST_FROM_JSON"),
        "NativeLib must emit KCRAWL_CRAWL_REQUEST_FROM_JSON MethodHandle for streaming request type CrawlRequest. Got:\n{}",
        native_lib.content
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// untagged_union_text_types — text() accessor emission
// ──────────────────────────────────────────────────────────────────────────────

fn make_assistant_content_enum() -> alef::core::ir::EnumDef {
    alef::core::ir::EnumDef {
        name: "AssistantContent".to_string(),
        rust_path: "test_lib::AssistantContent".to_string(),
        original_rust_path: String::new(),
        methods: vec![],
        doc: "Multimodal assistant content.".to_string(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: None,
        serde_untagged: true,
        serde_rename_all: None,
        variants: vec![
            alef::core::ir::EnumVariant {
                name: "Text".to_string(),
                doc: String::new(),
                fields: vec![make_newtype_field(TypeRef::String)],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            alef::core::ir::EnumVariant {
                name: "Parts".to_string(),
                doc: String::new(),
                fields: vec![make_newtype_field(TypeRef::Vec(Box::new(TypeRef::Named(
                    "ContentPart".to_string(),
                ))))],
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: true,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

fn resolved_with_text_types(text_types: &[&str]) -> ResolvedCrateConfig {
    let list = text_types
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    resolved_one(&format!(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]
untagged_union_text_types = [{list}]

[crates.ffi]
prefix = "test"

[crates.java]
package = "dev.test"
"#
    ))
}

/// Without `untagged_union_text_types` configured, no `text()` method appears.
#[test]
fn java_untagged_wrapper_without_text_types_does_not_emit_text_method() {
    let backend = JavaBackend;
    let config = make_test_config("dev.test");

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![make_assistant_content_enum()],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let content_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("AssistantContent.java"))
        .expect("AssistantContent.java must be emitted");

    assert!(
        content_file.content.contains("class AssistantContent"),
        "wrapper class must be present"
    );
    assert!(
        !content_file.content.contains("public String text()"),
        "text() must NOT be emitted when untagged_union_text_types is empty:\n{}",
        content_file.content
    );
}

/// With `untagged_union_text_types = ["AssistantContent"]`, a `text()` method
/// is emitted with the correct JSON-string and JSON-array branches.
#[test]
fn java_untagged_wrapper_with_text_types_emits_text_method() {
    let backend = JavaBackend;
    let config = resolved_with_text_types(&["AssistantContent"]);

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![make_assistant_content_enum()],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let content_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("AssistantContent.java"))
        .expect("AssistantContent.java must be emitted");

    let src = &content_file.content;
    assert!(src.contains("public String text()"), "text() must be emitted:\n{src}");
    // Must return string when value is textual
    assert!(src.contains("value.isTextual()"), "must handle JSON string:\n{src}");
    // Must iterate array for parts
    assert!(src.contains("value.isArray()"), "must handle JSON array:\n{src}");
    // Must filter by type=="text"
    assert!(
        src.contains("\"text\".equals(typeNode.asText())"),
        "must filter by type=text:\n{src}"
    );
    // Returns empty string by default
    assert!(
        src.contains("return \"\";"),
        "must return empty string as fallback:\n{src}"
    );
}
