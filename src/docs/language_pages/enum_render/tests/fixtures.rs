use crate::core::ir::{ApiSurface, DeprecationInfo, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::docs::test_helpers::{make_field, make_minimal_api};

pub(super) struct Fixture {
    pub name: &'static str,
    pub api: ApiSurface,
}

pub(super) fn fixtures() -> [Fixture; 3] {
    [normal(), tagged(), untagged()]
}

fn fixture(name: &'static str, en: EnumDef, types: Vec<TypeDef>) -> Fixture {
    let mut api = make_minimal_api("1.2.3");
    api.enums = vec![en];
    api.types = types;
    Fixture { name, api }
}

fn normal() -> Fixture {
    let mut en = EnumDef {
        name: "RenderMode".into(),
        rust_path: "mylib::RenderMode".into(),
        has_serde: true,
        doc: "# Choices\n\nA [mode] with `code|value`.".into(),
        variants: vec![
            documented_variant(),
            deprecated_variant("SinceOnly", Some("2.4.9"), None),
            deprecated_variant("NoteOnly", None, Some("Use [Next] | `replacement`")),
            deprecated_variant("BareDeprecated", None, None),
            EnumVariant {
                name: "GeneratedDescription".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    en.version.since = Some("1.2.9".into());
    fixture("normal", en, vec![])
}

fn documented_variant() -> EnumVariant {
    let mut variant = deprecated_variant("PreferContent", Some("2.3.8"), Some("Prefer `next|mode` [now]"));
    variant.doc = "Explicit [description] | `code|span`\nsecond line".into();
    variant.version.since = Some("1.7.9".into());
    variant.fields = vec![visible_field(), excluded_field()];
    variant
}

fn deprecated_variant(name: &str, since: Option<&str>, note: Option<&str>) -> EnumVariant {
    let mut variant = EnumVariant {
        name: name.into(),
        ..Default::default()
    };
    variant.version.deprecated = Some(DeprecationInfo {
        since: since.map(str::to_owned),
        note: note.map(str::to_owned),
    });
    variant
}

fn visible_field() -> FieldDef {
    make_field("visible_value", TypeRef::String, false, None)
}

fn excluded_field() -> FieldDef {
    FieldDef {
        binding_excluded: true,
        ..make_field("internal_only", TypeRef::String, false, None)
    }
}

fn tagged() -> Fixture {
    let payload = TypeDef {
        name: "Payload".into(),
        rust_path: "mylib::Payload".into(),
        has_serde: true,
        fields: vec![visible_field(), excluded_field()],
        ..Default::default()
    };
    let en = EnumDef {
        name: "TaggedPayload".into(),
        rust_path: "mylib::TaggedPayload".into(),
        has_serde: true,
        serde_tag: Some("kind".into()),
        serde_content: Some("payload|[data]".into()),
        variants: vec![EnumVariant {
            name: "NamedPayload".into(),
            is_tuple: true,
            fields: vec![make_field("_0", TypeRef::Named("Payload".into()), false, None)],
            ..Default::default()
        }],
        ..Default::default()
    };
    fixture("tagged", en, vec![payload])
}

fn untagged() -> Fixture {
    let en = EnumDef {
        name: "EmbeddingInput".into(),
        rust_path: "mylib::EmbeddingInput".into(),
        has_serde: true,
        has_default: true,
        serde_untagged: true,
        doc: "Text or a batch of text.".into(),
        variants: vec![
            EnumVariant {
                name: "Single".into(),
                is_tuple: true,
                fields: vec![make_field("_0", TypeRef::String, false, None)],
                ..Default::default()
            },
            EnumVariant {
                name: "Multiple".into(),
                is_tuple: true,
                fields: vec![make_field("_0", TypeRef::Vec(Box::new(TypeRef::String)), false, None)],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    fixture("untagged", en, vec![])
}
