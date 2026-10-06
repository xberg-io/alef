//! Error introspection methods returning `Duration` (`retry_after`) across the host-language emitters.

use super::*;
use crate::codegen::error_gen::{csharp_error_bases_with_last_error_fields, csharp_last_error_getter_name};

fn error_with_duration_methods() -> ErrorDef {
    let mut error = error_with_methods();
    error.methods.push(sample_method(
        "retry_after",
        TypeRef::Optional(Box::new(TypeRef::Duration)),
    ));
    error.methods.push(sample_method("timeout", TypeRef::Duration));
    error
}

/// Regression: a `Duration` method fell through the Java type map to `String`, so the generated
/// class declared `private final String retryAfter` and assigned it `0` -- a compile error --
/// and its all-fields constructor changed signature for every existing caller. ~keep
#[test]
fn test_gen_java_error_types_maps_duration_methods_without_changing_existing_constructor() {
    let files = gen_java_error_types(
        &error_with_duration_methods(),
        "dev.sample_crate.sampleapp",
        "SampleApp",
    );
    let base = &files[0].1;
    assert!(base.contains("private final java.time.Duration retryAfter;"), "{base}");
    assert!(base.contains("private final java.time.Duration timeout;"), "{base}");
    assert!(
        base.contains("this.retryAfter = null;"),
        "an absent Option<Duration> is null: {base}"
    );
    assert!(
        base.contains("this.timeout = java.time.Duration.ZERO;"),
        "a bare Duration defaults to zero: {base}"
    );
    assert!(
        base.contains("public java.time.Duration getRetryAfter() { return this.retryAfter; }"),
        "{base}"
    );
    assert!(!base.contains("String retryAfter"), "{base}");
    assert!(!base.contains("this.retryAfter = 0"), "{base}");
    assert!(
        base.contains("public SampleAppErrorException(final String message, final int statusCode, final boolean isTransientFlag, final String errorType) {"),
        "the pre-Duration constructor must keep its exact signature: {base}"
    );
    assert!(
        base.contains("public SampleAppErrorException(final String message, final int statusCode, final boolean isTransientFlag, final String errorType, final java.time.Duration retryAfter, final java.time.Duration timeout) {"),
        "a second overload carries the durations: {base}"
    );
}

#[test]
fn test_gen_java_error_types_duration_only_error_has_a_single_all_fields_constructor() {
    let mut error = error_with_methods();
    error.methods = vec![sample_method(
        "retry_after",
        TypeRef::Optional(Box::new(TypeRef::Duration)),
    )];
    let files = gen_java_error_types(&error, "dev.sample_crate.sampleapp", "SampleApp");
    let base = &files[0].1;
    assert_eq!(
        base.matches("public SampleAppErrorException(final String message, final java.time.Duration retryAfter)")
            .count(),
        1,
        "{base}"
    );
    assert_eq!(
        base.matches("public SampleAppErrorException(final String message) {")
            .count(),
        1,
        "the message-only constructor must not be emitted twice: {base}"
    );
}

#[test]
fn test_gen_csharp_error_types_maps_duration_methods_without_changing_existing_constructors() {
    let files = gen_csharp_error_types(&error_with_duration_methods(), "SampleCrate.SampleApp", None, &[]);
    let base = &files[0].1;
    assert!(base.contains("public TimeSpan? RetryAfter { get; }"), "{base}");
    assert!(base.contains("public TimeSpan Timeout { get; }"), "{base}");
    assert!(
        base.contains("RetryAfter = null;"),
        "an absent Option<Duration> is null: {base}"
    );
    assert!(base.contains("Timeout = TimeSpan.Zero;"), "{base}");
    assert!(!base.contains("string RetryAfter"), "{base}");
    assert!(!base.contains("RetryAfter = 0;"), "{base}");
    assert!(
        base.contains("public SampleAppErrorException(string message, ushort statusCode, bool isTransientFlag, string errorType) : base(message)"),
        "the pre-Duration constructor must keep its exact signature: {base}"
    );
    assert!(
        base.contains("public SampleAppErrorException(string message, Exception innerException, ushort statusCode, bool isTransientFlag, string errorType) : base(message, innerException)"),
        "{base}"
    );
    assert!(
        base.contains("public SampleAppErrorException(string message, ushort statusCode, bool isTransientFlag, string errorType, TimeSpan? retryAfter, TimeSpan timeout) : base(message)"),
        "a second overload carries the durations: {base}"
    );
}

