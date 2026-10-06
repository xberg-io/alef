use super::*;

/// `error_gen` used to carry its own snake-caser that split before every uppercase letter, so
/// `GraphQLError` became `graph_q_l_error` — a third spelling that disagreed with both
/// `naming::pascal_to_snake` (used for every other C symbol component) and heck's
/// `to_snake_case`. It now delegates, so a consecutive-uppercase run stays one word.
#[test]
fn should_snake_case_acronym_runs_as_one_word() {
    assert_eq!(to_snake_case("GraphQLError"), "graph_ql_error");
    assert_eq!(to_snake_case("IOError"), "io_error");
    assert_eq!(to_snake_case("XMLHttpRequest"), "xml_http_request");
    assert_eq!(
        to_snake_case("GraphQLError"),
        crate::codegen::naming::pascal_to_snake("GraphQLError"),
        "the error-gen caser must not re-diverge from the repo's single snake-case derivation"
    );
}

/// Control: names with no consecutive-uppercase run are the shape the old and new casers
/// already agreed on, and must be spelled exactly as before.
#[test]
fn should_leave_non_acronym_names_unchanged_when_snake_casing() {
    assert_eq!(to_snake_case("ConversionError"), "conversion_error");
    assert_eq!(to_snake_case("SampleAppError"), "sample_app_error");
    assert_eq!(to_snake_case("Other"), "other");
}

/// `gen_ffi_error_codes` emits the enum entries with `to_screaming_snake` and the typedef and
/// message-function names with `to_snake_case`, so the two must split words identically or one
/// generated snippet contains two different word splits of the same type name.
#[test]
fn should_split_words_identically_in_snake_and_screaming_snake() {
    for name in ["GraphQLError", "IOError", "ConversionError", "XMLHttpRequest"] {
        assert_eq!(
            to_screaming_snake(name),
            to_snake_case(name).to_ascii_uppercase(),
            "screaming and lowercase casers disagree on `{name}`"
        );
    }
    assert_eq!(to_screaming_snake("GraphQLError"), "GRAPH_QL_ERROR");
    assert_eq!(to_screaming_snake("ConversionError"), "CONVERSION_ERROR");
}

#[test]
fn test_gen_ffi_error_codes() {
    let error = sample_error();
    let output = gen_ffi_error_codes(&error);
    assert!(output.contains("CONVERSION_ERROR_NONE = 0"));
    assert!(output.contains("CONVERSION_ERROR_PARSE_ERROR = 1"));
    assert!(output.contains("CONVERSION_ERROR_IO_ERROR = 2"));
    assert!(output.contains("CONVERSION_ERROR_OTHER = 3"));
    assert!(output.contains("conversion_error_t;"));
    assert!(output.contains("conversion_error_error_message(conversion_error_t code)"));
}

#[test]
fn test_gen_go_error_types() {
    let error = sample_error();
    let output = gen_go_error_types(&error, "mylib");
    assert!(output.contains("ErrParseError = errors.New("));
    assert!(output.contains("ErrIoError = errors.New("));
    assert!(output.contains("ErrOther = errors.New("));
    assert!(output.contains("type ConversionError struct {"));
    assert!(output.contains("Code    string"));
    assert!(output.contains("func (e *ConversionError) Error() string"));
    assert!(output.contains("// ErrParseError is returned when"));
    assert!(output.contains("// ErrIoError is returned when"));
    assert!(output.contains("// ErrOther is returned when"));
}

#[test]
fn test_gen_go_error_types_stutter_strip() {
    let error = sample_error();
    let output = gen_go_error_types(&error, "conversion");
    assert!(
        output.contains("type Error struct {"),
        "expected stutter strip, got:\n{output}"
    );
    assert!(
        output.contains("func (e *Error) Error() string"),
        "expected stutter strip, got:\n{output}"
    );
    assert!(output.contains("ErrParseError = errors.New("));
}

#[test]
fn test_gen_java_error_types() {
    let error = sample_error();
    let files = gen_java_error_types(&error, "dev.sample_crate.test", "Sample");
    assert_eq!(files.len(), 4);
    assert_eq!(files[0].0, "ConversionErrorException");
    assert!(
        files[0]
            .1
            .contains("public class ConversionErrorException extends SampleException"),
        "base exception must extend the method exception, not bare Exception: {}",
        files[0].1
    );
    assert!(files[0].1.contains("package dev.sample_crate.test;"));
    assert_eq!(files[1].0, "ParseErrorException");
    assert!(
        files[1]
            .1
            .contains("public class ParseErrorException extends ConversionErrorException")
    );
    assert_eq!(files[2].0, "IoErrorException");
    assert_eq!(files[3].0, "OtherException");
}

