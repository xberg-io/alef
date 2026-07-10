use super::json_values::elixir_safe_atom;
use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use ahash::AHashSet;

#[test]
fn test_elixir_field_name_with_type_payload_derived() {
    let name = elixir_field_name_with_type("_0", 0, Some("PdfMetadata"), "Pdf", 1);
    assert_eq!(name, "metadata");

    let name = elixir_field_name_with_type("_0", 0, Some("ExcelMetadata"), "Excel", 1);
    assert_eq!(name, "metadata");

    let name = elixir_field_name_with_type("_0", 0, Some("DocxMetadata"), "Docx", 1);
    assert_eq!(name, "metadata");
}

#[test]
fn test_elixir_field_name_with_type_primitive() {
    let name = elixir_field_name_with_type("_0", 0, Some("String"), "Error", 1);
    assert_eq!(name, "value");

    let name = elixir_field_name_with_type("_0", 0, Some("bool"), "Flag", 1);
    assert_eq!(name, "value");
}

#[test]
fn test_elixir_field_name_with_type_multiple_fields() {
    let name = elixir_field_name_with_type("_0", 0, None, "Pair", 2);
    assert_eq!(name, "value0");

    let name = elixir_field_name_with_type("_1", 1, None, "Pair", 2);
    assert_eq!(name, "value1");
}

#[test]
fn test_elixir_field_name_with_type_named_field() {
    let name = elixir_field_name_with_type("reason", 0, Some("String"), "Error", 1);
    assert_eq!(name, "reason");
}

