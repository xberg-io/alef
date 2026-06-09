use alef::backends::go::GoBackend;
use alef::backends::go::trait_bridge::gen_trait_bridges_file;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Helper to create a FieldDef with all defaults
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
        original_type: None,
    }
}

/// Helper to create a ResolvedCrateConfig with both FFI and Go configs.
fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.go]
module = "github.com/test/test-lib"
"#,
    )
}

#[test]
fn test_basic_generation() {
    let backend = GoBackend;

    // Create test API surface with 1 type, 1 function, 1 enum
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("name", TypeRef::String, true),
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
            has_serde: true,
            super_traits: vec![],
            doc: "Configuration type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "data".to_string(),
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
            doc: "Process data".to_string(),
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
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Slow".to_string(),
                    fields: vec![],
                    doc: "Slow mode".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
            ],
            doc: "Processing mode".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
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

    let config = make_config();

    // Generate bindings
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    assert!(!files.is_empty(), "Should generate files");

    let binding_file = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("binding.go"))
        .expect("Should generate binding.go file");

    let content = &binding_file.content;

    // Verify Go package declaration
    assert!(content.contains("package testlib"), "Should declare Go package");

    // Verify cgo directives
    assert!(content.contains("#cgo CFLAGS:"), "Should have cgo CFLAGS directive");
    assert!(content.contains("import \"C\""), "Should import C");

    // Verify standard Go imports
    assert!(content.contains("import ("), "Should have import block");
    assert!(content.contains("\"fmt\""), "Should import fmt");
    assert!(content.contains("\"encoding/json\""), "Should import encoding/json");
    assert!(content.contains("\"unsafe\""), "Should import unsafe");

    // Verify error helper
    assert!(content.contains("func lastError()"), "Should define lastError helper");
    assert!(
        content.contains("C.test_last_error_code()"),
        "Should call FFI error code function"
    );
    assert!(
        content.contains("if ctx == nil {\n\t\treturn fmt.Errorf(\"[%d] native error\", code)\n\t}"),
        "lastError must tolerate a nonzero error code with no context pointer"
    );

    // Verify struct generation
    assert!(content.contains("type Config struct"), "Should define Config struct");
    assert!(content.contains("Timeout"), "Should have Timeout field");
    assert!(content.contains("Name"), "Should have Name field");
    assert!(content.contains("json:"), "Should have JSON tags");

    // Verify enum generation
    assert!(
        content.contains("type Mode string"),
        "Should define Mode as string enum"
    );
    assert!(content.contains("const ("), "Should have const block for enum values");
    assert!(content.contains("ModeFast"), "Should have ModeFast constant");
    assert!(content.contains("ModeSlow"), "Should have ModeSlow constant");

    // Verify function wrapper
    assert!(content.contains("func Process("), "Should define Process function");
}

#[test]
fn bytes_params_are_pinned_before_c_calls() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "upload".to_string(),
            rust_path: "test_lib::upload".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::Bytes,
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
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GoBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = files
        .iter()
        .find(|f| f.path.ends_with("binding.go"))
        .expect("binding.go")
        .content
        .as_str();

    assert!(
        content.contains("\"runtime\""),
        "byte-slice pinning must import runtime: {content}"
    );
    assert!(
        content.contains("var cPayloadPinner runtime.Pinner"),
        "byte-slice params must allocate a runtime.Pinner: {content}"
    );
    assert!(
        content.contains("cPayloadPinner.Pin(&payload[0])"),
        "byte-slice params must pin the first element before taking its address: {content}"
    );
    assert!(
        content.contains("defer cPayloadPinner.Unpin()"),
        "byte-slice params must unpin after the C call scope returns: {content}"
    );
}

#[test]
fn test_ffi_excluded_types_are_not_generated_for_cgo() {
    let backend = GoBackend;
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"
exclude_types = ["HiddenHandle"]

[crates.go]
module = "github.com/test/test-lib"
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
        crate_name: "test-lib".to_string(),
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
    let binding_go = files.iter().find(|file| file.path.ends_with("binding.go")).unwrap();

    assert!(!binding_go.content.contains("type HiddenHandle struct"));
    assert!(!binding_go.content.contains("C.test_hidden_handle"));
    assert!(!binding_go.content.contains("C.test_visible_handle_hidden"));
    assert!(binding_go.content.contains("type VisibleHandle struct"));
}