#[test]
fn test_gen_csharp_error_types() {
    let error = sample_error();
    let files = gen_csharp_error_types(&error, "SampleCrate.Test", None, &[]);
    assert_eq!(files.len(), 4);
    assert_eq!(files[0].0, "ConversionErrorException");
    assert!(files[0].1.contains("public class ConversionErrorException : Exception"));
    assert!(files[0].1.contains("namespace SampleCrate.Test;"));
    assert_eq!(files[1].0, "ParseErrorException");
    assert!(
        files[1]
            .1
            .contains("public class ParseErrorException : ConversionErrorException")
    );
    assert_eq!(files[2].0, "IoErrorException");
    assert_eq!(files[3].0, "OtherException");
}

#[test]
fn test_gen_csharp_error_types_with_fallback() {
    let error = sample_error();
    let files = gen_csharp_error_types(&error, "SampleCrate.Test", Some("TestLibException"), &[]);
    assert_eq!(files.len(), 4);
    assert!(
        files[0]
            .1
            .contains("public class ConversionErrorException : TestLibException")
    );
    assert!(
        files[1]
            .1
            .contains("public class ParseErrorException : ConversionErrorException")
    );
}

#[test]
fn test_python_exception_name_no_conflict() {
    assert_eq!(python_exception_name("ParseError", "ConversionError"), "ParseError");
    assert_eq!(python_exception_name("Other", "ConversionError"), "OtherError");
}

#[test]
fn test_python_exception_name_shadows_builtin() {
    assert_eq!(
        python_exception_name("Connection", "CrawlError"),
        "CrawlConnectionError"
    );
    assert_eq!(python_exception_name("Timeout", "CrawlError"), "CrawlTimeoutError");
    assert_eq!(
        python_exception_name("ConnectionError", "CrawlError"),
        "CrawlConnectionError"
    );
}

#[test]
fn test_python_exception_name_no_double_prefix() {
    assert_eq!(
        python_exception_name("CrawlConnectionError", "CrawlError"),
        "CrawlConnectionError"
    );
}

#[test]
fn test_gen_wasm_error_methods_empty_when_no_methods() {
    let error = sample_error();
    let output = gen_wasm_error_methods(&error, "sample_markup_rs", "");
    assert!(output.is_empty(), "should produce no output when methods is empty");
}

#[test]
fn test_gen_wasm_error_methods_struct_and_impl() {
    let error = error_with_methods();
    let output = gen_wasm_error_methods(&error, "sample_app", "Wasm");
    assert!(
        output.contains("pub struct WasmSampleAppError"),
        "must emit opaque struct: {output}"
    );
    assert!(
        output.contains("pub(crate) inner: sample_app::error::SampleAppError"),
        "{output}"
    );
    assert!(output.contains("#[wasm_bindgen]\nimpl WasmSampleAppError"), "{output}");
    assert!(output.contains("js_name = \"statusCode\""), "{output}");
    assert!(output.contains("pub fn status_code(&self) -> u16"), "{output}");
    assert!(output.contains("self.inner.status_code()"), "{output}");
    assert!(output.contains("js_name = \"isTransient\""), "{output}");
    assert!(output.contains("pub fn is_transient(&self) -> bool"), "{output}");
    assert!(output.contains("self.inner.is_transient()"), "{output}");
    assert!(output.contains("js_name = \"errorType\""), "{output}");
    assert!(output.contains("pub fn error_type(&self) -> String"), "{output}");
    assert!(output.contains("self.inner.error_type().to_string()"), "{output}");
}

#[test]
fn test_gen_ffi_error_methods_empty_when_no_methods() {
    let error = sample_error();
    let output = gen_ffi_error_methods(&error, "sample_markup_rs", "sample_markup");
    assert!(output.is_empty(), "should produce no output when methods is empty");
}

#[test]
fn test_gen_ffi_error_methods_status_code() {
    let error = error_with_methods();
    let output = gen_ffi_error_methods(&error, "sample_app", "sampleapp");
    assert!(
        output.contains("pub unsafe extern \"C\" fn sampleapp_sample_app_error_status_code("),
        "must emit status_code fn: {output}"
    );
    assert!(
        output.contains("err: *const sample_app::error::SampleAppError"),
        "{output}"
    );
    assert!(output.contains("-> u16"), "{output}");
    assert!(output.contains("(*err).status_code()"), "{output}");
    assert!(output.contains("if err.is_null()"), "{output}");
    assert!(output.contains("return 0;"), "{output}");
}

