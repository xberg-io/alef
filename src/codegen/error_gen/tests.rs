use ahash::AHashSet;

use super::shared::{to_screaming_snake, variant_display_message};
use super::*;
use crate::core::ir::{ErrorDef, ErrorVariant};

use crate::core::ir::{CoreWrapper, FieldDef, TypeRef};

#[path = "tests/native_methods.rs"]
mod native_methods;

/// Helper to create a tuple-style field (e.g. `_0: String`).
fn tuple_field(index: usize) -> FieldDef {
    FieldDef {
        name: format!("_{index}"),
        ty: TypeRef::String,
        optional: false,
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

/// Helper to create a named struct field.
fn named_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        optional: false,
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

fn sample_error() -> ErrorDef {
    ErrorDef {
        name: "ConversionError".to_string(),
        rust_path: "sample_markup_rs::ConversionError".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "ParseError".to_string(),
                message_template: Some("HTML parsing error: {0}".to_string()),
                fields: vec![tuple_field(0)],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "IoError".to_string(),
                message_template: Some("I/O error: {0}".to_string()),
                fields: vec![tuple_field(0)],
                has_source: false,
                has_from: true,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "Other".to_string(),
                message_template: Some("Conversion error: {0}".to_string()),
                fields: vec![tuple_field(0)],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
        ],
        doc: "Error type for conversion operations.".to_string(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn sample_method(name: &str, return_type: TypeRef) -> crate::core::ir::MethodDef {
    crate::core::ir::MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn error_with_methods() -> ErrorDef {
    ErrorDef {
        name: "SampleAppError".to_string(),
        rust_path: "sample_app::error::SampleAppError".to_string(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: String::new(),
        methods: vec![
            sample_method("status_code", TypeRef::Primitive(crate::core::ir::PrimitiveType::U16)),
            sample_method("is_transient", TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)),
            sample_method("error_type", TypeRef::String),
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn test_gen_error_types() {
    let error = sample_error();
    let output = gen_pyo3_error_types(&error, "_module", &mut AHashSet::new());
    assert!(output.contains("pyo3::create_exception!(_module, ConversionError, pyo3::exceptions::PyException);"));
    assert!(output.contains("pyo3::create_exception!(_module, ParseError, ConversionError);"));
    assert!(output.contains("pyo3::create_exception!(_module, IoError, ConversionError);"));
    assert!(output.contains("pyo3::create_exception!(_module, OtherError, ConversionError);"));
}

#[test]
fn test_gen_error_converter() {
    let error = sample_error();
    let output = gen_pyo3_error_converter(&error, "sample_markup_rs");
    assert!(output.contains("fn conversion_error_to_py_err(e: sample_markup_rs::ConversionError) -> pyo3::PyErr {"));
    assert!(output.contains("sample_markup_rs::ConversionError::ParseError(..) => ParseError::new_err(msg),"));
    assert!(output.contains("sample_markup_rs::ConversionError::IoError(..) => IoError::new_err(msg),"));
}

#[test]
fn test_gen_error_registration() {
    let error = sample_error();
    let regs = gen_pyo3_error_registration(&error, &mut AHashSet::new());
    assert_eq!(regs.len(), 4);
    assert!(regs[0].contains("\"ParseError\""));
    assert!(regs[3].contains("\"ConversionError\""));
}

#[test]
fn test_unit_variant_pattern() {
    let error = ErrorDef {
        name: "MyError".to_string(),
        rust_path: "my_crate::MyError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "NotFound".to_string(),
            message_template: Some("not found".to_string()),
            fields: vec![],
            has_source: false,
            has_from: false,
            is_unit: true,
            is_tuple: false,
            doc: String::new(),
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let output = gen_pyo3_error_converter(&error, "my_crate");
    assert!(output.contains("my_crate::MyError::NotFound => NotFoundError::new_err(msg),"));
    assert!(!output.contains("NotFound(..)"));
}

#[test]
fn test_struct_variant_pattern() {
    let error = ErrorDef {
        name: "MyError".to_string(),
        rust_path: "my_crate::MyError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "Parsing".to_string(),
            message_template: Some("parsing error: {message}".to_string()),
            fields: vec![named_field("message")],
            has_source: false,
            has_from: false,
            is_unit: false,
            is_tuple: false,
            doc: String::new(),
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let output = gen_pyo3_error_converter(&error, "my_crate");
    assert!(
        output.contains("my_crate::MyError::Parsing { .. } => ParsingError::new_err(msg),"),
        "Struct variants must use {{ .. }} pattern, got:\n{output}"
    );
    assert!(!output.contains("Parsing(..)"));
}

#[test]
fn test_gen_napi_error_types() {
    let error = sample_error();
    let output = gen_napi_error_types(&error);
    assert!(output.contains("CONVERSION_ERROR_ERROR_PARSE_ERROR"));
    assert!(output.contains("CONVERSION_ERROR_ERROR_IO_ERROR"));
    assert!(output.contains("CONVERSION_ERROR_ERROR_OTHER"));
}

#[test]
fn test_gen_napi_error_converter() {
    let error = sample_error();
    let output = gen_napi_error_converter(&error, "sample_markup_rs");
    assert!(output.contains("fn conversion_error_to_napi_err(e: sample_markup_rs::ConversionError) -> napi::Error {"));
    assert!(output.contains("napi::Error::new(napi::Status::GenericFailure,"));
    assert!(output.contains("[ParseError]"));
    assert!(output.contains("[IoError]"));
    assert!(output.contains("#[allow(dead_code)]"));
}

#[test]
fn test_napi_unit_variant() {
    let error = ErrorDef {
        name: "MyError".to_string(),
        rust_path: "my_crate::MyError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "NotFound".to_string(),
            message_template: None,
            fields: vec![],
            has_source: false,
            has_from: false,
            is_unit: true,
            is_tuple: false,
            doc: String::new(),
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let output = gen_napi_error_converter(&error, "my_crate");
    assert!(output.contains("my_crate::MyError::NotFound =>"));
    assert!(!output.contains("NotFound(..)"));
}

#[test]
fn test_gen_wasm_error_converter() {
    let error = sample_error();
    let output = gen_wasm_error_converter(&error, "sample_markup_rs", &[]);
    assert!(
        output.contains(
            "fn conversion_error_to_js_value(e: sample_markup_rs::ConversionError) -> wasm_bindgen::JsValue {"
        )
    );
    assert!(output.contains("js_sys::Object::new()"));
    assert!(output.contains("js_sys::Reflect::set(&obj, &\"code\".into(), &code.into()).ok()"));
    assert!(output.contains("js_sys::Reflect::set(&obj, &\"message\".into(), &message.into()).ok()"));
    assert!(output.contains("obj.into()"));
    assert!(output.contains("fn conversion_error_error_code(e: &sample_markup_rs::ConversionError) -> &'static str {"));
    assert!(output.contains("\"parse_error\""));
    assert!(output.contains("\"io_error\""));
    assert!(output.contains("\"other\""));
    assert!(output.contains("#[allow(dead_code)]"));
}

#[test]
fn test_gen_php_error_converter() {
    let error = sample_error();
    let output = gen_php_error_converter(&error, "sample_markup_rs");
    assert!(output.contains(
        "fn conversion_error_to_php_err(e: sample_markup_rs::ConversionError) -> ext_php_rs::exception::PhpException {"
    ));
    assert!(output.contains("PhpException::default(format!(\"[ParseError] {}\", msg))"));
    assert!(output.contains("#[allow(dead_code)]"));
}

#[test]
fn test_gen_magnus_error_converter() {
    let error = sample_error();
    let output = gen_magnus_error_converter(&error, "sample_markup_rs");
    assert!(
        output.contains("fn conversion_error_to_magnus_err(e: sample_markup_rs::ConversionError) -> magnus::Error {")
    );
    assert!(
        output.contains("magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), msg)")
    );
    assert!(output.contains("#[allow(dead_code)]"));
}

#[test]
fn test_gen_rustler_error_converter() {
    let error = sample_error();
    let output = gen_rustler_error_converter(&error, "sample_markup_rs");
    assert!(output.contains("fn conversion_error_to_rustler_err(e: sample_markup_rs::ConversionError) -> String {"));
    assert!(output.contains("e.to_string()"));
    assert!(output.contains("#[allow(dead_code)]"));
}

#[test]
fn test_gen_go_error_struct_with_methods() {
    let error = error_with_methods();
    let output = gen_go_error_struct(&error, "sampleapp");
    assert!(output.contains("type Error struct {"), "struct def: {output}");
    assert!(output.contains("StatusCode uint16"), "StatusCode field: {output}");
    assert!(output.contains("IsTransient bool"), "IsTransient field: {output}");
    assert!(output.contains("ErrorType string"), "ErrorType field: {output}");
    assert!(
        !output.contains("func (e Error) StatusCode()"),
        "no StatusCode accessor: {output}"
    );
    assert!(
        !output.contains("func (e Error) IsTransient()"),
        "no IsTransient accessor: {output}"
    );
    assert!(
        !output.contains("func (e Error) ErrorType()"),
        "no ErrorType accessor: {output}"
    );
}

#[test]
fn test_gen_go_error_struct_no_field_method_collision() {
    use crate::core::ir::{ErrorDef, ErrorVariant, PrimitiveType, TypeRef};
    let error = ErrorDef {
        name: "ApiError".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        doc: String::new(),
        variants: vec![ErrorVariant {
            name: "Network".to_string(),
            message_template: None,
            fields: vec![],
            has_source: false,
            has_from: false,
            is_unit: true,
            is_tuple: false,
            doc: String::new(),
        }],
        methods: vec![
            sample_method("retry_count", TypeRef::Primitive(PrimitiveType::U32)),
            sample_method("permanent", TypeRef::Primitive(PrimitiveType::Bool)),
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let output = gen_go_error_struct(&error, "mypkg");
    assert!(output.contains("RetryCount uint32"), "RetryCount field: {output}");
    assert!(output.contains("Permanent bool"), "Permanent field: {output}");
    assert!(
        !output.contains("func (e ApiError) RetryCount()"),
        "no RetryCount accessor: {output}"
    );
    assert!(
        !output.contains("func (e ApiError) Permanent()"),
        "no Permanent accessor: {output}"
    );
}

#[test]
fn test_gen_go_error_struct_no_methods() {
    let error = sample_error();
    let output = gen_go_error_struct(&error, "mylib");
    assert!(output.contains("type ConversionError struct {"), "{output}");
    assert!(!output.contains("StatusCode"), "{output}");
    assert!(!output.contains("IsTransient"), "{output}");
}

#[test]
fn test_gen_java_error_types_with_methods() {
    let error = error_with_methods();
    let files = gen_java_error_types(&error, "dev.sample_crate.sampleapp");
    assert_eq!(files.len(), 1);
    let base = &files[0].1;
    assert!(
        base.contains("private final int statusCode;"),
        "statusCode field: {base}"
    );
    assert!(
        base.contains("private final boolean isTransientFlag;"),
        "isTransientFlag field: {base}"
    );
    assert!(
        base.contains("private final String errorType;"),
        "errorType field: {base}"
    );
    assert!(
        base.contains("public int getStatusCode()"),
        "getStatusCode getter: {base}"
    );
    assert!(
        base.contains("public boolean isTransient()"),
        "isTransient getter: {base}"
    );
    assert!(
        base.contains("public String getErrorType()"),
        "getErrorType getter: {base}"
    );
    assert!(
        base.contains("public SampleAppErrorException(final String message)"),
        "simple ctor: {base}"
    );
    assert!(
            base.contains("public SampleAppErrorException(final String message, final int statusCode, final boolean isTransientFlag, final String errorType)"),
            "full ctor: {base}"
        );
}

#[test]
fn test_gen_java_error_types_no_methods() {
    let error = sample_error();
    let files = gen_java_error_types(&error, "dev.sample_crate.test");
    let base = &files[0].1;
    assert!(!base.contains("private final"), "no fields when no methods: {base}");
    assert!(
        base.contains("public ConversionErrorException(final String message)"),
        "{base}"
    );
}

#[test]
fn test_gen_csharp_error_types_with_methods() {
    let error = error_with_methods();
    let files = gen_csharp_error_types(&error, "SampleCrate.SampleApp", None);
    assert_eq!(files.len(), 1);
    let base = &files[0].1;
    assert!(
        base.contains("public ushort StatusCode { get; }"),
        "StatusCode prop: {base}"
    );
    assert!(
        base.contains("public bool IsTransient { get; }"),
        "IsTransient prop: {base}"
    );
    assert!(
        base.contains("public string ErrorType { get; }"),
        "ErrorType prop: {base}"
    );
    assert!(
        base.contains("public SampleAppErrorException(string message) : base(message)"),
        "simple ctor: {base}"
    );
    assert!(
            base.contains("public SampleAppErrorException(string message, ushort statusCode, bool isTransientFlag, string errorType) : base(message)"),
            "full ctor: {base}"
        );
}

#[test]
fn test_gen_csharp_error_types_no_methods() {
    let error = sample_error();
    let files = gen_csharp_error_types(&error, "SampleCrate.Test", None);
    let base = &files[0].1;
    assert!(!base.contains("{ get; }"), "no properties when no methods: {base}");
    assert!(
        base.contains("public ConversionErrorException(string message) : base(message) { }"),
        "{base}"
    );
}

/// Regression: the GraphQLErrorException base doc previously leaked raw rustdoc
/// (`# Examples` heading, ```ignore code fence containing `Self::error_code`,
/// `Result<T, E>`, intra-doc links) into the `<summary>` element, causing
/// CS1002/CS1519 Roslyn errors. The sanitizer must strip all of that.
#[test]
fn test_gen_csharp_error_types_strips_rust_idioms_in_doc() {
    let mut error = error_with_methods();
    error.name = "GraphQLError".to_string();
    error.doc = "Errors that can occur during GraphQL operations\n\n\
            These errors are compatible with async-graphql error handling.\n"
        .to_string();
    error.methods[0].doc = "Convert error to HTTP status code\n\n\
            Public alias for the same codes returned by [`Self::error_code`].\n\n\
            # Examples\n\n\
            ```ignore\n\
            use sample_router_graphql::error::GraphQLError;\n\
            let error = GraphQLError::AuthenticationError(\"Invalid token\".to_string());\n\
            assert_eq!(error.status_code(), 401);\n\
            ```\n"
        .to_string();
    let files = gen_csharp_error_types(&error, "SampleRouter", None);
    let base = &files[0].1;
    assert!(
        !base.contains("```"),
        "code fence markers must not leak into <summary>: {base}"
    );
    assert!(!base.contains("# Examples"), "section heading must be stripped: {base}");
    assert!(
        !base.contains("Self::error_code"),
        "Self::method must be normalised: {base}"
    );
    assert!(!base.contains("[`"), "intra-doc link brackets must be stripped: {base}");
    assert!(
        !base.contains("GraphQLError::AuthenticationError"),
        "rust path inside fence must be dropped: {base}"
    );
    assert!(
        base.contains("Convert error to HTTP status code"),
        "first prose line survives: {base}"
    );
    assert!(
        base.contains("Errors that can occur during GraphQL operations"),
        "base error prose survives: {base}"
    );
}

#[test]
fn test_to_screaming_snake() {
    assert_eq!(to_screaming_snake("ConversionError"), "CONVERSION_ERROR");
    assert_eq!(to_screaming_snake("IoError"), "IO_ERROR");
    assert_eq!(to_screaming_snake("Other"), "OTHER");
}

#[test]
fn test_strip_thiserror_placeholders_struct_field() {
    assert_eq!(strip_thiserror_placeholders("OCR error: {message}"), "OCR error");
    assert_eq!(
        strip_thiserror_placeholders("plugin error in '{plugin_name}': {message}"),
        "plugin error in"
    );
    let result = strip_thiserror_placeholders("extraction timed out after {elapsed_ms}ms (limit: {limit_ms}ms)");
    assert!(!result.contains('{'), "no braces: {result}");
    assert!(!result.contains('}'), "no braces: {result}");
    assert!(result.starts_with("extraction timed out after"), "{result}");
}

#[test]
fn test_strip_thiserror_placeholders_positional() {
    assert_eq!(strip_thiserror_placeholders("I/O error: {0}"), "I/O error");
    assert_eq!(strip_thiserror_placeholders("Parse error: {0}"), "Parse error");
}

#[test]
fn test_strip_thiserror_placeholders_no_placeholder() {
    assert_eq!(strip_thiserror_placeholders("not found"), "not found");
    assert_eq!(strip_thiserror_placeholders("lock poisoned"), "lock poisoned");
}

#[test]
fn test_acronym_aware_snake_phrase_recognizes_acronyms() {
    assert_eq!(acronym_aware_snake_phrase("IoError"), "IO error");
    assert_eq!(acronym_aware_snake_phrase("OcrError"), "OCR error");
    assert_eq!(acronym_aware_snake_phrase("PdfParse"), "PDF parse");
    assert_eq!(acronym_aware_snake_phrase("HttpRequestFailed"), "HTTP request failed");
    assert_eq!(acronym_aware_snake_phrase("UrlInvalid"), "URL invalid");
}

#[test]
fn test_acronym_aware_snake_phrase_plain_words() {
    assert_eq!(acronym_aware_snake_phrase("Other"), "other");
    assert_eq!(acronym_aware_snake_phrase("ParseError"), "parse error");
    assert_eq!(acronym_aware_snake_phrase("LockPoisoned"), "lock poisoned");
}

#[test]
fn test_variant_display_message_acronym_first_word() {
    let variant = ErrorVariant {
        name: "Io".to_string(),
        message_template: Some("I/O error: {0}".to_string()),
        fields: vec![tuple_field(0)],
        has_source: false,
        has_from: false,
        is_unit: false,
        is_tuple: false,
        doc: String::new(),
    };
    let msg = variant_display_message(&variant);
    assert!(!msg.contains('{'), "no placeholders allowed: {msg}");
}

#[test]
fn test_variant_display_message_no_template_uses_acronyms() {
    let variant = ErrorVariant {
        name: "IoError".to_string(),
        message_template: None,
        fields: vec![],
        has_source: false,
        has_from: false,
        is_unit: false,
        is_tuple: false,
        doc: String::new(),
    };
    assert_eq!(variant_display_message(&variant), "IO error");
}

#[test]
fn test_variant_display_message_struct_template_no_leak() {
    let variant = ErrorVariant {
        name: "Ocr".to_string(),
        message_template: Some("OCR error: {message}".to_string()),
        fields: vec![named_field("message")],
        has_source: false,
        has_from: false,
        is_unit: false,
        is_tuple: false,
        doc: String::new(),
    };
    let msg = variant_display_message(&variant);
    assert_eq!(msg, "OCR error", "must not leak {{message}} placeholder: {msg}");
}

#[test]
fn test_go_sentinels_no_placeholder_leak() {
    let error = ErrorDef {
        name: "SampleCrateError".to_string(),
        rust_path: "sample_crate::SampleCrateError".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            ErrorVariant {
                name: "Io".to_string(),
                message_template: Some("IO error: {message}".to_string()),
                fields: vec![named_field("message")],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "Ocr".to_string(),
                message_template: Some("OCR error: {message}".to_string()),
                fields: vec![named_field("message")],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
            ErrorVariant {
                name: "Timeout".to_string(),
                message_template: Some("extraction timed out after {elapsed_ms}ms (limit: {limit_ms}ms)".to_string()),
                fields: vec![named_field("elapsed_ms"), named_field("limit_ms")],
                has_source: false,
                has_from: false,
                is_unit: false,
                is_tuple: false,
                doc: String::new(),
            },
        ],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let output = gen_go_sentinel_errors(std::slice::from_ref(&error));
    assert!(
        !output.contains('{'),
        "Go sentinels must not contain raw placeholders:\n{output}"
    );
    assert!(
        output.contains("ErrIo = errors.New(\"IO error\")"),
        "expected acronym-preserving Io sentinel, got:\n{output}"
    );
    assert!(
        output.contains("var (\n\t// ErrIo is returned when IO error.\n\tErrIo = errors.New(\"IO error\")\n"),
        "Go sentinel comments must be emitted on separate lines, got:\n{output}"
    );
    assert!(
        output.contains("ErrOcr = errors.New(\"OCR error\")"),
        "expected acronym-preserving Ocr sentinel, got:\n{output}"
    );
    assert!(
        output.contains("ErrTimeout = errors.New(\"extraction timed out after"),
        "expected timeout sentinel to start with the prose, got:\n{output}"
    );
}
