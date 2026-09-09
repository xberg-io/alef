use super::*;
use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};

fn record(name: &str, fields: Vec<(&str, TypeRef)>, opaque: bool) -> TypeDef {
    TypeDef {
        name: name.into(),
        has_serde: true,
        is_opaque: opaque,
        fields: fields
            .into_iter()
            .map(|(name, ty)| FieldDef {
                name: name.into(),
                optional: matches!(&ty, TypeRef::Optional(_)),
                ty,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn indexed_json_bridge_presentation_is_valid_and_deduplicated() {
    let types = vec![
        record(
            "Response",
            vec![("choices", TypeRef::Vec(Box::new(TypeRef::Named("Choice".into()))))],
            true,
        ),
        record("Choice", vec![("message", TypeRef::Named("Message".into()))], true),
        record(
            "Message",
            vec![(
                "tool_calls",
                TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Tool".into()))))),
            )],
            true,
        ),
        record("Tool", vec![("name", TypeRef::String)], false),
    ];
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "tool_choice", "input": null, "docs": {"topic": "chat", "shows": [
            "choices[0].message.tool_calls", "choices[0].message.tool_calls[0].name"
        ]}
    }))
    .expect("fixture");
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.overrides.insert(
        "swift".into(),
        serde_json::from_value(serde_json::json!({"result_type": "Response"})).expect("override"),
    );
    e2e.result_fields.insert("choices".into());
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..Default::default()
    };
    let rendered = render(&fixture, &e2e, &config, &types, &[]).expect("snippet");
    assert_eq!(rendered.matches("debugPrint(").count(), 1, "{rendered}");
    assert!(
        rendered.contains("result.choices()[0].message().toolCalls()"),
        "{rendered}"
    );
    assert!(!rendered.contains("result.choices().message()"), "{rendered}");
}

fn usage_snippet(streaming: bool) -> String {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "usage", "input": null, "docs": {"topic": "streaming", "shows": ["usage.total_tokens"]}
    }))
    .expect("fixture");
    let mut e2e = E2eConfig::default();
    e2e.call.function = "stream_items".into();
    e2e.call.result_var = "response".into();
    e2e.call.r#async = true;
    e2e.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(streaming));
    e2e.call.overrides.insert(
        "swift".into(),
        serde_json::from_value(serde_json::json!({"result_type": "StreamItem"})).expect("override"),
    );
    e2e.result_fields.insert("usage".into());
    e2e.fields_optional.insert("usage".into());
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![serde_json::from_value(serde_json::json!({
            "name": "stream_items", "pattern": "streaming", "core_path": "sample::stream_items", "item_type": "StreamItem"
        })).expect("adapter")],
        ..Default::default()
    };
    let types = vec![
        record(
            "StreamItem",
            vec![("usage", TypeRef::Optional(Box::new(TypeRef::Named("Usage".into()))))],
            false,
        ),
        record(
            "Usage",
            vec![("total_tokens", TypeRef::Primitive(PrimitiveType::I64))],
            false,
        ),
    ];
    render(&fixture, &e2e, &config, &types, &[]).expect("snippet")
}

#[test]
fn stream_presentation_uses_collected_items_once_with_optional_access() {
    let rendered = usage_snippet(true);
    assert_eq!(rendered.matches("for try await").count(), 1, "{rendered}");
    assert!(rendered.contains("for responseChunk in chunks {"), "{rendered}");
    assert!(
        rendered.contains("debugPrint(responseChunk.usage?.totalTokens as Any)"),
        "{rendered}"
    );
    assert!(!rendered.contains("debugPrint(response.usage"), "{rendered}");
}

#[test]
fn nonstream_presentation_keeps_the_direct_optional_result_access() {
    let rendered = usage_snippet(false);
    assert!(
        rendered.contains("debugPrint(response.usage?.totalTokens as Any)"),
        "{rendered}"
    );
    assert!(!rendered.contains("for responseChunk"), "{rendered}");
    assert!(!rendered.contains("for try await"), "{rendered}");
}
