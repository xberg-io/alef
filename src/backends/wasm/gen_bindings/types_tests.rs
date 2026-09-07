use super::*;
use std::collections::HashMap;

fn mapper() -> WasmMapper {
    WasmMapper::new(HashMap::new(), "Wasm".to_string())
}

fn enum_names(names: &[&str]) -> AHashSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

fn no_untagged_ts_types() -> ahash::AHashMap<String, String> {
    ahash::AHashMap::default()
}

#[test]
fn gen_getter_option_vec_unit_enum_flattens_option() {
    let field = FieldDef {
        name: "modalities".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Modality".to_string()))),
        optional: true,
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();
    let out = gen_getter(&field, &mapper(), &enums, &tagged, false, &no_untagged_ts_types());
    assert!(
        out.contains("-> Option<Vec<String>>"),
        "getter must return Option<Vec<String>>: {out}"
    );
    assert!(
        out.contains("self.modalities.as_ref().map(|v| v.iter()"),
        "getter must flatten the Option via as_ref().map(...): {out}"
    );
    assert!(
        out.contains("x.to_api_str().to_owned()"),
        "getter must call to_api_str() on each element: {out}"
    );
}

#[test]
fn gen_setter_option_vec_unit_enum_wraps_some() {
    let field = FieldDef {
        name: "modalities".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Modality".to_string()))),
        optional: true,
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();
    let out = gen_setter(&field, &mapper(), &enums, false, &tagged, &no_untagged_ts_types());
    assert!(
        out.contains("value: Option<Vec<String>>"),
        "setter must take Option<Vec<String>>: {out}"
    );
    assert!(
        out.contains("value.map(|v| v.into_iter().filter_map(|s| WasmModality::from_api_str(&s)).collect())"),
        "setter must map the Option and collect into the inner Vec: {out}"
    );
}

#[test]
fn gen_getter_setter_required_vec_unit_enum_unchanged() {
    let field = FieldDef {
        name: "tags".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Modality".to_string()))),
        optional: false,
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();
    let getter = gen_getter(&field, &mapper(), &enums, &tagged, false, &no_untagged_ts_types());
    assert!(
        getter.contains("-> Vec<String>"),
        "getter must return Vec<String>: {getter}"
    );
    assert!(
        getter.contains("self.tags.iter().map(|v| v.to_api_str().to_owned()).collect()"),
        "getter must iterate the Vec directly: {getter}"
    );
    let setter = gen_setter(&field, &mapper(), &enums, false, &tagged, &no_untagged_ts_types());
    assert!(
        setter.contains("value: Vec<String>"),
        "setter must take Vec<String>: {setter}"
    );
    assert!(
        setter.contains("value.into_iter().filter_map(|s| WasmModality::from_api_str(&s)).collect()"),
        "setter must collect into the inner Vec: {setter}"
    );
    assert!(
        !setter.contains("value.map("),
        "required Vec setter must not Option::map: {setter}"
    );
}

#[test]
fn optional_u64_vec_accessors_keep_the_wasm_bigint_vector_shape() {
    let field = FieldDef {
        name: "values".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U64))),
        optional: true,
        ..Default::default()
    };
    let enums = AHashSet::new();
    let tagged = AHashSet::new();

    let getter = gen_getter(&field, &mapper(), &enums, &tagged, false, &no_untagged_ts_types());
    let setter = gen_setter(&field, &mapper(), &enums, false, &tagged, &no_untagged_ts_types());

    assert!(
        getter.contains("-> Option<Vec<u64>>"),
        "the getter must expose wasm-bindgen's optional BigUint64Array source shape: {getter}"
    );
    assert!(
        setter.contains("value: Option<Vec<u64>>"),
        "the setter must expose wasm-bindgen's optional BigUint64Array source shape: {setter}"
    );
}

/// A field and an inherent method sharing the same name each mint `pub fn {name}(&self)`
/// in the generated `#[wasm_bindgen]` impl block. Without a collision guard, both the
/// field-getter emitter and the method-wrapper emitter would define the function, which
/// rustc rejects with `E0592: duplicate definitions with name`. The getter is emitted
/// first and kept; the same-named method wrapper must be skipped.
#[test]
fn gen_struct_methods_skips_method_wrapper_when_field_getter_already_emitted() {
    let typ = TypeDef {
        name: "LlmConfig".to_string(),
        rust_path: "my_lib::LlmConfig".to_string(),
        fields: vec![FieldDef {
            name: "providers".to_string(),
            ty: TypeRef::String,
            optional: false,
            ..Default::default()
        }],
        methods: vec![MethodDef {
            name: "providers".to_string(),
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        }],
        ..Default::default()
    };

    let out = gen_struct_methods(
        &typ,
        &mapper(),
        &[],
        "my_lib",
        &AHashSet::default(),
        &[],
        "Wasm",
        &AHashSet::default(),
        &ahash::AHashMap::default(),
        &no_untagged_ts_types(),
        &[],
    );

    let occurrences = out.matches("fn providers(").count();
    assert_eq!(
        occurrences, 1,
        "field/method name collision must emit `providers` exactly once, got {occurrences}:\n{out}"
    );
}

/// Regression coverage for the `clippy::redundant_field_names` fix: a single-word field name
/// (e.g. `provider`) is already valid camelCase, so `to_node_name` maps it to itself. The
/// no-colon branch of `convert_constructor_params_to_camel_case` must keep the assignment as
/// field-init shorthand, not rewrite it to `provider: provider`.
#[test]
fn convert_constructor_params_keeps_shorthand_for_single_word_field() {
    let field_names = vec!["provider".to_string()];

    let (_params, assignments) = convert_constructor_params_to_camel_case("provider: String", "provider", &field_names);

    assert_eq!(
        assignments, "provider",
        "single-word field must stay shorthand, not be rewritten to `provider: provider`"
    );
}

