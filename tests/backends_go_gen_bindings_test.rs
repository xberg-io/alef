use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
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
                    cfg: None,
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
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
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
                    cfg: None,
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
                    cfg: None,
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
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
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

#[path = "backends_go_gen_bindings/regressions.rs"]
mod regressions;

#[path = "backends_go_gen_bindings/trait_bridge.rs"]
mod trait_bridge;
