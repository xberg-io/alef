use super::*;

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
    assert!(output.contains("func (e ConversionError) Error() string"));
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
        output.contains("func (e Error) Error() string"),
        "expected stutter strip, got:\n{output}"
    );
    assert!(output.contains("ErrParseError = errors.New("));
}

#[test]
fn test_gen_java_error_types() {
    let error = sample_error();
    let files = gen_java_error_types(&error, "dev.sample_crate.test");
    assert_eq!(files.len(), 4);
    assert_eq!(files[0].0, "ConversionErrorException");
    assert!(
        files[0]
            .1
            .contains("public class ConversionErrorException extends Exception")
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
    let files = gen_csharp_error_types(&error, "SampleCrate.Test", None);
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
    let files = gen_csharp_error_types(&error, "SampleCrate.Test", Some("TestLibException"));
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
