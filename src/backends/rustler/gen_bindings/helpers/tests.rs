use super::json_values::elixir_safe_atom;
use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use ahash::AHashSet;

#[test]
fn test_elixir_field_name_with_type_payload_derived() {
    // Named type Pdf with variant Pdf → strip Pdf → metadata
    let name = elixir_field_name_with_type("_0", 0, Some("PdfMetadata"), "Pdf", 1);
    assert_eq!(name, "metadata");

    // Named type Excel with variant Excel → strip Excel → metadata
    let name = elixir_field_name_with_type("_0", 0, Some("ExcelMetadata"), "Excel", 1);
    assert_eq!(name, "metadata");

    // Docx variant with DocxMetadata type → strip Docx → metadata
    let name = elixir_field_name_with_type("_0", 0, Some("DocxMetadata"), "Docx", 1);
    assert_eq!(name, "metadata");
}

#[test]
fn test_elixir_field_name_with_type_primitive() {
    // Primitive String type → value
    let name = elixir_field_name_with_type("_0", 0, Some("String"), "Error", 1);
    assert_eq!(name, "value");

    // Primitive bool type → value
    let name = elixir_field_name_with_type("_0", 0, Some("bool"), "Flag", 1);
    assert_eq!(name, "value");
}

#[test]
fn test_elixir_field_name_with_type_multiple_fields() {
    // Multiple fields → generic value0, value1
    let name = elixir_field_name_with_type("_0", 0, None, "Pair", 2);
    assert_eq!(name, "value0");

    let name = elixir_field_name_with_type("_1", 1, None, "Pair", 2);
    assert_eq!(name, "value1");
}

#[test]
fn test_elixir_field_name_with_type_named_field() {
    // Non-positional field name → use directly
    let name = elixir_field_name_with_type("reason", 0, Some("String"), "Error", 1);
    assert_eq!(name, "reason");
}

#[test]
fn test_gen_elixir_enum_module_data_enum_with_payload_derived_names() {
    // Create FormatMetadata enum with Pdf(PdfMetadata) and Docx(DocxMetadata) variants
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
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_elixir_enum_module(&format_enum, "SampleCrate");

    // Should emit @type pdf with metadata field (not value_0) and concrete type (not term())
    assert!(
        result.contains("@type pdf :: %{type: :pdf, metadata: map()}"),
        "should use payload-derived 'metadata' field name with concrete type map(); got:\n{result}"
    );

    // Should emit @type docx with metadata field (not value_0) and concrete type (not term())
    assert!(
        result.contains("@type docx :: %{type: :docx, metadata: map()}"),
        "should use payload-derived 'metadata' field name with concrete type map(); got:\n{result}"
    );

    // Must not use the old generic name for variant fields
    assert!(
        !result.contains("value_0: term()"),
        "should not use generic value_0 field name with term() type; got:\n{result}"
    );
}

#[test]
fn test_elixir_safe_atom_valid_identifier() {
    // Returns value without leading :, since template adds it
    assert_eq!(elixir_safe_atom("img"), "img");
    assert_eq!(elixir_safe_atom("picture_source"), "picture_source");
    assert_eq!(elixir_safe_atom("valid?"), "valid?");
    assert_eq!(elixir_safe_atom("valid!"), "valid!");
}

#[test]
fn test_elixir_safe_atom_with_special_chars() {
    // Atoms with colons must be quoted (without leading :, template adds it)
    assert_eq!(elixir_safe_atom("og:image"), r#""og:image""#);
    assert_eq!(elixir_safe_atom("twitter:image"), r#""twitter:image""#);
    // Atoms with dashes must be quoted
    assert_eq!(elixir_safe_atom("some-value"), r#""some-value""#);
}

#[test]
fn test_gen_elixir_enum_module_with_serde_rename_special_chars() {
    // Create ImageSource enum with serde_rename containing colons
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
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_elixir_enum_module(&image_source_enum, "SampleFixture");

    // @type should contain quoted atoms for special chars
    assert!(
        result.contains(":img | :\"og:image\" | :\"twitter:image\""),
        "should emit quoted atoms in @type for serde_rename with colons; got:\n{result}"
    );

    // Attributes should use snake_case identifiers, not the serde_rename value
    assert!(
        result.contains("@og_image "),
        "should use @og_image attribute name (from variant OgImage), not @og:image; got:\n{result}"
    );
    assert!(
        result.contains("@twitter_image "),
        "should use @twitter_image attribute name (from variant TwitterImage), not @twitter:image; got:\n{result}"
    );

    // Accessors (functions) should also use snake_case names
    assert!(
        result.contains("def og_image, do: @og_image"),
        "should emit def og_image() function name, not def og:image(); got:\n{result}"
    );
    assert!(
        result.contains("def twitter_image, do: @twitter_image"),
        "should emit def twitter_image() function name, not def twitter:image(); got:\n{result}"
    );

    // Ensure the attribute values are properly quoted atoms
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
    // Create FormatMetadata enum with both known and unknown payload types
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
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    // Simulate calling with known types available
    let mut known_types = AHashSet::new();
    known_types.insert("PdfMetadata".to_string());

    let result = gen_elixir_enum_module_with_known_types(&format_enum, "SampleCrate", &known_types);

    // Known type should resolve to module.t()
    assert!(
        result.contains("SampleCrate.PdfMetadata.t()"),
        "should resolve PdfMetadata to SampleCrate.PdfMetadata.t(); got:\n{result}"
    );

    // Unknown type should fall back to map()
    assert!(
        result.contains("value: map()"),
        "should fall back to map() for unknown type; got:\n{result}"
    );
}
