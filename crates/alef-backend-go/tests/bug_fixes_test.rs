use alef_backend_go::GoBackend;
use alef_core::backend::Backend;
use alef_core::config::new_config::NewAlefConfig;
use alef_core::config::ResolvedCrateConfig;
use alef_core::ir::*;

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

/// Bug A: Error.Error() should use value receiver, not pointer receiver
#[test]
fn test_error_method_uses_value_receiver() {
    let backend = GoBackend;
    let config = make_config();

    // Create test API surface with one error type
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: "TestError".to_string(),
            rust_path: "test_lib::TestError".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                ErrorVariant {
                    name: "ConfigError".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_unit: true,
                    message_template: None,
                    has_source: false,
                    has_from: false,
                },
                ErrorVariant {
                    name: "ParseError".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_unit: true,
                    message_template: None,
                    has_source: false,
                    has_from: false,
                },
            ],
            doc: String::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        excluded_type_paths: std::collections::HashMap::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Check that Error() uses value receiver (not pointer receiver)
    assert!(
        content.contains("func (e Error) Error() string"),
        "Error() method should use value receiver 'func (e Error) Error()', not pointer receiver"
    );
    assert!(
        !content.contains("func (e *Error) Error() string"),
        "Error() method must not use pointer receiver 'func (e *Error) Error()'"
    );
}

/// Bug B: unmarshalBytes should return []byte, not *[]byte
#[test]
fn test_unmarshal_bytes_returns_slice_not_pointer() {
    let backend = GoBackend;
    let config = make_config();

    // Create a simple API that uses Bytes return type
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ByteContainer".to_string(),
            rust_path: "test_lib::ByteContainer".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "get_bytes".to_string(),
                receiver: Some(ReceiverKind::Ref),
                is_static: false,
                params: vec![],
                return_type: TypeRef::Bytes,
                is_async: false,
                doc: String::new(),
                error_type: None,
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                has_default_impl: false,
            }],
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Check that unmarshalBytes returns []byte, not *[]byte
    // The function signature should be:
    // func unmarshalBytes(ptr *C.uint8_t) []byte {
    assert!(
        content.contains("func unmarshalBytes(ptr *C.uint8_t) []byte"),
        "unmarshalBytes should return []byte, not *[]byte"
    );
    assert!(
        !content.contains("func unmarshalBytes(ptr *C.uint8_t) *[]byte"),
        "unmarshalBytes must not return *[]byte"
    );
}

/// Bug C: DTOs with all-zero-default fields should not emit functional-options pattern
///
/// Span has 6 uint fields all with 0 as default. The functional-options pattern
/// (NewSpan(opts ...SpanOption)) is only useful for types with non-zero defaults
/// or complex configuration needs. For pure zero-default structs, idiomatic Go
/// is to use struct literals directly: &Span{StartByte: 1, EndByte: 5}.
#[test]
fn test_zero_default_dto_skips_functional_options() {
    let backend = GoBackend;
    let config = make_config();

    // Create a DTO with all primitive fields defaulting to zero
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Span".to_string(),
            rust_path: "test_lib::Span".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("start_byte", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("end_byte", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("start_line", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("end_line", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("start_column", TypeRef::Primitive(PrimitiveType::U32), false),
                make_field("end_column", TypeRef::Primitive(PrimitiveType::U32), false),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            // This is the key: has_default = true means the struct has defaults
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "A span of bytes in source code".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Check that functional-options pattern is NOT emitted for zero-default types
    // These patterns should NOT appear:
    // - type SpanOption func(*Span)
    // - func WithSpanStartByte(...) SpanOption
    // - func NewSpan(opts ...SpanOption) *Span
    assert!(
        !content.contains("type SpanOption func(*Span)"),
        "Span should not emit SpanOption functional-options type since all fields default to zero"
    );
    assert!(
        !content.contains("func WithSpanStartByte"),
        "Span should not emit WithSpan* option functions since all fields default to zero"
    );
    assert!(
        !content.contains("func NewSpan(opts ...SpanOption)"),
        "Span should not emit NewSpan functional-options factory since all fields default to zero"
    );

    // The struct definition itself should still be present
    assert!(
        content.contains("type Span struct"),
        "Span struct definition must still be present"
    );
}
