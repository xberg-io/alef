use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

/// Verify that methods with generic type parameters are skipped from C FFI wrapper generation.
/// Generic methods (like `App::route<H: Handler>(...)`) cannot be wrapped as C functions
/// because generic type parameters have no C FFI representation. These methods are handled
/// through the service-API registration path instead.
#[test]
fn test_skips_method_with_generic_type_parameter() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "App".to_string(),
            rust_path: "my_lib::App".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "route".to_string(),
                params: vec![
                    ParamDef {
                        name: "builder".to_string(),
                        ty: TypeRef::Named("RouteBuilder".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "handler".to_string(),
                        ty: TypeRef::Named("H".to_string()),
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
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("App".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Register a handler.".to_string(),
                receiver: Some(ReceiverKind::Owned),
                cfg: None,
                sanitized: false,
                trait_source: None,
                returns_ref: true,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
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
            doc: "App service.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("my_lib_app_route"),
        "method with generic type parameter H should NOT be wrapped as C function"
    );
}

/// Verify that methods returning a reference to the receiver (builder-style methods)
/// are skipped from C FFI wrapper generation. Methods returning `&mut Self` or `&Self`
/// cannot be represented as owned C handles, so they must be accessed through the
/// service-API registration path instead.
#[test]
fn test_skips_method_with_receiver_reference_return() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Builder".to_string(),
            rust_path: "my_lib::Builder".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "with_option".to_string(),
                params: vec![ParamDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
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
                return_type: TypeRef::Named("Builder".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Set an option (builder style).".to_string(),
                receiver: Some(ReceiverKind::RefMut),
                cfg: None,
                sanitized: false,
                trait_source: None,
                returns_ref: true,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
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
            doc: "Builder type.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("my_lib_builder_with_option"),
        "builder-style method returning &mut Self should NOT be wrapped as C function"
    );
}

/// Verify that a public field and an inherent method sharing the same name do not both
/// emit a C function under the same `{prefix}_{type_snake}_{name}` symbol. Both the
/// field-accessor emitter and the method-wrapper emitter mint that symbol independently,
/// so without a collision guard the crate defines the exported function twice, which
/// `rustc` rejects with `E0428: the name ... is defined multiple times`.
///
/// The field accessor is emitted first and kept; the same-named method wrapper is
/// skipped, since it is redundant with the field accessor and callers can reach the
/// field's value through it.
#[test]
fn test_field_and_same_named_method_do_not_emit_duplicate_symbol() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "LlmConfig".to_string(),
            rust_path: "my_lib::LlmConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                version: Default::default(),
                name: "providers".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
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
                original_type: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            methods: vec![MethodDef {
                name: "providers".to_string(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: "Return the resolved providers.".to_string(),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "LLM client configuration.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let occurrences = lib.content.matches("fn my_lib_llm_config_providers(").count();
    assert_eq!(
        occurrences, 1,
        "field/method name collision must emit the symbol exactly once, got {occurrences}:\n{}",
        lib.content
    );
}

/// The FFI backend (`gen_method_wrapper`) and the docs renderers (`docs::naming::method_name`)
/// used to derive a method's C symbol independently, so a rename could land on one side only
/// and the docs would publish a symbol that occurs zero times in the emitted header. Both now
/// route through `codegen::c_consumer::method_symbol`, but that shared helper is unreachable
/// from a docs-only or backend-only test: the docs side can't reach `gen_lib_rs` (it is
/// `pub(super)` inside `crate::backends::ffi::gen_bindings`), so the true end-to-end
/// cross-check belongs here, where both the generated source and the public docs helper are
/// reachable in the same test.
///
/// This also exercises the prefix-casing seam honestly instead of masking it: the real docs
/// pipeline holds `ffi_prefix` PascalCased (`docs::generate_docs`, `src/docs/mod.rs`) and
/// `method_name` snake-cases it back internally, while the backend passes `config.ffi_prefix()`
/// raw. Feeding both sides the same pre-cased string would hide a casing-seam regression that
/// this test exists to catch.
///
/// Catches: either side going back to deriving `{prefix}_{type}_{method}` (or any shape of it)
/// on its own instead of routing through `c_consumer::method_symbol`.
#[test]
fn test_ffi_method_symbol_matches_the_symbol_docs_would_publish() {
    use heck::ToPascalCase;

    let mut api = sample_api();
    api.types.push(TypeDef {
        name: "DefaultClient".to_string(),
        rust_path: "my_lib::DefaultClient".to_string(),
        methods: vec![MethodDef {
            name: "chat".to_string(),
            return_type: TypeRef::String,
            error_type: Some("MyError".to_string()),
            doc: "Send a chat message.".to_string(),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        }],
        is_opaque: true,
        ..TypeDef::default()
    });

    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let pascal_prefix = config.ffi_prefix().to_pascal_case();
    let documented_symbol = crate::docs::naming::method_name(
        "DefaultClient",
        "chat",
        crate::core::config::Language::C,
        &pascal_prefix,
    );

    assert!(
        lib.content.contains(&format!("fn {documented_symbol}(")),
        "expected the docs-published symbol `{documented_symbol}` to appear as an extern \"C\" \
         item in the generated FFI source, got:\n{}",
        lib.content
    );

    let old_wrong_shape = format!("fn {}_chat(", config.ffi_prefix());
    assert!(
        !lib.content.contains(&old_wrong_shape),
        "the old, wrong shape `{{prefix}}_{{method}}` with no type component must not appear \
         (it names a symbol that occurs zero times in the real header)"
    );
}

/// Narrow pin for the `gen_method_wrapper` rewire onto `c_consumer::method_symbol`
/// (`functions/orchestration.rs`). Uses a type name with an internal acronym, where the
/// backend's `pascal_to_snake` and a naive lowercase would diverge (`HTMLParser` ->
/// `html_parser`, not `htmlparser`) -- this is the shape of type name a re-diverged,
/// independently hand-rolled derivation would get wrong without a purely alphabetic fixture
/// like the other method tests in this file noticing.
#[test]
fn test_method_wrapper_symbol_matches_c_consumer_method_symbol() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "HTMLParser".to_string(),
            rust_path: "my_lib::HTMLParser".to_string(),
            methods: vec![MethodDef {
                name: "parse".to_string(),
                return_type: TypeRef::String,
                error_type: Some("MyError".to_string()),
                doc: "Parse the document.".to_string(),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..MethodDef::default()
            }],
            is_opaque: true,
            ..TypeDef::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let expected = crate::codegen::c_consumer::method_symbol(&config.ffi_prefix(), "HTMLParser", "parse");
    assert_eq!(expected, "my_lib_html_parser_parse");
    assert!(
        lib.content.contains(&format!("fn {expected}(")),
        "expected `{expected}` to appear as an extern \"C\" item, got:\n{}",
        lib.content
    );
}

/// Narrow pin for the `gen_opaque_static_constructor` rewire onto `c_consumer::method_symbol`
/// (`types.rs`) -- a site the original rewire brief for this task did not name explicitly, but
/// which builds the exact same `{prefix}_{type}_{method}` shape from `typ.methods`, so it is
/// documented via the same `docs::naming::method_name` path as any other method. Uses an
/// acronym type name for the same reason as the sibling instance-method pin above.
#[test]
fn test_static_constructor_symbol_matches_c_consumer_method_symbol() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "JSONParser".to_string(),
            rust_path: "my_lib::JSONParser".to_string(),
            methods: vec![MethodDef {
                name: "compile".to_string(),
                return_type: TypeRef::Named("JSONParser".to_string()),
                error_type: Some("MyError".to_string()),
                doc: "Compile a parser from a schema.".to_string(),
                is_static: true,
                receiver: None,
                cfg: None,
                ..MethodDef::default()
            }],
            is_opaque: true,
            ..TypeDef::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let expected = crate::codegen::c_consumer::method_symbol(&config.ffi_prefix(), "JSONParser", "compile");
    assert_eq!(expected, "my_lib_json_parser_compile");
    assert!(
        lib.content.contains(&format!("fn {expected}(")),
        "expected `{expected}` to appear as an extern \"C\" item, got:\n{}",
        lib.content
    );
}

/// Narrow pin for the `gen_free_function_len_companion` rewire (`functions/signatures.rs`):
/// the companion's name must be derived FROM `free_function_symbol`, not computed in parallel,
/// so it cannot drift from the primary free function's own symbol. Calls the generator
/// directly rather than through a full `String`-returning function fixture, to pin the
/// companion/primary pairing in isolation from unrelated FFI wrapper machinery.
#[test]
fn test_free_function_len_companion_name_is_derived_from_the_primary_symbol() {
    let func = FunctionDef {
        name: "extract_html_title".to_string(),
        rust_path: "my_lib::extract_html_title".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
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
    };
    let path_map = ahash::AHashMap::default();
    let enum_names = ahash::AHashSet::default();

    let primary = crate::codegen::c_consumer::free_function_symbol("my_lib", "extract_html_title");
    let companion =
        super::super::functions::gen_free_function_len_companion(&func, "my_lib", "my_lib", &path_map, &enum_names);

    assert!(
        companion.contains(&format!("fn {primary}_len(")),
        "expected companion symbol `{primary}_len`, got:\n{companion}"
    );
    assert!(
        companion.contains(&format!("last_return_len(\"{primary}\")")),
        "companion must look up the return length under the exact primary symbol `{primary}`, not a \
         parallel computation of it, got:\n{companion}"
    );
}

/// A streaming method's FFI export must carry the same gate as a plain method's, and the feature
/// naming that gate must reach cbindgen's `[defines]` — the two halves have to agree or the
/// header declares a symbol the cdylib never exported.
///
/// `gen_streaming_method_wrapper` emitted no `#[cfg]` at all, not even the owning type's, so a
/// gated streaming adapter produced an unguarded `pub unsafe extern "C" fn` whose body calls a
/// core method that feature set never compiled.
#[test]
fn streaming_method_wrapper_carries_the_combined_owner_and_method_gate() {
    let typ = TypeDef {
        name: "Client".to_string(),
        rust_path: "my_lib::Client".to_string(),
        is_opaque: true,
        cfg: Some(r#"feature = "client""#.to_string()),
        methods: vec![MethodDef {
            name: "stream".to_string(),
            cfg: Some(r#"all(feature = "client", feature = "streaming")"#.to_string()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let wrapper =
        super::super::functions::gen_streaming_method_wrapper(&typ, &typ.methods[0], "ml", "my_lib", "    Ok(0)");

    assert!(
        wrapper.contains(r#"#[cfg(all(feature = "client", feature = "streaming"))]"#),
        "the streaming export must be gated on owner AND method cfg, got:\n{wrapper}"
    );
    let cfg_position = wrapper.find("#[cfg(").expect("gate must be present");
    let no_mangle_position = wrapper
        .find("#[unsafe(no_mangle)]")
        .expect("export attribute must be present");
    assert!(
        cfg_position < no_mangle_position,
        "the gate must precede the export attribute so it applies to the item, got:\n{wrapper}"
    );

    // The registration side: cbindgen only guards a declaration for a feature it was told about,
    // so both features must land in `[defines]`. Missing ones make the header declare the symbol
    // unconditionally. ~keep
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![typ],
        ..ApiSurface::default()
    };
    let cbindgen_toml = super::super::helpers::gen_cbindgen_toml(
        "ml",
        &api,
        &std::collections::HashMap::new(),
        &std::collections::BTreeSet::new(),
    );
    assert!(
        cbindgen_toml.contains("feature = client"),
        "cbindgen [defines] must carry the owner feature, got:\n{cbindgen_toml}"
    );
    assert!(
        cbindgen_toml.contains("feature = streaming"),
        "cbindgen [defines] must carry the method feature, got:\n{cbindgen_toml}"
    );
}

/// The ungated control: an ungated streaming method on an ungated type must emit no `#[cfg]` at
/// all, so this change is a strict addition rather than a rewrite of existing output.
#[test]
fn streaming_method_wrapper_emits_no_gate_when_neither_side_is_gated() {
    let typ = TypeDef {
        name: "Client".to_string(),
        rust_path: "my_lib::Client".to_string(),
        is_opaque: true,
        cfg: None,
        methods: vec![MethodDef {
            name: "stream".to_string(),
            cfg: None,
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let wrapper =
        super::super::functions::gen_streaming_method_wrapper(&typ, &typ.methods[0], "ml", "my_lib", "    Ok(0)");

    assert!(
        !wrapper.contains("#[cfg("),
        "an ungated streaming method must emit no gate, got:\n{wrapper}"
    );
}

/// `AssertUnwindSafe(|| { Type::method() })` reduces to a closure whose entire body is a bare
/// zero-arg call when the method is static, takes no parameters, and needs no null check,
/// handle acquisition, or result conversion -- exactly the shape `clippy::redundant_closure`
/// flags. The fix passes the callee path directly to `AssertUnwindSafe` instead of opening a
/// closure around it.
#[test]
fn static_zero_arg_method_call_avoids_redundant_closure() {
    let typ = TypeDef {
        name: "Registry".to_string(),
        rust_path: "my_lib::Registry".to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "reset".to_string(),
            is_static: true,
            receiver: None,
            return_type: TypeRef::Unit,
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![typ],
        ..ApiSurface::default()
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Prove the fixture actually reached the wrapper before trusting the absence check below --
    // an absence assertion against a fixture that emitted nothing would be vacuous.
    assert!(
        lib.content.contains("fn my_lib_registry_reset("),
        "fixture must actually emit the wrapper, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("AssertUnwindSafe(my_lib::Registry::reset)"),
        "expected the bare callee path passed directly to AssertUnwindSafe, got:\n{}",
        lib.content
    );
    assert!(
        !lib.content
            .contains("AssertUnwindSafe(|| {\n my_lib::Registry::reset()"),
        "must not open a closure whose entire body is the bare zero-arg call \
         (clippy::redundant_closure):\n{}",
        lib.content
    );
    syn::parse_file(&lib.content).expect("generated FFI lib.rs must parse as Rust");
}

/// Control for the fix above: a static method that still needs a parameter conversion (i.e. is
/// NOT trivially inlinable) must keep opening a closure -- the fix must narrowly target the
/// zero-arg case, not stop emitting closures altogether.
#[test]
fn static_method_with_param_keeps_the_closure() {
    let typ = TypeDef {
        name: "Registry".to_string(),
        rust_path: "my_lib::Registry".to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "reset_named".to_string(),
            is_static: true,
            receiver: None,
            return_type: TypeRef::Unit,
            params: vec![ParamDef {
                name: "flag".to_string(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                ..ParamDef::default()
            }],
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![typ],
        ..ApiSurface::default()
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn my_lib_registry_reset_named("),
        "{}",
        lib.content
    );
    assert!(
        lib.content.contains("AssertUnwindSafe(|| {"),
        "a method with a parameter must still open a closure, got:\n{}",
        lib.content
    );
}

/// Mirror of the static-method fix for free functions: a zero-arg free function that needs no
/// result conversion reduces `AssertUnwindSafe(|| { core_fn_path() })` to a bare-call closure
/// body too, via the `call_inline.jinja` path in `gen_free_function` rather than
/// `static_method_call.jinja`. Uses a return type that is NOT `Bool` and NOT `returns_c_char`,
/// since a `c_char`-returning free function gets a `set_last_return_len(...)` statement injected
/// into the closure body, which would make this callee NOT trivially inlinable.
#[test]
fn free_zero_arg_function_call_avoids_redundant_closure() {
    let api = ApiSurface {
        crate_name: "sample".to_string(),
        functions: vec![FunctionDef {
            name: "poll_status".to_string(),
            rust_path: "my_lib::poll_status".to_string(),
            return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn my_lib_poll_status("),
        "fixture must actually emit the wrapper, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("AssertUnwindSafe(my_lib::poll_status)"),
        "expected the bare callee path passed directly to AssertUnwindSafe, got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("AssertUnwindSafe(|| {\n my_lib::poll_status()"),
        "must not open a closure whose entire body is the bare zero-arg call \
         (clippy::redundant_closure):\n{}",
        lib.content
    );
    syn::parse_file(&lib.content).expect("generated FFI lib.rs must parse as Rust");
}
