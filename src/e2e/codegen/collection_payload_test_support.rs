use crate::core::ir::ApiSurface;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

#[path = "testdata/list_payload_shapes.rs"]
pub mod core_fixture;

pub fn api() -> ApiSurface {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(&source, include_str!("testdata/list_payload_shapes.rs")).unwrap();
    let api = crate::extract::extractor::extract(&[source.as_path()], "sample_core", "1.0.0", None).unwrap();
    assert_eq!(api.enums.len(), 2);
    assert_eq!(api.types.len(), 1);
    api
}

pub fn resolver(text_projection: bool) -> FieldResolver {
    let mut api = api();
    // The extractor of a bare fixture leaves `has_serde` unset; the real IR sets it for a
    // type that derives `Serialize`, which this fixture does.
    for definition in &mut api.enums {
        definition.has_serde = true;
    }
    build_resolver(api, text_projection)
}

/// A text-projected resolver whose union has no serde support, so the WASM binding emits no
/// lossless getter for it.
pub fn resolver_without_serde() -> FieldResolver {
    let mut api = api();
    for definition in &mut api.enums {
        definition.has_serde = false;
    }
    build_resolver(api, true)
}

fn build_resolver(api: ApiSurface, text_projection: bool) -> FieldResolver {
    let projected = if text_projection {
        vec!["Body".into()]
    } else {
        Vec::new()
    };
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(
        FieldResolver::ir_enum_fields(&api.types, &api.enums),
        Some("Reply".into()),
    )
    .with_wasm_enum_representations(&api.enums)
    .with_text_projected_enums(&projected)
    .with_wasm_text_value_fields(&api.types, &api.enums, &[])
}

pub fn assertions() -> Vec<Assertion> {
    [
        ("count_equals", "content.parts", serde_json::json!(2)),
        ("equals", "content.parts[0].text", serde_json::json!("hello")),
        ("equals", "content.parts[1].value", serde_json::json!(" world")),
    ]
    .into_iter()
    .map(|(kind, field, value)| Assertion {
        assertion_type: kind.into(),
        field: Some(field.into()),
        value: Some(value),
        ..Default::default()
    })
    .collect()
}

pub fn wire_value() -> serde_json::Value {
    assert!(
        serde_json::to_value(core_fixture::Body::Text("plain".into()))
            .unwrap()
            .is_string()
    );
    serde_json::to_value(core_fixture::reply()).unwrap()
}

pub fn nested_wire_value() -> serde_json::Value {
    serde_json::to_value(core_fixture::Reply {
        summary: "report".into(),
        content: Some(core_fixture::Body::Parts(vec![core_fixture::Piece::Group {
            items: vec!["left".into(), "right".into()],
        }])),
    })
    .unwrap()
}