#[test]
fn test_type_mapping() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Numbers".to_string(),
            rust_path: "test_lib::Numbers".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("u32_val", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("i64_val", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("string_val", TypeRef::String, true),
                make_field("vec_val", TypeRef::Vec(Box::new(TypeRef::String)), false),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify Go type mappings — gofmt aligns fields with variable whitespace,
    // so we check for the field name and type without exact spacing.
    let lines: Vec<&str> = content.lines().collect();
    let struct_lines: Vec<&&str> = lines.iter().filter(|l| l.contains("Val")).collect();
    assert!(
        struct_lines
            .iter()
            .any(|l| l.contains("U32Val") && l.contains("uint32")),
        "U32 should map to uint32"
    );
    assert!(
        struct_lines.iter().any(|l| l.contains("I64Val") && l.contains("int64")),
        "I64 should map to int64"
    );
    assert!(
        struct_lines
            .iter()
            .any(|l| l.contains("StringVal") && l.contains("*string")),
        "Optional String should be *string"
    );
    assert!(
        struct_lines
            .iter()
            .any(|l| l.contains("VecVal") && l.contains("[]string")),
        "Vec<String> should map to []string"
    );
}

#[test]
fn test_enum_generation() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    doc: "Pending status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    doc: "Active status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Completed".to_string(),
                    fields: vec![],
                    doc: "Completed status".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    version: Default::default(),
                },
            ],
            doc: "Status enum".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify enum type declaration
    assert!(
        content.contains("type Status string"),
        "Should define Status as string type"
    );

    // Verify const block with all variants
    assert!(content.contains("const ("), "Should define const block");
    assert!(content.contains("StatusPending"), "Should have StatusPending constant");
    assert!(content.contains("StatusActive"), "Should have StatusActive constant");
    assert!(
        content.contains("StatusCompleted"),
        "Should have StatusCompleted constant"
    );

    // Verify string values follow Rust serde defaults when no rename_all is configured.
    assert!(content.contains("\"Pending\""), "Should use Rust variant values");
    assert!(content.contains("\"Active\""), "Should use Rust variant values");
    assert!(content.contains("\"Completed\""), "Should use Rust variant values");
}

#[test]
fn test_generated_header() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok());

    let files = result.unwrap();
    assert!(!files.is_empty());

    let binding_file = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("binding.go"))
        .expect("Should generate binding.go file");
    assert!(
        binding_file.generated_header,
        "binding.go should have generated_header=true"
    );
}

