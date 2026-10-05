use super::*;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

fn string_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        optional: true,
        ..FieldDef::default()
    }
}

fn named_field(name: &str, type_name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        optional: true,
        ..FieldDef::default()
    }
}

fn struct_named(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        fields,
        ..TypeDef::default()
    }
}

fn decl_for<'a>(decls: &'a [String], marker: &str) -> &'a str {
    decls
        .iter()
        .find(|decl| decl.contains(marker))
        .unwrap_or_else(|| panic!("no declaration contains {marker:?} in {decls:#?}"))
}

#[test]
fn should_route_a_nested_named_field_to_the_nested_wire_struct_name_in_both_families() {
    let inner = struct_named("DocxAppProperties", vec![string_field("company")]);
    let outer = struct_named(
        "PdfMetadata",
        vec![
            string_field("pdf_version"),
            named_field("app_properties", "DocxAppProperties"),
        ],
    );
    let api = ApiSurface {
        types: vec![outer, inner],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let outer_ref = wire.struct_by_name("PdfMetadata").expect("PdfMetadata registered");

    let out_name = wire.register_struct(outer_ref, Family::Out);
    let in_name = wire.register_struct(outer_ref, Family::In);
    assert_eq!(out_name, "__AlefWireOutJsPdfMetadata");
    assert_eq!(in_name, "__AlefWireInJsPdfMetadata");

    let decls = wire.declarations();
    let out_decl = decl_for(&decls, "pub struct __AlefWireOutJsPdfMetadata");
    assert!(
        out_decl.contains("pub app_properties: Option<Option<__AlefWireOutJsDocxAppProperties>>,"),
        "{out_decl}"
    );
    let in_decl = decl_for(&decls, "pub struct __AlefWireInJsPdfMetadata");
    assert!(
        in_decl.contains("pub app_properties: Option<Option<__AlefWireInJsDocxAppProperties>>,"),
        "{in_decl}"
    );
    assert!(decl_for(&decls, "pub struct __AlefWireOutJsDocxAppProperties").contains("pub company"));
    assert!(decl_for(&decls, "pub struct __AlefWireInJsDocxAppProperties").contains("pub company"));
}

#[test]
fn should_emit_no_alias_for_a_single_word_field() {
    let type_def = struct_named("PdfMetadata", vec![string_field("producer")]);
    let api = ApiSurface {
        types: vec![type_def],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let type_ref = wire.struct_by_name("PdfMetadata").expect("PdfMetadata registered");
    wire.register_struct(type_ref, Family::Out);

    let decls = wire.declarations();
    let decl = decl_for(&decls, "pub struct __AlefWireOutJsPdfMetadata");
    assert_eq!(
        decl,
        "#[derive(Default, serde::Serialize, serde::Deserialize)]\n\
         #[serde(rename_all = \"camelCase\")]\n\
         pub struct __AlefWireOutJsPdfMetadata {\n    \
         #[serde(default, skip_serializing_if = \"__alef_wire_absent_js\")]\n    \
         pub producer: Option<Option<serde_json::Value>>,\n}"
    );
    assert!(!decl.contains("alias ="), "{decl}");
}

#[test]
fn should_map_a_map_valued_field_to_a_btreemap_with_an_untouched_string_key() {
    let type_def = struct_named(
        "PdfMetadata",
        vec![FieldDef {
            name: "custom_properties".to_string(),
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: true,
            ..FieldDef::default()
        }],
    );
    let api = ApiSurface {
        types: vec![type_def],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let type_ref = wire.struct_by_name("PdfMetadata").expect("PdfMetadata registered");
    wire.register_struct(type_ref, Family::Out);

    let decls = wire.declarations();
    let decl = decl_for(&decls, "pub struct __AlefWireOutJsPdfMetadata");
    assert!(
        decl.contains("pub custom_properties: Option<Option<std::collections::BTreeMap<String, serde_json::Value>>>,"),
        "{decl}"
    );
}

#[test]
fn should_keep_a_variants_renamed_tag_value() {
    let payload = struct_named("FictionBookMetadata", vec![string_field("title")]);
    let enum_def = EnumDef {
        name: "FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        variants: vec![EnumVariant {
            name: "FictionBook".to_string(),
            serde_rename: Some("fiction_book".to_string()),
            fields: vec![named_field("_0", "FictionBookMetadata")],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };
    let api = ApiSurface {
        types: vec![payload],
        enums: vec![enum_def.clone()],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let (out_name, in_name) = wire.register_enum(&enum_def);
    assert_eq!(out_name, "__AlefWireOutJsFormatMetadata");
    assert_eq!(in_name, "__AlefWireInJsFormatMetadata");

    let decls = wire.declarations();
    let out_decl = decl_for(&decls, "pub enum __AlefWireOutJsFormatMetadata");
    assert!(out_decl.contains("#[serde(tag = \"format_type\")]"), "{out_decl}");
    assert!(
        out_decl.contains("#[serde(rename = \"fiction_book\")]\n    FictionBook(__AlefWireOutJsFictionBookMetadata),"),
        "{out_decl}"
    );
}

#[test]
fn should_terminate_and_declare_a_self_referential_type_exactly_once() {
    let node = struct_named(
        "Node",
        vec![FieldDef {
            name: "child".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("Node".to_string()))),
            optional: true,
            is_boxed: true,
            ..FieldDef::default()
        }],
    );
    let api = ApiSurface {
        types: vec![node],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let node_ref = wire.struct_by_name("Node").expect("Node registered");
    let name = wire.register_struct(node_ref, Family::Out);
    assert_eq!(name, "__AlefWireOutJsNode");

    let decls = wire.declarations();
    let occurrences = decls
        .iter()
        .filter(|d| d.contains("pub struct __AlefWireOutJsNode"))
        .count();
    assert_eq!(occurrences, 1, "{decls:#?}");
    let decl = decl_for(&decls, "pub struct __AlefWireOutJsNode");
    assert!(
        decl.contains("pub child: Option<Option<Box<__AlefWireOutJsNode>>>,"),
        "{decl}"
    );
}

#[test]
fn should_produce_byte_identical_output_across_two_runs_with_the_same_input() {
    let payload = struct_named("FictionBookMetadata", vec![string_field("title")]);
    let enum_def = EnumDef {
        name: "FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        variants: vec![
            EnumVariant {
                name: "FictionBook".to_string(),
                fields: vec![named_field("_0", "FictionBookMetadata")],
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Unknown".to_string(),
                is_default: true,
                fields: vec![],
                ..EnumVariant::default()
            },
        ],
        has_default: true,
        ..EnumDef::default()
    };
    let api = ApiSurface {
        types: vec![payload],
        enums: vec![enum_def.clone()],
        ..ApiSurface::default()
    };

    let mut first = JsonWireTypes::new(&api, "Js");
    first.register_enum(&enum_def);
    let mut second = JsonWireTypes::new(&api, "Js");
    second.register_enum(&enum_def);

    assert_eq!(first.declarations(), second.declarations());
}

#[test]
fn should_derive_default_only_when_a_unit_variant_is_marked_default() {
    let enum_def = EnumDef {
        name: "FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        serde_rename_all: Some("snake_case".to_string()),
        has_default: true,
        variants: vec![EnumVariant {
            name: "Unknown".to_string(),
            is_default: true,
            fields: vec![],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };
    let api = ApiSurface {
        enums: vec![enum_def.clone()],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    wire.register_enum(&enum_def);

    let decls = wire.declarations();
    let decl = decl_for(&decls, "pub enum __AlefWireOutJsFormatMetadata");
    assert!(
        decl.starts_with("#[derive(Default, serde::Serialize, serde::Deserialize)]"),
        "{decl}"
    );
    assert!(
        decl.contains("    #[default]\n    #[serde(rename = \"unknown\")]\n    Unknown,"),
        "{decl}"
    );
}

#[test]
fn should_flatten_a_field_with_no_option_wrapper_alias_or_skip_attribute() {
    let inner = struct_named("Extra", vec![string_field("note")]);
    let outer = struct_named(
        "PdfMetadata",
        vec![FieldDef {
            name: "extra".to_string(),
            ty: TypeRef::Named("Extra".to_string()),
            serde_flatten: true,
            ..FieldDef::default()
        }],
    );
    let api = ApiSurface {
        types: vec![outer, inner],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let outer_ref = wire.struct_by_name("PdfMetadata").expect("PdfMetadata registered");
    wire.register_struct(outer_ref, Family::Out);

    let decls = wire.declarations();
    let decl = decl_for(&decls, "pub struct __AlefWireOutJsPdfMetadata");
    assert!(
        decl.contains("    #[serde(flatten)]\n    pub extra: __AlefWireOutJsExtra,"),
        "{decl}"
    );
}

#[test]
fn should_skip_a_field_marked_serde_skip() {
    let type_def = struct_named(
        "PdfMetadata",
        vec![
            string_field("producer"),
            FieldDef {
                name: "internal_only".to_string(),
                ty: TypeRef::String,
                serde_skip: true,
                sensitive: false,
                ..FieldDef::default()
            },
        ],
    );
    let api = ApiSurface {
        types: vec![type_def],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let type_ref = wire.struct_by_name("PdfMetadata").expect("PdfMetadata registered");
    wire.register_struct(type_ref, Family::Out);

    let decls = wire.declarations();
    let decl = decl_for(&decls, "pub struct __AlefWireOutJsPdfMetadata");
    assert!(!decl.contains("internal_only"), "{decl}");
}

#[test]
fn should_route_every_non_flattened_field_through_the_shared_absent_predicate() {
    let type_def = struct_named("PdfMetadata", vec![string_field("pdf_version")]);
    let api = ApiSurface {
        types: vec![type_def],
        ..ApiSurface::default()
    };
    let mut wire = JsonWireTypes::new(&api, "Js");
    let type_ref = wire.struct_by_name("PdfMetadata").expect("PdfMetadata registered");
    wire.register_struct(type_ref, Family::Out);

    let decls = wire.declarations();
    let helper = decl_for(&decls, "fn __alef_wire_absent_js");
    assert_eq!(
        helper,
        "fn __alef_wire_absent_js<T>(value: &Option<Option<T>>) -> bool {\n    matches!(value, None | Some(None))\n}"
    );
    let field_decl = decl_for(&decls, "pub struct __AlefWireOutJsPdfMetadata");
    assert!(
        field_decl.contains("skip_serializing_if = \"__alef_wire_absent_js\""),
        "{field_decl}"
    );
    // `matches!(value, None | Some(None))` is true for BOTH the absent (`None`) and the
    // present-but-null (`Some(None)`) states, which is what makes a null-valued field OMITTED
    // from the serialized output rather than round-tripped as `"field":null` -- the declared
    // `.d.ts` for these payloads has no `null` state for an optional property. Actually
    // executing that serialization would require compiling this emitted source, which is
    // outside what a string-emission unit test in this crate can do; this asserts the emitted
    // predicate's exact logic instead.
    fn mirrors_the_emitted_predicate<T>(value: &Option<Option<T>>) -> bool {
        matches!(value, None | Some(None))
    }
    assert!(mirrors_the_emitted_predicate::<u8>(&None));
    assert!(mirrors_the_emitted_predicate(&Some(None::<u8>)));
    assert!(!mirrors_the_emitted_predicate(&Some(Some(7u8))));
}
