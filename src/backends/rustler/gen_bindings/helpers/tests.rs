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
                    version: Default::default(),
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
                    serde_with: None,
                    serde_skip_serializing_if: false,
                    serde_skip: false,
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
                    version: Default::default(),
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
                    serde_with: None,
                    serde_skip_serializing_if: false,
                    serde_skip: false,
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
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
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
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let result = gen_elixir_enum_module(&image_source_enum, "SampleFixture");

    assert!(
        result.contains(":img | :og_image | :twitter_image"),
        "@type t must advertise the atoms the NIF actually produces (from the Rust variant \
         names), not the serde wire names; got:\n{result}"
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
        result.contains("@og_image :og_image"),
        "the attribute's VALUE is the runtime atom, not the wire name; got:\n{result}"
    );
    assert!(
        result.contains("@twitter_image :twitter_image"),
        "the attribute's VALUE is the runtime atom, not the wire name; got:\n{result}"
    );

    // The reachability pin. `og_image/0` returns whatever `@og_image` holds, so a `wire_value/1`
    // clause keyed on a different spelling is unreachable through the module's own public
    // surface -- and nothing catches the value that does arrive, because `wire_value/1` has no
    // fallback clause. This composition (`wire_value(og_image())`) is the shape that raised
    // `FunctionClauseError` before the fix; asserting the two spellings match is what makes it
    // impossible to reintroduce. ~keep
    assert!(
        result.contains(r#"def wire_value(:og_image), do: "og:image""#),
        "wire_value/1 must have a clause for the atom og_image/0 actually returns, and map it \
         to the serde wire name; got:\n{result}"
    );
    assert!(
        result.contains(r#"def wire_value(:twitter_image), do: "twitter:image""#),
        "got:\n{result}"
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
                    version: Default::default(),
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
                    serde_with: None,
                    serde_skip_serializing_if: false,
                    serde_skip: false,
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
                    version: Default::default(),
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
                    serde_with: None,
                    serde_skip_serializing_if: false,
                    serde_skip: false,
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
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let mut known_types = AHashSet::new();
    known_types.insert("PdfMetadata".to_string());

    let result = gen_elixir_enum_module_with_known_types(&format_enum, "SampleCrate", &known_types, "mylib", None);

    assert!(
        result.contains("SampleCrate.PdfMetadata.t()"),
        "should resolve PdfMetadata to SampleCrate.PdfMetadata.t(); got:\n{result}"
    );

    assert!(
        result.contains("value: map()"),
        "should fall back to map() for unknown type; got:\n{result}"
    );
}

/// The Elixir binding used to have no way to recover a unit enum's serde wire value at all --
/// `to_string(:key_value)` returns the atom's own Elixir spelling ("key_value"), never the wire
/// value ("KeyValue") a fixture literal carries. `wire_value/1` must map every variant atom to
/// `wire_variant_value`, and a bare atom has no per-value dispatch target, so no
/// `defimpl String.Chars` is emitted for this shape.
#[test]
fn gen_elixir_enum_module_unit_enum_exposes_wire_value_not_atom_spelling() {
    let def = EnumDef {
        name: "DataNodeKind".to_string(),
        variants: vec![
            EnumVariant {
                name: "KeyValue".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Sequence".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };

    let result = gen_elixir_enum_module(&def, "SampleCrate");

    assert!(
        result.contains("def wire_value(:key_value), do: \"KeyValue\""),
        "unit enum must expose the serde wire value (PascalCase), not the atom's own \
         snake_case spelling; got:\n{result}"
    );
    assert!(
        result.contains("def wire_value(:sequence), do: \"Sequence\""),
        "got:\n{result}"
    );
    assert!(
        !result.contains("defimpl String.Chars"),
        "a bare-atom unit enum has no per-value dispatch target for String.Chars; got:\n{result}"
    );
}

/// A data-carrying enum whose data variants are all single-field tuples of Named types uses
/// Rustler's flat `NifStruct` shape (`gen_rustler_flat_data_enum` in `gen_bindings/types.rs`):
/// the discriminator field it decodes to already holds the exact `wire_variant_value` string
/// (see `flat_enum_from_core_variant_*.jinja`), so `wire_value/1` need only read that field --
/// and because the runtime value is a real map/struct term, a `defimpl String.Chars` can
/// dispatch on it too, unlike the bare-atom unit case.
#[test]
fn gen_elixir_enum_module_flat_data_enum_exposes_wire_value_and_string_chars() {
    let def = EnumDef {
        name: "FormatMetadata".to_string(),
        variants: vec![
            EnumVariant {
                name: "Pdf".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("PdfMetadata".to_string()),
                    ..FieldDef::default()
                }],
                is_tuple: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Docx".to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named("DocxMetadata".to_string()),
                    ..FieldDef::default()
                }],
                is_tuple: true,
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    };

    let result = gen_elixir_enum_module(&def, "SampleCrate");

    assert!(
        result.contains("def wire_value(value) when is_map(value), do: Map.fetch!(value, :type)"),
        "the flat-struct shape's discriminator field already holds the wire value; reading it \
         beats calling to_string on a struct with no String.Chars impl; got:\n{result}"
    );
    assert!(result.contains("def wire_value(:pdf), do: \"Pdf\""), "got:\n{result}");
    assert!(result.contains("def wire_value(:docx), do: \"Docx\""), "got:\n{result}");
    assert!(
        result.contains("defimpl String.Chars, for: SampleCrate.FormatMetadata"),
        "the flat-struct shape decodes to a real map/struct term, so (unlike a bare atom) it \
         can dispatch a String.Chars impl; got:\n{result}"
    );
    assert!(
        result.contains("SampleCrate.FormatMetadata.wire_value(value)"),
        "String.Chars must delegate to the same wire_value/1 the enum module exposes, not \
         re-derive the wire string; got:\n{result}"
    );
}

/// The `@type` alias documents the flat-struct shape as `%{DISCRIMINATOR: variant_atom, ...}`,
/// and `wire_value/1`'s map clause reads `Map.fetch!(value, :DISCRIMINATOR)` off the exact same
/// real runtime term. Both descriptions must name the same key -- extracted here from the
/// rendered output rather than pinned as two independent hard-coded literals, so a future
/// regression that reintroduces two independently-hard-coded fallbacks (the bug this guards)
/// would fail this assertion even if both literals happened to still be spelled the same way. ~keep
#[test]
fn gen_elixir_enum_module_flat_data_enum_typespec_and_wire_value_discriminator_agree() {
    fn extract_discriminator<'a>(result: &'a str, needle: &str, terminator: char) -> &'a str {
        let after_needle = result
            .split_once(needle)
            .unwrap_or_else(|| panic!("expected to find `{needle}` in:\n{result}"))
            .1;
        after_needle
            .split(terminator)
            .next()
            .unwrap_or_else(|| panic!("expected `{terminator}` after `{needle}` in:\n{result}"))
    }

    let def = EnumDef {
        name: "Payload".to_string(),
        serde_tag: Some("custom_tag".to_string()),
        variants: vec![EnumVariant {
            name: "Pdf".to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::Named("PdfMetadata".to_string()),
                ..FieldDef::default()
            }],
            is_tuple: true,
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    };

    let result = gen_elixir_enum_module(&def, "SampleCrate");

    let typespec_discriminator = extract_discriminator(&result, "@type pdf :: %{", ':');
    let wire_value_discriminator = extract_discriminator(&result, "Map.fetch!(value, :", ')');

    assert_eq!(
        typespec_discriminator, wire_value_discriminator,
        "the @type alias and wire_value/1 must key the flat-struct discriminator identically; \
         got:\n{result}"
    );
    assert_eq!(
        typespec_discriminator, "custom_tag",
        "an explicit serde_tag override must thread into both emitters; got:\n{result}"
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
            serde_content: None,
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

    /// `Shape` has struct variants (`Circle { radius }`, `Rect { width, height }`), so it is
    /// NOT the flat-struct shape (`is_flat_data_enum` requires every data variant to be a
    /// single-field tuple) -- it stays a `NifTaggedEnum` `{atom, ...}` tuple. `wire_value/1`
    /// must still resolve it via the same per-variant atom clauses the unit-enum shape uses
    /// (the tuple clause just recurses on the tag), but has no discriminator field to read and
    /// no struct/`__struct__` carrier to hang a `String.Chars` impl on.
    #[test]
    fn tagged_enum_wire_value_dispatches_atoms_and_tuples_but_has_no_struct_dispatch() {
        let result = gen_elixir_enum_module(&shape_enum(), "SampleCrate");
        assert!(result.contains("def wire_value(:circle), do: \"Circle\""), "{result}");
        assert!(result.contains("def wire_value(:rect), do: \"Rect\""), "{result}");
        assert!(
            result.contains("def wire_value(value) when is_tuple(value), do: wire_value(elem(value, 0))"),
            "a NifTaggedEnum variant is a `{{atom, ...}}` tuple; wire_value/1 must dispatch \
             through the same atom clauses, got:\n{result}"
        );
        assert!(
            !result.contains("defimpl String.Chars"),
            "a NifTaggedEnum has no struct/__struct__ carrier to dispatch a protocol on; got:\n{result}"
        );
        assert!(
            !result.contains("is_map(value)"),
            "only the flat-struct shape has a discriminator field to read; got:\n{result}"
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
            serde_content: None,
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

    /// Regression for the `ContentPart` bug: no backend forwards `enum_def.methods` (a
    /// hand-written inherent static method extracted from a separate `impl EnumType { .. }`
    /// block) into the generated Elixir module, so suppressing the derived factory on a name
    /// collision used to drop the constructor entirely with nothing to replace it (this is
    /// exactly what happened to `ContentPart.text/1` and `ContentPart.image_url/1`). Every
    /// data-carrying variant must always get a reachable constructor.
    #[test]
    fn emits_factory_even_with_colliding_hand_written_method() {
        let def = EnumDef {
            methods: vec![MethodDef {
                name: "circle".to_string(),
                is_static: true,
                ..Default::default()
            }],
            ..shape_enum()
        };
        let result = gen_elixir_enum_module(&def, "SampleCrate");
        assert!(
            result.contains("def circle(radius), do: {:circle, %{radius: radius}}"),
            "circle factory must stay reachable despite the colliding hand-written method: {result}"
        );
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
            serde_content: None,
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
            serde_content: None,
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
            serde_content: None,
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

#[test]
fn gen_rustler_unimplemented_body_string_return_fails_loudly() {
    let body = gen_rustler_unimplemented_body(&TypeRef::String, "extract_text", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible String return must fail the build: {body}"
    );
    assert!(
        !body.contains("[unimplemented:"),
        "must not fabricate a placeholder string: {body}"
    );
}

#[test]
fn gen_rustler_unimplemented_body_vec_return_fails_loudly() {
    let body = gen_rustler_unimplemented_body(&TypeRef::Vec(Box::new(TypeRef::String)), "list_entries", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible Vec return must fail the build: {body}"
    );
    assert!(!body.contains("Vec::new()"), "must not fabricate an empty Vec: {body}");
}

#[test]
fn gen_rustler_unimplemented_body_optional_return_fails_loudly() {
    let body = gen_rustler_unimplemented_body(&TypeRef::Optional(Box::new(TypeRef::String)), "find_entry", false);
    assert!(
        body.contains("compile_error!"),
        "non-fallible Optional return must fail the build: {body}"
    );
    assert!(!body.contains("\"None\""), "must not fabricate a None literal: {body}");
}

#[test]
fn gen_rustler_unimplemented_body_primitive_return_fails_loudly() {
    let body = gen_rustler_unimplemented_body(
        &TypeRef::Primitive(crate::core::ir::PrimitiveType::I64),
        "count_entries",
        false,
    );
    assert!(
        body.contains("compile_error!"),
        "non-fallible primitive return must fail the build: {body}"
    );
}

#[test]
fn gen_rustler_unimplemented_body_unit_return_stays_void() {
    let body = gen_rustler_unimplemented_body(&TypeRef::Unit, "run_side_effect", false);
    assert_eq!(body, "()", "Unit return must stay a legitimate void value: {body}");
}

#[test]
fn gen_rustler_unimplemented_body_with_error_type_raises_runtime_error() {
    let body = gen_rustler_unimplemented_body(&TypeRef::String, "extract_text", true);
    assert_eq!(
        body, "Err(String::from(\"Not implemented: extract_text\"))",
        "fallible functions must keep raising a real runtime error: {body}"
    );
    assert!(
        !body.contains("compile_error!"),
        "fallible path must not also emit compile_error!: {body}"
    );
}