#[test]
fn test_methods_generation() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Handler".to_string(),
            rust_path: "test_lib::Handler".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("id", TypeRef::Primitive(PrimitiveType::U64), false)],
            methods: vec![
                // Instance method that returns a value
                MethodDef {
                    name: "get_name".to_string(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Get the name".to_string(),
                    receiver: Some(alef::core::ir::ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                // Static method that returns a primitive (not skipped)
                MethodDef {
                    name: "version".to_string(),
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    is_static: true,
                    error_type: None,
                    doc: "Get the version string".to_string(),
                    receiver: None,
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
                // Instance method with parameters and error
                MethodDef {
                    name: "process".to_string(),
                    params: vec![ParamDef {
                        name: "data".to_string(),
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
                    return_type: TypeRef::Unit,
                    is_async: false,
                    is_static: false,
                    error_type: Some("Error".to_string()),
                    doc: "Process data".to_string(),
                    receiver: Some(alef::core::ir::ReceiverKind::Ref),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                },
            ],
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
            doc: "A handler type".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify instance method wrapper (receiver with pointer).
    // Non-opaque receivers must be marshaled to JSON, so even methods without explicit error_type
    // return (T, error). Verify GetName returns (string, error) with bare string (not *string).
    assert!(
        content.contains("func (r *Handler) GetName(") && content.contains(") (string, error) {"),
        "Should define instance method GetName returning (string, error) with bare string type"
    );

    // Verify static method (no receiver, becomes function)
    assert!(
        content.contains("func HandlerVersion()"),
        "Should define static method as package-level function"
    );

    // Verify instance method with error return.
    // The generator emits single-line canonical signatures, so check for the
    // complete signature or unambiguous substrings.
    assert!(
        content.contains("func (r *Handler) Process("),
        "Should define Process method receiver with name"
    );
    assert!(
        content.contains("data string) error"),
        "Process should take data string and return error"
    );
    assert!(
        content.contains("return lastError()"),
        "Process should call lastError() for error handling"
    );
}

#[test]
fn test_error_types() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "GoError".to_string(),
            rust_path: "test_lib::GoError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "NotFound".to_string(),
                    fields: vec![],
                    doc: "Resource not found".to_string(),
                    message_template: Some("not found".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: true,
                    is_tuple: false,
                },
                ErrorVariant {
                    name: "InvalidInput".to_string(),
                    fields: vec![make_field("reason", TypeRef::String, false)],
                    doc: "Invalid input provided".to_string(),
                    message_template: Some("invalid input: {reason}".to_string()),
                    has_source: false,
                    has_from: false,
                    is_unit: false,
                    is_tuple: false,
                },
            ],
            doc: "Error type for library".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify error type generation via alef_codegen
    // The gen_go_error_types function generates sentinel errors and error wrappers
    assert!(
        content.contains("errors") || content.contains("Error"),
        "Should import or reference errors package"
    );
    // Verify error-related code is generated
    assert!(
        content.contains("GoError") || content.contains("lastError"),
        "Should generate error-related code"
    );
}

#[test]
fn test_async_function() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "async_process".to_string(),
            rust_path: "test_lib::async_process".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
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
            is_async: true,
            error_type: Some("Error".to_string()),
            doc: "Process data asynchronously".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify async function is generated
    assert!(content.contains("func AsyncProcess("), "Should define async function");
    // Async functions in Go use the FFI with block_on() internally, but the wrapper is still generated
    assert!(
        content.contains("AsyncProcess"),
        "Async function should be included in generated code"
    );
}

#[test]
fn test_opaque_type() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
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
            doc: "An opaque handle to Rust state".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // Verify opaque type wraps unsafe.Pointer
    assert!(
        content.contains("type OpaqueHandle struct"),
        "Should define OpaqueHandle struct"
    );
    assert!(
        content.contains("ptr unsafe.Pointer"),
        "Should have ptr field of unsafe.Pointer type"
    );
    assert!(
        content.contains("\"unsafe\""),
        "Should import unsafe package for opaque types"
    );

    // Verify Free method
    assert!(
        content.contains("func (h *OpaqueHandle) Free()"),
        "Should define Free method for opaque type"
    );
    assert!(
        content.contains("test_opaque_handle_free") || content.contains("Free"),
        "Free method should call FFI free function"
    );
}

#[test]
fn test_default_config() {
    let backend = GoBackend;
    let field_with_default = |name: &str, ty: TypeRef, default| {
        let mut field = make_field(name, ty, false);
        field.typed_default = Some(default);
        field
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                field_with_default(
                    "timeout",
                    TypeRef::Primitive(PrimitiveType::U32),
                    DefaultValue::IntLiteral(30),
                ),
                field_with_default(
                    "retries",
                    TypeRef::Primitive(PrimitiveType::U8),
                    DefaultValue::IntLiteral(3),
                ),
                make_field("name", TypeRef::String, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true, // Enable functional options
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Configuration with defaults".to_string(),
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

    let config = make_config();

    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "Generation should succeed");

    let files = result.unwrap();
    let content = &files[0].content;

    // As of STY-9 the Go backend defaults to plain struct literals + a single
    // `Ptr[T]` helper, and only emits `With<Field>` / `New<Struct>` for struct
    // names listed in `[crates.go] functional_options`. With no allowlist the
    // functional-options shape must be absent.
    assert!(
        !content.contains("type ConfigOption"),
        "Should NOT emit functional-options type alias by default; got:\n{content}"
    );
    assert!(
        !content.contains("func WithConfig"),
        "Should NOT emit With<Field> functional-options helpers by default; got:\n{content}"
    );
    assert!(
        !content.contains("func NewConfig("),
        "Should NOT emit a New<Struct> functional-options constructor by default; got:\n{content}"
    );

    // The plain struct shape and the shared `Ptr[T]` helper must be present so
    // callers can construct `Config{Timeout: Ptr[uint32](30)}` directly.
    assert!(
        content.contains("type Config struct"),
        "Should emit the plain struct definition; got:\n{content}"
    );
    assert!(
        content.contains("func Ptr[T any](v T) *T"),
        "Should emit the shared generic Ptr[T] helper for the plain-DTO shape; got:\n{content}"
    );
}

#[test]
fn test_optional_primitive_uses_cgo_types() {
    // Regression test: optional primitive params must be declared using CGo types
    // (C.uint64_t, C.uint32_t, etc.) rather than Go native types, because CGo
    // does not implicitly convert between Go numeric types and C typedef types
    // when calling C functions.
    let backend = GoBackend;

    let make_param = |name: &str, prim: PrimitiveType| ParamDef {
        name: name.to_string(),
        ty: TypeRef::Primitive(prim),
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "create_thing".to_string(),
            rust_path: "test_lib::create_thing".to_string(),
            original_rust_path: String::new(),
            params: vec![
                make_param("timeout_secs", PrimitiveType::U64),
                make_param("max_retries", PrimitiveType::U32),
            ],
            return_type: TypeRef::Unit,
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

    let config = make_config();
    let result = backend.generate_bindings(&api, &config).unwrap();
    let content = &result[0].content;

    // The temporary variables must be declared as CGo types, not Go native types.
    // Wrong (old): var cTimeoutSecs uint64 = ^uint64(0)
    // Right (new): var cTimeoutSecs C.uint64_t = C.uint64_t(^uint64(0))
    assert!(
        content.contains("C.uint64_t(^uint64(0))"),
        "U64 optional sentinel should be cast to C.uint64_t, got:\n{}",
        content
    );
    assert!(
        content.contains("C.uint32_t(^uint32(0))"),
        "U32 optional sentinel should be cast to C.uint32_t"
    );
    assert!(
        !content.contains("var cTimeoutSecs uint64"),
        "Should not declare cTimeoutSecs as Go uint64 — must use C.uint64_t"
    );
    assert!(
        !content.contains("var cMaxRetries uint32"),
        "Should not declare cMaxRetries as Go uint32 — must use C.uint32_t"
    );
}

#[test]
fn test_optional_return_type_no_double_pointer() {
    // Regression test: a function returning Option<String> (TypeRef::Optional(String))
    // must produce a *string return type, not **string.
    // go_type(Optional(String)) already emits "*string"; adding an extra "*" prefix
    // in the return type calculation produced "**string" which is invalid Go.
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "detect_language".to_string(),
            rust_path: "test_lib::detect_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "ext".to_string(),
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
            doc: "Detect language from extension".to_string(),
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

    let config = make_config();
    let result = backend.generate_bindings(&api, &config).unwrap();
    let content = &result[0].content;

    // Must NOT contain a double-pointer return type
    assert!(
        !content.contains("**string"),
        "Optional<String> return must not produce **string, got:\n{}",
        content
    );
    // Must contain the correct single-pointer return type
    assert!(
        content.contains("*string"),
        "Optional<String> return should produce *string, got:\n{}",
        content
    );
}

// ---------------------------------------------------------------------------
// Trait bridge helpers
// ---------------------------------------------------------------------------

fn make_trait_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("my_lib::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods,
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
    }
}

fn make_trait_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef, has_error: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: false,
        is_static: false,
        error_type: if has_error {
            Some("Box<dyn std::error::Error + Send + Sync>".to_string())
        } else {
            None
        },
        doc: format!("{name} method."),
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
    }
}

