use super::super::FfiBackend;
use super::common::*;
use crate::backends::ffi::gen_bindings::types::gen_field_accessor;
use crate::core::backend::Backend;
use crate::core::ir::*;

#[test]
fn test_legacy_visitor_callbacks_use_configured_function_signature() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "doc"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "RenderHandle"
param_name = "renderer"
context_type = "NodeContext"
result_type = "VisitResult"
"#,
    );
    let mut api = visitor_api();
    api.types.push(TypeDef {
        name: "RenderSettings".to_string(),
        rust_path: "my_lib::RenderSettings".to_string(),
        fields: vec![],
        is_clone: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "RenderedDocument".to_string(),
        rust_path: "my_lib::RenderedDocument".to_string(),
        fields: vec![],
        is_clone: true,
        is_return_type: true,
        ..TypeDef::default()
    });
    api.functions.push(FunctionDef {
        name: "render_document".to_string(),
        rust_path: "my_lib::render_document".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                is_ref: false,
                ..ParamDef::default()
            },
            ParamDef {
                name: "settings".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("RenderSettings".to_string()))),
                optional: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "renderer".to_string(),
                ty: TypeRef::Named("RenderHandle".to_string()),
                optional: true,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("RenderedDocument".to_string()),
        is_async: false,
        error_type: Some("RenderError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: true,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("fn doc_render_document("));
    assert!(lib.content.contains("fn doc_render_document_with_visitor("));
    assert!(lib.content.contains("settings: *const my_lib::RenderSettings"));
    assert!(lib.content.contains(") -> *mut my_lib::RenderedDocument"));
    assert!(
        lib.content
            .contains("match my_lib::render_document(source_rs, settings_rs, None)")
    );
    assert!(
        lib.content
            .contains("match my_lib::render_document(source_rs, settings_rs, visitor_handle)")
    );
    assert!(!lib.content.contains("my_lib::convert("));
    assert!(
        !lib.content.contains("ConversionOptions") && !lib.content.contains("ConversionResult"),
        "legacy visitor callback path must not assume conversion-shaped names"
    );
}

/// Fix 1 regression test: `type_ref_to_rust_type` must use the configured `core_import`
/// for `TypeRef::Named` variants, not a hard-coded `"sample_core"` prefix.
///
/// When a crate uses `core_import = "my_custom_lib"`, generated Vec/Map turbofish type
/// annotations that reference Named types must use `my_custom_lib::TypeName`, not
/// `sample_core::TypeName`.
#[test]
fn test_core_import_parameterization_uses_configured_import_not_hardcoded_sample_crate() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-custom-lib"
sources = ["src/lib.rs"]
core_import = "my_custom_lib"
"#,
    );
    let api = sample_api();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The generated code must not contain the old hard-coded sample_core prefix
    // in any type annotation position.  (It may legitimately appear in doc comments
    // or string literals, but never as a Rust path qualifier in generated code.)
    assert!(
        !lib.content.contains("sample_crate::"),
        "generated code must not hard-code 'sample_crate::' when core_import is 'my_custom_lib'; got:\n{}",
        &lib.content[..lib.content.len().min(2000)]
    );
    // The configured import must appear as a qualifier for core types
    assert!(
        lib.content.contains("my_custom_lib::"),
        "generated code must use the configured core_import 'my_custom_lib::' as a type qualifier"
    );
}

/// Fix 2 regression test: functions returning `Result<Vec<u8>>` must use the out-param
/// convention (i32 return + out_ptr/out_len/out_cap parameters) and the module must
/// include a companion `{prefix}_free_bytes` function.
#[test]
fn test_bytes_result_return_uses_out_params_and_emits_free_bytes() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "render_page".to_string(),
            rust_path: "my_lib::render_page".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "page_index".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
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
            }],
            return_type: TypeRef::Bytes,
            is_async: false,
            error_type: Some("MyError".to_string()),
            doc: "Render a page to PNG bytes.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The function must use out-params, not return *mut u8 directly
    assert!(
        lib.content.contains("out_ptr: *mut *mut u8"),
        "Result<Vec<u8>> function must have out_ptr out-param"
    );
    assert!(
        lib.content.contains("out_len: *mut usize"),
        "Result<Vec<u8>> function must have out_len out-param"
    );
    assert!(
        lib.content.contains("out_cap: *mut usize"),
        "Result<Vec<u8>> function must have out_cap out-param"
    );
    // The function must return i32, not *mut u8
    assert!(
        lib.content.contains("fn my_lib_render_page("),
        "function must be emitted with the correct FFI name"
    );
    // Vec::into_raw_parts must be used to decompose the result
    assert!(
        lib.content.contains("into_raw_parts()"),
        "Result<Vec<u8>> success arm must use Vec::into_raw_parts()"
    );
    // The module must include a free_bytes companion
    assert!(
        lib.content.contains("fn my_lib_free_bytes("),
        "module must include my_lib_free_bytes companion function"
    );
    assert!(
        lib.content.contains("Vec::from_raw_parts(ptr, len, cap)"),
        "free_bytes must reconstruct and drop the Vec via Vec::from_raw_parts"
    );
}

