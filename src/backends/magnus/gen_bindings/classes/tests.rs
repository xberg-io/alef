use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional,
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
    }
}

fn make_typedef(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn explicit_default_impl_preserves_serde_default_fn_instead_of_type_zero_value() {
    // Regression: this generator exists *because* a struct has field-level defaults that differ
    // from the derived Default, yet it called the context-free `default_value_for_field` and so
    // emitted `Default::default()` for `#[serde(default = "path")]` fields — the exact value it
    // was written to avoid. Against html-to-markdown's real `GridCell` that shipped
    // `GridCell.default().row_span == 0` while `default_span()` returns 1, and the kwargs
    // constructor in the same generated file returned the correct 1: two different defaults for
    // one field. ~keep
    let mut span = make_field(
        "row_span",
        TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
        false,
    );
    span.typed_default = Some(crate::core::ir::DefaultValue::FunctionCall("default_span".to_string()));

    let mut typ = make_typedef(
        "GridCell",
        vec![
            make_field("content", TypeRef::String, false),
            make_field("row", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32), false),
            span,
        ],
    );
    typ.has_serde = true;

    let map_fn = |ty: &TypeRef| match ty {
        TypeRef::String => "String".to_string(),
        _ => "u32".to_string(),
    };

    let output = gen_struct_default_impl_explicit(&typ, &map_fn, &[], &std::collections::HashSet::default())
        .expect("a struct with a field-level default must get an explicit Default impl");

    assert!(
        !output.contains("row_span: Default::default()"),
        "row_span must not fall back to the type's zero value, which is not `default_span()`:\n{output}"
    );
    assert!(
        output.contains("serde_json::from_str::<test_lib::GridCell>"),
        "row_span must recover the real serde default by deserializing a stub:\n{output}"
    );
}

/// Regression: a struct with one field carrying a real default (`element_id`, via
/// `#[serde(default)]`) and a second, unrelated *required* field of a `Named` type that has no
/// `Default` impl of its own (mirrors `xberg::Element.metadata: ElementMetadata`, where
/// `ElementMetadata` derives no `Default`). Before the fix, `has_non_trivial_default` being true
/// because of `element_id` alone was enough to synthesize a whole-struct Default impl, and the
/// untyped fallback table blindly emitted `ElementMetadata::default()` for the `metadata` field —
/// code that fails to compile with "no function or associated item named `default` found for
/// struct `ElementMetadata`" because `ElementMetadata` never implements `Default`. The fix must
/// recognize this and skip the whole impl rather than emit a call the type cannot satisfy.
#[test]
fn gen_struct_default_impl_explicit_skips_struct_with_undefaultable_required_field() {
    let mut element_id = make_field("element_id", TypeRef::String, false);
    element_id.typed_default = Some(crate::core::ir::DefaultValue::Empty);

    let typ = make_typedef(
        "Element",
        vec![
            element_id,
            make_field("metadata", TypeRef::Named("ElementMetadata".to_string()), false),
        ],
    );

    let map_fn = |ty: &TypeRef| match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Named(name) => name.clone(),
        _ => "()".to_string(),
    };

    // `ElementMetadata` is deliberately absent from `types_with_default`: it has no `Default`
    // impl in the source, matching the real xberg struct.
    let output = gen_struct_default_impl_explicit(&typ, &map_fn, &[], &std::collections::HashSet::default());
    assert!(
        output.is_none(),
        "a struct with a required field whose type has no Default must not get a synthesized \
         Default impl: {output:?}"
    );
}

/// Positive control for the above: same shape, but `ElementMetadata` IS in `types_with_default`
/// (it derives `Default` in this scenario). The Default impl must still be generated, and the
/// `metadata` field must still call `ElementMetadata::default()` — proving the fix is a real
/// per-type lookup, not a blanket refusal to ever call `{Type}::default()`.
#[test]
fn gen_struct_default_impl_explicit_still_calls_default_when_field_type_has_one() {
    let mut element_id = make_field("element_id", TypeRef::String, false);
    element_id.typed_default = Some(crate::core::ir::DefaultValue::Empty);

    let typ = make_typedef(
        "Element",
        vec![
            element_id,
            make_field("metadata", TypeRef::Named("ElementMetadata".to_string()), false),
        ],
    );

    let map_fn = |ty: &TypeRef| match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Named(name) => name.clone(),
        _ => "()".to_string(),
    };

    let mut types_with_default = std::collections::HashSet::default();
    types_with_default.insert("ElementMetadata");

    let output = gen_struct_default_impl_explicit(&typ, &map_fn, &[], &types_with_default)
        .expect("a struct whose required field type has Default must still get an impl");
    assert!(
        output.contains("metadata: ElementMetadata::default()"),
        "metadata must still call the field type's own default(): {output}"
    );
}

