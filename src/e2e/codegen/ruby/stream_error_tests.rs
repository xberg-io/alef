use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ErrorDef, ErrorVariant};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn render(declared: Option<&str>, errors: &[ErrorDef]) -> String {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "stream_error", "input": null, "assertions": [
            {"type": "error", "value": declared},
            {"type": "equals", "field": "error.status_code", "value": 401}
        ]
    }))
    .expect("fixture");
    let mut config = E2eConfig::default();
    config.call.function = "chat_stream".into();
    config.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));
    let _ = crate::e2e::codegen::take_skip_records();
    super::spec_file::render_spec_file(
        "streaming",
        &[&fixture],
        "Sample",
        None,
        "sample",
        None,
        &Default::default(),
        false,
        &config,
        false,
        false,
        &[],
        &ResolvedCrateConfig::default(),
        &[],
        errors,
        &[],
        &[],
    )
}

#[test]
fn streaming_errors_require_runtime_error_even_without_a_declared_value() {
    let source = render(None, &[]);
    assert!(
        source.contains("{ |_chunk| } }.to raise_error(RuntimeError)"),
        "{source}"
    );
    assert!(
        source.contains("has no accessor for error field error.status_code"),
        "{source}"
    );
}

#[test]
fn streaming_literal_error_messages_retain_the_escaped_predicate() {
    let source = render(Some("missing.field[0]"), &[]);
    assert!(source.contains("raise_error(RuntimeError) { |error|"), "{source}");
    assert!(source.contains(r"/missing\.field\[0\]/"), "{source}");
}

#[test]
fn streaming_known_variants_record_identity_and_status_gaps() {
    let errors = vec![ErrorDef {
        name: "ApiError".into(),
        variants: vec![ErrorVariant {
            name: "Authentication".into(),
            ..Default::default()
        }],
        rust_path: "sample::ApiError".into(),
        original_rust_path: String::new(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }];
    let source = render(Some("Authentication"), &errors);
    assert!(source.contains("raise_error(RuntimeError)"), "{source}");
    assert!(source.contains("declared error variant 'Authentication'"), "{source}");
    assert!(!source.contains("/Authentication/"), "{source}");
    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 2, "{records:?}");
}
