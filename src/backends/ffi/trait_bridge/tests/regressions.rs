use super::*;

// ---------------------------------------------------------------------------
// Bug-regression tests: one per fixed bug so regressions are caught immediately.
// ---------------------------------------------------------------------------

/// Bug 1: Bare excluded-type references.
///
/// When a trait method references a type that was excluded from the binding surface
/// (present in `api.excluded_type_paths`), the generated trait impl must use the
/// fully-qualified Rust path, not the bare type name.
///
/// Example: an excluded `HiddenDoc` argument must emit the fully-qualified
/// Rust path, not the bare type name.
#[test]
fn bug1_excluded_type_is_fully_qualified_in_trait_impl() {
    let internal_doc_method = MethodDef {
        name: "render".to_string(),
        params: vec![crate::core::ir::ParamDef {
            name: "doc".to_string(),
            ty: TypeRef::Named("HiddenDoc".to_string()),
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let trait_def = make_trait_def("Renderer", vec![internal_doc_method]);
    let bridge_cfg = sample_bridge_cfg("Renderer");

    // Include HiddenDoc as an excluded type path.
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: {
            let mut m = ::std::collections::HashMap::new();
            m.insert("HiddenDoc".to_string(), "my_lib::internal::HiddenDoc".to_string());
            m
        },
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    assert!(
        code.contains("&my_lib::internal::HiddenDoc"),
        "excluded type must be fully-qualified, not bare;\n\
         actual code:\n{code}"
    );
    assert!(
        !code.contains("&HiddenDoc"),
        "bare type reference must not appear in generated trait impl;\n\
         actual code:\n{code}"
    );
}

/// Bug 2: Sync method bodies must use the trait's error type, not `Box::from`.
///
/// `gen_vtable_call_body(inside_closure=false)` is used for synchronous trait method
/// bodies.  Those methods return `Result<T, SampleCrateError>`, so error construction
/// must call `spec.make_error(...)` (e.g. `MyError::from(...)`), not `Box::from(...)`.
/// `Box::from` is correct only inside the async `_SendFn` closure where the return type
/// is `Box<dyn Error + Send + Sync>`.
#[test]
fn bug2_sync_method_body_uses_trait_error_type_not_box_from() {
    use crate::codegen::generators::trait_bridge::TraitBridgeSpec;

    let method = MethodDef {
        name: "run".to_string(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        is_static: false,
        error_type: Some("MyError".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let trait_def = make_trait_def("Backend", vec![method.clone()]);
    let bridge_cfg = sample_bridge_cfg("Backend");

    let spec = TraitBridgeSpec {
        trait_def: &trait_def,
        bridge_config: &bridge_cfg,
        core_import: "my_lib",
        wrapper_prefix: "Ml",
        type_paths: ::std::collections::HashMap::new(),
        lifetime_type_names: std::collections::HashSet::new(),
        error_type: "MyError".to_string(),
        error_constructor: "MyError::from({msg})".to_string(),
    };

    let generator = FfiBridgeGenerator {
        prefix: "ml".to_string(),
        core_import: "my_lib".to_string(),
        type_paths: ::std::collections::HashMap::new(),
        error_type: "MyError".to_string(),
        plugin_error_constructor: None,
        lifetime_type_names: ::std::collections::HashSet::new(),
    };

    // Sync body (inside_closure = false): must use MyError::from, not Box::from
    let sync_body = generator.gen_vtable_call_body(&method, &spec, false);
    assert!(
        sync_body.contains("MyError::from("),
        "sync method body must use the trait's error constructor;\n\
         actual body:\n{sync_body}"
    );
    assert!(
        !sync_body.contains("Err(Box::from("),
        "sync method body must NOT use Box::from (that's for the async closure);\n\
         actual body:\n{sync_body}"
    );

    // Closure body (inside_closure = true): must use Box::from, not MyError::from
    let closure_body = generator.gen_vtable_call_body(&method, &spec, true);
    assert!(
        closure_body.contains("Err(Box::from("),
        "async closure body must use Box::from;\n\
         actual body:\n{closure_body}"
    );
}

/// Bug 3: `Vec<String> + returns_ref` methods must emit `&[&str]` in the trait impl,
/// and the bridge struct must gain a `{method_name}_strs: &'static [&'static str]` field
/// populated at construction time.
#[test]
fn bug3_returns_ref_vec_string_emits_slice_ref_and_cache_field() {
    let method = MethodDef {
        name: "supported_mime_types".to_string(),
        params: vec![],
        return_type: TypeRef::Vec(Box::new(TypeRef::String)),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: true, // `fn supported_mime_types(&self) -> &[&str]`
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let trait_def = make_trait_def("DocumentExtractor", vec![method]);
    let bridge_cfg = sample_bridge_cfg("DocumentExtractor");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "kr",
        "sample_crate",
        "SampleCrateError",
        "SampleCrateError::from({msg})",
        None,
        &api,
    );

    // The trait impl return type must be `&[&str]`, not `Vec<String>`
    assert!(
        code.contains("fn supported_mime_types(&self) -> &[&str]"),
        "returns_ref Vec<String> must produce &[&str] in trait impl;\n\
         actual code:\n{code}"
    );

    // The bridge struct must have the cache field
    assert!(
        code.contains("supported_mime_types_strs: &'static [&'static str]"),
        "bridge struct must have supported_mime_types_strs cache field;\n\
         actual code:\n{code}"
    );

    // The trait impl body must return from the cache field
    assert!(
        code.contains("self.supported_mime_types_strs"),
        "trait impl body must return from the cached field;\n\
         actual code:\n{code}"
    );

    // The constructor must populate the cache field by calling the vtable
    assert!(
        code.contains("Box::leak"),
        "constructor must use Box::leak to build &'static [&'static str];\n\
         actual code:\n{code}"
    );
}

/// Bug 4 (FFI variant): `ffi_skip_methods` opts a method out of the FFI trait impl.
///
/// FFI's vtable bridge intentionally emits every trait method (including those with
/// `has_default_impl = true`) so the vtable can forward them — this is required for
/// visitor-style traits where every method has a default. The only way
/// to opt out of vtable forwarding (and fall back to the trait's own default) is to
/// list the method in `ffi_skip_methods`.
#[test]
fn bug4_ffi_skip_methods_opts_out_of_trait_impl() {
    let required = make_method("run", TypeRef::String, true, false);
    let optional = make_method("shutdown", TypeRef::Unit, false, true);
    let trait_def = make_trait_def("Backend", vec![required, optional]);
    let mut bridge_cfg = sample_bridge_cfg("Backend");
    bridge_cfg.ffi_skip_methods = vec!["shutdown".to_string()];
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // Required method must appear in the trait impl
    assert!(
        code.contains("fn run("),
        "required method must appear in trait impl;\n\
         actual code:\n{code}"
    );

    // Method in ffi_skip_methods must NOT appear — let the trait's own default take effect
    assert!(
        !code.contains("fn shutdown("),
        "method listed in ffi_skip_methods must NOT get a generated body;\n\
         actual code:\n{code}"
    );
}

/// Bug 5: Async method with a `&str` param must clone the param with `.to_string()`
/// before moving it into the `spawn_blocking` closure, not with `.clone()`.
///
/// `.clone()` on `&str` returns `&str` — the original borrow escapes into the closure,
/// triggering E0521 ("borrowed data escapes outside of method").  `.to_string()`
/// produces an owned `String` that is `'static` and safe to move into the closure.
#[test]
fn bug5_async_str_param_uses_to_string_not_clone() {
    let method = MethodDef {
        name: "process".to_string(),
        params: vec![ParamDef {
            name: "mime_type".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true, // &str — the borrow that escapes without .to_string()
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Unit,
        is_async: true, // async method — closure must own all captured data
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let trait_def = make_trait_def("Backend", vec![method]);
    let bridge_cfg = sample_bridge_cfg("Backend");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // The closure capture must convert &str to String, not clone the borrow.
    assert!(
        code.contains("let mime_type = mime_type.to_string()"),
        "async &str param must be captured via .to_string() to avoid E0521;\n\
         actual code:\n{code}"
    );
    assert!(
        !code.contains("let mime_type = mime_type.clone()"),
        "async &str param must NOT use .clone() (returns &str, still borrows);\n\
         actual code:\n{code}"
    );
}

/// Bug 6: Async method whose trait return type is an excluded Named type must:
///   (a) emit the fully-qualified path in the method SIGNATURE, and
///   (b) deserialize JSON from the C ABI back to that type in the closure BODY.
///
/// Before the fix the generator emitted `Result<String, _>` in the signature and
/// `Ok(cs.to_string_lossy().into_owned())` in the body — both wrong for Named returns.
#[test]
fn bug6_async_excluded_type_return_signature_and_deserialization() {
    let method = MethodDef {
        name: "extract_bytes".to_string(),
        params: vec![ParamDef {
            name: "content".to_string(),
            ty: TypeRef::Bytes,
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Named("HiddenDoc".to_string()),
        is_async: true,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let trait_def = make_trait_def("Extractor", vec![method]);
    let bridge_cfg = sample_bridge_cfg("Extractor");

    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: {
            let mut m = ::std::collections::HashMap::new();
            m.insert("HiddenDoc".to_string(), "my_lib::internal::HiddenDoc".to_string());
            m
        },
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // Signature must use the fully-qualified path, not String.
    assert!(
        code.contains("-> std::result::Result<my_lib::internal::HiddenDoc,"),
        "async method return type must be qualified excluded type in signature;\n\
         actual code:\n{code}"
    );
    assert!(
        !code.contains("-> std::result::Result<String,"),
        "async method return type must NOT be String for Named return types;\n\
         actual code:\n{code}"
    );

    // Closure body must deserialize JSON back to the excluded core type, not pass String through.
    assert!(
        code.contains("serde_json::from_str::<my_lib::internal::HiddenDoc>"),
        "async closure body must deserialize JSON to HiddenDoc;\n\
         actual code:\n{code}"
    );
    assert!(
        !code.contains("Ok(cs.to_string_lossy().into_owned())"),
        "async closure body must NOT return raw String for Named return types;\n\
         actual code:\n{code}"
    );
}

/// Regression: `gen_ffi_trait_impl` was calling `format_type_ref` which ignores
/// `is_ref`/`is_mut`, causing `&[u8]` → `Vec<u8>`, `&str` → `String`, `&Path` →
/// `PathBuf`, and excluded named references in the trait impl method
/// signatures.  The fix uses `format_param_type` which respects those flags.
#[test]
fn bug_ffi1_trait_impl_param_types_respect_is_ref() {
    let method = MethodDef {
        name: "process".to_string(),
        params: vec![
            ParamDef {
                name: "content".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true, // &[u8]
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            },
            ParamDef {
                name: "mime_type".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true, // &str
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            },
            ParamDef {
                name: "path".to_string(),
                ty: TypeRef::Path,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true, // &Path
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            },
        ],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: Some("Box<dyn std::error::Error + Send + Sync>".to_string()),
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let trait_def = make_trait_def("Extractor", vec![method]);
    let bridge_cfg = sample_bridge_cfg("Extractor");
    let api = sample_api();

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "ml",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // trait impl must emit the reference types, not the owned equivalents
    assert!(
        code.contains("content: &[u8]"),
        "is_ref Bytes param must be &[u8] in trait impl, not Vec<u8>;\n\
         actual code:\n{code}"
    );
    assert!(
        code.contains("mime_type: &str"),
        "is_ref String param must be &str in trait impl, not String;\n\
         actual code:\n{code}"
    );
    assert!(
        code.contains("path: &std::path::Path"),
        "is_ref Path param must be &std::path::Path in trait impl, not PathBuf;\n\
         actual code:\n{code}"
    );
}

/// Named types with `has_lifetime_params = true` (e.g. `SyntaxContext<'a>`) must be
/// emitted as `&Type<'_>` in the FFI trait impl method signature so it matches the
/// trait definition exactly. Without `<'_>`, rustc rejects the impl because the
/// concrete struct is `SyntaxContext<'_>` but the generated method omits the lifetime.
#[test]
fn lifetime_param_named_type_emits_angle_lifetime_placeholder() {
    use crate::core::ir::{ParamDef, TypeDef};

    let method = MethodDef {
        name: "visit_element".to_string(),
        params: vec![ParamDef {
            name: "context".to_string(),
            ty: TypeRef::Named("NodeContext".to_string()),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true, // &NodeContext<'_> in the trait definition
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: true, // visitor method — has default but FFI still emits it
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let trait_def = make_trait_def("HtmlVisitor", vec![method]);
    let bridge_cfg = sample_bridge_cfg("HtmlVisitor");

    // Include the context type as a type with lifetime params.
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "NodeContext".to_string(),
            rust_path: "my_lib::NodeContext".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: true, // <-- the key flag
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "htm",
        "my_lib",
        "MyError",
        "MyError::from({msg})",
        None,
        &api,
    );

    // The trait impl method signature must include a lifetime placeholder, not a bare type.
    assert!(
        code.contains("context: &my_lib::NodeContext<'_>"),
        "lifetime-parameterized Named type must be &Type<'_> in trait impl signature;\n\
         actual code:\n{code}"
    );
    assert!(
        !code.contains("context: &my_lib::NodeContext,") && !code.contains("context: &my_lib::NodeContext\n"),
        "bare &NodeContext without lifetime placeholder must NOT appear in trait impl;\n\
         actual code:\n{code}"
    );
}

#[test]
fn vtable_registration_signature_takes_const_pointer() {
    // Regression test for C9: Go cgo vtable cimport unification.
    // FFI registration function must take `vtable: *const VTableName` (pointer),
    // not `vtable: VTableName` (value), so that Go can consistently pass `&vtable`
    // without cgo type unification issues.

    let trait_def = make_trait_def(
        "TestBackend",
        vec![make_method("process", TypeRef::String, true, false)],
    );

    let bridge_cfg = crate::core::config::TraitBridgeConfig {
        trait_name: "TestBackend".to_string(),
        register_fn: Some("register_backend".to_string()),
        registry_getter: Some("get_registry".to_string()),
        ..Default::default()
    };

    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![trait_def.clone()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let code = gen_trait_bridge(
        &trait_def,
        &bridge_cfg,
        "test",
        "test_lib",
        "TestError",
        "TestError::from({msg})",
        None,
        &api,
    );

    // The registration function signature must use `*const` for the vtable parameter.
    assert!(
        code.contains("vtable: *const TestTestBackendVTable"),
        "FFI registration function must take vtable as `*const VTableName` pointer;\n\
         actual code:\n{code}"
    );

    // Should not use value-type vtable parameter.
    assert!(
        !code.contains(
            "pub unsafe extern \"C\" fn test_register_backend(\n    name: *const std::ffi::c_char,\n    vtable: TestTestBackendVTable,"
        ),
        "FFI registration function must NOT take vtable as value type (bare struct);\n\
         actual code:\n{code}"
    );

    // Should dereference the pointer in the function body.
    assert!(
        code.contains("let vtable_ref = &*vtable;"),
        "FFI registration function body must dereference vtable pointer;\n\
         actual code:\n{code}"
    );
}