#[test]
fn gen_enum_unit_variants_emit_ruby_symbols() {
    let enum_def = EnumDef {
        name: "Status".to_string(),
        rust_path: "test_lib::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Pending".to_string(),
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
                name: "Done".to_string(),
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
    let code = gen_enum(&enum_def, "test_lib", None);
    assert!(code.contains("enum Status"), "must emit enum definition");
    assert!(code.contains("to_symbol"), "unit enums use Ruby symbols");
    assert!(
        code.contains("Status::Pending => \"Pending\","),
        "no rename_all declared, so the IntoValue output is the verbatim wire value:\n{code}"
    );
}

/// Serde's real default (no `#[serde(rename_all = "...")]` on the enum) serializes unit variants
/// verbatim under their Rust name, e.g. `"KeyValue"` — not snake_cased. Regression for a Magnus
/// `IntoValue` impl that unconditionally snake_cased every unit variant regardless of the
/// enum's actual serde attributes, so a Ruby caller comparing the returned Symbol against a
/// real JSON payload (e.g. `"kind": "KeyValue"`) never matched.
#[test]
fn gen_enum_unit_variant_wire_value_is_verbatim_without_rename_all() {
    let enum_def = EnumDef {
        name: "DataNodeKind".to_string(),
        rust_path: "test_lib::DataNodeKind".to_string(),
        variants: vec![make_variant("KeyValue", vec![]), make_variant("Sequence", vec![])],
        ..Default::default()
    };
    let code = gen_enum(&enum_def, "test_lib", None);
    assert!(
        code.contains("DataNodeKind::KeyValue => \"KeyValue\","),
        "serde's real default (no rename_all) serializes unit variants verbatim, not snake_cased:\n{code}"
    );
    assert!(
        !code.contains("=> \"key_value\","),
        "must not fabricate a snake_case wire value the Rust enum never declared:\n{code}"
    );
}

/// Widening the `IntoValue` output to the real wire value must not narrow what `TryConvert`
/// accepts on input — existing consumer code passing the old always-snake_case symbol
/// (`:key_value`) must keep working alongside the new verbatim wire spelling (`"KeyValue"`).
#[test]
fn gen_enum_unit_variant_try_convert_still_accepts_the_legacy_snake_case_spelling() {
    let enum_def = EnumDef {
        name: "DataNodeKind".to_string(),
        rust_path: "test_lib::DataNodeKind".to_string(),
        variants: vec![make_variant("KeyValue", vec![])],
        ..Default::default()
    };
    let code = gen_enum(&enum_def, "test_lib", None);
    assert!(
        code.contains("\"key_value\""),
        "existing consumer code passing the old snake_case symbol must keep working:\n{code}"
    );
    assert!(
        code.contains("\"KeyValue\""),
        "the new verbatim wire value must also be accepted on input:\n{code}"
    );
}

fn make_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn make_data_enum(name: &str, serde_tag: Option<&str>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: vec![
            make_variant("Png", vec![]),
            make_variant("Jpeg", vec![make_field("quality", TypeRef::String, false)]),
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: serde_tag.map(str::to_string),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn gen_enum_wraps_string_for_internally_tagged_enum() {
    // For an internally-tagged enum (`#[serde(tag = "...")]`), serde cannot deserialize a bare
    let code = gen_enum(&make_data_enum("ImageOutputFormat", Some("type")), "test_lib", None);
    assert!(
        code.contains(r#".or_else(|_| serde_json::from_value(serde_json::json!({ "type": json_str })))"#),
        "expected tagged string wrap for internally-tagged enum: {code}"
    );
}

#[test]
fn gen_enum_keeps_bare_string_for_externally_tagged_enum() {
    // An externally-tagged data enum (no `#[serde(tag)]`) must not gain the tag-wrap branch.
    let code = gen_enum(&make_data_enum("ExternallyTagged", None), "test_lib", None);
    assert!(
        !code.contains("serde_json::from_value(serde_json::json!({"),
        "externally-tagged enum must not wrap the string in a tag object: {code}"
    );
    assert!(
        code.contains("serde_json::from_str(&json_str)"),
        "data enum must keep the from_str path: {code}"
    );
}

#[test]
fn gen_enum_emits_adjacent_serde_representation() {
    let mut enum_def = make_data_enum("OperationResult", Some("type"));
    enum_def.serde_content = Some("output".to_string());
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].fields[0].name = "_0".to_string();

    let code = gen_enum(&enum_def, "test_lib", None);

    assert!(code.contains(r#"#[serde(tag = "type", content = "output")]"#));
    assert!(code.contains("Jpeg(String)"));
    assert!(code.contains("Self::Jpeg(_0) => Some(_0)"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0 }"), "{code}");
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

#[test]
fn adjacent_tuple_default_uses_tuple_constructor_syntax() {
    let mut enum_def = make_data_enum("OperationResult", Some("type"));
    enum_def.serde_content = Some("output".to_string());
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].is_default = true;
    enum_def.variants[1].fields[0].name = "_0".to_string();

    let code = gen_enum(&enum_def, "test_lib", None);

    assert!(code.contains("Self::Jpeg(Default::default())"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0:"), "{code}");
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

#[test]
fn gen_struct_emits_magnus_wrap_attribute() {
    let typ = make_typedef("Config", vec![make_field("value", TypeRef::String, false)]);
    let mapper = crate::backends::magnus::type_map::MagnusMapper;
    let code = gen_struct(&typ, &mapper, "TestLib", "test_lib", false, &[], false);
    assert!(code.contains("magnus::wrap"), "struct must have magnus::wrap");
    assert!(code.contains("struct Config"), "must emit struct Config");
}

fn container_conversion() -> crate::core::ir::SerdeContainerConversion {
    crate::core::ir::SerdeContainerConversion {
        from: Some("(f64, f64)".to_string()),
        into: Some("(f64, f64)".to_string()),
        try_from: None,
        transparent: false,
    }
}

/// Wire-shape class: a two-field primitive pair (e.g. `Point { x, y }` with a positional-array
/// `#[serde(from/into)]`). Asserts on the actual rendered code, not an intermediate flag.
#[test]
fn gen_struct_delegates_deserialize_when_caller_confirms_eligibility() {
    let mut typ = make_typedef(
        "Point",
        vec![
            make_field("x", TypeRef::Primitive(crate::core::ir::PrimitiveType::F64), false),
            make_field("y", TypeRef::Primitive(crate::core::ir::PrimitiveType::F64), false),
        ],
    );
    typ.serde_container_conversion = container_conversion();
    let mapper = crate::backends::magnus::type_map::MagnusMapper;
    let code = gen_struct(&typ, &mapper, "TestLib", "test_lib", false, &[], true);

    let derive_line = code.lines().find(|l| l.trim_start().starts_with("#[derive(")).unwrap();
    assert!(
        !derive_line.contains("serde::Deserialize"),
        "derive must drop Deserialize when delegating: {derive_line}"
    );
    assert!(
        derive_line.contains("serde::Serialize"),
        "Serialize stays derived: {derive_line}"
    );
    assert!(
        code.contains("impl<'de> serde::Deserialize<'de> for Point {"),
        "expected a delegating Deserialize impl in: {code}"
    );
    assert!(
        code.contains("<test_lib::Point as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "delegating impl must read the core type: {code}"
    );
    // The pre-existing TryConvert bridge is untouched and picks up whichever Deserialize impl
    // the type has -- verify it still calls through to `Self`, not a hand-picked shape.
    assert!(code.contains("serde_json::from_str::<Point>(&json_str)"));
}

/// Caller did not confirm eligibility (e.g. no matching `From<core::Type>` this run) -- must
/// keep the ordinary derived, field-by-field `Deserialize` so the existing
/// `SerdeContainerConversionUnsupported` diagnostic keeps naming the real gap.
#[test]
fn gen_struct_keeps_derive_when_delegation_not_confirmed() {
    let mut typ = make_typedef(
        "Point",
        vec![
            make_field("x", TypeRef::Primitive(crate::core::ir::PrimitiveType::F64), false),
            make_field("y", TypeRef::Primitive(crate::core::ir::PrimitiveType::F64), false),
        ],
    );
    typ.serde_container_conversion = container_conversion();
    let mapper = crate::backends::magnus::type_map::MagnusMapper;
    let code = gen_struct(&typ, &mapper, "TestLib", "test_lib", false, &[], false);

    let derive_line = code.lines().find(|l| l.trim_start().starts_with("#[derive(")).unwrap();
    assert!(derive_line.contains("serde::Deserialize"), "{derive_line}");
    assert!(!code.contains("impl<'de> serde::Deserialize<'de> for Point"));
}

#[test]
fn gen_opaque_struct_emits_arc_inner() {
    let typ = make_typedef("Handle", vec![]);
    let code = gen_opaque_struct(&typ, "test_lib", "TestLib");
    assert!(code.contains("inner: Arc<"), "opaque struct must have Arc inner");
    assert!(code.contains("struct Handle"), "must emit struct Handle");
}

use crate::core::ir::MethodDef;

fn shape_enum() -> EnumDef {
    EnumDef {
        name: "Shape".to_string(),
        rust_path: "test_lib::Shape".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            make_variant("Circle", vec![make_field("radius", TypeRef::String, false)]),
            make_variant(
                "Rect",
                vec![
                    make_field("width", TypeRef::String, false),
                    make_field("height", TypeRef::String, false),
                ],
            ),
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn variant_constructors_emit_singleton_per_struct_variant() {
    let code = gen_data_enum_variant_constructors(&shape_enum(), "test_lib", None);

    assert!(code.contains("impl Shape {"), "must emit an impl block: {code}");
    assert!(
        code.contains("pub fn _factory_circle(radius: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Circle { radius }"), "{code}");
    assert!(
        code.contains("pub fn _factory_rect(width: String, height: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Rect { width, height }"), "{code}");
}

#[test]
fn variant_constructors_use_serde_shaped_named_field_type() {
    let def = EnumDef {
        name: "Wrapper".to_string(),
        rust_path: "test_lib::Wrapper".to_string(),
        original_rust_path: String::new(),
        variants: vec![make_variant(
            "Llm",
            vec![
                make_field("llm", TypeRef::Named("LlmConfig".to_string()), false),
                make_field(
                    "opts",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
            ],
        )],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let code = gen_data_enum_variant_constructors(&def, "test_lib", None);

    assert!(
        code.contains("pub fn _factory_llm(llm: LlmConfig, opts: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Llm { llm, opts }"), "{code}");
    assert!(
        !code.contains("_core"),
        "magnus enum is binding-shaped, no core conversion: {code}"
    );
}

#[test]
fn variant_constructors_skip_unit_tuple_and_excluded() {
    let mut tuple_variant = make_variant("Pair", vec![make_field("_0", TypeRef::String, false)]);
    tuple_variant.is_tuple = true;
    let mut excluded = make_variant("Hidden", vec![make_field("value", TypeRef::String, false)]);
    excluded.binding_excluded = true;

    let def = EnumDef {
        variants: vec![
            make_variant("Empty", vec![]),
            tuple_variant,
            excluded,
            make_variant("Real", vec![make_field("value", TypeRef::String, false)]),
        ],
        ..shape_enum()
    };

    let code = gen_data_enum_variant_constructors(&def, "test_lib", None);

    assert!(!code.contains("_factory_empty"), "{code}");
    assert!(!code.contains("_factory_pair"), "{code}");
    assert!(!code.contains("_factory_hidden"), "{code}");
    assert!(code.contains("pub fn _factory_real(value: String) -> Self"), "{code}");
}

/// Regression for the `ContentPart` bug: no backend forwards `enum_def.methods` (a hand-written
/// inherent static method extracted from a separate `impl EnumType { .. }` block) into the
/// generated Ruby bindings, so suppressing the derived factory on a name collision used to drop
/// the constructor entirely with nothing to replace it. Every data-carrying variant must always
/// get a reachable factory.
#[test]
fn variant_constructors_emit_factory_even_with_colliding_hand_written_method() {
    let def = EnumDef {
        methods: vec![MethodDef {
            name: "circle".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..shape_enum()
    };

    let code = gen_data_enum_variant_constructors(&def, "test_lib", None);

    assert!(
        code.contains("pub fn _factory_circle(radius: String) -> Self"),
        "Circle factory must stay reachable despite the colliding hand-written method: {code}"
    );
    assert!(code.contains("Self::Circle { radius }"), "{code}");
    assert!(
        code.contains("pub fn _factory_rect(width: String, height: String) -> Self"),
        "{code}"
    );
}

#[test]
fn variant_constructors_empty_for_unit_only_enum() {
    let def = EnumDef {
        variants: vec![make_variant("A", vec![]), make_variant("B", vec![])],
        ..shape_enum()
    };
    let code = gen_data_enum_variant_constructors(&def, "test_lib", None);
    assert!(code.is_empty(), "expected no output for unit-only enum: {code}");
}

fn cfg_shape_enum_with_rust_path(rust_path: &str) -> EnumDef {
    let mut gated = make_variant(
        "Rect",
        vec![
            make_field("width", TypeRef::String, false),
            make_field("height", TypeRef::String, false),
        ],
    );
    gated.cfg = Some(r#"feature = "extra-shapes""#.to_string());
    EnumDef {
        rust_path: rust_path.to_string(),
        variants: vec![
            make_variant("Circle", vec![make_field("radius", TypeRef::String, false)]),
            gated,
            make_variant("Point", vec![make_field("value", TypeRef::String, false)]),
        ],
        ..shape_enum()
    }
}

/// The factory builds `Self::<Variant> { .. }` against the SAME wrapper `enum` `gen_enum` declares
/// (see `declared_enum_variants`'s doc comment): a FOREIGN cfg-gated variant `gen_enum` already
/// drops must not leave a constructor still naming it -- that literal would be a hard `E0599`
/// against the (correctly) narrower wrapper `enum` `gen_enum` renders alongside it.
#[test]
fn variant_constructors_drop_foreign_cfg_gated_variant() {
    let def = cfg_shape_enum_with_rust_path("dep_crate::Shape");

    // `Some(&[])` is load-bearing, not `None`: a foreign cfg-gated variant is only DROPPED once
    // the configured feature set is known and provably excludes its gate. With `None` the
    // nuanced authority keeps it unconditionally on purpose -- feature unification could enable
    // a dependency's feature in a way alef's static read cannot observe. ~keep
    let code = gen_data_enum_variant_constructors(&def, "crate", Some(&[]));

    assert!(!code.contains("_factory_rect"), "{code}");
    assert!(
        code.contains("pub fn _factory_circle(radius: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("pub fn _factory_point(value: String) -> Self"), "{code}");
}

/// Control: the identical gate on a HOST-owned enum is never dropped -- `enum_variant_declaration`
/// never resolves a host-owned gate to `Drop`.
#[test]
fn variant_constructors_keep_host_owned_cfg_gated_variant() {
    let def = cfg_shape_enum_with_rust_path("crate::Shape");

    let code = gen_data_enum_variant_constructors(&def, "crate", None);

    assert!(
        code.contains("pub fn _factory_rect(width: String, height: String) -> Self"),
        "a host-owned cfg-gated variant's factory must stay reachable: {code}"
    );
}

/// The registration list feeding `method!(Shape::_factory_rect, ..)` must resolve the identical
/// FOREIGN/host verdict as the constructor generator above -- registering a path the constructor
/// no longer emits is a hard `E0599` at the registration site, not a missing Ruby method.
#[test]
fn variant_constructor_registrations_match_generated_constructors() {
    let foreign = cfg_shape_enum_with_rust_path("dep_crate::Shape");
    let registrations = data_enum_variant_constructor_registrations(&foreign, "crate", Some(&[]));
    let names: std::collections::BTreeSet<&str> = registrations
        .iter()
        .map(|(ruby_name, _, _)| ruby_name.as_str())
        .collect();

    assert_eq!(
        names,
        ["circle", "point"].into_iter().collect(),
        "registrations must exclude the dropped foreign cfg-gated `rect` variant: {registrations:?}"
    );
}

/// Issue #232: an adjacently-tagged enum (`tag` + `content`) emits tuple-form variants
/// exactly like an untagged one, but the conversion match arms keyed only on
/// `serde_untagged` and so destructured struct-form. Definition and `From` impls
/// disagreed in shape and rustc rejected them (E0559 / E0769). Both sides must now
/// consult the same predicate.
#[test]
fn adjacently_tagged_tuple_variant_uses_tuple_form_in_both_definition_and_conversions() {
    use crate::codegen::conversions::helpers::variant_emits_tuple_form;

    let mut adjacent = make_data_enum("OperationResult", Some("type"));
    adjacent.serde_content = Some("output".to_string());
    adjacent.variants[1].is_tuple = true;
    adjacent.variants[1].fields[0].name = "_0".to_string();

    // The definition emits tuple form ...
    let code = gen_enum(&adjacent, "test_lib", None);
    assert!(code.contains("Jpeg(String)"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0 }"), "{code}");

    // ... and the shared predicate agrees, so conversions destructure the same way.
    assert!(
        variant_emits_tuple_form(&adjacent, &adjacent.variants[1]),
        "adjacently-tagged tuple variant must report tuple form to the conversion layer"
    );

    // Untagged keeps working.
    let mut untagged = make_data_enum("OperationResult", None);
    untagged.serde_untagged = true;
    untagged.variants[1].is_tuple = true;
    untagged.variants[1].fields[0].name = "_0".to_string();
    assert!(variant_emits_tuple_form(&untagged, &untagged.variants[1]));

    // A non-tuple variant of an adjacently-tagged enum keeps struct form.
    assert!(
        !variant_emits_tuple_form(&adjacent, &adjacent.variants[0]),
        "struct-form variants must not be reported as tuple form"
    );
}

/// alef #102: an async method with no declared `error_type` still opens its delegable body with
/// `let rt = tokio::runtime::Builder::new_multi_thread()...build().map_err(...)?;` (building the
/// tokio runtime is itself fallible, independent of whether the core method's own return type
/// is `Result`). If the
/// annotation is keyed on `method.error_type.is_some()` instead of that fact, a no-error async
/// method gets a bare-`T` signature around a body that still has a `?` in it — rustc rejects the
/// generated Ruby extension with E0277. ~keep
fn async_method_without_error(name: &str, receiver: ReceiverKind) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type: TypeRef::String,
        is_async: true,
        error_type: None,
        receiver: Some(receiver),
        cfg: None,
        ..Default::default()
    }
}

#[test]
fn opaque_async_method_without_error_type_still_returns_result() {
    let typ = make_typedef("Widget", vec![]);
    let method = async_method_without_error("process", ReceiverKind::Ref);
    let mapper = MagnusMapper;
    let code = gen_opaque_async_instance_method(
        &typ,
        &method,
        &mapper,
        "Widget",
        &AHashSet::default(),
        &AHashSet::default(),
        "test_lib",
        false,
    );
    assert!(
        code.contains("fn process_async(&self, ) -> Result<String, Error> {"),
        "async opaque method must stay Result-shaped even without a declared error type, got: {code}"
    );
    assert!(
        code.contains("tokio::runtime::Builder::new_multi_thread()"),
        "delegable async body must build a runtime, got: {code}"
    );
    assert!(
        code.contains(".thread_stack_size(ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES)"),
        "delegable async body's runtime must set an explicit worker stack size, got: {code}"
    );
    assert!(
        !code.contains("tokio::runtime::Runtime::new()"),
        "delegable async body must not use tokio's default (undersized) worker stack, got: {code}"
    );
}

#[test]
fn non_opaque_async_method_without_error_type_still_returns_result() {
    let typ = make_typedef("Widget", vec![]);
    let method = async_method_without_error("process", ReceiverKind::Ref);
    let mapper = MagnusMapper;
    let code = gen_async_instance_method(&method, &mapper, &typ, &AHashSet::default(), "test_lib");
    assert!(
        code.contains("fn process_async(&self, ) -> Result<String, Error> {"),
        "async instance method must stay Result-shaped even without a declared error type, got: {code}"
    );
    assert!(
        code.contains("tokio::runtime::Builder::new_multi_thread()"),
        "delegable async body must build a runtime, got: {code}"
    );
    assert!(
        code.contains(".thread_stack_size(ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES)"),
        "delegable async body's runtime must set an explicit worker stack size, got: {code}"
    );
    assert!(
        !code.contains("tokio::runtime::Runtime::new()"),
        "delegable async body must not use tokio's default (undersized) worker stack, got: {code}"
    );
}