fn make_trait_param(name: &str, ty: TypeRef) -> ParamDef {
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

fn make_config_with_bridges(bridge_configs: Vec<TraitBridgeConfig>) -> ResolvedCrateConfig {
    let mut config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "krz"
visitor_callbacks = true

[crates.go]
module = "github.com/test/test-lib"
"#,
    );
    config.trait_bridges = bridge_configs;
    config
}

fn make_api_with_type(trait_type: TypeDef) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn test_options_field_visitor_wrapper_uses_bridge_config_not_convert_names() {
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        type_alias: Some("RendererHandle".to_string()),
        param_name: Some("renderer".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("renderer".to_string()),
        ..TraitBridgeConfig::default()
    };
    let mut config = make_config_with_bridges(vec![bridge_cfg]);
    config.go.as_mut().unwrap().functional_options = vec![];

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "Renderer".to_string(),
                rust_path: "my_lib::Renderer".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![make_trait_method(
                    "visit_text",
                    vec![make_trait_param("text", TypeRef::String)],
                    TypeRef::Unit,
                    false,
                )],
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
            TypeDef {
                name: "RenderOptions".to_string(),
                rust_path: "my_lib::RenderOptions".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    make_field("renderer", TypeRef::Named("RendererHandle".to_string()), true),
                    make_field("visitor", TypeRef::Named("AuditVisitor".to_string()), true),
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
                name: "RenderOutput".to_string(),
                rust_path: "my_lib::RenderOutput".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("html", TypeRef::String, false)],
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
        ],
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "my_lib::render".to_string(),
            original_rust_path: String::new(),
            params: vec![
                make_trait_param("document", TypeRef::String),
                ParamDef {
                    optional: true,
                    ..make_trait_param(
                        "settings",
                        TypeRef::Optional(Box::new(TypeRef::Named("RenderOptions".to_string()))),
                    )
                },
            ],
            return_type: TypeRef::Named("RenderOutput".to_string()),
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: "Render a document.".to_string(),
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

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go must be generated")
        .content
        .as_str();

    assert!(binding.contains("func Render(document string, settings *RenderOptions) (*RenderOutput, error)"));
    assert!(binding.contains("Renderer Visitor `json:\"-\"`"));
    assert!(binding.contains("Visitor *json.RawMessage `json:\"visitor,omitempty\"`"));
    assert!(binding.contains("if settings != nil && settings.Renderer != nil"));
    assert!(binding.contains("return renderWithVisitorHelper(document, settings, settings.Renderer)"));
    assert!(binding.contains("var cOptions *C.KRZRenderOptions"));
    assert!(binding.contains("cOptions = C.krz_render_options_from_json(tmpStr)"));
    assert!(binding.contains("ptr := C.krz_render(cDocument, cOptions)"));
    assert!(binding.contains("defer C.krz_render_output_free(ptr)"));
    assert!(binding.contains("jsonPtr := C.krz_render_output_to_json(ptr)"));
    assert!(!binding.contains("convertWithVisitorHelper"));
    assert!(!binding.contains("HTMConversionOptions"));
    assert!(!binding.contains("ConversionResult"));
}

// ---------------------------------------------------------------------------
// Go interface generation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_produces_go_interface() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_ocr_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("type OcrBackend interface"),
        "should generate Go interface for the trait"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_includes_plugin_lifecycle_methods() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_scanner_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Plugin lifecycle methods must always be present in the interface
    assert!(
        code.contains("Name() string"),
        "Go interface must include Name() string"
    );
    assert!(
        code.contains("Version() string"),
        "Go interface must include Version() string"
    );
    assert!(
        code.contains("Initialize() error"),
        "Go interface must include Initialize() error"
    );
    assert!(
        code.contains("Shutdown() error"),
        "Go interface must include Shutdown() error"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_includes_trait_methods_in_pascal_case() {
    let trait_type = make_trait_type(
        "ImageProcessor",
        vec![
            make_trait_method("process_image", vec![], TypeRef::String, true),
            make_trait_method(
                "get_format",
                vec![make_trait_param("path", TypeRef::String)],
                TypeRef::String,
                false,
            ),
        ],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "ImageProcessor".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_image_processor".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("ProcessImage("),
        "trait method names must be converted to PascalCase in the Go interface"
    );
    assert!(
        code.contains("GetFormat("),
        "trait method names must be converted to PascalCase in the Go interface"
    );
}

#[test]
fn test_gen_trait_bridges_file_interface_method_with_error_returns_tuple_or_error() {
    let trait_type = make_trait_type(
        "Analyzer",
        vec![
            make_trait_method("analyze", vec![], TypeRef::String, true), // (string, error)
            make_trait_method("ping", vec![], TypeRef::Unit, true),      // error
        ],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("(string, error)"),
        "method with non-unit return and error must produce (T, error) return type"
    );
    // Unit return with error: just "error"
    assert!(
        code.contains("Ping() error") || code.contains("Ping()"),
        "method with unit return and error must produce 'error' return type"
    );
}

// ---------------------------------------------------------------------------
// Trampoline generation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_generates_exported_trampolines() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Each trait method must have a //export trampoline
    assert!(
        code.contains("//export goOcrBackendProcess"),
        "trampoline for 'process' must be exported as goOcrBackendProcess"
    );
    // Plugin lifecycle trampolines
    assert!(
        code.contains("//export goOcrBackendName"),
        "plugin Name trampoline must be exported"
    );
    assert!(
        code.contains("//export goOcrBackendInitialize"),
        "plugin Initialize trampoline must be exported"
    );
    assert!(
        code.contains("//export goOcrBackendFreeUserData"),
        "free_user_data trampoline must be exported"
    );
}

#[test]
fn test_gen_trait_bridges_file_trampolines_retrieve_go_object_via_cgo_handle() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("cgo.Handle(uintptr(unsafe.Pointer(userData)))"),
        "trampolines must retrieve the Go object via cgo.Handle from userData"
    );
    assert!(
        code.contains("runtime/cgo"),
        "must import runtime/cgo for cgo.Handle support"
    );
}

