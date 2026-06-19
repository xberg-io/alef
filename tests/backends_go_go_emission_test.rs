use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

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
        original_type: None,
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
                    is_tuple: false,
                    message_template: None,
                    has_source: false,
                    has_from: false,
                },
                ErrorVariant {
                    name: "ParseError".to_string(),
                    fields: vec![],
                    doc: String::new(),
                    is_unit: true,
                    is_tuple: false,
                    message_template: None,
                    has_source: false,
                    has_from: false,
                },
            ],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Check that Error() uses value receiver (not pointer receiver)
    // The actual type name is TestError, not Error
    assert!(
        content.contains("func (e TestError) Error() string"),
        "Error() method should use value receiver 'func (e TestError) Error()', not pointer receiver"
    );
    assert!(
        !content.contains("func (e *TestError) Error() string"),
        "Error() method must not use pointer receiver 'func (e *TestError) Error()'"
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
                version: Default::default(),
            }],
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
    assert!(
        content.contains("func (r *ByteContainer) GetBytes() ([]byte, error)"),
        "bytes-returning methods that can fail while marshaling the receiver must return []byte, not *[]byte"
    );
    assert!(
        !content.contains("func (r *ByteContainer) GetBytes() (*[]byte, error)"),
        "bytes-returning methods must not expose *[]byte"
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
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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

/// Bug D: UnmarshalX for untagged enums must not access wire.Type on an empty struct.
///
/// Enums with `serde_untagged: true` and struct variants route to `gen_data_enum_type`.
/// The old code unconditionally emitted `switch wire.Type { ... }`, which references a
/// field that doesn't exist on the empty wire struct, causing a Go compile error:
///
///   wire.Type undefined (type struct{} has no field or method Type)
///
/// Fix: when `serde_untagged` is set, emit shape-discriminated try-each-variant
/// unmarshalling (sniff first byte, try `json.Unmarshal` into each variant in order).
///
/// This test uses struct variants (named fields) which route through `gen_data_enum_type`.
/// Enums with only tuple fields that include Vec<_> route to the separate
/// `gen_passthrough_raw_message_enum` path which is already correct.
#[test]
fn test_untagged_enum_unmarshal_does_not_access_wire_type() {
    let backend = GoBackend;
    let config = make_config();

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "InputDoc".to_string(),
            rust_path: "test_lib::InputDoc".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                // Struct variant with a single named string field
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![make_field("content", TypeRef::String, false)],
                    doc: "A plain-text document.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                // Struct variant with multiple named fields
                EnumVariant {
                    name: "Object".to_string(),
                    fields: vec![
                        make_field("title", TypeRef::String, false),
                        make_field("body", TypeRef::String, false),
                    ],
                    doc: "A structured document.".to_string(),
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
            doc: "A document for input.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: None,
            serde_untagged: true,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Must NOT have the broken switch on an empty wire struct
    assert!(
        !content.contains("switch wire.Type"),
        "Untagged enum must not emit 'switch wire.Type' (wire struct is empty)"
    );
    assert!(
        !content.contains("var wire struct {\n\t}"),
        "Untagged enum must not emit an empty wire struct"
    );

    // Must emit the shape-sniffing preamble
    assert!(
        content.contains("firstByte"),
        "Untagged enum Unmarshal must sniff the first JSON byte"
    );

    // Must try both variants
    assert!(content.contains("var v InputDocText"), "Must try InputDocText variant");
    assert!(
        content.contains("var v InputDocObject"),
        "Must try InputDocObject variant"
    );

    // Struct variants are always objects — gate on '{'
    assert!(
        content.contains("firstByte == '{'"),
        "Struct variants must be gated on firstByte == '{{'"
    );

    // Error message must mention the enum name and the raw JSON shape
    assert!(
        content.contains("unknown InputDoc shape"),
        "Error message must identify the enum and say 'shape'"
    );
}

/// Bug D (multi-variant object case): untagged enum whose all variants are struct variants
/// (named fields, no tuple) must also use shape-discriminated unmarshalling.
///
/// Mirrors the `OcrDocument` shape where multiple struct variants are tried in order.
#[test]
fn test_untagged_enum_with_object_variants_uses_shape_discriminated_unmarshal() {
    let backend = GoBackend;
    let config = make_config();

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "OcrDocument".to_string(),
            rust_path: "test_lib::OcrDocument".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                // Struct variant: OcrDocument::Source { source_path: String }
                EnumVariant {
                    name: "Source".to_string(),
                    fields: vec![make_field("source_path", TypeRef::String, false)],
                    doc: "A file path pointing to a document.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    is_tuple: false,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                // Struct variant: OcrDocument::Encoded { data: String, mime: String }
                EnumVariant {
                    name: "Encoded".to_string(),
                    fields: vec![
                        make_field("data", TypeRef::String, false),
                        make_field("mime", TypeRef::String, false),
                    ],
                    doc: "Base64-encoded document bytes.".to_string(),
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
            doc: "A document for OCR.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: None,
            serde_untagged: true,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Must NOT fall back to the broken wire.Type switch
    assert!(
        !content.contains("switch wire.Type"),
        "Untagged enum must not emit 'switch wire.Type'"
    );

    // Both struct variants are objects — must gate on '{'
    assert!(
        content.contains("firstByte == '{'"),
        "Struct variants must be gated on firstByte == '{{'"
    );

    // Must try both variants (to_go_name("Source") = "Source", "Encoded" = "Encoded")
    assert!(
        content.contains("var v OcrDocumentSource"),
        "Must try OcrDocumentSource variant"
    );
    assert!(
        content.contains("var v OcrDocumentEncoded"),
        "Must try OcrDocumentEncoded variant"
    );

    // Error message must reference shape
    assert!(
        content.contains("unknown OcrDocument shape"),
        "Error message must say 'shape' for untagged enums"
    );
}

/// Bug E (this fix): parent struct with a required data-enum field must emit
/// custom UnmarshalJSON that delegates to UnmarshalX().
///
/// Without the fix, `json.Unmarshal` tries to unmarshal directly into the
/// sealed interface type and fails at runtime:
///   json: cannot unmarshal object into Go struct field OcrRequest.document of type samplellm.OcrDocument
#[test]
fn test_parent_struct_with_required_data_enum_field_emits_custom_unmarshal_json() {
    let backend = GoBackend;
    let config = make_config();

    // Mirrors OcrDocument: internally-tagged data enum
    let ocr_document_enum = EnumDef {
        name: "OcrDocument".to_string(),
        rust_path: "test_lib::OcrDocument".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Url".to_string(),
                fields: vec![make_field("url", TypeRef::String, false)],
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
                name: "Base64".to_string(),
                fields: vec![
                    make_field("data", TypeRef::String, false),
                    make_field("mime_type", TypeRef::String, false),
                ],
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
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // OcrRequest has a required field `document: OcrDocument`
    let ocr_request_type = TypeDef {
        name: "OcrRequest".to_string(),
        rust_path: "test_lib::OcrRequest".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("model", TypeRef::String, false),
            make_field("document", TypeRef::Named("OcrDocument".to_string()), false),
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![ocr_request_type],
        functions: vec![],
        enums: vec![ocr_document_enum],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Must emit a custom UnmarshalJSON on OcrRequest
    assert!(
        content.contains("func (s *OcrRequest) UnmarshalJSON(data []byte) error"),
        "OcrRequest must have custom UnmarshalJSON; got:\n{content}"
    );

    // The helper struct must use json.RawMessage for the document field
    assert!(
        content.contains("Document json.RawMessage"),
        "document field in helper struct must be json.RawMessage; got:\n{content}"
    );

    // Must call UnmarshalOcrDocument to decode the document field
    assert!(
        content.contains("UnmarshalOcrDocument(raw.Document)"),
        "must call UnmarshalOcrDocument to decode the document field; got:\n{content}"
    );

    // Non-enum fields must be copied directly
    assert!(
        content.contains("s.Model = raw.Model"),
        "non-enum field Model must be copied directly; got:\n{content}"
    );
}

/// Bug E (optional variant): parent struct with an optional data-enum field
/// (e.g. `response_format: Option<ResponseFormat>`) must also get custom UnmarshalJSON.
///
/// The optional field is emitted as `*ResponseFormat` in Go, which is still an
/// interface pointer and equally non-unmarshalable by default json.Unmarshal.
#[test]
fn test_parent_struct_with_optional_data_enum_field_emits_custom_unmarshal_json() {
    let backend = GoBackend;
    let config = make_config();

    // Mirrors ResponseFormat: internally-tagged data enum
    let response_format_enum = EnumDef {
        name: "ResponseFormat".to_string(),
        rust_path: "test_lib::ResponseFormat".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                fields: vec![],
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
                name: "JsonObject".to_string(),
                fields: vec![],
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
                name: "JsonSchema".to_string(),
                fields: vec![make_field("json_schema", TypeRef::String, false)],
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
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // ChatRequest has an optional field `response_format: Option<ResponseFormat>`
    let chat_request_type = TypeDef {
        name: "ChatRequest".to_string(),
        rust_path: "test_lib::ChatRequest".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            make_field("model", TypeRef::String, false),
            make_field(
                "response_format",
                TypeRef::Optional(Box::new(TypeRef::Named("ResponseFormat".to_string()))),
                true,
            ),
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![chat_request_type],
        functions: vec![],
        enums: vec![response_format_enum],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|f| f.path.ends_with("binding.go")).unwrap();
    let content = &binding.content;

    // Must emit custom UnmarshalJSON on ChatRequest
    assert!(
        content.contains("func (s *ChatRequest) UnmarshalJSON(data []byte) error"),
        "ChatRequest must have custom UnmarshalJSON; got:\n{content}"
    );

    // The helper struct must use json.RawMessage for the response_format field
    assert!(
        content.contains("ResponseFormat json.RawMessage"),
        "response_format field in helper struct must be json.RawMessage; got:\n{content}"
    );

    // Must call UnmarshalResponseFormat
    assert!(
        content.contains("UnmarshalResponseFormat(raw.ResponseFormat)"),
        "must call UnmarshalResponseFormat to decode the optional field; got:\n{content}"
    );

    // Optional sealed-interface fields are stored as the bare interface (no pointer):
    // Go interfaces are already nullable, and `*Iface` is "pointer to interface", which
    // is not assignable from the interface. Assignment must be `s.X = v`, not `&v`.
    assert!(
        content.contains("s.ResponseFormat = v"),
        "optional data-enum field must be assigned as v (not &v); got:\n{content}"
    );
    assert!(
        !content.contains("s.ResponseFormat = &v"),
        "optional data-enum field must not be assigned as &v (pointer-to-interface); got:\n{content}"
    );

    // Non-enum fields must be copied directly
    assert!(
        content.contains("s.Model = raw.Model"),
        "non-enum field Model must be copied directly; got:\n{content}"
    );
}
