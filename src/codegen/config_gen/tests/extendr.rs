use super::*;

// -------------------------------------------------------------------------
// gen_extendr_kwargs_constructor
// -------------------------------------------------------------------------

#[test]
fn test_gen_extendr_kwargs_constructor_basic() {
    let typ = make_test_type();
    let empty_enums = ahash::AHashSet::new();
    let output = gen_extendr_kwargs_constructor(&typ, &simple_type_mapper, &empty_enums);

    assert!(output.contains("#[extendr]"), "should have extendr attribute");
    assert!(
        output.contains("pub fn new_config("),
        "function name should be lowercase type name"
    );
    // Fields appear as Option<T> parameters — Rust does not support param defaults.
    assert!(
        output.contains("timeout: Option<u64>"),
        "should accept timeout as Option<u64>: {output}"
    );
    assert!(
        output.contains("enabled: Option<bool>"),
        "should accept enabled as Option<bool>: {output}"
    );
    assert!(
        output.contains("name: Option<String>"),
        "should accept name as Option<String>: {output}"
    );
    assert!(output.contains("-> Config {"), "should return Config");
    assert!(
        output.contains("let mut __out = <Config>::default();"),
        "should base on Default impl: {output}"
    );
    assert!(
        output.contains("if let Some(v) = timeout { __out.timeout = v; }"),
        "should overlay caller-provided timeout"
    );
    assert!(
        output.contains("if let Some(v) = enabled { __out.enabled = v; }"),
        "should overlay caller-provided enabled"
    );
    assert!(
        output.contains("if let Some(v) = name { __out.name = v; }"),
        "should overlay caller-provided name"
    );
}

#[test]
fn test_gen_extendr_kwargs_constructor_uses_option_for_all_fields() {
    // Rust function-parameter defaults (`x: T = expr`) are a syntax error and
    // extendr 0.9 only supports defaults via the `#[extendr(default = "...")]`
    // attribute.  Verify that no field is emitted with a Rust-syntax default.
    let typ = make_test_type();
    let empty_enums = ahash::AHashSet::new();
    let output = gen_extendr_kwargs_constructor(&typ, &simple_type_mapper, &empty_enums);
    assert!(
        !output.contains("= TRUE") && !output.contains("= FALSE") && !output.contains("= \"default\""),
        "constructor must not use Rust-syntax param defaults: {output}"
    );
}

// -------------------------------------------------------------------------
// gen_go_functional_options — tuple-field filtering
// -------------------------------------------------------------------------

#[test]
fn test_gen_go_functional_options_skips_tuple_fields() {
    let mut typ = make_test_type();
    typ.fields.push(FieldDef {
        name: "_0".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::U32),
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
    let output = gen_go_functional_options(&typ, &simple_type_mapper);
    assert!(
        !output.contains("_0"),
        "tuple field _0 should be filtered out from Go output"
    );
}

// -------------------------------------------------------------------------
// as_type_path_prefix — tested indirectly through hash constructor
// -------------------------------------------------------------------------

#[test]
fn test_gen_magnus_hash_constructor_generic_type_prefix() {
    // A field with a Vec type should use <Vec<...>>::try_convert UFCS form
    let fields: Vec<FieldDef> = (0..16)
        .map(|i| FieldDef {
            name: format!("field_{i}"),
            ty: if i == 0 {
                TypeRef::Vec(Box::new(TypeRef::String))
            } else {
                TypeRef::Primitive(PrimitiveType::U32)
            },
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
        })
        .collect();
    let typ = TypeDef {
        name: "WideConfig".to_string(),
        rust_path: "crate::WideConfig".to_string(),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
    // Vec<String> is a generic type; must use <Vec<String>>::try_convert
    assert!(
        output.contains("<Vec<String>>::try_convert"),
        "generic types should use UFCS angle-bracket prefix: {output}"
    );
}

// -------------------------------------------------------------------------
// Bug B regression: Option<Option<T>> must not appear when field.optional==true
// and field.ty==Optional(T). This happens for "Update" structs where the core
// field is Option<Option<T>> — the binding flattens to Option<T>.
// -------------------------------------------------------------------------

#[test]
fn test_magnus_hash_constructor_no_double_option_when_ty_is_optional() {
    // field with optional=true AND ty=Optional(Usize) — represents a core Option<Option<usize>>
    // that should flatten to Option<usize> in the binding constructor.
    // simple_type_mapper maps Usize → "i64" (catch-all primitive arm).
    let field = FieldDef {
        name: "max_depth".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
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
    };
    // Build a large type (>15 fields) so the hash constructor is used
    let mut fields: Vec<FieldDef> = (0..15)
        .map(|i| FieldDef {
            name: format!("field_{i}"),
            ty: TypeRef::Primitive(PrimitiveType::U32),
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
        })
        .collect();
    fields.push(field);
    let typ = TypeDef {
        name: "UpdateConfig".to_string(),
        rust_path: "crate::UpdateConfig".to_string(),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let output = gen_magnus_kwargs_constructor(&typ, &simple_type_mapper);
    // The try_convert call must be for the inner type (i64, as mapped by simple_type_mapper),
    // not Option<i64> (which would yield Option<Option<i64>>).
    assert!(
        !output.contains("Option<Option<"),
        "hash constructor must not emit double Option: {output}"
    );
    assert!(
        output.contains("i64::try_convert"),
        "hash constructor should call inner-type::try_convert, not Option<T>::try_convert: {output}"
    );
}