/// Same no-colon branch, but for a multi-word field whose camelCase form (`chunkSize`)
/// genuinely differs from the snake_case field name (`chunk_size`) — the real `field: camel`
/// form must still be emitted so the JS-facing local variable is in scope.
#[test]
fn convert_constructor_params_renames_multi_word_field() {
    let field_names = vec!["chunk_size".to_string()];

    let (_params, assignments) =
        convert_constructor_params_to_camel_case("chunk_size: u32", "chunk_size", &field_names);

    assert_eq!(assignments, "chunk_size: chunkSize");
}

/// A mapper whose `named` for `Modality` is redirected by a `wasm.type_overrides` entry — the
/// documented way a consumer replaces a generated wrapper with another binding type.
fn mapper_with_override(name: &str, mapped: &str) -> WasmMapper {
    let mut overrides = HashMap::new();
    overrides.insert(name.to_string(), mapped.to_string());
    WasmMapper::new(overrides, "Wasm".to_string())
}

/// `to_api_str` exists only on the generated `Wasm{Enum}`. When the mapper redirects the enum to
/// another binding type, `gen_struct` types the field from that mapper while the getter would
/// still emit the wrapper's inherent call against it — an `E0599` on a field the same backend
/// declared. Only the mapper knows which type the name resolves to, so the enum-shaped getter
/// must be conditional on it.
#[test]
fn gen_getter_named_enum_mapped_to_js_value_skips_to_api_str() {
    let field = FieldDef {
        name: "modality".to_string(),
        ty: TypeRef::Named("Modality".to_string()),
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();

    let out = gen_getter(
        &field,
        &mapper_with_override("Modality", "JsValue"),
        &enums,
        &tagged,
        false,
        &no_untagged_ts_types(),
    );
    assert!(
        !out.contains("to_api_str"),
        "an overridden enum is not stored as the wrapper, so to_api_str does not exist: {out}"
    );
    assert!(out.contains("-> JsValue"), "getter must return the mapped type: {out}");
    assert!(
        out.contains("self.modality.clone()"),
        "a non-Copy mapped type must be cloned out of &self, not moved: {out}"
    );
}

/// Positive control for the above: with no override the mapper still renders `WasmModality`, so
/// the enum-shaped getter is correct and must be unchanged.
#[test]
fn gen_getter_named_enum_without_override_still_uses_to_api_str() {
    let field = FieldDef {
        name: "modality".to_string(),
        ty: TypeRef::Named("Modality".to_string()),
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();

    let out = gen_getter(&field, &mapper(), &enums, &tagged, false, &no_untagged_ts_types());
    assert!(out.contains("-> String"), "getter must expose the wire string: {out}");
    assert!(
        out.contains("self.modality.to_api_str().to_owned()"),
        "wrapper-backed enum getter must keep using to_api_str: {out}"
    );
}

/// The setter's mirror image: `from_api_str` is a `Wasm{Enum}` associated function, and naming
/// `WasmModality` at all is an `E0433` once the mapper resolved `Modality` to something else.
#[test]
fn gen_setter_vec_enum_mapped_to_js_value_skips_from_api_str() {
    let field = FieldDef {
        name: "modalities".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Modality".to_string()))),
        ..Default::default()
    };
    let enums = enum_names(&["Modality"]);
    let tagged: AHashSet<String> = AHashSet::new();

    let out = gen_setter(
        &field,
        &mapper_with_override("Modality", "JsValue"),
        &enums,
        false,
        &tagged,
        &no_untagged_ts_types(),
    );
    assert!(
        !out.contains("from_api_str") && !out.contains("WasmModality"),
        "an overridden enum has no generated wrapper to parse into: {out}"
    );

    let control = gen_setter(&field, &mapper(), &enums, false, &tagged, &no_untagged_ts_types());
    assert!(
        control.contains("WasmModality::from_api_str"),
        "wrapper-backed enum setter must keep parsing wire strings: {control}"
    );
}

#[test]
fn gen_struct_never_derives_serde_even_with_container_conversion() {
    // The wasm binding struct is bridged to JS via #[wasm_bindgen] getters/setters, not serde:
    // every JSON/JsValue decode path (the serde_*.jinja parameter templates and the trait-bridge
    // return path in gen_{a,}sync_method_body.jinja) targets the *core* type directly and only
    // `.into()`s into this struct afterward, so it never needs (and must never gain) its own
    // Deserialize. If a future edit to gen_struct.jinja adds a serde derive here, it must also
    // wire `crate::codegen::generators::gen_delegating_deserialize_impl` the way pyo3/extendr do,
    // or the same core->binding wire-shape defect that mechanism fixes elsewhere reappears here
    // silently. ~keep
    let typ = TypeDef {
        name: "Point".to_string(),
        rust_path: "sample_core::Point".to_string(),
        fields: vec![
            FieldDef {
                name: "x".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::F64),
                ..Default::default()
            },
            FieldDef {
                name: "y".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::F64),
                ..Default::default()
            },
        ],
        serde_container_conversion: crate::core::ir::SerdeContainerConversion {
            from: Some("(f64, f64)".to_string()),
            into: Some("(f64, f64)".to_string()),
            ..Default::default()
        },
        has_serde: true,
        ..Default::default()
    };
    let out = gen_struct(&typ, &mapper(), &[], "sample_core", "Wasm", &AHashSet::new(), &[], true);
    assert!(
        !out.contains("Serialize"),
        "wasm binding struct must not derive Serialize: {out}"
    );
    assert!(
        !out.contains("Deserialize"),
        "wasm binding struct must not derive Deserialize: {out}"
    );
}