/// Regression: `retry_after` entered the error-method whitelist but Magnus has no implementation
/// for it, so the registration list named `ErrorInfo::retry_after` and the generated crate failed
/// to compile. Registration must cover exactly the methods the struct generator implements. ~keep
#[test]
fn test_magnus_registrations_only_name_methods_the_struct_implements() {
    let error = error_with_duration_methods();
    let struct_src = gen_magnus_error_methods_struct(&error, "sample_app");
    let registrations = magnus_error_methods_registrations(&error).join("\n");
    assert!(
        registrations.contains("magnus::method!(SampleAppErrorInfo::status_code, 0)"),
        "{registrations}"
    );
    for name in ["retry_after", "timeout"] {
        assert!(
            !registrations.contains(name),
            "{name} has no Magnus implementation: {registrations}"
        );
        assert!(!struct_src.contains(&format!("pub fn {name}(")), "{struct_src}");
    }
}

/// Regression: a method the wasm emitter does not implement used to be exported as an empty
/// `pub fn name(&self) {}`, which JS callers saw as a method that always returns `undefined`.
#[test]
fn test_wasm_error_methods_do_not_export_unimplemented_methods() {
    let output = gen_wasm_error_methods(&error_with_duration_methods(), "sample_app", "Wasm");
    assert!(output.contains("pub fn status_code(&self) -> u16"), "{output}");
    assert!(!output.contains("pub fn retry_after"), "{output}");
    assert!(!output.contains("pub fn timeout"), "{output}");
}

/// A variant class must be able to carry the native error's introspection values: the dispatch
/// that turns an FFI code into `new <Variant>Exception(...)` has nowhere to put them otherwise.
#[test]
fn test_gen_java_variant_exceptions_forward_introspection_fields_to_the_base() {
    let mut error = error_with_duration_methods();
    error.variants = sample_error().variants;
    let files = gen_java_error_types(&error, "dev.sample_crate.sampleapp", "SampleApp");
    let variant = &files[1].1;
    assert!(
        variant.contains("final String message, final int statusCode, final boolean isTransientFlag, final String errorType, final java.time.Duration retryAfter, final java.time.Duration timeout) {"),
        "{variant}"
    );
    assert!(
        variant.contains("super(message, statusCode, isTransientFlag, errorType, retryAfter, timeout);"),
        "{variant}"
    );

    let mut plain = sample_error();
    plain.methods.clear();
    let files = gen_java_error_types(&plain, "dev.sample_crate.test", "Sample");
    assert!(
        !files[1].1.contains("introspection values"),
        "no extra constructor without methods"
    );
}

fn getters_for(error: &ErrorDef) -> Vec<LastErrorField> {
    last_error_fields(std::slice::from_ref(error))
}

/// Regression: the C# introspection properties were get-only and only ever set by constructors
/// no throw site called, so `StatusCode`, `RetryAfter` and the rest were always their defaults
/// for an error thrown from a native call. They are now filled from the FFI getters. ~keep
#[test]
fn test_gen_csharp_error_types_fills_properties_from_the_native_getters() {
    let error = error_with_duration_methods();
    let files = gen_csharp_error_types(&error, "SampleCrate.SampleApp", None, &getters_for(&error));
    let base = &files[0].1;
    assert!(
        base.contains("public ushort StatusCode { get; private set; }"),
        "{base}"
    );
    assert!(base.contains("internal void ApplyLastError()"), "{base}");
    assert!(
        base.contains("StatusCode = NativeMethods.LastErrorStatusCode();"),
        "{base}"
    );
    assert!(
        base.contains("IsTransient = NativeMethods.LastErrorIsTransient();"),
        "{base}"
    );
    assert!(
        base.contains("var errorTypePtr = NativeMethods.LastErrorErrorType();"),
        "{base}"
    );
    assert!(
        base.contains("RetryAfter = retryAfterMillis < 0 ? null : TimeSpan.FromMilliseconds(retryAfterMillis);"),
        "{base}"
    );
    assert!(
        base.contains("Timeout = TimeSpan.FromMilliseconds(Math.Max(timeoutMillis, 0L));"),
        "a bare Duration cannot be null: {base}"
    );
}

#[test]
fn test_gen_csharp_error_types_leaves_unwired_methods_get_only() {
    let error = error_with_duration_methods();
    let files = gen_csharp_error_types(&error, "SampleCrate.SampleApp", None, &[]);
    let base = &files[0].1;
    assert!(base.contains("public ushort StatusCode { get; }"), "{base}");
    assert!(!base.contains("ApplyLastError"), "no getters, nothing to apply: {base}");
}

#[test]
fn test_csharp_last_error_getter_is_named_after_the_method() {
    assert_eq!(csharp_last_error_getter_name("retry_after"), "LastErrorRetryAfter");
    let error = error_with_duration_methods();
    assert_eq!(
        csharp_error_bases_with_last_error_fields(std::slice::from_ref(&error), &getters_for(&error)),
        ["SampleAppErrorException"]
    );
    assert!(csharp_error_bases_with_last_error_fields(std::slice::from_ref(&error), &[]).is_empty());
}
