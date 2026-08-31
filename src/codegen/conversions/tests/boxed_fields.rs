use crate::codegen::conversions::*;
use crate::core::ir::*;
use ahash::AHashSet;

fn boxed_named_field_core(name: &str, optional: bool) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty: TypeRef::Named("Inner".to_string()),
        optional,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: true,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
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

fn boxed_field_type(field: FieldDef) -> TypeDef {
    TypeDef {
        name: "Wrapper".to_string(),
        rust_path: "my_crate::Wrapper".to_string(),
        original_rust_path: String::new(),
        fields: vec![field],
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

/// Regression guard: a plain (non-opaque) `Option<Box<Inner>>` core-struct field must keep
/// generating the existing single-map shape core→binding — this is the path every generated
/// binding relies on today (e.g. `Option<Box<ExtractedDocument>>`), so it must not change
/// shape while fixing the opaque+boxed combination below.
#[test]
fn boxed_named_field_core_to_binding_optional_transparent_path_unchanged() {
    let typ = boxed_field_type(boxed_named_field_core("child", true));

    let out = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());

    assert!(
        out.contains("child: val.child.map(|v| (*v).into())"),
        "expected the existing transparent Option<Box<T>> shape to be preserved; got:\n{out}"
    );
}

/// Companion to the above for the non-optional `Box<Inner>` shape.
#[test]
fn boxed_named_field_core_to_binding_non_optional_transparent_path_unchanged() {
    let typ = boxed_field_type(boxed_named_field_core("child", false));

    let out = gen_from_core_to_binding(&typ, "my_crate", &AHashSet::new());

    assert!(
        out.contains("child: (*val.child).into()"),
        "expected the existing transparent Box<T> shape to be preserved; got:\n{out}"
    );
}

/// Regression: an opaque, prefixed wrapper (`JsInner { inner: Arc<T> }`, the NAPI/WASM-style
/// shape) sourced from a `Box<T>` core field must unbox `Box<T>` to `T` BEFORE wrapping in
/// `Arc::new`. Wrapping the whole `Box<T>` yields `Arc<Box<T>>` where `Arc<T>` is required.
#[test]
fn boxed_opaque_prefixed_field_core_to_binding_optional_unboxes_before_wrapping() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Inner".to_string());
    let config = ConversionConfig {
        type_name_prefix: "Js",
        ..ConversionConfig::default()
    };

    let typ = boxed_field_type(boxed_named_field_core("child", true));
    let out = gen_from_core_to_binding_cfg(&typ, "my_crate", &opaque_types, &config);

    assert!(
        out.contains("child: val.child.map(|v| *v).map(|v| JsInner { inner: Arc::new(v) })"),
        "expected the Box<T> to be unboxed before wrapping in the opaque Arc handle, got:\n{out}"
    );
    assert!(
        !out.contains("child: val.child.map(|v| JsInner { inner: Arc::new(v) })"),
        "must not wrap the un-derefed Box<T> directly in Arc::new (would yield Arc<Box<T>>), got:\n{out}"
    );
}

/// Non-optional companion: `JsInner { inner: Arc<T> }` sourced from a `Box<T>` core field.
#[test]
fn boxed_opaque_prefixed_field_core_to_binding_non_optional_unboxes_before_wrapping() {
    let mut opaque_types = AHashSet::new();
    opaque_types.insert("Inner".to_string());
    let config = ConversionConfig {
        type_name_prefix: "Js",
        ..ConversionConfig::default()
    };

    let typ = boxed_field_type(boxed_named_field_core("child", false));
    let out = gen_from_core_to_binding_cfg(&typ, "my_crate", &opaque_types, &config);

    assert!(
        out.contains("child: JsInner { inner: Arc::new((*val.child)) }"),
        "expected the Box<T> to be deref'd before wrapping in the opaque Arc handle, got:\n{out}"
    );
}
