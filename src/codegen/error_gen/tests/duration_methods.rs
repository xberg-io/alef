//! Error introspection methods returning `Duration` (`retry_after`) across the host-language emitters.

use super::*;

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
    let files = gen_csharp_error_types(&error_with_duration_methods(), "SampleCrate.SampleApp", None);
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