/// Verify that a `Streaming` adapter causes codegen to emit the three iterator-handle
/// functions (`_start`, `_next`, `_free`) plus the opaque handle struct.
#[test]
fn test_streaming_adapter_emits_iterator_handle_functions() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatChunk"
error_type = "MyError"
request_type = "my_lib::ChatRequest"

[[crates.adapters.params]]
name = "req"
type = "ChatRequest"
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "my_lib::DefaultClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![MethodDef {
                name: "chat_stream".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: true,
                is_static: false,
                error_type: Some("MyError".to_string()),
                doc: String::new(),
                sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                receiver: Some(ReceiverKind::Ref),
                trait_source: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            }],
            is_opaque: true,
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
            has_lifetime_params: false,
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
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Opaque handle struct must be present
    assert!(
        lib.content.contains("MlDefaultClientChatStreamStreamHandle"),
        "handle struct must be emitted: got\n{}",
        &lib.content[..lib.content.len().min(3000)]
    );

    // All three exported functions must be present
    assert!(
        lib.content.contains("fn ml_default_client_chat_stream_start("),
        "_start function must be emitted"
    );
    assert!(
        lib.content.contains("fn ml_default_client_chat_stream_next("),
        "_next function must be emitted"
    );
    assert!(
        lib.content.contains("fn ml_default_client_chat_stream_free("),
        "_free function must be emitted"
    );

    // Functions must be #[unsafe(no_mangle)] extern "C"
    assert!(
        lib.content.contains("#[unsafe(no_mangle)]"),
        "functions must be marked #[unsafe(no_mangle)]"
    );
    assert!(
        lib.content
            .contains("pub unsafe extern \"C\" fn ml_default_client_chat_stream_start"),
        "_start must be pub unsafe extern C"
    );
    assert!(
        lib.content
            .contains("pub unsafe extern \"C\" fn ml_default_client_chat_stream_next"),
        "_next must be pub unsafe extern C"
    );
    assert!(
        lib.content
            .contains("pub unsafe extern \"C\" fn ml_default_client_chat_stream_free"),
        "_free must be pub unsafe extern C"
    );

    // _next must return a pointer to the item type
    assert!(
        lib.content.contains("-> *mut my_lib::ChatChunk"),
        "_next must return *mut my_lib::ChatChunk"
    );

    // _free must be null-safe
    assert!(
        lib.content.contains("if !handle.is_null()"),
        "_free must check for null before dropping"
    );

    // SAFETY comments must be present
    assert!(
        lib.content.contains("// SAFETY:"),
        "generated code must include SAFETY comments on unsafe blocks"
    );

    // Error protocol: _next sets last_error on stream errors
    assert!(
        lib.content.contains("set_last_error"),
        "_next must call set_last_error on error"
    );
}

#[test]
fn test_client_constructors_emits_type_new_function() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"

[workspace.client_constructors.DefaultClient]
body = "my_lib::DefaultClient::new(api_key)"
error_type = "String"

[[workspace.client_constructors.DefaultClient.params]]
name = "api_key"
type = "*const std::ffi::c_char"
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "my_lib::DefaultClient".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
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
            has_lifetime_params: false,
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
    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn ml_default_client_new("),
        "should emit _new function: got\n{}",
        &lib.content[..lib.content.len().min(2000)]
    );
    assert!(
        lib.content.contains("api_key: *const std::ffi::c_char"),
        "should include typed param in signature"
    );
    assert!(
        lib.content.contains("-> *mut my_lib::DefaultClient"),
        "should return *mut TypeName"
    );
    assert!(
        lib.content.contains("clear_last_error"),
        "should call clear_last_error at function entry"
    );
    assert!(
        lib.content.contains("set_last_error"),
        "should call set_last_error on Err path"
    );
    assert!(
        lib.content.contains("Box::into_raw(Box::new(val))"),
        "should box the value on Ok path"
    );
}

