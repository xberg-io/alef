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
            serde_content: None,
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
        assert!(code.contains("test_lib::Shape::Circle { radius }.into()"), "{code}");
        assert!(code.contains(r#"#[php(name = "rect")]"#), "{code}");
        assert!(
            code.contains("pub fn _factory_rect(width: f64, height: f64) -> Self"),
            "{code}"
        );
        assert!(
            code.contains("test_lib::Shape::Rect { width, height }.into()"),
            "{code}"
        );
        // The factory parameter is already named after the field, so spelling the initializer
        // out is `clippy::redundant_field_names` in the consumer's generated PHP crate. ~keep
        assert!(
            !code.contains("radius: radius") && !code.contains("width: width"),
            "core variant construction must not emit redundant field names:\n{code}"
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
            serde_content: None,
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
            serde_content: None,
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
            serde_content: None,
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
            serde_content: None,
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        // `param_conversion_is_fallible` (`helpers/params.rs`) now answers `true` for a bare
        // `Json` param -- the SAME predicate the struct-constructor path
        // (`gen_struct_methods_impl` in `types/structs.rs`) uses to decide `PhpResult<Self>`
        // wrapping for its OWN honest, `?`-propagating JSON decode -- so this variant factory
        // wraps in `PhpResult<Self>` too, even though its body (below) still silently defaults
        // on malformed input via the shared `gen_php_call_args_with_let_bindings_vec` this
        // function deliberately does not touch (that helper is used broadly across every
        // function/method call site in this backend, not just constructors, so changing its
        // error handling is a separate, much larger change than this widening). The wrapping is
        // harmless here (an infallible body inside `Ok(..)` still compiles) and consistent with
        // how the same predicate already governs the `Vec<Named>` case just below. ~keep
        assert!(
            code.contains("pub fn _factory_doc(body: String) -> PhpResult<Self>"),
            "{code}"
        );
        // JSON fields deserialize inline from the incoming string param; there is no separate ~keep
        // `body_json` let-binding (emitting a bare `body_json` reference without a binding was the ~keep
        // old behaviour that produced `cannot find value` errors — E0425). ~keep
        assert!(
            code.contains(
                "Ok(test_lib::Payload::Doc { body: serde_json::from_str(&body).unwrap_or_default() }.into())"
            ),
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
            serde_content: None,
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
            serde_content: None,
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
            serde_content: None,
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
            serde_content: None,
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(!code.contains("_factory_empty"), "{code}");
        assert!(!code.contains("_factory_pair"), "{code}");
        assert!(!code.contains("_factory_hidden"), "{code}");
        assert!(code.contains("pub fn _factory_real(value: String) -> Self"), "{code}");
        assert!(code.contains("test_lib::Mixed::Real { value }.into()"), "{code}");
    }

    /// Regression for the `ContentPart` bug: a hand-written inherent static method
    /// (`enum_def.methods`, extracted from a separate `impl EnumType { .. }` block) is never
    /// forwarded into the generated `#[php_impl]` block, so suppressing the derived factory on a
    /// name collision used to drop the constructor entirely (`ContentPart.text(...)` was
    /// unreachable from PHP). Every data-carrying variant must always get a reachable factory.
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
        let code = run(&def, &mapper());
        assert!(
            code.contains("test_lib::Shape::Circle { radius }.into()"),
            "Circle factory must stay reachable despite the colliding hand-written method: {code}"
        );
        assert!(code.contains(r#"#[php(name = "circle")]"#), "{code}");
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
            serde_content: None,
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let empty = AHashSet::new();
        let code = gen_flat_data_enum_variant_constructors(&def, &mapper(), &empty, &empty, &empty, "crate");
        assert!(code.is_empty(), "no constructors for unit-only enum: {code:?}");
    }

    /// Extract the exact set of `_factory_<snake>` names a rendered constructor set declares,
    /// parsed from `pub fn _factory_<name>(`, shape-agnostic (works whether the renderer puts one
    /// method per line or several).
    fn factory_names(code: &[String]) -> std::collections::BTreeSet<String> {
        let joined = code.join("\n");
        joined
            .split("pub fn _factory_")
            .skip(1)
            .filter_map(|rest| rest.split('(').next())
            .map(|name| name.trim().to_string())
            .collect()
    }

    fn cfg_shape_enum(cfg: &str) -> EnumDef {
        EnumDef {
            name: "Shape".to_string(),
            rust_path: "dep_crate::Shape".to_string(),
            variants: vec![
                variant("Circle", vec![field("radius", TypeRef::Primitive(PrimitiveType::F64))]),
                EnumVariant {
                    cfg: Some(cfg.to_string()),
                    ..variant(
                        "Rect",
                        vec![
                            field("width", TypeRef::Primitive(PrimitiveType::F64)),
                            field("height", TypeRef::Primitive(PrimitiveType::F64)),
                        ],
                    )
                },
                variant("Point", vec![field("value", TypeRef::Primitive(PrimitiveType::F64))]),
            ],
            serde_content: None,
            serde_tag: Some("type".to_string()),
            ..Default::default()
        }
    }

    /// The factory body builds `<core_path>::<Variant> { .. }` directly (asserted above by
    /// `emits_static_constructor_building_core_variant_then_into`): a FOREIGN variant behind a
    /// `#[cfg(...)]` this crate cannot declare as its own Cargo feature has no compile-safe
    /// fallback, so its factory must be dropped entirely, not merely left referencing a variant the
    /// dependency may not have compiled in.
    #[test]
    fn drops_factory_for_foreign_cfg_gated_variant() {
        let def = cfg_shape_enum(r#"feature = "extra-shapes""#);
        let empty = AHashSet::new();
        // core_import ("crate") does not prefix-match rust_path ("dep_crate::Shape"), so the enum
        // is classified FOREIGN.
        let code = gen_flat_data_enum_variant_constructors(&def, &mapper(), &empty, &empty, &empty, "crate");

        assert_eq!(
            factory_names(&code),
            ["circle", "point"].into_iter().map(String::from).collect(),
            "the foreign cfg-gated `rect` factory must be dropped, leaving exactly the two \
             unconditional variants:\n{code:?}"
        );
    }

    /// Control: the identical `#[cfg(...)]` gate on a HOST-owned enum must never be dropped --
    /// `enum_variant_declaration`'s authority never resolves a host-owned gate to `Drop`, and this
    /// helper mirrors that for factory reachability too.
    #[test]
    fn keeps_factory_for_host_owned_cfg_gated_variant() {
        let mut def = cfg_shape_enum(r#"feature = "extra-shapes""#);
        def.rust_path = "crate::Shape".to_string();
        let empty = AHashSet::new();
        let code = gen_flat_data_enum_variant_constructors(&def, &mapper(), &empty, &empty, &empty, "crate");

        assert_eq!(
            factory_names(&code),
            ["circle", "point", "rect"].into_iter().map(String::from).collect(),
            "a host-owned cfg-gated variant's factory must stay unconditionally reachable:\n{code:?}"
        );
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
    use super::super::{gen_flat_data_enum, gen_flat_data_enum_from_impls, gen_flat_data_enum_methods};
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant};
    use ahash::AHashSet;

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
            serde_content: None,
            serde_tag: serde_tag.map(|s| s.to_string()),
            variants,
            excluded_variants,
            ..Default::default()
        }
    }

    #[test]
    fn flat_enum_with_default_variant_emits_default_fallback() {
        let enum_def = make_enum("Message", Some("role"), true, false);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate", None);

        // When the enum has a #[default] variant, should emit `_ => CorePath::default()`
        assert!(
            generated.contains("_ => module::Message::default()"),
            "Should emit default() fallback for enum with #[default] variant; got:\n{generated}"
        );
    }

    #[test]
    fn flat_enum_without_default_and_with_excluded_emits_unreachable() {
        let enum_def = make_enum("Message", Some("role"), false, true);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate", None);

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
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate", None);

        assert!(
            generated.contains("_ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\")"),
            "Should emit unreachable!() wildcard for &str match when core has no visible Default; got:\n{generated}"
        );
        assert!(
            !generated.contains("_ => module::SimpleEnum::default()"),
            "Should NOT emit default() fallback when core type has no visible Default impl; got:\n{generated}"
        );
    }

    #[test]
    fn flat_enum_tag_is_read_only_and_json_is_allowlisted() {
        let enum_def = make_enum("Message", Some("kind"), false, false);
        let mapper = PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::from_iter(["Message".to_string()]),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        };
        let generated_struct = gen_flat_data_enum(&enum_def, &mapper, None);
        let empty = AHashSet::new();
        let generated_methods = gen_flat_data_enum_methods(&enum_def, &mapper, &empty, &empty, &empty, "crate", None);

        assert!(
            !generated_struct.contains("#[php(prop"),
            "tag must not be writable from PHP"
        );
        assert!(
            generated_struct.contains("kind_tag: String"),
            "tag remains available to generated conversions"
        );
        assert!(
            generated_methods.contains("matches!(value.kind_tag.as_str(),"),
            "{generated_methods}"
        );
        assert!(
            generated_methods.contains("\"Variant1\" | \"Variant2\""),
            "{generated_methods}"
        );
        assert!(generated_methods.contains("return Err(PhpException::from_message(format!("));
    }

    /// The regression this task fixes: neither direction of the flat-enum `From` impl ever read
    /// `variant.cfg`, so a cfg-gated variant's match arm was always emitted unconditionally --
    /// an `E0599: no variant found` when the variant does not exist in the build. A host-owned
    /// cfg-gated variant (`rust_path` rooted in the same crate as `core_import`) must keep its
    /// arm in both directions, now carrying its `#[cfg(...)]` guard exactly once per direction.
    ///
    /// A second, later regression: the `From<CoreType> for Binding` direction unconditionally
    /// added `_ => Default::default()` whenever ANY variant carried a cfg, host-owned or not. A
    /// host-owned variant's arm carries the same `#[cfg(...)]` as the variant declaration itself,
    /// so the two always compile in or out together -- the catch-all is unreachable and trips
    /// `-D warnings`' `unreachable_patterns` once the gating feature is active (the default once
    /// cfg features are forwarded, alef #464).
    #[test]
    fn host_cfg_variant_keeps_its_arm_and_gains_a_cfg_guard_in_both_directions() {
        let mut enum_def = make_enum("Message", Some("kind"), false, false);
        enum_def.rust_path = "core_lib::Message".to_string();
        enum_def.variants[1].cfg = Some(r#"feature = "thumbnails""#.to_string());
        let generated = gen_flat_data_enum_from_impls(&enum_def, "core_lib", None);

        assert!(
            generated.contains("core_lib::Message::Variant2 =>"),
            "host-owned variant's From<CoreType> arm must still be emitted, got:\n{generated}"
        );
        assert!(
            generated.contains("\"Variant2\" => core_lib::Message::Variant2,"),
            "host-owned variant's From<Binding> arm must still be emitted, got:\n{generated}"
        );
        assert_eq!(
            generated.matches("#[cfg(feature = \"thumbnails\")]").count(),
            2,
            "the gate must land on both directions' arms exactly once each, got:\n{generated}"
        );
        assert!(
            !generated.contains("_ => Default::default(),"),
            "a host-owned cfg-gated variant must not trigger a catch-all (unreachable pattern \
             under -D warnings), got:\n{generated}"
        );
    }

    /// Same regression, the foreign-crate half: a variant merged in from a foreign
    /// `[[crates.source_crates]]` crate (`rust_path` rooted in a crate other than `core_import`)
    /// carries that crate's own cfg. Forwarding it verbatim as `#[cfg(...)]` names a feature this
    /// PHP crate never declares -- an `unexpected cfg condition value` error -- so the arm must be
    /// dropped entirely in both directions instead.
    #[test]
    fn foreign_cfg_variant_arm_is_dropped_not_gated_in_both_directions() {
        let mut enum_def = make_enum("Message", Some("kind"), false, false);
        enum_def.rust_path = "dep_crate::Message".to_string();
        enum_def.variants[1].cfg = Some(r#"feature = "testkit""#.to_string());
        let generated = gen_flat_data_enum_from_impls(&enum_def, "core_lib", None);

        assert!(
            !generated.contains("#[cfg(feature = \"testkit\")]"),
            "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{generated}"
        );
        assert!(
            !generated.contains("Message::Variant2 =>") && !generated.contains("\"Variant2\" =>"),
            "a foreign-crate cfg-gated variant must not be referenced in either direction, got:\n{generated}"
        );
        assert!(
            generated.contains("_ => Default::default(),"),
            "dropping the arm must still leave the core→binding match exhaustive via the catch-all, got:\n{generated}"
        );
    }

    /// Negative control: an ungated enum (`make_enum`'s variants, `cfg: None`) must emit no
    /// `#[cfg(...)]` at all.
    #[test]
    fn ungated_enum_emits_no_cfg_in_either_direction() {
        let enum_def = make_enum("Message", Some("kind"), false, false);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "core_lib", None);
        assert!(
            !generated.contains("#[cfg("),
            "ungated enum must not emit #[cfg(...)], got:\n{generated}"
        );
    }
}
