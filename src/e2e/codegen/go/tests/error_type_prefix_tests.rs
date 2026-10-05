//! Regression coverage for the Go docs-snippet generator's typed-error import.
//!
//! Split out as a new file rather than added to `snippet.rs` or `tests.rs`: both are already at
//! their `tests/file_size_ratchet.rs` baseline ceiling and must not grow.
//!
//! `[crate] error_type = "SampleCrateError"` names the *Rust-side* error type
//! (`ResolvedCrateConfig::error_type_name()`), the same value `error_constructor_expr` uses to
//! build `Crate::Error::from(msg)`. It is not what the Go backend actually exports: the Go error
//! generator (`gen_go_error_struct`, via `crate::codegen::naming::go_error_type_name`) strips a
//! leading case-insensitive match of the Go package name from that Rust name to avoid revive's
//! stutter lint, so a crate whose package is `samplecrate` and whose Rust error type is
//! `SampleCrateError` is exported as `samplecrate.Error`, not `samplecrate.SampleCrateError`.
//! The snippet generator used to reference the raw config value verbatim, so every published
//! snippet asserting a typed error failed to compile against the real binding with
//! `undefined: samplecrate.SampleCrateError`. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::go::snippet::render_snippet_body;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture};

fn error_fixture() -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "invalid_input".to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: vec![Assertion {
            assertion_type: "error".to_string(),
            ..Default::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn error_e2e_config() -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.module = "example.com/sample".into();
    e2e.call.returns_result = true;
    e2e
}

/// The real-world shape: a crate whose Go package is `samplecrate` and whose configured
/// `[crate] error_type` is `SampleCrateError` must reference the stripped Go name
/// `samplecrate.Error`, not the raw Rust name `samplecrate.SampleCrateError` the Go binding
/// never declares.
#[test]
fn snippet_typed_error_strips_the_package_prefix_like_the_go_backend_does() {
    let config = ResolvedCrateConfig {
        name: "samplecrate".into(),
        error_type: Some("SampleCrateError".into()),
        ..Default::default()
    };

    let body =
        render_snippet_body(&error_fixture(), &error_e2e_config(), &config, &[], &[], &[]).expect("snippet renders");

    assert!(body.contains("var typedError *pkg.Error"), "{body}");
    assert!(!body.contains("pkg.SampleCrateError"), "{body}");
}

/// Negative control: when the configured error type name shares no prefix with the Go package
/// name, nothing is stripped and the snippet must still reference it verbatim. This must keep
/// passing -- it pins that the fix is a targeted prefix-strip, not a blanket rename to `Error`.
#[test]
fn snippet_typed_error_keeps_an_unrelated_name_untouched() {
    let config = ResolvedCrateConfig {
        name: "converter".into(),
        error_type: Some("ConversionError".into()),
        ..Default::default()
    };

    let body =
        render_snippet_body(&error_fixture(), &error_e2e_config(), &config, &[], &[], &[]).expect("snippet renders");

    assert!(body.contains("var typedError *pkg.ConversionError"), "{body}");
}
