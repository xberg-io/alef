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

/// Coverage for the declaration-surface parity fix: `enum_constant_entries` (the class-constants
/// declaration) and `gen_flat_data_enum_methods`'s `allowed_tags` (the flat-enum `from_json`
/// validation list) must both defer to `codegen::conversions::enum_variant_declaration` -- the
/// same authority the napi backend's `gen_enum` already consults -- so a FOREIGN cfg-gated
/// variant this binding's configured feature set proves unreachable is never advertised as
/// constructible, while a host-owned cfg-gated variant (or a foreign one that is merely unproven,
/// not proven absent) keeps its prior unconditional behavior. ~keep
#[cfg(test)]
mod enum_declaration_parity_tests {
    use super::super::{enum_constant_entries, gen_enum_constants, gen_flat_data_enum_methods};
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant};
    use ahash::AHashSet;
    use std::collections::HashSet;

    const HOST_CRATE: &str = "hostlib";

    fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    /// `Base` is always ungated; `Extra` carries `cfg`. `owner` selects whether the whole enum
    /// (and therefore `Extra`'s cfg) is host- or foreign-owned relative to [`HOST_CRATE`].
    fn sample_enum(owner: &str, cfg: &str) -> EnumDef {
        EnumDef {
            name: "SampleMode".to_string(),
            rust_path: format!("{owner}::SampleMode"),
            serde_tag: Some("kind".to_string()),
            variants: vec![unit_variant("Base", None), unit_variant("Extra", Some(cfg))],
            ..Default::default()
        }
    }

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    /// A foreign-owned `Extra` behind a feature absent from `configured_features` is proven
    /// unreachable -- neither the class constant nor the flat-enum `allowed_tags` entry may
    /// advertise it, since the binding->core conversion can never produce it either.
    #[test]
    fn foreign_variant_proven_unreachable_is_absent_from_both_declaration_surfaces() {
        let en = sample_enum("dep_crate", r#"feature = "testkit""#);
        let is_host_enum = false;
        let configured: HashSet<&str> = HashSet::new();

        let entries = enum_constant_entries(&en, is_host_enum, Some(&configured));
        assert_eq!(
            entries,
            vec![("BASE".to_string(), "Base".to_string())],
            "the proven-unreachable foreign variant must be dropped, got: {entries:?}"
        );
        let constants = gen_enum_constants(&en, None, is_host_enum, Some(&configured));
        assert!(!constants.contains("EXTRA"), "got:\n{constants}");

        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&en, &mapper(), &empty, &empty, &empty, HOST_CRATE, Some(&configured));
        assert!(
            methods.contains("matches!(value.kind_tag.as_str(), \"Base\" )"),
            "allowed_tags must list only the reachable variant, got:\n{methods}"
        );
        assert!(!methods.contains("\"Extra\""), "got:\n{methods}");
    }

    /// The same foreign `Extra`, but `configured_features` is `None` ("unknown" -- alef's static
    /// read cannot prove the feature off, since Cargo feature unification could still enable it).
    /// Both declaration surfaces must keep advertising it, unchanged from before this fix.
    #[test]
    fn foreign_variant_not_proven_unreachable_stays_on_both_declaration_surfaces() {
        let en = sample_enum("dep_crate", r#"feature = "testkit""#);
        let is_host_enum = false;

        let entries = enum_constant_entries(&en, is_host_enum, None);
        assert_eq!(
            entries,
            vec![
                ("BASE".to_string(), "Base".to_string()),
                ("EXTRA".to_string(), "Extra".to_string()),
            ],
            "an unproven foreign variant must stay declared, got: {entries:?}"
        );

        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&en, &mapper(), &empty, &empty, &empty, HOST_CRATE, None);
        assert!(
            methods.contains("matches!(value.kind_tag.as_str(), \"Base\" | \"Extra\" )"),
            "got:\n{methods}"
        );
    }

    /// A host-owned cfg-gated `Extra` (the enum's `rust_path` is rooted in [`HOST_CRATE`] itself)
    /// must never be dropped by [`enum_variant_declaration`] -- existing behavior, unchanged by
    /// this fix -- regardless of what `configured_features` says.
    #[test]
    fn host_owned_cfg_gated_variant_stays_on_both_declaration_surfaces() {
        let en = sample_enum(HOST_CRATE, r#"feature = "extra_feature""#);
        let is_host_enum = true;
        let configured: HashSet<&str> = HashSet::new();

        let entries = enum_constant_entries(&en, is_host_enum, Some(&configured));
        assert_eq!(
            entries,
            vec![
                ("BASE".to_string(), "Base".to_string()),
                ("EXTRA".to_string(), "Extra".to_string()),
            ],
            "a host-owned cfg-gated variant must stay declared, got: {entries:?}"
        );

        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&en, &mapper(), &empty, &empty, &empty, HOST_CRATE, Some(&configured));
        assert!(
            methods.contains("matches!(value.kind_tag.as_str(), \"Base\" | \"Extra\" )"),
            "got:\n{methods}"
        );
    }
}

