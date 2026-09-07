use super::*;

/// Build a minimal serde-tagged data enum with a single struct-style variant carrying exactly
/// one field, for exercising the wasm backend's tagged-enum-as-struct field conversion in
/// isolation (e.g. boxed vs. non-boxed variant fields).
fn tagged_enum_with_single_field(field: FieldDef) -> EnumDef {
    EnumDef {
        name: "ModelSource".to_string(),
        rust_path: "test_lib::ModelSource".to_string(),
        original_rust_path: String::new(),
        variants: vec![EnumVariant {
            serde_untagged: false,
            name: "Llm".to_string(),
            fields: vec![field],
            doc: String::new(),
            is_default: true,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_tag: Some("type".to_string()),
        serde_content: None,
        serde_untagged: false,
        serde_rename_all: Some("snake_case".to_string()),
        rename_all_fields: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

/// A function taking `enum_name` by value, so the enum lands in the wasm backend's
/// `input_types` set and `gen_tagged_enum_binding_to_core` (not just the reverse direction)
/// is emitted.
fn visit_fn_for(enum_name: &str) -> FunctionDef {
    FunctionDef {
        name: "visit_model_source".to_string(),
        rust_path: "test_lib::visit_model_source".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "source".to_string(),
            ty: TypeRef::Named(enum_name.to_string()),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn generate_tagged_enum_lib_content(field: FieldDef) -> String {
    let backend = WasmBackend;
    let enum_def = tagged_enum_with_single_field(field);
    let api = ApiSurface {
        enums: vec![enum_def],
        functions: vec![visit_fn_for("ModelSource")],
        ..Default::default()
    };
    let result = backend.generate_bindings(&api, &make_config());
    assert!(result.is_ok(), "generate_bindings should not fail: {:?}", result.err());
    let files = result.unwrap();
    files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs must be generated")
        .content
        .clone()
}

/// A bare `Box<T>` field on a tagged-enum struct-style variant (e.g.
/// `Llm { llm: Box<LlmConfig> }`) must be boxed on the way into core (`Box::new`) and
/// dereferenced on the way back out to the binding (`(*local).into()`) — the same handling
/// already applied to boxed plain-struct fields.
#[test]
fn test_boxed_named_field_in_tagged_enum_variant_wraps_and_unwraps_box() {
    let mut field = make_field("config", TypeRef::Named("InnerConfig".to_string()), false);
    field.is_boxed = true;
    let content = generate_tagged_enum_lib_content(field);

    assert!(
        content.contains("config: val.config.clone().map(Into::into).map(Box::new).unwrap_or_default()"),
        "binding->core conversion for a bare Box<T> variant field must wrap the converted value \
         in Box::new before falling back to a default:\n{content}"
    );
    assert!(
        content.contains("config: Some((*config).into())"),
        "core->binding conversion for a bare Box<T> variant field must deref before .into():\n{content}"
    );
}

/// An `Option<Box<T>>` field on a tagged-enum struct-style variant must map `Box::new` over the
/// `Option` on the way into core, and deref inside the `Option::map` closure on the way back out.
#[test]
fn test_optional_boxed_named_field_in_tagged_enum_variant_wraps_and_unwraps_box() {
    let mut field = make_field("maybe_config", TypeRef::Named("InnerConfig".to_string()), true);
    field.is_boxed = true;
    let content = generate_tagged_enum_lib_content(field);

    assert!(
        content.contains("maybe_config: val.maybe_config.clone().map(Into::into).map(Box::new)"),
        "binding->core conversion for an Option<Box<T>> variant field must map Box::new over \
         the Option:\n{content}"
    );
    assert!(
        content.contains("maybe_config: maybe_config.map(|v| (*v).into())"),
        "core->binding conversion for an Option<Box<T>> variant field must deref inside the \
         Option::map closure:\n{content}"
    );
}

/// Negative control: a variant field that is NOT boxed must still emit the plain, unwrapped
/// conversion. This proves the box handling above does not blanket-wrap every enum variant
/// field in `Box::new`/deref regardless of `is_boxed`.
#[test]
fn test_non_boxed_named_field_in_tagged_enum_variant_is_not_wrapped_in_box() {
    let field = make_field("plain", TypeRef::Named("InnerConfig".to_string()), false);
    let content = generate_tagged_enum_lib_content(field);

    assert!(
        content.contains("plain: val.plain.clone().map(Into::into).unwrap_or_default()"),
        "binding->core conversion for a non-boxed variant field must not be Box::new-wrapped:\n{content}"
    );
    assert!(
        content.contains("plain: Some(plain.into())"),
        "core->binding conversion for a non-boxed variant field must not be deref'd:\n{content}"
    );
    assert!(
        !content.contains("Box::new(") && !content.contains("(*plain)"),
        "non-boxed field must never trigger Box::new or deref codegen:\n{content}"
    );
}