/// Build an `ApiSurface` with a free function whose `metadata` param is
/// `Option<&AHashMap<Cow<'static, str>, serde_json::Value>>` — the shape that
/// `sample_core::text::quality::calculate_quality_score` uses. The IR records
/// `map_is_ahash=true` and `map_key_is_cow=true` on the param.
fn ahashmap_cow_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "calculate_quality_score".to_string(),
            rust_path: "my_lib::calculate_quality_score".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "text".to_string(),
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
                },
                ParamDef {
                    name: "metadata".to_string(),
                    ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json)),
                    optional: true,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: true,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: true,
                    map_key_is_cow: true,
                    vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                },
            ],
            return_type: TypeRef::Primitive(PrimitiveType::F64),
            is_async: false,
            error_type: None,
            doc: "Calculate quality score for text.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// The FFI wrapper for a function with `Option<&AHashMap<Cow<'static, str>, Value>>` must:
/// 1. Deserialize using `ahash::AHashMap<std::borrow::Cow<'static, str>, ...>` turbofish
/// 2. Pass `.as_ref()` to the core function (not `.as_deref()`, which fails for HashMap)
#[test]
fn test_optional_ahashmap_cow_key_uses_as_ref_not_as_deref() {
    let api = ahashmap_cow_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // The deserialization turbofish must target AHashMap with Cow key, not HashMap<String, _>
    assert!(
        lib.content.contains("ahash::AHashMap<std::borrow::Cow<'static, str>,"),
        "should deserialize into AHashMap<Cow<'static, str>, ...>, got:\n{}",
        if lib.content.len() > 3000 {
            &lib.content[lib.content.len() - 3000..]
        } else {
            &lib.content
        }
    );

    // The call must use .as_ref() not .as_deref() — HashMap doesn't impl Deref
    assert!(
        lib.content.contains("metadata_rs.as_ref()"),
        "should pass metadata_rs.as_ref() (not .as_deref()), got:\n{}",
        if lib.content.len() > 3000 {
            &lib.content[lib.content.len() - 3000..]
        } else {
            &lib.content
        }
    );
    assert!(
        !lib.content.contains("metadata_rs.as_deref()"),
        "must NOT use .as_deref() on HashMap — HashMap does not impl Deref"
    );
}

/// Regression guard: `Option<Vec<String>>` with `is_ref=true` must still use
/// `.as_deref()` since `Vec<T>: Deref<Target=[T]>`.
#[test]
fn test_optional_vec_still_uses_as_deref() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_items".to_string(),
            rust_path: "my_lib::process_items".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "items".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::String)),
                optional: true,
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
            return_type: TypeRef::Unit,
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
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
        lib.content.contains("items_rs.as_deref()"),
        "Optional Vec<String> with is_ref=true should still use .as_deref()"
    );
}

/// Regression test for the sample_crate issue tracker.
/// Struct fields typed `Option<Bytes>` / `Option<Vec<u8>>` (e.g. EmailAttachment.data)
/// must emit the same (ptr, out_len: *mut usize) contract as non-optional Bytes fields.
/// Previously the needs_len_out predicate only matched `Bytes && !optional`.
#[test]
fn test_optional_bytes_field_accessor_emits_out_len_and_length_writes() {
    let field = FieldDef {
        name: "data".to_string(),
        ty: TypeRef::Bytes,
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
        original_type: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let typ = TypeDef {
        name: "EmailAttachment".to_string(),
        rust_path: "my_lib::EmailAttachment".to_string(),
        original_rust_path: String::new(),
        fields: vec![field.clone()],
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
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    let code = gen_field_accessor(
        &typ,
        &field,
        "kr",
        "my_lib",
        &ahash::AHashMap::<String, String>::new(),
        &ahash::AHashSet::<String>::new(),
        &ahash::AHashSet::<String>::new(),
        &::std::collections::HashMap::<String, String>::new(),
    );

    // The header must include the out_len companion (the reported contract violation).
    assert!(
        code.contains("out_len: *mut usize"),
        "optional Bytes field accessor must declare out_len param (issue #118), got:\n{code}"
    );

    // Body must write real length on Some path.
    assert!(
        code.contains("*out_len"),
        "optional Bytes field must write length to out_len (Some path writes real len, None writes 0), got:\n{code}"
    );

    // None arm must write 0, not just any *out_len write.
    assert!(
        code.contains("*out_len = 0"),
        "optional Bytes None arm must write 0 to out_len, got:\n{code}"
    );

    // Both arms must null-check out_len before dereferencing it.
    assert!(
        code.contains("!out_len.is_null()"),
        "optional Bytes field must null-check out_len before writing, got:\n{code}"
    );
}