/// Coverage for the `Custom(String)` label-loss fix: an externally-tagged enum whose only data
/// variant is a single-field `String` tuple variant (`EntityCategory`/`PiiCategory`/`OutputFormat`
/// in the xberg core crate) must round-trip the caller-supplied label through a flat PHP class
/// instead of being flattened to a bare string constant that can only ever produce
/// `Custom(Default::default())`. Before this fix, `is_tagged_data_enum` required
/// `enum_def.serde_tag.is_some()`, so every assertion below that depends on the broadened
/// predicate failed: `entity_category()` routed through `enum_constant_entries` /
/// `gen_string_to_enum_expr` instead of `gen_flat_data_enum*`, and the generated binding→core
/// conversion literally read `"custom" => xberg::EntityCategory::Custom(Default::default())`. ~keep
#[cfg(test)]
mod labeled_string_enum_tests {
    use super::super::{
        gen_flat_data_enum_from_impls, gen_flat_data_enum_methods, is_labeled_string_enum, is_tagged_data_enum,
    };
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
    use ahash::AHashSet;

    fn unit(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// A single-field tuple variant (`Custom(String)`): tuple-variant fields are always named
    /// `_0`, `_1`, ... (see `codegen::conversions::is_tuple_variant`).
    fn label_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            // Real tuple variants carry `is_tuple: true` from extraction (this is what
            // `collect_all_variant_constructors` keys its "skip tuple variants" filter on --
            // separate from, but expected to agree with, the field-naming heuristic
            // `is_tuple_variant` derives from `_0`/`_1`/... names).
            is_tuple: true,
            ..Default::default()
        }
    }

    /// Mirrors `EntityCategory`/`PiiCategory`/`OutputFormat`: `#[serde(rename_all = "snake_case")]`
    /// (no `#[serde(tag = ...)]`, not `#[serde(untagged)]`), several unit variants, one `Custom(String)`.
    fn entity_category() -> EnumDef {
        EnumDef {
            name: "EntityCategory".to_string(),
            rust_path: "xberg::EntityCategory".to_string(),
            variants: vec![unit("Person"), unit("Organization"), label_variant("Custom")],
            serde_tag: None,
            serde_untagged: false,
            ..Default::default()
        }
    }

    fn mapper_with(data_enum_names: &[&str]) -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: data_enum_names.iter().map(|s| s.to_string()).collect(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    #[test]
    fn recognises_the_caller_supplied_label_shape() {
        assert!(
            is_labeled_string_enum(&entity_category()),
            "unit variants + one single-field String tuple variant, no serde tag, not untagged"
        );
    }

    #[test]
    fn rejects_an_internally_tagged_enum_already_handled_by_the_struct_variant_path() {
        let mut def = entity_category();
        def.serde_tag = Some("type".to_string());
        assert!(
            !is_labeled_string_enum(&def),
            "serde_tag.is_some() must take the existing path"
        );
    }

    #[test]
    fn rejects_an_untagged_enum_already_handled_by_the_json_value_path() {
        let mut def = entity_category();
        def.serde_untagged = true;
        assert!(
            !is_labeled_string_enum(&def),
            "serde_untagged must take the existing JSON-Value path"
        );
    }

    #[test]
    fn rejects_a_boxed_label_field() {
        let mut def = entity_category();
        def.variants[2].fields[0].is_boxed = true;
        assert!(
            !is_labeled_string_enum(&def),
            "a boxed field is outside the narrow, known-to-compile shape"
        );
    }

    #[test]
    fn rejects_a_multi_field_tuple_variant() {
        let mut def = entity_category();
        def.variants[2].fields.push(FieldDef {
            name: "_1".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        });
        assert!(
            !is_labeled_string_enum(&def),
            "a multi-field tuple variant is outside the narrow shape"
        );
    }

    #[test]
    fn rejects_a_non_string_tuple_field() {
        let mut def = entity_category();
        def.variants[2].fields[0].ty = TypeRef::Primitive(crate::core::ir::PrimitiveType::I64);
        assert!(
            !is_labeled_string_enum(&def),
            "only a bare String payload is in the narrow shape"
        );
    }

    #[test]
    fn rejects_a_struct_variant_with_no_label_variant() {
        // No data variant at all -- nothing for this predicate to lower.
        let def = EnumDef {
            name: "UnitOnly".to_string(),
            rust_path: "xberg::UnitOnly".to_string(),
            variants: vec![unit("A"), unit("B")],
            ..Default::default()
        };
        assert!(!is_labeled_string_enum(&def));
    }

    #[test]
    fn is_tagged_data_enum_now_true_for_a_labeled_string_enum() {
        assert!(
            is_tagged_data_enum(&entity_category()),
            "is_tagged_data_enum must fold in is_labeled_string_enum so every dispatch site \
             (struct-field type mapping, PHPStan stub, constants-vs-flat-class routing) treats \
             it as a flat class"
        );
    }

    /// The core regression check: the generated `From<Binding> for Core` conversion must
    /// construct `Custom(val.custom.unwrap_or_default())` (or equivalent), never
    /// `Custom(Default::default())` -- the literal expression the pre-fix string-enum path always
    /// produced regardless of what the PHP caller supplied.
    #[test]
    fn from_impls_never_default_the_label_away() {
        let def = entity_category();
        let generated = gen_flat_data_enum_from_impls(&def, "xberg", None);

        assert!(
            !generated.contains("Custom(Default::default())"),
            "the label must never be discarded, got:\n{generated}"
        );
        assert!(
            generated.contains("xberg::EntityCategory::Custom(") && generated.contains("val.custom"),
            "binding→core must read the flat class's `custom` field into the tuple variant, got:\n{generated}"
        );
        assert!(
            generated.contains("xberg::EntityCategory::Custom(_0) => Self {")
                || generated.contains("xberg::EntityCategory::Custom(_0)=>Self{"),
            "core→binding must bind the real tuple field to populate `custom`, got:\n{generated}"
        );
        assert!(
            generated.contains("custom: Some(_0.into())") || generated.contains("custom: Some(_0)"),
            "core→binding must carry the label into the flat class's `custom` field, got:\n{generated}"
        );
    }

    /// The usability half of the fix: PHP callers need a way to construct a `Custom` value (there
    /// is no wire tag to build one from besides `from_json`), and unit variants need the same
    /// factories the class constants they replace used to provide directly as values.
    #[test]
    fn variant_constructors_cover_both_unit_and_label_variants() {
        let def = entity_category();
        let mapper = mapper_with(&["EntityCategory"]);
        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&def, &mapper, &empty, &empty, &empty, "xberg", None);

        assert!(
            methods.contains("#[php(name = \"person\")]") && methods.contains("xberg::EntityCategory::Person.into()"),
            "a unit-variant factory must be generated, got:\n{methods}"
        );
        assert!(
            methods.contains("#[php(name = \"custom\")]")
                && methods.contains("pub fn _factory_custom(custom: String) -> Self {"),
            "the label-variant factory must accept the label as a plain String param, got:\n{methods}"
        );
        assert!(
            methods.contains("xberg::EntityCategory::Custom(custom).into()"),
            "the label-variant factory must forward the caller's value into the real core variant, got:\n{methods}"
        );
    }

    /// ext-php-rs 0.16 camel-cases method-backed property names by default. The PHPStan stub and
    /// generated e2e access both expose the flat enum's discriminator and payload under their
    /// snake_case names, so every getter must opt out of that new default explicitly. ~keep
    #[test]
    fn flat_enum_getters_preserve_the_snake_case_properties_declared_to_php_callers() {
        let def = entity_category();
        let mapper = mapper_with(&["EntityCategory"]);
        let empty = AHashSet::new();
        let methods = gen_flat_data_enum_methods(&def, &mapper, &empty, &empty, &empty, "xberg", None);

        assert!(
            methods.contains("#[php(getter, change_case = \"snake_case\")]\n    pub fn get_type_tag(&self) -> String"),
            "the discriminator getter must preserve the stub/e2e `$type_tag` property:\n{methods}"
        );
        assert!(
            methods.contains(
                "#[php(getter, change_case = \"snake_case\")]\n    pub fn get_custom(&self) -> Option<String>"
            ),
            "the payload getter must preserve the stub/e2e `$custom` property:\n{methods}"
        );
        assert!(
            !methods.contains("#[php(getter)]"),
            "a bare getter silently camel-cases multi-word properties on ext-php-rs 0.16:\n{methods}"
        );
    }
}