#[test]
fn test_gen_elixir_enum_module_data_enum_with_payload_derived_names() {
    let format_enum = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "my_crate::FormatMetadata".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Pdf".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("PdfMetadata".into()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Docx".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("DocxMetadata".into()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_elixir_enum_module(&format_enum, "SampleCrate");

    assert!(
        result.contains("@type pdf :: %{type: :pdf, metadata: map()}"),
        "should use payload-derived 'metadata' field name with concrete type map(); got:\n{result}"
    );

    assert!(
        result.contains("@type docx :: %{type: :docx, metadata: map()}"),
        "should use payload-derived 'metadata' field name with concrete type map(); got:\n{result}"
    );

    assert!(
        !result.contains("value_0: term()"),
        "should not use generic value_0 field name with term() type; got:\n{result}"
    );
}

#[test]
fn test_elixir_safe_atom_valid_identifier() {
    assert_eq!(elixir_safe_atom("img"), "img");
    assert_eq!(elixir_safe_atom("picture_source"), "picture_source");
    assert_eq!(elixir_safe_atom("valid?"), "valid?");
    assert_eq!(elixir_safe_atom("valid!"), "valid!");
}

#[test]
fn test_elixir_safe_atom_with_special_chars() {
    assert_eq!(elixir_safe_atom("og:image"), r#""og:image""#);
    assert_eq!(elixir_safe_atom("twitter:image"), r#""twitter:image""#);
    assert_eq!(elixir_safe_atom("some-value"), r#""some-value""#);
}

#[test]
fn test_gen_elixir_enum_module_with_serde_rename_special_chars() {
    let image_source_enum = EnumDef {
        name: "ImageSource".to_string(),
        rust_path: "my_crate::ImageSource".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Img".into(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "OgImage".into(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("og:image".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "TwitterImage".into(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: Some("twitter:image".to_string()),
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_elixir_enum_module(&image_source_enum, "SampleFixture");

    assert!(
        result.contains(":img | :\"og:image\" | :\"twitter:image\""),
        "should emit quoted atoms in @type for serde_rename with colons; got:\n{result}"
    );

    assert!(
        result.contains("@og_image "),
        "should use @og_image attribute name (from variant OgImage), not @og:image; got:\n{result}"
    );
    assert!(
        result.contains("@twitter_image "),
        "should use @twitter_image attribute name (from variant TwitterImage), not @twitter:image; got:\n{result}"
    );

    assert!(
        result.contains("def og_image, do: @og_image"),
        "should emit def og_image() function name, not def og:image(); got:\n{result}"
    );
    assert!(
        result.contains("def twitter_image, do: @twitter_image"),
        "should emit def twitter_image() function name, not def twitter:image(); got:\n{result}"
    );

    assert!(
        result.contains(r#"@og_image :"og:image""#),
        "should emit @og_image with quoted atom value; got:\n{result}"
    );
    assert!(
        result.contains(r#"@twitter_image :"twitter:image""#),
        "should emit @twitter_image with quoted atom value; got:\n{result}"
    );
}

#[test]
fn test_gen_elixir_enum_module_resolves_known_payload_types() {
    let format_enum = EnumDef {
        name: "FormatMetadata".to_string(),
        rust_path: "my_crate::FormatMetadata".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Pdf".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("PdfMetadata".into()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Other".into(),
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("UnknownType".into()),
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    original_type: None,
                }],
                is_tuple: true,
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let mut known_types = AHashSet::new();
    known_types.insert("PdfMetadata".to_string());

    let result = gen_elixir_enum_module_with_known_types(&format_enum, "SampleCrate", &known_types);

    assert!(
        result.contains("SampleCrate.PdfMetadata.t()"),
        "should resolve PdfMetadata to SampleCrate.PdfMetadata.t(); got:\n{result}"
    );

    assert!(
        result.contains("value: map()"),
        "should fall back to map() for unknown type; got:\n{result}"
    );
}

mod variant_constructors {
    use super::*;
    use crate::core::ir::{MethodDef, PrimitiveType};

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..Default::default()
        }
    }

    fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            ..Default::default()
        }
    }

    /// A tagged data enum with struct variants — the NifTaggedEnum shape.
    fn shape_enum() -> EnumDef {
        EnumDef {
            name: "Shape".to_string(),
            rust_path: "test_lib::Shape".to_string(),
            variants: vec![
                variant("Circle", vec![field("radius", TypeRef::Primitive(PrimitiveType::F64))]),
                variant(
                    "Rect",
                    vec![
                        field("width", TypeRef::Primitive(PrimitiveType::F64)),
                        field("height", TypeRef::Primitive(PrimitiveType::F64)),
                    ],
                ),
            ],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn emits_constructor_per_struct_variant_as_tagged_tuple() {
        let result = gen_elixir_enum_module(&shape_enum(), "SampleCrate");
        assert!(
            result.contains("def circle(radius), do: {:circle, %{radius: radius}}"),
            "{result}"
        );
        assert!(
            result.contains("def rect(width, height), do: {:rect, %{width: width, height: height}}"),
            "{result}"
        );
    }

    #[test]
    fn skips_unit_tuple_and_excluded_variants() {
        let mut tuple_variant = variant("Pair", vec![field("_0", TypeRef::String)]);
        tuple_variant.is_tuple = true;
        let mut excluded = variant("Hidden", vec![field("value", TypeRef::String)]);
        excluded.binding_excluded = true;

        let def = EnumDef {
            name: "Mixed".to_string(),
            rust_path: "test_lib::Mixed".to_string(),
            variants: vec![
                variant("Empty", vec![]),
                tuple_variant,
                excluded,
                variant("Real", vec![field("value", TypeRef::String)]),
            ],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let result = gen_elixir_enum_module(&def, "SampleCrate");
        assert!(!result.contains("def empty"), "{result}");
        assert!(!result.contains("def pair"), "{result}");
        assert!(!result.contains("def hidden"), "{result}");
        assert!(
            result.contains("def real(value), do: {:real, %{value: value}}"),
            "{result}"
        );
    }

    #[test]
    fn yields_to_hand_written_method() {
        let def = EnumDef {
            methods: vec![MethodDef {
                name: "circle".to_string(),
                is_static: true,
                ..Default::default()
            }],
            ..shape_enum()
        };
        let result = gen_elixir_enum_module(&def, "SampleCrate");
        assert!(!result.contains("def circle("), "consumer method wins: {result}");
        assert!(result.contains("def rect("), "{result}");
    }

    #[test]
    fn no_constructors_for_unit_enum() {
        let def = EnumDef {
            name: "Color".to_string(),
            rust_path: "test_lib::Color".to_string(),
            variants: vec![variant("Red", vec![]), variant("Blue", vec![])],
            ..Default::default()
        };
        let result = gen_elixir_enum_module(&def, "SampleCrate");
        assert!(
            !result.contains(", do: {:"),
            "unit enum must not emit tagged-tuple ctor: {result}"
        );
    }

    #[test]
    fn reserved_word_variant_name_is_escaped() {
        let def = EnumDef {
            name: "Marker".to_string(),
            rust_path: "test_lib::Marker".to_string(),
            variants: vec![variant("End", vec![field("at", TypeRef::String)])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let result = gen_elixir_enum_module(&def, "SampleCrate");
        assert!(result.contains("{:end, %{at: at}}"), "{result}");
    }

    #[test]
    fn reserved_word_variant_typespec_atom_matches_constructor_and_decoder() {
        let def = EnumDef {
            name: "Marker".to_string(),
            rust_path: "test_lib::Marker".to_string(),
            variants: vec![variant("End", vec![field("at", TypeRef::String)])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let result = gen_elixir_enum_module(&def, "SampleCrate");
        assert!(
            result.contains("@type end_val :: %{type: :end,"),
            "typespec LHS guards the reserved word, atom value stays `:end`: {result}"
        );
        assert!(
            !result.contains("type: :end_val"),
            "typespec atom must not use the reserved-word-guarded form: {result}"
        );
    }

    #[test]
    fn serde_renamed_struct_variant_constructor_uses_snake_atom() {
        // A `#[serde(rename = "...")]` struct variant: the constructor's `{:atom, ...}` derives the
        let mut renamed = variant("EmojiBased", vec![field("shortcode", TypeRef::String)]);
        renamed.serde_rename = Some("emoji-based".to_string());
        let def = EnumDef {
            name: "Token".to_string(),
            rust_path: "test_lib::Token".to_string(),
            variants: vec![renamed],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let result = gen_elixir_enum_module(&def, "SampleCrate");
        assert!(
            result.contains("def emoji_based(shortcode), do: {:emoji_based, %{shortcode: shortcode}}"),
            "constructor atom must derive from snake_case variant name, ignoring serde_rename: {result}"
        );
        assert!(
            !result.contains(":\"emoji-based\""),
            "constructor must not emit the wire-renamed atom: {result}"
        );
    }
}
