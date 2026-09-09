use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::codegen::{E2eCodegen, python::PythonE2eCodegen};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

fn render(streaming: bool) -> String {
    let source = format!(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "chat"
module = "example_api"
streaming = {streaming}
result_var = "response"
"#
    );
    let config: NewAlefConfig = toml::from_str(&source).expect("config parses");
    let e2e: E2eConfig = config.crates[0].e2e.clone().expect("e2e config");
    let resolved: ResolvedCrateConfig = config.resolve().expect("config resolves").remove(0);
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "stream_usage", "description": "Show usage", "input": null,
        "docs": {"topic": "streaming", "shows": ["usage.total_tokens"]},
        "assertions": [{"type": "not_error"}]
    }))
    .expect("fixture parses");
    PythonE2eCodegen
        .render_snippet_body(&fixture, &e2e, &resolved, &types(), &[])
        .expect("snippet renders")
}

#[test]
fn streaming_presentation_reads_guarded_fields_from_collected_items() {
    let rendered = render(true);
    assert!(rendered.contains("async for chunk in response:"), "{rendered}");
    assert!(rendered.contains("for response_chunk in chunks:"), "{rendered}");
    assert!(rendered.contains("if response_chunk.usage is not None:"), "{rendered}");
    assert!(rendered.contains("response_chunk.usage.total_tokens"), "{rendered}");
    assert!(!rendered.contains("response.usage"), "{rendered}");
}

#[test]
fn ordinary_presentation_remains_rooted_on_the_call_result() {
    let rendered = render(false);
    assert!(rendered.contains("response.usage.total_tokens"), "{rendered}");
    assert!(!rendered.contains("response_chunk"), "{rendered}");
    assert!(!rendered.contains("for response_chunk in chunks:"), "{rendered}");
}

fn types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Chunk".into(),
            fields: vec![FieldDef {
                name: "usage".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("Usage".into()))),
                optional: true,
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "Usage".into(),
            fields: vec![FieldDef {
                name: "total_tokens".into(),
                ty: TypeRef::Primitive(PrimitiveType::U64),
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}
