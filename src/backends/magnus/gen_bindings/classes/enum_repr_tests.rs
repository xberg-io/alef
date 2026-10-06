//! Magnus enum representation and variant constructor coverage, split from tests.rs.

use super::tests::*;
use super::*;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};

#[test]
fn gen_enum_emits_adjacent_serde_representation() {
    let mut enum_def = make_data_enum("OperationResult", Some("type"));
    enum_def.serde_content = Some("output".to_string());
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].fields[0].name = "_0".to_string();

    let code = gen_enum(&enum_def, "test_lib", None, &[]);

    assert!(code.contains(r#"#[serde(tag = "type", content = "output")]"#));
    assert!(code.contains("Jpeg(String)"));
    assert!(code.contains("Self::Jpeg(_0) => Some(_0)"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0 }"), "{code}");
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

#[test]
fn adjacent_tuple_default_uses_tuple_constructor_syntax() {
    let mut enum_def = make_data_enum("OperationResult", Some("type"));
    enum_def.has_default = true;
    enum_def.serde_content = Some("output".to_string());
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].is_default = true;
    enum_def.variants[1].fields[0].name = "_0".to_string();

    let code = gen_enum(&enum_def, "test_lib", None, &[]);

    assert!(code.contains("Self::Jpeg(Default::default())"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0:"), "{code}");
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

/// `EntityCategory`/`OutputFormat`/`PiiCategory` (real xberg consumer enums) are
/// externally-tagged (`#[serde(rename_all = "snake_case")]`, no `tag`/`content`, not
/// `#[serde(untagged)]`) unit variants plus one newtype `Custom(String)` variant. A tuple
/// variant is modeled in the IR with BOTH `is_tuple: true` AND fields named `_0`, `_1`, ... --
/// set together by the extractor -- so this fixture sets both, matching an IR state the
/// extractor can actually produce (a fixture setting only the field name would model a
/// named-fields variant whose field happens to be called `_0`, a shape serde never emits).
///
/// Regression: this used to emit STRUCT form for the newtype variant (`Custom { _0: String }`),
/// so `serde_json::to_value` produced `{"custom": {"_0": "foo"}}` -- an extra level of nesting
/// versus core's own wire `{"custom": "foo"}` (confirmed against
/// `crates/xberg/src/types/entity.rs`'s `EntityCategory::Custom(String)` in the consumer repo),
/// with `_0` as the most visible symptom. `variant_emits_tuple_form`
/// (`codegen/conversions/helpers/eligibility.rs`) now emits TUPLE form for externally-tagged
/// newtype variants too, matching core's wire exactly with no field name on the wire at all.
#[test]
fn gen_enum_emits_tuple_form_matching_core_wire_for_externally_tagged_newtype_variant() {
    let mut enum_def = make_data_enum("EntityCategory", None);
    enum_def.variants[1].name = "Custom".to_string();
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].fields[0].name = "_0".to_string();

    let code = gen_enum(&enum_def, "test_lib", None, &[]);

    assert!(
        code.contains("Custom(String)"),
        "a single-field newtype variant under default (externally-tagged) serde representation \
         must emit tuple form, matching core's own `Custom(String)` declaration and its \
         `{{\"custom\": \"foo\"}}` wire -- struct form would add a nesting level core's wire \
         does not have:\n{code}"
    );
    assert!(
        !code.contains("_0"),
        "tuple form carries no field name at all, so the synthesized positional name `_0` must \
         not reach the generated code in any form:\n{code}"
    );
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

/// The one shape `positional_field_serde_rename` still guards after the predicate widening
/// above: an INTERNALLY-tagged (`tag` set, no `content`) newtype variant whose payload type is
/// `Named` and serde would flatten at runtime, but whose type definition is not present in the
/// `types` slice `gen_enum` received (passing `&[]` here, as if the payload type's `TypeDef`
/// were simply unreachable from this call site). `serde_flattens_newtype_payload` then falls
/// back to `false`, so this generator keeps struct form -- correctly, since internally-tagged
/// newtype variants have no positional slot for tuple form to fill -- and the field still needs
/// a semantic rename rather than the bare synthesized `_0`. Mirrors
/// `backends::kotlin::gen_bindings::shared::kotlin_field_name_with_type`'s type-derived naming.
#[test]
fn gen_enum_renames_positional_field_for_internally_tagged_variant_with_unreachable_payload_type() {
    let mut enum_def = make_data_enum("DocumentSource", Some("type"));
    enum_def.variants[1].name = "Pdf".to_string();
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].fields[0] = make_field("_0", TypeRef::Named("PdfMetadata".to_string()), false);

    let code = gen_enum(&enum_def, "test_lib", None, &[]);

    assert!(
        code.contains("Pdf { "),
        "internally-tagged must stay struct form even though the variant is a tuple in the IR:\n{code}"
    );
    assert!(
        code.contains(r#"#[serde(rename = "metadata")] _0: PdfMetadata"#),
        "a `Pdf(PdfMetadata)` payload should derive `metadata`, not the generic `value`:\n{code}"
    );
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

/// A multi-field tuple variant (`_0`, `_1`, ...) under externally-tagged representation now also
/// gets tuple form from the widened predicate, so neither field name reaches the wire at all --
/// the fix for the historical `_1`-never-renamed gap (`tagged_enums.rs` matched only the literal
/// `"_0"`, never `"_1"`) is that tuple form carries no field names to begin with.
#[test]
fn gen_enum_emits_tuple_form_for_multi_field_externally_tagged_variant() {
    let mut enum_def = make_data_enum("Pair", None);
    enum_def.variants[1].name = "Range".to_string();
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].fields = vec![
        make_field("_0", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32), false),
        make_field("_1", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32), false),
    ];

    let code = gen_enum(&enum_def, "test_lib", None, &[]);

    assert!(
        code.contains("Range(u32, u32)"),
        "a multi-field tuple variant under externally-tagged representation must emit tuple \
         form:\n{code}"
    );
    assert!(
        !code.contains("_0") && !code.contains("_1"),
        "tuple form carries no field names, so neither synthesized positional name may reach \
         the generated code:\n{code}"
    );
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

#[test]
fn gen_struct_redacts_sensitive_fields_in_debug() {
    let mut secret = make_field("token", TypeRef::String, false);
    secret.sensitive = true;
    let typ = make_typedef("Credentials", vec![make_field("label", TypeRef::String, false), secret]);
    let mapper = crate::backends::magnus::type_map::MagnusMapper;

    let code = gen_struct(&typ, &mapper, "TestLib", "test_lib", false, &[], false);

    let derive_line = code.lines().find(|line| line.starts_with("#[derive(")).unwrap();
    assert!(!derive_line.contains("Debug"), "{code}");
    assert!(code.contains("impl std::fmt::Debug for Credentials"), "{code}");
    assert!(code.contains(".field(\"token\", &\"<redacted>\")"), "{code}");
    assert!(code.contains(".field(\"label\", &self.label)"), "{code}");
    crate::codegen::generators::sensitive_runtime_test_support::assert_struct_debug_and_serde_round_trip(
        &code,
        "Credentials",
    );
}

#[test]
fn gen_enum_redacts_sensitive_payload_debug() {
    let mut token = make_field("token", TypeRef::String, false);
    token.sensitive = true;
    let def = EnumDef {
        name: "Auth".into(),
        variants: vec![EnumVariant {
            name: "Bearer".into(),
            fields: vec![token],
            ..Default::default()
        }],
        ..Default::default()
    };

    let code = gen_enum(&def, "test_lib", None, &[]);

    let derive_line = code.lines().find(|line| line.starts_with("#[derive(")).unwrap();
    assert!(!derive_line.contains("Debug"), "{code}");
    assert!(code.contains("impl std::fmt::Debug for Auth"), "{code}");
    assert!(code.contains("Auth(<redacted>)"), "{code}");
    crate::codegen::generators::sensitive_runtime_test_support::assert_enum_debug_and_serde_round_trip(&code, "Auth");
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
    let code = gen_enum(&adjacent, "test_lib", None, &[]);
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
