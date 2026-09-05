use super::*;

/// Regression: PHP standalone functions carrying a source `cfg` predicate must NOT be wrapped in
/// `#[cfg(...)]` at all in the generated `#[php_impl]` facade — neither a plain
/// `#[cfg(feature = "X")]` nor a self-cancelling `any(X, not(X))`.
///
/// ext-php-rs's `#[php_impl]` derive (verified against the pinned `ext-php-rs-derive 0.11.14`
/// source) walks every `syn::ImplItem::Fn` in the block and unconditionally emits a
/// `FunctionBuilder` registration entry referencing the method by its Rust identifier — it does
/// not consult `#[cfg]` at all. A plain per-method `#[cfg(feature = "X")]` therefore breaks the
/// build with feature `X` off (E0599/E0425: the registration array still references a method
/// that no longer exists), while the old `any(X, not(X))` wrapper broke it with `X` off in a
/// different way (E0433: the always-compiled body calls a core-crate symbol that itself is
/// cfg'd away). Per-method cfg inside `#[php_impl]` is not expressible either way, so PHP must
/// emit the method unconditionally and let `scaffold_php_cargo` require the underlying core
/// feature(s) unconditionally instead (see `cfg_gate.rs`'s sibling scaffold test).
#[test]
fn php_standalone_function_never_wraps_body_in_cfg() {
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "count_tokens".to_string(),
            rust_path: "test_lib::count_tokens".to_string(),
            params: vec![ParamDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            cfg: Some(r#"feature = "tokenizer""#.to_string()),
            ..FunctionDef::default()
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs generated");

    assert!(
        lib.content.contains("fn count_tokens"),
        "facade method must always be emitted (ext-php-rs's #[php_impl] can't tolerate a\
         cfg'd-out method), got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("#[cfg("),
        "facade method must never be cfg-gated, got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("not("),
        "must never reintroduce a self-cancelling any(X, not(X)) predicate, got:\n{}",
        lib.content
    );
}

#[test]
fn php_struct_and_conversions_strip_cfg_fields_by_default() {
    let backend = PhpBackend;
    let mut feature_only = make_field("feature_only", TypeRef::String, false);
    feature_only.cfg = Some(r#"feature = "enterprise""#.to_string());
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            fields: vec![make_field("name", TypeRef::String, false), feature_only],
            has_default: true,
            has_serde: true,
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = backend
        .generate_bindings(&api, &make_config())
        .expect("PHP bindings generate");
    let lib = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs generated");

    assert!(
        lib.content.contains("pub name: String"),
        "ordinary field missing:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("feature_only"),
        "a default-stripped cfg field must not survive in the struct, constructor, accessors, or conversions:\n{}",
        lib.content
    );
}