#[test]
fn test_trait_bridge_string_return_is_not_json_quoted() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("cResult := C.CString(result)"),
        "string callback returns must cross the FFI boundary as raw UTF-8, not JSON: {code}"
    );
    assert!(
        !code.contains("json.Marshal(result)\n\tcResult := C.CString(string(jsonBytes))"),
        "string callback return must not be JSON-quoted before Rust decodes it: {code}"
    );
}

#[test]
fn test_gen_trait_bridges_file_trampoline_converts_string_param_from_c() {
    let trait_type = make_trait_type(
        "Greeter",
        vec![make_trait_method(
            "greet",
            vec![make_trait_param("message", TypeRef::String)],
            TypeRef::Unit,
            false,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Greeter".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_greeter".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("C.GoString(message)"),
        "trampoline must convert *C.char parameter to Go string via C.GoString"
    );
}

// ---------------------------------------------------------------------------
// Registration function
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_registration_fn_builds_vtable_and_calls_c_register() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("func RegisterOcrBackend(impl OcrBackend) error"),
        "registration function must have the correct Go signature"
    );
    assert!(
        code.contains("cgo.NewHandle(impl)"),
        "registration must create a cgo.Handle for the Go object"
    );
    assert!(
        code.contains("C.krz_register_ocr_backend("),
        "registration must call the C FFI register function with correct name format"
    );
    assert!(
        code.contains("func UnregisterOcrBackend(name string) error"),
        "unregistration function must also be generated"
    );
    assert!(
        code.contains("C.krz_unregister_ocr_backend("),
        "unregistration must call the C FFI unregister function with correct name format"
    );
}

