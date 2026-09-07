use super::test_case::render_test_case;
use crate::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, EnumVariant};
use crate::e2e::config::{CallConfig, E2eConfig, StreamingConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn render_adapter_inferred_stream_test() -> String {
    let fixture = Fixture {
        id: "stream_pages".into(),
        description: "stream pages".into(),
        assertions: vec![Assertion {
            assertion_type: "is_true".into(),
            field: Some("stream.has_page_event".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let e2e = E2eConfig {
        call: CallConfig {
            function: "stream_pages".into(),
            module: "example".into(),
            r#async: true,
            streaming: Some(StreamingConfig::Enabled(true)),
            ..Default::default()
        },
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        adapters: vec![AdapterConfig {
            name: "stream_pages".into(),
            pattern: AdapterPattern::Streaming,
            core_path: "example::stream_pages".into(),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: None,
            item_type: Some("WorkflowEvent".into()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }],
        ..Default::default()
    };
    let enums = vec![EnumDef {
        name: "WorkflowEvent".into(),
        serde_rename_all: Some("snake_case".into()),
        variants: vec![EnumVariant {
            name: "Page".into(),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let mut output = String::new();
    render_test_case(
        &mut output,
        &fixture,
        None,
        None,
        &e2e,
        "node",
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &[],
        &enums,
        &[],
        "",
        &config,
        &mut Default::default(),
        &[],
    );
    output
}

fn tsc(required: bool, source: &str) -> std::process::Output {
    match crate::core::tool_command("tsc")
        .args([
            "--strict",
            "--noUncheckedIndexedAccess",
            "--noEmit",
            "--target",
            "ES2022",
        ])
        .arg(source)
        .output()
    {
        Ok(output) => output,
        Err(error) if required => panic!("ALEF_REQUIRE_TSC is set but tsc is unavailable: {error}"),
        Err(error) => panic!("tsc is required for this generated-code regression: {error}"),
    }
}

#[test]
fn adapter_item_type_drives_a_compiling_string_enum_event_assertion() {
    let generated = render_adapter_inferred_stream_test();
    assert!(
        generated.contains("chunks.some((event: WorkflowEvent) => event === \"page\")"),
        "adapter-inferred item type must select the string-enum representation:\n{generated}"
    );

    let directory = tempfile::tempdir().expect("temporary TypeScript project");
    let source_path = directory.path().join("generated.ts");
    let source = format!(
        "type WorkflowEvent = \"page\";\ndeclare function streamPages(): AsyncIterable<WorkflowEvent>;\ndeclare function describe(name: string, body: () => void): void;\ndeclare function it(name: string, body: () => Promise<void>, timeout?: number): void;\ndeclare function expect(value: unknown): {{ toBe(value: unknown): void; toBeDefined(): void }};\n{generated}"
    );
    std::fs::write(&source_path, source).expect("write generated TypeScript test");
    let output = tsc(
        std::env::var_os("ALEF_REQUIRE_TSC").is_some(),
        source_path.to_str().expect("UTF-8 path"),
    );
    assert!(
        output.status.success(),
        "strict TypeScript rejected the emitted test:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn discriminator_access_on_a_string_enum_is_rejected_by_tsc() {
    let directory = tempfile::tempdir().expect("temporary TypeScript project");
    let source_path = directory.path().join("sabotaged.ts");
    std::fs::write(
        &source_path,
        "type WorkflowEvent = \"page\";\nconst chunks: WorkflowEvent[] = [\"page\"];\nchunks.some((event: WorkflowEvent) => event[\"type\"] === \"Page\");\n",
    )
    .expect("write TypeScript sabotage");
    let output = tsc(
        std::env::var_os("ALEF_REQUIRE_TSC").is_some(),
        source_path.to_str().expect("UTF-8 path"),
    );
    assert!(
        !output.status.success(),
        "strict TypeScript must reject discriminator access on a string enum"
    );
}
