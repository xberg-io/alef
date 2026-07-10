use super::*;

#[test]
fn test_gen_lossy_binding_to_core_fields_string_field() {
    let typ = simple_type_def();

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("name: self.name.clone(),"),
        "String field should be cloned"
    );
    assert!(
        result.contains("count: self.count,"),
        "Primitive field should be copied directly"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_cow_string_field() {
    let mut typ = simple_type_def();
    typ.fields[0].core_wrapper = CoreWrapper::Cow;

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("name: self.name.clone().into(),"),
        "Cow-backed String field should convert back into Cow"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_named_field() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "inner".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("inner: self.inner.clone().into(),"),
        "Named field should be cloned and converted"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_path_field() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "file_path".to_string(),
        ty: TypeRef::Path,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("file_path: self.file_path.clone().into(),"),
        "Path field should be cloned and converted to PathBuf"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_path_optional() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "output_path".to_string(),
        ty: TypeRef::Path,
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("output_path: self.output_path.clone().map(Into::into),"),
        "Optional Path field should be cloned and mapped into PathBuf"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_json_field() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "metadata".to_string(),
        ty: TypeRef::Json,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("serde_json::from_str(&self.metadata).unwrap_or_default()"),
        "Json field should be parsed from string"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_json_optional() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "extra".to_string(),
        ty: TypeRef::Json,
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("self.extra.as_ref().and_then(|s| serde_json::from_str(s).ok())"),
        "Optional Json field should be conditionally parsed"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_vec_named() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("items: self.items.clone().into_iter().map(Into::into).collect(),"),
        "Vec<Named> field should clone and convert elements"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_vec_named_optional() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "entries".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string()))),
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("entries: self.entries.clone().map(|v| v.into_iter().map(Into::into).collect()),"),
        "Optional Vec<Named> field should map and convert"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_mut_declares_mutable() {
    let typ = simple_type_def();

    let result = binding_helpers::gen_lossy_binding_to_core_fields_mut(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("let mut core_self"),
        "gen_lossy_binding_to_core_fields_mut should declare core_self as mutable"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_has_stripped_cfg_fields() {
    let mut typ = simple_type_def();
    typ.has_stripped_cfg_fields = true;

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("..Default::default()"),
        "should include ..Default::default() for stripped cfg fields"
    );
    assert!(
        result.contains("needless_update"),
        "should suppress needless_update lint for stripped cfg fields"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_char_field() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "separator".to_string(),
        ty: TypeRef::Char,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("separator: self.separator.chars().next().unwrap_or('*'),"),
        "Char field should extract first char with fallback"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_char_optional() {
    let mut typ = simple_type_def();
    typ.fields.push(FieldDef {
        name: "delimiter".to_string(),
        ty: TypeRef::Char,
        optional: true,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("delimiter: self.delimiter.as_ref().and_then(|s| s.chars().next()),"),
        "Optional Char field should conditionally extract first char"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_duration_option_on_defaults() {
    let mut typ = simple_type_def();
    typ.has_default = true;
    typ.fields.push(FieldDef {
        name: "timeout".to_string(),
        ty: TypeRef::Duration,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        true,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("self.timeout.map(std::time::Duration::from_millis).unwrap_or_default()"),
        "option_duration_on_defaults should use map+unwrap_or_default pattern"
    );
}

#[test]
fn test_has_named_params_vec_string_with_is_ref() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "labels".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: true,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }];

    assert!(
        binding_helpers::has_named_params(&params, &opaque_types),
        "Vec<String> with is_ref=true should require let bindings"
    );
}

#[test]
fn test_has_named_params_vec_string_without_is_ref() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "labels".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::String)),
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
    }];

    assert!(
        !binding_helpers::has_named_params(&params, &opaque_types),
        "Vec<String> without is_ref should not require let bindings"
    );
}

#[test]
fn test_has_named_params_vec_named_always_requires_binding() {
    let opaque_types = AHashSet::new();
    let params = vec![ParamDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
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
    }];

    assert!(
        binding_helpers::has_named_params(&params, &opaque_types),
        "Vec<Named> non-opaque should require let bindings"
    );
}

#[test]
fn test_has_named_params_vec_opaque_named_no_binding_needed() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Item".to_string());
    let params = vec![ParamDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
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
    }];

    assert!(
        !binding_helpers::has_named_params(&params, &opaque_types),
        "Vec<Opaque> should not require let bindings"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_binding_excluded_no_default_per_field_fallback() {
    // `cursor: Cursor<Bytes>` field annotated `#[cfg_attr(alef, alef(skip))]` but
    let mut typ = simple_type_def();
    typ.has_default = false;
    typ.fields.push(FieldDef {
        name: "cursor".to_string(),
        ty: TypeRef::Named("Cursor".to_string()),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: true,
        binding_exclusion_reason: Some("internal read cursor".to_string()),
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("cursor: Default::default()"),
        "binding-excluded field on a no-Default core type must fall back to per-field \
         `Default::default()`; got:\n{result}"
    );
    assert!(
        !result.contains("..Default::default()"),
        "must not emit `..Default::default()` trailer when the core type lacks Default — \
         it fails to compile (E0277); got:\n{result}"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_binding_excluded_with_default_uses_spread() {
    let mut typ = simple_type_def();
    typ.has_default = true;
    typ.fields.push(FieldDef {
        name: "policy".to_string(),
        ty: TypeRef::Named("SsrfPolicy".to_string()),
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: true,
        binding_exclusion_reason: Some("derived from env".to_string()),
        original_type: None,
    });

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("..Default::default()"),
        "binding-excluded field on a has_default core type must use the spread trailer to \
         preserve bespoke core Default semantics; got:\n{result}"
    );
    assert!(
        !result.contains("policy: Default::default()"),
        "must not emit per-field Default::default() for binding-excluded fields when the core \
         type has Default — would bypass bespoke Default semantics; got:\n{result}"
    );
}

#[test]
fn test_gen_lossy_binding_to_core_fields_fully_mirrored_with_default_emits_spread() {
    let mut typ = simple_type_def();
    typ.has_default = true;

    let result = binding_helpers::gen_lossy_binding_to_core_fields(
        &typ,
        "my_crate",
        false,
        &ahash::AHashSet::new(),
        false,
        false,
        &[],
    );

    assert!(
        result.contains("..Default::default()"),
        "fully-mirrored has_default core type must get the spread trailer; got:\n{result}"
    );
    assert!(
        result.contains("#[allow(clippy::needless_update)]"),
        "the spread over a fully-mirrored literal needs the needless_update allow; got:\n{result}"
    );
}