#[cfg(test)]
mod external_enum_serde_tests {
    use super::super::{gen_external_enum_serde_impls, gen_flat_data_enum};
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
    use ahash::AHashSet;

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::from_iter(["OutputFormat".to_string()]),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    /// `xberg::OutputFormat`: externally tagged, `rename_all = "lowercase"`, unit variants plus one
    /// `#[serde(untagged)] Custom(String)`.
    fn output_format() -> EnumDef {
        EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: "crate::OutputFormat".to_string(),
            serde_rename_all: Some("lowercase".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Plain".to_string(),
                    is_default: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Markdown".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    is_tuple: true,
                    serde_untagged: true,
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// Same shape without the `#[serde(untagged)]` marker: serde writes the data variant as a
    /// single-keyed object whose value IS the payload, and an unknown bare string is an error.
    fn keyed_label_enum() -> EnumDef {
        let mut def = output_format();
        def.name = "EntityCategory".to_string();
        def.rust_path = "crate::EntityCategory".to_string();
        def.variants[2].serde_untagged = false;
        def
    }

    #[test]
    fn labeled_string_enum_struct_drops_the_derived_serde_impls() {
        let generated = gen_flat_data_enum(&output_format(), &mapper(), None);
        assert!(
            generated.contains("#[derive(Clone, Default)]") && !generated.contains("#[derive(Clone, Default, serde::"),
            "the derived impl writes {{\"type\":\"markdown\"}}, which is not serde's external wire; got:\n{generated}"
        );
        assert!(
            !generated.contains("#[serde("),
            "field-level serde attributes do not compile without a serde derive on the struct; got:\n{generated}"
        );
        assert!(
            generated.contains("type_tag: String"),
            "the tag field itself stays, generated conversions read it; got:\n{generated}"
        );
    }

    #[test]
    fn unit_variant_round_trips_as_a_bare_string() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            generated.contains(r#""markdown" => serializer.serialize_str("markdown")"#),
            "{generated}"
        );
        assert!(
            generated.contains(r#""markdown" => Ok(OutputFormat {"#),
            "visit_str must accept the bare tag serde writes; got:\n{generated}"
        );
    }

    #[test]
    fn untagged_variant_round_trips_as_its_bare_payload() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            generated.contains(r#""custom" => serializer.serialize_str(self.custom.as_deref().unwrap_or_default())"#),
            "an untagged variant writes its payload with no tag wrapper; got:\n{generated}"
        );
        assert!(
            generated.contains("custom: Some(value.to_string())"),
            "an unrecognised bare string IS the untagged variant's payload, not an error; got:\n{generated}"
        );
        assert!(
            generated.contains("value => Ok(OutputFormat {"),
            "with an untagged fallback the catch-all constructs it rather than erroring; got:\n{generated}"
        );
    }

    #[test]
    fn keyed_label_variant_round_trips_as_a_single_keyed_object() {
        let generated = gen_external_enum_serde_impls(&keyed_label_enum());
        assert!(
            generated.contains(r#"map.serialize_entry("custom", self.custom.as_deref().unwrap_or_default())"#),
            "a tagged newtype variant writes {{\"custom\": payload}}; got:\n{generated}"
        );
        assert!(
            generated.contains(
                r#"value => Err(serde::de::Error::unknown_variant(value, &["plain", "markdown", "custom"]))"#
            ),
            "without an untagged fallback an unrecognised bare string is an error; got:\n{generated}"
        );
    }

    #[test]
    fn the_empty_default_tag_round_trips_rather_than_becoming_the_untagged_payload() {
        let generated = gen_external_enum_serde_impls(&output_format());
        let empty_arm = generated
            .find(r#""" => Ok(OutputFormat::default())"#)
            .unwrap_or_else(|| panic!("no empty-tag arm; got:\n{generated}"));
        let untagged_arm = generated
            .find("value => Ok(OutputFormat {")
            .unwrap_or_else(|| panic!("no untagged catch-all; got:\n{generated}"));
        assert!(
            empty_arm < untagged_arm,
            "`Default::default()` leaves the tag empty and serializes as a bare `\"\"`; the \
             untagged catch-all would otherwise read it back as Custom(\"\") and break the \
             round trip; got:\n{generated}"
        );
    }

    #[test]
    fn constructed_values_list_every_field_rather_than_using_update_syntax() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            !generated.contains("..Default::default()"),
            "`..Default::default()` on an arm that already sets every field trips \
             clippy::needless_update, and the generated crate compiles under -D warnings; got:\n{generated}"
        );
        assert!(
            generated.contains(r#""markdown" => Ok(OutputFormat { type_tag: "markdown".to_string(), custom: None })"#),
            "a unit variant must still null the label field explicitly; got:\n{generated}"
        );
    }

    #[test]
    fn map_form_still_reads_the_classes_own_flat_shape() {
        let generated = gen_external_enum_serde_impls(&output_format());
        assert!(
            generated.contains(r#"entries.iter().any(|(key, _)| key == "type")"#),
            "the flat {{\"type\":..}} form this class used to emit must keep deserialising; got:\n{generated}"
        );
    }
}
