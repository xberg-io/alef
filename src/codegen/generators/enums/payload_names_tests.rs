use super::{dict_getter_py_name, dict_getter_py_names, gen_pyo3_data_enum_with_mapper};
use crate::codegen::type_mapper::IdentityMapper;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn stub_content(definition: &EnumDef) -> String {
    use crate::core::backend::Backend;
    use crate::core::config::new_config::NewAlefConfig;
    let configuration: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
[crates.python]
module_name = "_sample_core"
[crates.python.stubs]
output = "stubs"
"#,
    )
    .unwrap();
    let configuration = configuration.resolve().unwrap().remove(0);
    let api = crate::core::ir::ApiSurface {
        crate_name: "sample-core".to_string(),
        enums: vec![definition.clone()],
        ..Default::default()
    };
    let files = crate::backends::pyo3::Pyo3Backend
        .generate_type_stubs(&api, &configuration)
        .unwrap();
    assert_eq!(files.len(), 1, "the configured native stub sink must emit one file");
    assert_eq!(files[0].path, std::path::Path::new("stubs/_sample_core.pyi"));
    files.into_iter().next().unwrap().content
}

fn data_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "value".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn payload_getters_avoid_sibling_factories_and_other_payload_getters() {
    for (names, expected) in [
        (vec!["Text", "TextPayload"], "text_payload_2"),
        (vec!["Text", "TextPayload", "TextPayload_2"], "text_payload_3"),
    ] {
        let definition = EnumDef {
            name: "SamplePart".to_string(),
            variants: names.into_iter().map(data_variant).collect(),
            serde_tag: Some("kind".to_string()),
            has_serde: true,
            ..Default::default()
        };
        let getter = dict_getter_py_name(&definition, &definition.variants[0]);
        assert_eq!(getter, expected);
        let generated = gen_pyo3_data_enum_with_mapper(&definition, "sample_core", Some(&IdentityMapper));
        assert!(
            generated.contains(&format!("fn {getter}(&self, py: Python<'_>)")),
            "{generated}"
        );
        assert!(generated.contains("#[pyo3(name = \"text_payload\")]"), "{generated}");
        let stub = stub_content(&definition);
        assert!(stub.contains(&format!("{getter}: Any | None")), "{stub}");
    }
}

#[test]
fn unit_payload_getters_share_the_legal_rust_name_with_the_stub() {
    for (name, expected) in [
        ("Self", "self_"),
        ("Crate", "crate_"),
        ("Super", "super_"),
        ("Class", "class_"),
        ("Type", "type_"),
    ] {
        let variant = EnumVariant {
            name: name.to_string(),
            ..Default::default()
        };
        let definition = EnumDef {
            name: "SamplePart".to_string(),
            variants: vec![variant, data_variant("Value")],
            serde_tag: Some("kind".to_string()),
            has_serde: true,
            ..Default::default()
        };
        assert_eq!(dict_getter_py_name(&definition, &definition.variants[0]), expected);
        let generated = gen_pyo3_data_enum_with_mapper(&definition, "sample_core", Some(&IdentityMapper));
        assert!(
            generated.contains(&format!("fn {expected}(&self, py: Python<'_>)")),
            "{generated}"
        );
        let stub = stub_content(&definition);
        assert!(stub.contains(&format!("{expected}: Any | None")), "{stub}");
    }
}

#[test]
fn typed_tuple_getters_share_collision_allocations_with_the_stub() {
    let variants = ["Text", "FromText"]
        .into_iter()
        .map(|name| EnumVariant {
            name: name.to_string(),
            is_tuple: true,
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::Named("SampleValue".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .collect();
    let definition = EnumDef {
        name: "SamplePart".to_string(),
        variants,
        has_serde: true,
        ..Default::default()
    };
    let getter = dict_getter_py_name(&definition, &definition.variants[1]);
    assert_eq!(getter, "from_text_2");
    let generated = gen_pyo3_data_enum_with_mapper(&definition, "sample_core", Some(&IdentityMapper));
    assert!(
        generated.contains("fn from_text_2(&self) -> Option<SampleValue>"),
        "{generated}"
    );
    let stub = stub_content(&definition);
    assert!(stub.contains("from_text_2: SampleValue | None"), "{stub}");
}

fn naming_definition(names: &[&str]) -> EnumDef {
    EnumDef {
        name: "SamplePart".to_string(),
        variants: names.iter().map(|name| data_variant(name)).collect(),
        serde_tag: Some("kind".to_string()),
        has_serde: true,
        ..Default::default()
    }
}

#[test]
fn ordered_payload_names_preserve_collision_suffixes_and_variant_order() {
    for (variants, expected) in [
        (
            vec!["Text", "TextPayload", "TextPayload_2"],
            vec!["text_payload_3", "text_payload_payload", "text_payload_2_payload"],
        ),
        (
            vec!["TextPayload_2", "Text", "TextPayload"],
            vec!["text_payload_2_payload", "text_payload_3", "text_payload_payload"],
        ),
        (
            vec!["Text", "Text", "TextPayload"],
            vec!["text_payload_2", "text_payload_2", "text_payload_payload"],
        ),
    ] {
        let definition = naming_definition(&variants);
        let names: Vec<_> = definition
            .variants
            .iter()
            .map(|variant| dict_getter_py_name(&definition, variant))
            .collect();
        assert_eq!(names, expected);
        assert_eq!(dict_getter_py_names(&definition), expected);
    }
}

#[test]
fn payload_names_preserve_absent_variant_fallback_after_all_allocations() {
    let definition = naming_definition(&["Text", "Text", "TextPayload"]);
    assert_eq!(dict_getter_py_name(&definition, &data_variant("Missing")), "missing");
    assert_eq!(
        dict_getter_py_name(&definition, &data_variant("TextPayload_2")),
        "text_payload_2_2"
    );
    assert_eq!(
        dict_getter_py_name(&naming_definition(&[]), &data_variant("Type")),
        "type_"
    );
}