#[test]
fn test_gen_trait_bridges_file_registration_fn_handles_c_error_response() {
    let trait_type = make_trait_type(
        "Scanner",
        vec![make_trait_method("scan", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Scanner".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_scanner".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    assert!(
        code.contains("if rc != 0"),
        "registration must check the C return code for errors"
    );
    assert!(
        code.contains("fmt.Errorf"),
        "registration must return a Go error on C failure"
    );
    assert!(
        code.contains("handle.Delete()"),
        "registration must delete the cgo.Handle on failure to avoid leaking"
    );
    assert!(
        code.contains("C.krz_free_string(cErr)"),
        "registration/unregistration must free Rust-allocated error strings with the generated FFI free function"
    );
    assert!(
        code.contains("if old, ok := reg.handles[name]; ok {\n\t\told.Delete()\n\t}"),
        "handle registry must delete any replaced handle on duplicate registration"
    );
}

// ---------------------------------------------------------------------------
// VTable struct name derivation
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_uses_correct_vtable_struct_name() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    // With crate_name="sample_crate", the VTable struct should be SAMPLE_CRATESampleCrateOcrBackendVTable
    let code = gen_trait_bridges_file(
        &api,
        &config,
        "testlib",
        "sample_crate",
        "test.h",
        "../ffi",
        "..",
        "sample_crate",
    );

    assert!(
        code.contains("vtable := &C.struct_SAMPLE_CRATESampleCrateOcrBackendVTable{"),
        "must use correct cbindgen-generated VTable struct name format: {{CRATE_UPPER}}{{CratePascal}}{{TraitPascal}}VTable"
    );
}

// ---------------------------------------------------------------------------
// CGo preamble
// ---------------------------------------------------------------------------

#[test]
fn test_gen_trait_bridges_file_cgo_preamble_forward_declares_trampolines() {
    let trait_type = make_trait_type(
        "Analyzer",
        vec![make_trait_method("analyze", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Analyzer".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_analyzer".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // CGo preamble must forward-declare all exported Go functions
    assert!(
        code.contains("extern int32_t goAnalyzerAnalyze("),
        "CGo preamble must forward-declare the analyze trampoline"
    );
    assert!(
        code.contains("import \"C\""),
        "must import C after the CGo preamble block"
    );
}

// ---------------------------------------------------------------------------
// via generate_bindings (end-to-end)
// ---------------------------------------------------------------------------

#[test]
fn test_generate_bindings_with_trait_bridge_emits_trait_bridges_go_file() {
    let backend = GoBackend;

    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config_with_bridges(vec![bridge_cfg]);
    let result = backend.generate_bindings(&api, &config);

    assert!(
        result.is_ok(),
        "generate_bindings must succeed with trait_bridges configured"
    );
    let files = result.unwrap();

    let bridge_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("trait_bridges.go"));
    assert!(
        bridge_file.is_some(),
        "generate_bindings should emit trait_bridges.go when trait_bridges are configured"
    );

    let content = &bridge_file.unwrap().content;
    assert!(
        content.contains("type OcrBackend interface"),
        "trait_bridges.go must contain the Go interface"
    );
    assert!(
        content.contains("func RegisterOcrBackend"),
        "trait_bridges.go must contain the registration function"
    );
}

/// Regression: when the same name appears as both an opaque `TypeDef` and an
/// `ErrorDef`, the structured error struct (Code/Message fields) is emitted by
/// `gen_go_error_struct` and the opaque-handle struct/Free method should be
/// suppressed. Methods on the opaque type must NOT be emitted either —
/// otherwise the codegen produces method bodies that dereference `h.ptr` on a
/// value-type struct that has no `ptr` field, which fails to compile.
#[test]
fn test_opaque_error_type_uses_value_semantics() {
    let backend = GoBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "GraphQLError".to_string(),
            rust_path: "test_lib::GraphQLError".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "status_code".to_string(),
                params: vec![],
                return_type: TypeRef::Primitive(PrimitiveType::U16),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Returns the HTTP status code.".to_string(),
                receiver: Some(alef::core::ir::ReceiverKind::Ref),
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
            doc: "GraphQL error type".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "GraphQLError".to_string(),
            rust_path: "test_lib::GraphQLError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "ValidationError".to_string(),
                fields: vec![],
                doc: "Validation failed".to_string(),
                message_template: Some("validation failed".to_string()),
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
            }],
            doc: "GraphQL error".to_string(),
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

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("generation succeeds");
    let content = &files[0].content;

    // The error struct emitted by `gen_go_error_struct` provides the Go-side
    // type. It carries Code/Message string fields and an Error() method —
    // not a ptr field.
    assert!(
        content.contains("type GraphQLError struct"),
        "value-type error struct must be emitted"
    );
    assert!(
        content.contains("Code    string") && content.contains("Message string"),
        "value-type error struct must have Code/Message fields, got:\n{}",
        content
    );
    assert!(
        content.contains("func (e GraphQLError) Error() string"),
        "value-type error must implement the error interface"
    );

    // Methods on the opaque variant must NOT be emitted — they would
    // reference `h.ptr` which does not exist on the value-type struct.
    assert!(
        !content.contains("func (h *GraphQLError) StatusCode"),
        "opaque-style method must not be generated for value-type error, got:\n{}",
        content
    );
    assert!(
        !content.contains("h.ptr"),
        "no `h.ptr` references should appear when the only opaque type is also an error type, got:\n{}",
        content
    );
}

/// Regression: a type with a `TypeRef::Bytes` return value previously emitted
/// `unmarshalBytes(ptr)` without ever defining the helper, and tried to free
/// the byte buffer via `_free_string` (which expects `*C.char`, not
/// `*C.uint8_t`). Both produced cgo compile errors. The fix emits a single
/// package-level `unmarshalBytes` helper and stops emitting `_free_string`
/// for `Bytes` returns (the FFI hands out aliasing pointers into a parent
/// handle's storage that the caller does not own).
#[test]
fn test_bytes_return_emits_helper_and_no_string_free() {
    let backend = GoBackend;

    fn make_bytes_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Bytes,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: format!("Get {}", name),
            receiver: Some(alef::core::ir::ReceiverKind::Ref),
            sanitized: false,
            returns_ref: true,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            trait_source: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "UploadFile".to_string(),
            rust_path: "test_lib::UploadFile".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("filename", TypeRef::String, false)],
            // Two bytes-returning methods on the same type — the helper must
            // still be emitted exactly once.
            methods: vec![make_bytes_method("as_bytes"), make_bytes_method("raw_content")],
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
            doc: "Upload file".to_string(),
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

    let config = make_config();
    let files = backend.generate_bindings(&api, &config).expect("generation succeeds");
    let content = &files[0].content;

    // The helper is emitted exactly once, regardless of how many bytes-returning
    // methods reference it.
    let helper_decls = content.matches("func unmarshalBytes(").count();
    assert_eq!(
        helper_decls, 1,
        "unmarshalBytes helper must be emitted exactly once per package, got {} occurrences in:\n{}",
        helper_decls, content
    );
    assert!(
        content.matches("unmarshalBytes(").count() > helper_decls,
        "bytes-returning methods must call the package-level helper, got:\n{}",
        content
    );

    // `*C.uint8_t` (the FFI return type for raw byte buffers) must not be
    // passed to `_free_string`, which expects `*C.char` and would fail to
    // compile under cgo's strict type checking.
    let bytes_method_block = content
        .split("AsBytes")
        .nth(1)
        .expect("AsBytes method must be generated");
    assert!(
        !bytes_method_block.starts_with_str_after_first("defer C.test_free_string(ptr)"),
        "bytes return must not be freed via _free_string"
    );
    // More directly: ensure no emission of `_free_string(ptr)` after a Bytes
    // method's `ptr := C.test_upload_file_as_bytes(...)` call site.
    let as_bytes_call_idx = content
        .find("C.test_upload_file_as_bytes")
        .expect("AsBytes FFI call must be present");
    let next_500 = &content[as_bytes_call_idx..(as_bytes_call_idx + 500).min(content.len())];
    assert!(
        !next_500.contains("test_free_string"),
        "no _free_string call should follow a Bytes-returning FFI call, got:\n{}",
        next_500
    );
}

// Tiny helper trait for the regression test above.
trait StartsWithStrAfterFirst {
    fn starts_with_str_after_first(&self, needle: &str) -> bool;
}
impl StartsWithStrAfterFirst for str {
    fn starts_with_str_after_first(&self, needle: &str) -> bool {
        self.lines().any(|l| l.trim_start().starts_with(needle))
    }
}

// ---------------------------------------------------------------------------
// Trait bridge typed params (regression: D1 - interface{} instead of concrete types)
// ---------------------------------------------------------------------------

#[test]
fn test_trait_bridge_string_param_emitted_as_string_not_interface() {
    // Regression test D1: path: String should emit "path string", not "path interface{}"
    let trait_type = make_trait_type(
        "Backend",
        vec![make_trait_method(
            "process_file",
            vec![make_trait_param("path", TypeRef::String)],
            TypeRef::String,
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Backend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go interface method signature should emit "path string", NOT "path interface{}"
    assert!(
        code.contains("ProcessFile(path string"),
        "String parameter must emit as 'string', not 'interface{{}}' in trait interface method\nGenerated code:\n{code}"
    );
}

#[test]
fn test_trait_bridge_named_config_param_emitted_as_concrete_type() {
    // Regression test D1: config: OcrConfig should emit "config OcrConfig", not "config map[string]interface{}"
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method(
            "process_image",
            vec![
                make_trait_param("image_bytes", TypeRef::Bytes),
                make_trait_param("config", TypeRef::Named("OcrConfig".to_string())),
            ],
            TypeRef::Named("OcrResult".to_string()),
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);

    // Add OcrConfig and OcrResult structs to the API
    let mut api = make_api_with_type(trait_type);
    api.types.push(TypeDef {
        name: "OcrConfig".to_string(),
        rust_path: "my_lib::OcrConfig".to_string(),
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
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    });
    api.types.push(TypeDef {
        name: "OcrResult".to_string(),
        rust_path: "my_lib::OcrResult".to_string(),
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
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    });

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go interface method signature should emit "config OcrConfig", NOT "config map[string]interface{}"
    assert!(
        code.contains("ProcessImage(") && code.contains("config OcrConfig"),
        "Named config parameter must emit as concrete type 'OcrConfig', not 'map[string]interface{{}}' in trait interface method\nGenerated code:\n{code}"
    );
}

#[test]
fn test_trait_bridge_enum_return_type_emitted_as_concrete_type() {
    // Regression test D1: return BackendType should emit "OcrBackendType", not "map[string]interface{}"
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method(
            "backend_type",
            vec![],
            TypeRef::Named("OcrBackendType".to_string()),
            false,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);

    // Add OcrBackendType enum to the API
    let mut api = make_api_with_type(trait_type);
    api.enums.push(EnumDef {
        name: "OcrBackendType".to_string(),
        rust_path: "my_lib::OcrBackendType".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            name: "Tesseract".to_string(),
            fields: vec![],
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            version: Default::default(),
        }],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    });

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go interface method signature should emit "OcrBackendType", NOT "map[string]interface{}"
    assert!(
        code.contains("BackendType() OcrBackendType"),
        "Named return type must emit as concrete enum type 'OcrBackendType', not 'map[string]interface{{}}' in trait interface method\nGenerated code:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Excluded-type substitution (regression: sample_crate's InternalDocument)
// ---------------------------------------------------------------------------

/// Regression: when a trait method references a type that was extracted from Rust
/// but excluded from the public binding (e.g. `#[cfg_attr(alef, alef(skip))]`),
/// the Go trait interface and trampoline must fall back to `json.RawMessage`
/// — otherwise the generated Go code refers to an undefined type and the build
/// fails with `undefined: <Name>`.
#[test]
fn test_trait_bridge_substitutes_excluded_named_types_with_json_raw_message() {
    let trait_type = make_trait_type(
        "Renderer",
        vec![make_trait_method(
            "render",
            vec![make_trait_param("doc", TypeRef::Named("InternalDocument".to_string()))],
            TypeRef::String,
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        super_trait: None,
        registry_getter: Some("get_renderer_registry".to_string()),
        register_fn: Some("register_renderer".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let mut api = make_api_with_type(trait_type);
    // Mark InternalDocument as excluded — this is what `#[cfg_attr(alef, alef(skip))]`
    // produces in the real sample_crate IR.
    api.excluded_type_paths.insert(
        "InternalDocument".to_string(),
        "sample_crate::types::internal::InternalDocument".to_string(),
    );

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // The Go trait interface and trampoline must NOT name `InternalDocument` — that
    // type was never emitted into binding.go and the build would fail with
    // `undefined: InternalDocument`.
    assert!(
        !code.contains("InternalDocument"),
        "trait_bridges.go must not reference excluded type InternalDocument\nGenerated code:\n{code}"
    );
    // The trampoline parameter declaration must use json.RawMessage instead.
    assert!(
        code.contains("json.RawMessage"),
        "expected json.RawMessage fallback for excluded named type\nGenerated code:\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Function deduplication (regression: D2 - snake_case + PascalCase duplicates)
// ---------------------------------------------------------------------------

#[test]
fn test_trait_bridge_dedup_snake_case_unregister_functions() {
    // Regression test D2: when unregister_fn is set to snake_case version of Unregister{Trait},
    // don't emit both versions — only emit the PascalCase standard function.
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method("process_image", vec![], TypeRef::String, true)],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("my_lib::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: Some("unregister_ocr_backend".to_string()), // snake_case — should NOT emit
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Must have the PascalCase function
    assert!(
        code.contains("func UnregisterOcrBackend(name string) error {"),
        "Must emit PascalCase UnregisterOcrBackend function"
    );

    // Must NOT have the snake_case duplicate
    assert!(
        !code.contains("func unregister_ocr_backend(name string) error {"),
        "Must NOT emit snake_case unregister_ocr_backend function — Go convention is PascalCase only"
    );

    // Count occurrences of unregister to ensure only one version is present
    let unregister_count = code.matches("func Unregister").count();
    assert_eq!(
        unregister_count, 1,
        "Must emit exactly one Unregister function (PascalCase), got {unregister_count}"
    );
}

// ---------------------------------------------------------------------------
// Trait-bridge config marshalling (T1.5 — avoid interface{} for typed configs)
// ---------------------------------------------------------------------------

#[test]
fn test_trait_bridge_unmarshals_config_into_concrete_type() {
    let trait_type = make_trait_type(
        "OcrBackend",
        vec![make_trait_method(
            "process_image",
            vec![
                make_trait_param("image_bytes", TypeRef::Bytes),
                make_trait_param("config", TypeRef::Named("OcrConfig".to_string())),
            ],
            TypeRef::String,
            true,
        )],
    );
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        registry_getter: Some("get_ocr_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: Vec::new(),
        ffi_skip_methods: Vec::new(),
        bind_via: alef::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    };
    let config = make_config_with_bridges(vec![bridge_cfg]);
    let api = make_api_with_type(trait_type);

    let code = gen_trait_bridges_file(&api, &config, "testlib", "krz", "test.h", "../ffi", "..", "testlib");

    // Debug: print what was generated
    eprintln!("Full generated code:\n{}", &code);
    eprintln!("---");
    if let Some(pos) = code.find("go") {
        eprintln!("Code starting at 'go': {}", &code[pos..pos.min(pos + 500)]);
    }

    // Assert: config parameter should unmarshal directly into OcrConfig, not interface{}
    assert!(
        code.contains("var goConfig OcrConfig"),
        "trampoline must declare config variable as concrete OcrConfig type"
    );
    assert!(
        code.contains("json.Unmarshal([]byte(C.GoString(config)), &goConfig)"),
        "trampoline must unmarshal directly into concrete OcrConfig type"
    );

    // Assert: the generated code should NOT contain the problematic interface{} pattern
    // for config parameter unmarshalling
    let problem_pattern = "var rawData interface{}\n\t\t\tjson.Unmarshal([]byte(C.GoString(config))";
    assert!(
        !code.contains(problem_pattern),
        "trampoline callback body must not use 'var rawData interface{{}}' for typed config params"
    );
}

// ---------------------------------------------------------------------------
// CFLAGS bundled include dir (regression: downstream go get compatibility)
// ---------------------------------------------------------------------------

#[test]
fn test_cflags_uses_bundled_include_dir() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[crates.go]
module = "github.com/example/mylib"
"#,
    );
    let api = ApiSurface {
        crate_name: "mylib".to_string(),
        version: "0.1.0".to_string(),
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
    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding_go = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();

    assert!(
        binding_go.content.contains("#cgo CFLAGS: -I${SRCDIR}/include"),
        "binding.go must use bundled include dir, not a monorepo-relative path"
    );
    assert!(
        !binding_go.content.contains("../crates/"),
        "binding.go must not contain monorepo-relative paths like ../crates/ in CFLAGS"
    );
}

// ---------------------------------------------------------------------------
// Regression: no duplicate "var raw struct" in UnmarshalJSON wrappers
// ---------------------------------------------------------------------------

#[test]
fn test_no_duplicate_var_raw_struct_in_unmarshal_json() {
    // Regression: the struct_unmarshal_json_header template outputs "var raw struct {"
    // and the gen_bindings code was also manually emitting it, causing duplicates.
    let config = make_config();

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), true),
                make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false),
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
            doc: "Configuration struct".to_string(),
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

    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding_go = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();

    // Count consecutive "var raw struct {" lines — should be exactly 0 duplicates
    let mut found_consecutive = false;
    let lines: Vec<&str> = binding_go.content.lines().collect();
    for i in 0..lines.len().saturating_sub(1) {
        let current = lines[i].trim();
        let next = lines[i + 1].trim();
        if current == "var raw struct {" && next == "var raw struct {" {
            found_consecutive = true;
            eprintln!("Found duplicate at lines {}-{}: {}", i + 1, i + 2, current);
        }
    }

    assert!(
        !found_consecutive,
        "binding.go must not contain duplicate 'var raw struct {{' lines in UnmarshalJSON"
    );
}
