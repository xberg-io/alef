#[cfg(test)]
mod variant_constructor_tests {
    use super::super::*;
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

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

    fn join(parts: Vec<String>) -> String {
        parts.join("\n")
    }

    /// Run the generator with the common (empty opaque/bridge/enum) sets and the `crate` core import.
    fn run(def: &EnumDef, mapper: &PhpMapper) -> String {
        let empty = AHashSet::new();
        join(gen_flat_data_enum_variant_constructors(
            def,
            mapper,
            &empty,
            &empty,
            &mapper.enum_names,
            "crate",
        ))
    }

    #[test]
    fn emits_static_constructor_building_core_variant_then_into() {
        let code = run(&shape_enum(), &mapper());

        assert!(code.contains(r#"#[php(name = "circle")]"#), "{code}");
        assert!(code.contains("pub fn _factory_circle(radius: f64) -> Self"), "{code}");
        assert!(
            code.contains("test_lib::Shape::Circle { radius: radius }.into()"),
            "{code}"
        );
        assert!(code.contains(r#"#[php(name = "rect")]"#), "{code}");
        assert!(
            code.contains("pub fn _factory_rect(width: f64, height: f64) -> Self"),
            "{code}"
        );
        assert!(
            code.contains("test_lib::Shape::Rect { width: width, height: height }.into()"),
            "{code}"
        );
    }

    #[test]
    fn converts_named_dto_field_inline_without_path_annotation() {
        let def = EnumDef {
            name: "Wrapper".to_string(),
            rust_path: "test_lib::Wrapper".to_string(),
            variants: vec![variant(
                "Llm",
                vec![field("llm", TypeRef::Named("LlmConfig".to_string()))],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(code.contains("pub fn _factory_llm(llm: &LlmConfig) -> Self"), "{code}");
        assert!(!code.contains("llm_core"), "must inline, no _core binding: {code}");
        assert!(
            code.contains("test_lib::Wrapper::Llm { llm: llm.clone().into() }.into()"),
            "{code}"
        );
    }

    #[test]
    fn boxes_boxed_named_field_in_factory() {
        let boxed = FieldDef {
            is_boxed: true,
            ..field("result", TypeRef::Named("CrawlPageResult".to_string()))
        };
        let def = EnumDef {
            name: "CrawlEvent".to_string(),
            rust_path: "test_lib::CrawlEvent".to_string(),
            variants: vec![variant("Page", vec![boxed])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(
            code.contains("test_lib::CrawlEvent::Page { result: Box::new(result.clone().into()) }.into()"),
            "{code}"
        );
    }

    #[test]
    fn converts_bytes_field() {
        let def = EnumDef {
            name: "Blob".to_string(),
            rust_path: "test_lib::Blob".to_string(),
            variants: vec![variant("Raw", vec![field("data", TypeRef::Bytes)])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(code.contains("pub fn _factory_raw(data: PhpBytes) -> Self"), "{code}");
        assert!(code.contains("test_lib::Blob::Raw { data: data.0 }.into()"), "{code}");
    }

    #[test]
    fn converts_json_field() {
        let def = EnumDef {
            name: "Payload".to_string(),
            rust_path: "test_lib::Payload".to_string(),
            variants: vec![variant("Doc", vec![field("body", TypeRef::Json)])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(code.contains("pub fn _factory_doc(body: String) -> Self"), "{code}");
        // JSON fields deserialize inline from the incoming string param; there is no separate
        // `body_json` let-binding (emitting a bare `body_json` reference without a binding was the
        // old behaviour that produced `cannot find value` errors — E0425).
        assert!(
            code.contains("test_lib::Payload::Doc { body: serde_json::from_str(&body).unwrap_or_default() }.into()"),
            "{code}"
        );
    }

    #[test]
    fn converts_vec_named_struct_field_fallibly() {
        let def = EnumDef {
            name: "Batch".to_string(),
            rust_path: "test_lib::Batch".to_string(),
            variants: vec![variant(
                "Many",
                vec![field(
                    "items",
                    TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
                )],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(
            code.contains("pub fn _factory_many(items: &ext_php_rs::types::ZendHashTable) -> PhpResult<Self>"),
            "{code}"
        );
        assert!(
            code.contains("Ok(test_lib::Batch::Many { items: items_core }.into())"),
            "{code}"
        );
    }

    #[test]
    fn converts_enum_as_string_field() {
        let mut m = mapper();
        m.enum_names.insert("Color".to_string());
        let def = EnumDef {
            name: "Painted".to_string(),
            rust_path: "test_lib::Painted".to_string(),
            variants: vec![variant(
                "Fill",
                vec![field("color", TypeRef::Named("Color".to_string()))],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &m);
        assert!(code.contains("pub fn _factory_fill(color: String) -> Self"), "{code}");
        assert!(
            code.contains("let color_core"),
            "must use the shared _core let binding: {code}"
        );
        assert!(
            code.contains("test_lib::Painted::Fill { color: color_core }.into()"),
            "{code}"
        );
    }

    #[test]
    fn casts_wide_int_field() {
        let def = EnumDef {
            name: "Sized_".to_string(),
            rust_path: "test_lib::Sized_".to_string(),
            variants: vec![variant(
                "Big",
                vec![field("count", TypeRef::Primitive(PrimitiveType::U64))],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(
            code.contains("test_lib::Sized_::Big { count: count as u64 }.into()"),
            "{code}"
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
        let code = run(&def, &mapper());
        assert!(!code.contains("_factory_empty"), "{code}");
        assert!(!code.contains("_factory_pair"), "{code}");
        assert!(!code.contains("_factory_hidden"), "{code}");
        assert!(code.contains("pub fn _factory_real(value: String) -> Self"), "{code}");
        assert!(code.contains("test_lib::Mixed::Real { value: value }.into()"), "{code}");
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
        let code = run(&def, &mapper());
        assert!(
            !code.contains("test_lib::Shape::Circle"),
            "consumer method wins for Circle: {code}"
        );
        assert!(
            code.contains("pub fn _factory_rect(width: f64, height: f64) -> Self"),
            "{code}"
        );
    }

    #[test]
    fn empty_for_unit_only_enum() {
        let def = EnumDef {
            name: "UnitOnly".to_string(),
            rust_path: "test_lib::UnitOnly".to_string(),
            variants: vec![variant("A", vec![]), variant("B", vec![])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let empty = AHashSet::new();
        let code = gen_flat_data_enum_variant_constructors(&def, &mapper(), &empty, &empty, &empty, "crate");
        assert!(code.is_empty(), "no constructors for unit-only enum: {code:?}");
    }
}

#[cfg(test)]
mod escape_php_reserved_constant_tests {
    use super::super::escape_php_reserved_constant;

    #[test]
    fn appends_underscore_to_reserved_words() {
        assert_eq!(escape_php_reserved_constant("CLASS"), "CLASS_");
        assert_eq!(escape_php_reserved_constant("INTERFACE"), "INTERFACE_");
        assert_eq!(escape_php_reserved_constant("ENUM"), "ENUM_");
    }

    #[test]
    fn leaves_normal_identifiers_alone() {
        assert_eq!(escape_php_reserved_constant("VARIABLE"), "VARIABLE");
        assert_eq!(escape_php_reserved_constant("STRUCT"), "STRUCT");
        assert_eq!(escape_php_reserved_constant("OTHER"), "OTHER");
    }
}

#[cfg(test)]
mod flat_data_enum_from_impls_tests {
    use super::super::gen_flat_data_enum_from_impls;
    use crate::core::ir::{EnumDef, EnumVariant};

    /// Constructs a minimal EnumDef for testing.
    fn make_enum(name: &str, serde_tag: Option<&str>, has_default_variant: bool, has_excluded: bool) -> EnumDef {
        let variants = vec![
            EnumVariant {
                name: "Variant1".to_string(),
                is_default: has_default_variant,
                ..Default::default()
            },
            EnumVariant {
                name: "Variant2".to_string(),
                is_default: false,
                ..Default::default()
            },
        ];

        let excluded_variants = if has_excluded {
            vec![EnumVariant {
                name: "ExcludedVariant".to_string(),
                is_default: false,
                ..Default::default()
            }]
        } else {
            vec![]
        };

        EnumDef {
            name: name.to_string(),
            rust_path: format!("module::{}", name),
            serde_tag: serde_tag.map(|s| s.to_string()),
            variants,
            excluded_variants,
            ..Default::default()
        }
    }

    #[test]
    fn flat_enum_with_default_variant_emits_default_fallback() {
        let enum_def = make_enum("Message", Some("role"), true, false);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate");

        // When the enum has a #[default] variant, should emit `_ => CorePath::default()`
        assert!(
            generated.contains("_ => module::Message::default()"),
            "Should emit default() fallback for enum with #[default] variant; got:\n{generated}"
        );
    }

    #[test]
    fn flat_enum_without_default_and_with_excluded_emits_unreachable() {
        let enum_def = make_enum("Message", Some("role"), false, true);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate");

        // When the enum has NO #[default] variant but HAS excluded variants,
        assert!(
            generated.contains("_ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\")"),
            "Should emit unreachable!() fallback for enum with excluded variants but no default; got:\n{generated}"
        );

        assert!(
            !generated.contains("_ => module::Message::default()"),
            "Should NOT emit default() fallback when core type has no visible Default impl; got:\n{generated}"
        );
    }

    #[test]
    fn flat_enum_without_default_and_no_excluded_emits_unreachable() {
        let enum_def = make_enum("SimpleEnum", Some("type"), false, false);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate");

        assert!(
            generated.contains("_ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\")"),
            "Should emit unreachable!() wildcard for &str match when core has no visible Default; got:\n{generated}"
        );
        assert!(
            !generated.contains("_ => module::SimpleEnum::default()"),
            "Should NOT emit default() fallback when core type has no visible Default impl; got:\n{generated}"
        );
    }
}
