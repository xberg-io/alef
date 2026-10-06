use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

#[test]
fn untagged_record_enum_retains_native_identity_and_typed_factories() {
    let def = EnumDef {
        name: "Record".into(),
        serde_untagged: true,
        variants: vec![
            EnumVariant {
                name: "Canonical".into(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("Payload".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Migration".into(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("Legacy".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let code = gen_enum_with_module(&def, "core", None, &[], "Example");
    assert!(code.contains("#[magnus::wrap(class = \"Example::Record\")]"), "{code}");
    assert!(code.contains("pub fn from_canonical(value: Payload) -> Self"), "{code}");
    assert!(code.contains("pub fn migration(&self) -> Option<Legacy>"), "{code}");
    assert!(
        !code.contains("json_to_ruby"),
        "records must not degrade into hashes: {code}"
    );
    assert!(
        code.find("<&Self as magnus::TryConvert>::try_convert").unwrap() < code.find("serde_json::from_str").unwrap(),
        "the wrapped instance must win over the serde reader, never the reverse: {code}"
    );
    syn::parse_file(&code).expect("native enum Rust must parse");
}

#[test]
fn boxed_native_payload_enum_preserves_boxed_variants_and_unboxed_accessors() {
    let def = EnumDef {
        name: "Record".into(),
        serde_untagged: true,
        variants: vec![EnumVariant {
            name: "Canonical".into(),
            is_tuple: true,
            fields: vec![FieldDef {
                name: "_0".into(),
                ty: TypeRef::Named("Payload".into()),
                is_boxed: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let code = gen_enum_with_module(&def, "core", None, &[], "Example");
    assert!(code.contains("Canonical(Box<Payload>)"), "{code}");
    assert!(code.contains("from_canonical(value: Payload)"), "{code}");
    assert!(code.contains("Self::Canonical(Box::new(value))"), "{code}");
    assert!(code.contains("Some((**value).clone())"), "{code}");
    syn::parse_file(&code).expect("boxed native enum Rust must parse");
}

/// Tagged enums stay on the legacy Hash/JSON representation even when every variant is a single
/// named-payload tuple, matching a homogeneous untagged record shape. A tagged enum's own
/// discriminator key already makes `gen_tagged_enum_ruby_classes` (`mod.rs`) produce an
/// unambiguous `from_hash` marker module named after the enum; a native `#[magnus::wrap]` class
/// would share that exact name and crash Ruby at require time (`TypeError: ... is not a module`).
/// `is_native_payload_enum` is therefore untagged-only; the tagged path still extracts a typed
/// Data-variant instance before falling back to JSON. ~keep
#[test]
fn typed_tagged_newtypes_extract_native_payloads_before_json_fallback() {
    let mut def = EnumDef {
        name: "Policy".into(),
        serde_tag: Some("operation".into()),
        serde_content: Some("policy".into()),
        variants: vec![EnumVariant {
            name: "Analyze".into(),
            is_tuple: true,
            fields: vec![FieldDef {
                name: "_0".into(),
                ty: TypeRef::Named("AnalyzePolicy".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let types = vec![TypeDef {
        name: "AnalyzePolicy".into(),
        ..Default::default()
    }];
    let code = gen_enum_with_module(&def, "core", None, &types, "CustomModule");
    assert!(
        !code.contains("#[magnus::wrap"),
        "a tagged enum must not collide with its own from_hash marker module: {code}"
    );
    assert!(code.contains("Some(\"CustomModule::PolicyAnalyze\")"), "{code}");
    assert!(
        code.contains("let payload: AnalyzePolicy = val.funcall(\"value\", ())?;"),
        "{code}"
    );
    assert!(code.contains("return Ok(Self::Analyze(payload));"), "{code}");
    assert!(code.find("let payload:").unwrap() < code.find("let json_str:").unwrap());
    syn::parse_file(&code).expect("native ingress conversion must be valid Rust");
    // Mixed unit/payload enums take the identical legacy representation and input adapter.
    def.variants.push(EnumVariant {
        name: "None".into(),
        ..Default::default()
    });
    let mixed_code = gen_enum_with_module(&def, "core", None, &types, "CustomModule");
    assert!(
        mixed_code.contains("Some(\"CustomModule::PolicyAnalyze\")"),
        "{mixed_code}"
    );
    assert!(
        mixed_code.contains("let payload: AnalyzePolicy = val.funcall(\"value\", ())?;"),
        "{mixed_code}"
    );
    assert!(
        mixed_code.contains("return Ok(Self::Analyze(payload));"),
        "{mixed_code}"
    );
    syn::parse_file(&mixed_code).expect("native ingress conversion must be valid Rust");
}

pub(super) fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
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
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

pub(super) fn make_typedef(name: &str, fields: Vec<FieldDef>) -> TypeDef {
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
                serde_untagged: false,
                name: "Pending".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                serde_untagged: false,
                name: "Done".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                sensitive: false,
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
    let code = gen_enum(&enum_def, "test_lib", None, &[]);
    assert!(code.contains("enum Status"), "must emit enum definition");
    assert!(
        code.contains("PartialEq, Eq, Hash"),
        "unit enums must support Rust map keys: {code}"
    );
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
    let code = gen_enum(&enum_def, "test_lib", None, &[]);
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
    let code = gen_enum(&enum_def, "test_lib", None, &[]);
    assert!(code.contains("magnus::Symbol::from_value(val)"));
    assert!(code.contains("symbol.name()?.into_owned()"));
    assert!(!code.contains("funcall(\"to_s\""));
    assert!(
        code.contains("\"key_value\""),
        "existing consumer code passing the old snake_case symbol must keep working:\n{code}"
    );
    assert!(
        code.contains("\"KeyValue\""),
        "the new verbatim wire value must also be accepted on input:\n{code}"
    );
}

pub(super) fn make_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        serde_untagged: false,
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        sensitive: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

pub(super) fn make_data_enum(name: &str, serde_tag: Option<&str>) -> EnumDef {
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
    let code = gen_enum(
        &make_data_enum("ImageOutputFormat", Some("type")),
        "test_lib",
        None,
        &[],
    );
    assert!(
        code.contains(r#".or_else(|_| serde_json::from_value(serde_json::json!({ "type": json_str })))"#),
        "expected tagged string wrap for internally-tagged enum: {code}"
    );
}

#[test]
fn gen_enum_keeps_bare_string_for_externally_tagged_enum() {
    // An externally-tagged data enum (no `#[serde(tag)]`) must not gain the tag-wrap branch.
    let code = gen_enum(&make_data_enum("ExternallyTagged", None), "test_lib", None, &[]);
    assert!(
        !code.contains("impl Default"),
        "required data enums must not get invented defaults: {code}"
    );
    assert!(
        !code.contains("serde_json::from_value(serde_json::json!({"),
        "externally-tagged enum must not wrap the string in a tag object: {code}"
    );
    assert!(
        code.contains("serde_json::from_str(&json_str)"),
        "data enum must keep the from_str path: {code}"
    );
}