#[test]
fn test_gen_ffi_error_methods_is_transient() {
    let error = error_with_methods();
    let output = gen_ffi_error_methods(&error, "sample_app", "sampleapp");
    assert!(
        output.contains("pub unsafe extern \"C\" fn sampleapp_sample_app_error_is_transient("),
        "must emit is_transient fn: {output}"
    );
    assert!(output.contains("-> bool"), "{output}");
    assert!(output.contains("(*err).is_transient()"), "{output}");
    assert!(output.contains("return false;"), "{output}");
}

#[test]
fn test_gen_ffi_error_methods_error_type_with_free() {
    let error = error_with_methods();
    let output = gen_ffi_error_methods(&error, "sample_app", "sampleapp");
    assert!(
        output.contains("pub unsafe extern \"C\" fn sampleapp_sample_app_error_error_type("),
        "must emit error_type fn: {output}"
    );
    assert!(output.contains("-> *mut std::ffi::c_char"), "{output}");
    assert!(output.contains("(*err).error_type()"), "{output}");
    assert!(output.contains("CString::new(s)"), "{output}");
    assert!(output.contains(".into_raw()"), "{output}");
    assert!(output.contains("return std::ptr::null_mut();"), "{output}");
    assert!(
        output.contains("pub unsafe extern \"C\" fn sampleapp_sample_app_error_error_type_free("),
        "must emit _free companion: {output}"
    );
    assert!(output.contains("drop(std::ffi::CString::from_raw(ptr))"), "{output}");
}

#[test]
fn test_gen_ffi_error_methods_safety_comments() {
    let error = error_with_methods();
    let output = gen_ffi_error_methods(&error, "sample_app", "sampleapp");
    assert!(output.contains("// SAFETY:"), "must include SAFETY comments: {output}");
}

/// Regression test for a 0.54.0 edition-2024 sweep gap: `gen_ffi_error_methods` kept
/// emitting bare `#[no_mangle]` on its `status_code` / `is_transient` / `error_type` /
/// `error_type_free` accessors after generated crates moved to `edition = "2024"`, where
/// a bare `#[no_mangle]` on an `unsafe extern "C" fn` is rejected with
/// `error: unsafe attribute used without unsafe`. Every emitted attribute must be the
/// edition-2024-safe `#[unsafe(no_mangle)]` form.
#[test]
fn test_gen_ffi_error_methods_uses_unsafe_no_mangle() {
    let error = error_with_methods();
    let output = gen_ffi_error_methods(&error, "sample_app", "sampleapp");
    assert!(
        !output.contains("#[no_mangle]"),
        "must not emit a bare #[no_mangle] under edition 2024: {output}"
    );
    let unsafe_no_mangle_count = output.matches("#[unsafe(no_mangle)]").count();
    assert_eq!(
        unsafe_no_mangle_count, 4,
        "expected status_code, is_transient, error_type, and error_type_free to each carry \
         #[unsafe(no_mangle)]: {output}"
    );
}

/// Regression test for a third edition-2024 sweep gap: `gen_ffi_error_methods` dereferenced
/// the raw `err` pointer (and called `CString::from_raw`) directly in the body of an
/// `unsafe extern "C" fn`, relying on the enclosing `unsafe` function to cover it. Edition
/// 2024 enables `unsafe_op_in_unsafe_fn` by default, so every raw-pointer deref and
/// unsafe-fn call inside an `unsafe fn` body now needs its own explicit `unsafe {}` block,
/// or `-D warnings` CI fails with `error[E0133]`.
#[test]
fn test_gen_ffi_error_methods_wraps_derefs_in_explicit_unsafe_blocks() {
    let error = error_with_methods();
    let output = gen_ffi_error_methods(&error, "sample_app", "sampleapp");

    assert!(
        output.contains("unsafe { (*err).status_code() }"),
        "status_code accessor must wrap its raw-pointer deref in an explicit `unsafe` block: \
         {output}"
    );
    assert!(
        output.contains("unsafe { (*err).is_transient() }"),
        "is_transient accessor must wrap its raw-pointer deref in an explicit `unsafe` block: \
         {output}"
    );
    assert!(
        output.contains("unsafe { (*err).error_type() }"),
        "error_type accessor must wrap its raw-pointer deref in an explicit `unsafe` block: \
         {output}"
    );
    assert!(
        output.contains("unsafe { drop(std::ffi::CString::from_raw(ptr)) }"),
        "error_type_free must wrap its `CString::from_raw` call in an explicit `unsafe` block: \
         {output}"
    );
}
