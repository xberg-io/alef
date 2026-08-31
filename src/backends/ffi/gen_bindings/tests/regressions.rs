use super::super::FfiBackend;
use super::common::*;

#[test]
fn emitted_code_blocks_preserve_newline_after_safety_comments() {
    let first = crate::backends::ffi::template_env::render(
        "emitted_code_block.jinja",
        minijinja::context! { content => "// SAFETY: the pointer was validated." },
    );
    let second = crate::backends::ffi::template_env::render(
        "emitted_code_block.jinja",
        minijinja::context! { content => "unsafe { consume(); }" },
    );
    let source = format!("fn boundary() {{\n{first}{second}}}\n");

    assert!(source.contains("validated.\nunsafe"), "{source}");
    syn::parse_file(&source).expect("a safety comment must not absorb the following unsafe block");
}
use crate::backends::ffi::gen_bindings::types::gen_field_accessor;
use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::*;

#[test]
fn ffi_result_errors_use_stable_variant_taxonomy_codes() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let error = ErrorDef {
        name: "RequestError".to_string(),
        rust_path: "sample_lib::RequestError".to_string(),
        variants: vec![
            ErrorVariant {
                error_code: Some(100),
                name: "InvalidInput".to_string(),
                is_unit: true,
                ..ErrorVariant::default()
            },
            ErrorVariant {
                error_code: Some(101),
                name: "Unavailable".to_string(),
                is_tuple: true,
                ..ErrorVariant::default()
            },
            ErrorVariant {
                error_code: None,
                name: "Legacy".to_string(),
                is_unit: true,
                ..ErrorVariant::default()
            },
        ],
        original_rust_path: String::new(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: VersionAnnotation::default(),
    };
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        errors: vec![
            error,
            ErrorDef {
                name: "StorageError".to_string(),
                rust_path: "sample_lib::StorageError".to_string(),
                variants: vec![ErrorVariant {
                    error_code: Some(102),
                    name: "Unavailable".to_string(),
                    is_unit: true,
                    ..ErrorVariant::default()
                }],
                original_rust_path: String::new(),
                doc: String::new(),
                methods: Vec::new(),
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: VersionAnnotation::default(),
            },
        ],
        functions: vec![FunctionDef {
            name: "execute".to_string(),
            rust_path: "sample_lib::execute".to_string(),
            return_type: TypeRef::Unit,
            error_type: Some("RequestError".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    let codes = api.error_taxonomy();

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    assert!(lib.content.contains("pub enum AlefFfiErrorCode"));
    assert!(lib.content.contains("SampleAlefNone = 0"));
    assert!(lib.content.contains("SampleAlefConversion = 1"));
    assert!(lib.content.contains("SampleAlefUnknown = 2"));
    assert!(lib.content.contains("SampleAlefPanic = 3"));
    assert!(lib.content.contains("SampleAlefInvalidHandle = 4"));
    assert!(lib.content.contains("SampleLibRequestErrorUnavailable = 101"));
    assert!(lib.content.contains("SampleLibStorageErrorUnavailable = 102"));
    assert!(lib.content.contains("sample_lib::RequestError::InvalidInput =>"));
    assert!(lib.content.contains("sample_lib::RequestError::Unavailable(..) =>"));
    assert!(
        lib.content
            .contains("sample_lib::RequestError::Legacy => ALEF_FFI_UNKNOWN_ERROR")
    );
    assert!(lib.content.contains("_ => ALEF_FFI_UNKNOWN_ERROR"));
    assert!(lib.content.contains("set_last_error(alef_ffi_error_code(&e)"));
    for taxonomy in codes {
        assert!(lib.content.contains(&format!("= {}", taxonomy.code)));
    }
}

/// cbindgen copies enum member identifiers into the C header verbatim, and C enum
/// members live in the global namespace. A bare member therefore collides with
/// platform headers — X11 defines `None` as `0L` — and with a second alef-generated
/// library linked into the same translation unit. ~keep
#[test]
fn ffi_error_enum_members_are_namespaced_for_the_c_global_scope() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        errors: vec![ErrorDef {
            name: "Error".to_string(),
            rust_path: "sample_lib::error::Error".to_string(),
            variants: vec![ErrorVariant {
                error_code: Some(100),
                name: "NotFound".to_string(),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: VersionAnnotation::default(),
        }],
        functions: vec![FunctionDef {
            name: "execute".to_string(),
            rust_path: "sample_lib::execute".to_string(),
            return_type: TypeRef::Unit,
            error_type: Some("Error".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    let body = lib
        .content
        .split_once("pub enum AlefFfiErrorCode {")
        .expect("the generated crate must declare the error enum")
        .1
        .split_once('}')
        .expect("the error enum must be terminated")
        .0;

    let members: Vec<&str> = body
        .lines()
        .filter_map(|line| line.split_once('=').map(|(member, _)| member.trim()))
        .collect();
    assert_eq!(
        members.len(),
        6,
        "expected five built-ins plus one taxonomy member: {members:?}"
    );
    for member in &members {
        assert!(
            member.starts_with("SampleAlef") || member.starts_with("SampleLib"),
            "member `{member}` is not namespaced and would collide in C's global scope"
        );
    }

    assert!(members.contains(&"SampleLibErrorNotFound"), "{members:?}");
}

#[test]
fn ffi_rejects_duplicate_and_reserved_domain_error_codes() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "sample"
"#,
    );
    let make_api = |codes: [u32; 2]| ApiSurface {
        errors: vec![ErrorDef {
            name: "RequestError".to_string(),
            rust_path: "sample_lib::RequestError".to_string(),
            variants: codes
                .into_iter()
                .enumerate()
                .map(|(index, code)| ErrorVariant {
                    error_code: Some(code),
                    name: format!("Variant{index}"),
                    is_unit: true,
                    ..ErrorVariant::default()
                })
                .collect(),
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: VersionAnnotation::default(),
        }],
        ..ApiSurface::default()
    };

    let duplicate = FfiBackend
        .generate_bindings(&make_api([100, 100]), &config)
        .unwrap_err();
    assert!(
        duplicate
            .to_string()
            .contains("sample_lib::RequestError::Variant1 duplicates")
    );
    let reserved = FfiBackend.generate_bindings(&make_api([2, 100]), &config).unwrap_err();
    assert!(
        reserved
            .to_string()
            .contains("sample_lib::RequestError::Variant0 is outside the domain range")
    );
}

#[test]
fn cbindgen_maps_feature_gates_to_header_macros() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let mut api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        ..ApiSurface::default()
    };
    api.functions.push(FunctionDef {
        name: "render".to_string(),
        rust_path: "sample_lib::render".to_string(),
        cfg: Some(r#"all(feature = "document-render", feature = "native")"#.to_string()),
        ..FunctionDef::default()
    });

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|file| file.path.ends_with("cbindgen.toml")).unwrap();

    // cbindgen's `[defines]` key matcher (`DefineKey::load` in cbindgen's
    // `ir::cfg`) splits the key on `=` and only trims whitespace — it never
    // strips quotes. The value must therefore be the bare feature name
    // (matching cbindgen's own documented `"feature = serde" = "DEFINE_SERDE"`
    // example), not a quoted string: a quoted value never equals the unquoted
    // `cfg_value` cbindgen extracts from a parsed `#[cfg(feature = "...")]`
    // attribute via `LitStr::value()`, so the mapping silently fails to match
    // and the item is emitted with no `#if` guard at all. ~keep
    assert!(
        cbindgen
            .content
            .contains(r#""feature = document-render" = "SAMPLE_FEATURE_DOCUMENT_RENDER""#)
    );
    assert!(
        cbindgen
            .content
            .contains(r#""feature = native" = "SAMPLE_FEATURE_NATIVE""#)
    );
    assert!(
        !cbindgen.content.contains('\\'),
        "cbindgen.toml [defines] keys must not carry escaped quotes, got:\n{}",
        cbindgen.content
    );
}

#[test]
fn ffi_omits_free_bytes_without_an_out_param_producer() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![FunctionDef {
            name: "render".to_string(),
            rust_path: "sample_lib::render".to_string(),
            return_type: TypeRef::String,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    assert!(!lib.content.contains("fn sample_free_bytes("));
}

#[test]
fn ffi_entrypoints_clear_stale_errors_but_error_accessors_preserve_them() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        ..ApiSurface::default()
    };

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content
            .contains("fn catch_ffi_panic<T>(fallback: T, body: impl FnOnce() -> T) -> T {\n    clear_last_error();")
    );
    assert!(
        lib.content
            .contains("catch_ffi_panic_preserving_error(ALEF_FFI_PANIC_ERROR, || LAST_ERROR_CODE.with_borrow")
    );
    assert!(
        lib.content
            .contains("catch_ffi_panic_preserving_error(std::ptr::null(), || LAST_ERROR_CONTEXT.with_borrow")
    );
}

/// Regression: `catch_ffi_panic`/`catch_ffi_panic_preserving_error` unconditionally stamped
/// the generic panic marker on its `Err` branch, clobbering a more specific error a body may
/// have already reported via `set_last_error`/`set_handle_error` moments before an unrelated
/// panic later in the same call. Compiles and runs the emitted `last_error.jinja` module
/// standalone to prove the specific error now survives, while an unmarked panic still gets
/// the generic marker (the guard must not suppress it unconditionally). ~keep
#[test]
fn catch_ffi_panic_preserves_a_more_specific_error_set_before_an_unrelated_panic() {
    let module = crate::backends::ffi::template_env::render(
        "last_error.jinja",
        minijinja::context! {
            prefix => "sample",
            builtin_prefix => "",
            error_code_impls => Vec::<String>::new(),
            has_error_code_impls => false,
            taxonomy => Vec::<String>::new(),
            no_error_code => 0,
            conversion_error_code => 1,
            unknown_error_code => 2,
            panic_error_code => 3,
            invalid_handle_error_code => 4,
        },
    );

    let source = format!(
        r#"
use std::cell::RefCell;
use std::ffi::{{c_char, CString}};

{module}

fn main() {{
    std::panic::set_hook(Box::new(|_| {{}}));

    let result = catch_ffi_panic(0, || {{
        set_last_error(7, "a specific domain error");
        panic!("unrelated internal bug");
    }});
    assert_eq!(result, 0);
    assert_eq!(
        LAST_ERROR_CODE.with_borrow(|c| *c),
        7,
        "the specific error code must survive the panic"
    );
    let ctx = LAST_ERROR_CONTEXT.with_borrow(std::clone::Clone::clone);
    assert_eq!(
        ctx.as_deref().map(|s| s.to_str().unwrap()),
        Some("a specific domain error"),
        "the specific error message must survive the panic"
    );

    let result = catch_ffi_panic(0, || {{
        panic!("no prior error was recorded");
    }});
    assert_eq!(result, 0);
    assert_eq!(
        LAST_ERROR_CODE.with_borrow(|c| *c),
        3,
        "an unmarked panic must still be reported via the generic panic marker"
    );
}}
"#
    );

    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("catch_ffi_panic_preserving_error.rs");
    let binary_path = directory.path().join("catch-ffi-panic-preserving-error-test");
    std::fs::write(&source_path, &source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(
        compile.status.success(),
        "{}\n---source---\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(&binary_path)
        .current_dir(directory.path())
        .output()
        .expect("run compiled harness");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
}

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

    assert!(
        lib.content
            .contains("#![allow(unsafe_op_in_unsafe_fn, unsafe_attr_outside_unsafe)]"),
        "generated FFI crate must contain implementation-only unsafe lints"
    );
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

    assert!(
        !lib.content.contains("sample_crate::"),
        "generated code must not hard-code 'sample_crate::' when core_import is 'my_custom_lib'; got:\n{}",
        &lib.content[..lib.content.len().min(2000)]
    );
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
    assert!(
        lib.content.contains("fn my_lib_render_page("),
        "function must be emitted with the correct FFI name"
    );
    assert!(
        lib.content.contains("into_boxed_slice()"),
        "Result<Vec<u8>> success arm must transfer a boxed slice"
    );
    assert!(
        lib.content.contains("fn my_lib_free_bytes("),
        "module must include my_lib_free_bytes companion function"
    );
    assert!(
        lib.content.contains("Box::<[u8]>::from_raw(slice)"),
        "free_bytes must reconstruct and drop the boxed slice"
    );
    assert!(
        lib.content.contains("if len != cap"),
        "free_bytes must reject modified allocation metadata"
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
                cfg: None,
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

    syn::parse_file(&lib.content).expect("streaming adapter output must be valid Rust syntax");

    assert!(
        lib.content.contains("MlDefaultClientChatStreamStreamHandle"),
        "handle struct must be emitted: got\n{}",
        &lib.content[..lib.content.len().min(3000)]
    );

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

    assert!(
        lib.content.contains("-> AlefHandle"),
        "stream functions must return generational handles"
    );

    assert!(
        lib.content
            .contains("remove_handle::<MlDefaultClientChatStreamStreamHandle>(handle)"),
        "_free must invalidate the stream token through the registry"
    );

    assert!(
        lib.content.contains("insert_handle(chunk)"),
        "chunks must use registry tokens"
    );
    assert!(
        lib.content.contains("acquire_handles(&requests)"),
        "inputs must be validated together"
    );
    assert!(
        lib.content.contains("TypeId::of::<my_lib::ChatRequest>()"),
        "qualified request types must be used verbatim"
    );
    assert!(
        lib.content.contains("locked_handle_ptr::<my_lib::ChatRequest>"),
        "request handle lookup must not duplicate the configured module path"
    );
    assert!(!lib.content.contains("my_lib::my_lib::ChatRequest"));

    // SAFETY comments must be present
    assert!(
        lib.content.contains("// SAFETY:"),
        "generated code must include SAFETY comments on unsafe blocks"
    );

    assert!(
        lib.content.contains("set_last_error"),
        "_next must call set_last_error on error"
    );
}

/// A client type whose handle is produced by an ordinary function (`connect`, mirroring a
/// constructor) and consumed by the callback-style streaming FFI wrapper emitted from
/// `streaming_method_wrapper.jinja`. Every FFI producer hands out `Named` types — client
/// types included — as a scalar `AlefHandle` via `insert_handle`, never `Box::into_raw`,
/// so the streaming wrapper's `client` parameter must accept that same scalar shape rather
/// than a `TYPE *`/`const TYPE *` struct pointer built by construction from a handle that
/// was never a raw pointer to begin with.
fn streaming_client_api() -> (ApiSurface, ResolvedCrateConfig) {
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
            is_opaque: true,
            methods: vec![MethodDef {
                name: "chat_stream".to_string(),
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: true,
                error_type: Some("MyError".to_string()),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        functions: vec![FunctionDef {
            name: "connect".to_string(),
            rust_path: "my_lib::connect".to_string(),
            return_type: TypeRef::Named("DefaultClient".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };
    (api, config)
}

/// C ABI regression: the streaming wrapper's `client` parameter must be declared as the same
/// scalar `AlefHandle` every producer of that type returns, not a `TYPE *`/`const TYPE *`
/// struct pointer. Before the fix, `streaming_method_wrapper.jinja` unconditionally declared
/// `client: *const {{ qualified }}`, so cbindgen rendered a pointer-shaped C parameter for a
/// handle that was never a raw pointer, and a caller holding a valid client handle from any
/// producer could not call the streaming entry point without a cast that is wrong by
/// construction.
#[test]
fn streaming_wrapper_client_param_is_scalar_handle_not_struct_pointer() {
    let (api, config) = streaming_client_api();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let sig_marker = "pub unsafe extern \"C\" fn ml_default_client_chat_stream(";
    let sig_start = lib
        .content
        .find(sig_marker)
        .unwrap_or_else(|| panic!("streaming wrapper must be emitted, got:\n{}", lib.content));
    let after_start = &lib.content[sig_start..];
    let sig_end = after_start.find(") -> i32").expect("streaming wrapper must return i32");
    let signature = &after_start[..sig_end];

    assert!(
        signature.contains("client: AlefHandle"),
        "streaming wrapper's client parameter must be a scalar AlefHandle, got:\n{signature}"
    );
    assert!(
        !signature.contains("*const my_lib::DefaultClient") && !signature.contains("*mut my_lib::DefaultClient"),
        "streaming wrapper's declared signature must not accept the client as a struct pointer, got:\n{signature}"
    );

    syn::parse_file(&lib.content).expect("streaming wrapper output must parse as valid Rust");
}

/// Rustc-consistency regression: the streaming wrapper's `client` handle must be resolved
/// through the same `acquire_handles`/`locked_handle_ptr` registry idiom every other consumer
/// of a client handle uses (see `null_check_self_ref.jinja` and `gen_stream_handle_functions`'s
/// own `_start` function, which already got this right), and must agree with what an ordinary
/// producer function (`connect`) hands out for the same type.
#[test]
fn streaming_wrapper_client_handle_agrees_with_producer_and_uses_registry() {
    let (api, config) = streaming_client_api();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn ml_connect(") && lib.content.contains(") -> AlefHandle"),
        "producer function must hand out a scalar handle, got:\n{}",
        lib.content
    );

    let streaming_fn = lib
        .content
        .split("fn ml_default_client_chat_stream(")
        .nth(1)
        .expect("streaming wrapper must be emitted");
    assert!(
        streaming_fn.contains("TypeId::of::<my_lib::DefaultClient>()"),
        "streaming wrapper must validate the client handle against the qualified client type, got:\n{streaming_fn}"
    );
    assert!(
        streaming_fn.contains("locked_handle_ptr::<my_lib::DefaultClient>"),
        "streaming wrapper must resolve the client handle through the shared registry, not a raw cast, got:\n{streaming_fn}"
    );
    assert!(
        !lib.content.contains("my_lib::my_lib::DefaultClient"),
        "qualified client type must not duplicate the configured module path"
    );

    syn::parse_file(&lib.content).expect("streaming wrapper output must parse as valid Rust");
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
        lib.content.contains("-> AlefHandle"),
        "should return a generational handle"
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
        lib.content.contains("insert_handle(val)"),
        "should register the value on the Ok path"
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

    assert!(
        lib.content.contains("ahash::AHashMap<std::borrow::Cow<'static, str>,"),
        "should deserialize into AHashMap<Cow<'static, str>, ...>, got:\n{}",
        if lib.content.len() > 3000 {
            &lib.content[lib.content.len() - 3000..]
        } else {
            &lib.content
        }
    );

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
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
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
    )
    .expect("optional bytes accessor");

    assert!(
        code.contains("out_len: *mut usize"),
        "optional Bytes field accessor must declare out_len param (issue #118), got:\n{code}"
    );

    assert!(
        code.contains("*out_len"),
        "optional Bytes field must write length to out_len (Some path writes real len, None writes 0), got:\n{code}"
    );

    assert!(
        code.contains("*out_len = 0"),
        "optional Bytes None arm must write 0 to out_len, got:\n{code}"
    );

    assert!(
        code.contains("!out_len.is_null()"),
        "optional Bytes field must null-check out_len before writing, got:\n{code}"
    );
}

#[test]
fn overridden_named_field_accessor_returns_handle_token() {
    let field = FieldDef {
        name: "child".to_string(),
        ty: TypeRef::Named("Child".to_string()),
        ..FieldDef::default()
    };
    let typ = TypeDef {
        name: "Container".to_string(),
        rust_path: "sample_lib::Container".to_string(),
        fields: vec![field.clone()],
        ..TypeDef::default()
    };
    let fields_c_types = std::collections::HashMap::from([("container.child".to_string(), "Child".to_string())]);
    let clone_names = ahash::AHashSet::from_iter(["Child".to_string()]);

    let code = gen_field_accessor(
        &typ,
        &field,
        "sample",
        "sample_lib",
        &ahash::AHashMap::new(),
        &ahash::AHashSet::new(),
        &clone_names,
        &fields_c_types,
    )
    .expect("cloneable handle accessor");

    assert!(code.contains(") -> AlefHandle"), "{code}");
    assert!(code.contains("catch_ffi_panic(0, ||"), "{code}");
    assert!(!code.contains("std::ptr::null_mut()"), "{code}");
    assert!(code.contains("insert_handle"), "{code}");
    syn::parse_file(&code).expect("handle-token accessor must parse");
}

/// A data-carrying enum (`Verdict`) returned by one function and taken by another,
/// so the fixture exercises both the return-side (`_free`/`_to_json`/`_to_string`) and
/// the parameter-side (`_from_json`, and the enum used directly as a function argument)
/// of the enum FFI surface. `has_serde: true` makes it eligible for the JSON/string
/// companions, matching how a real tagged-union type (e.g. an outcome or content enum)
/// looks once extracted.
fn scalar_handle_enum_api() -> ApiSurface {
    let verdict = EnumDef {
        name: "Verdict".to_string(),
        rust_path: "my_lib::Verdict".to_string(),
        variants: vec![
            EnumVariant {
                name: "Approved".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Rejected".to_string(),
                fields: vec![visitor_result_string_field("reason")],
                ..EnumVariant::default()
            },
        ],
        has_serde: true,
        ..EnumDef::default()
    };
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        enums: vec![verdict],
        functions: vec![
            FunctionDef {
                name: "evaluate_text".to_string(),
                rust_path: "my_lib::evaluate_text".to_string(),
                params: vec![ParamDef {
                    name: "text".to_string(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("Verdict".to_string()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "describe_verdict".to_string(),
                rust_path: "my_lib::describe_verdict".to_string(),
                params: vec![ParamDef {
                    name: "verdict".to_string(),
                    ty: TypeRef::Named("Verdict".to_string()),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::String,
                ..FunctionDef::default()
            },
        ],
        ..ApiSurface::default()
    }
}

/// C ABI regression for the alef-abi provider fix: a data-carrying enum returned by a
/// function must have its `_free`/`_to_json`/`_to_string` companions declared over the
/// *same* scalar `AlefHandle` the producer hands out, never a `TYPE *`/`const TYPE *`
/// struct pointer. Before the fix, `gen_enum_free`/`gen_enum_to_json`/`gen_enum_to_string`
/// unconditionally emitted pointer-shaped signatures for every enum that ever appeared as
/// a return type, which cbindgen renders as an incompatible C type from the scalar
/// `uint64_t` handle the producer function returns — this is the defect that broke the
/// generated Go binding (`cannot use ... as *_Ctype_struct_... value`).
#[test]
fn enum_free_to_json_and_to_string_take_scalar_handle_matching_producer_return() {
    let api = scalar_handle_enum_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn my_lib_evaluate_text(") && lib.content.contains(") -> AlefHandle"),
        "producer function must hand out a scalar handle, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("fn my_lib_verdict_free(handle: AlefHandle)"),
        "enum free must take the same scalar handle the producer returns, got:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("fn my_lib_verdict_to_json(handle: AlefHandle) -> *mut c_char"),
        "enum to_json must take a scalar handle, not a struct pointer, got:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("fn my_lib_verdict_to_string(handle: AlefHandle) -> *mut c_char"),
        "enum to_string must take a scalar handle, not a struct pointer, got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("*mut my_lib::Verdict") && !lib.content.contains("*const my_lib::Verdict"),
        "no function may reference Verdict by raw pointer once it is a scalar-handle type, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("remove_handle::<my_lib::Verdict>(handle)"),
        "free must release the same handle-registry slot the producer inserted into, got:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("with_handle::<my_lib::Verdict, _>(handle, serde_json::to_string)"),
        "to_json must borrow the enum out of the handle registry, not dereference a pointer, got:\n{}",
        lib.content
    );

    syn::parse_file(&lib.content).expect("scalar-handle enum exports must parse as valid Rust");
}

/// Rustc-consistency regression: the enum's `_from_json` producer and any function that
/// consumes the same enum *as a parameter* must agree on the scalar `AlefHandle` type too.
/// Before the fix, `gen_enum_from_json` returned `*mut Verdict` while every consumer of an
/// FFI-crossing enum parameter already expected `AlefHandle` — a mismatch that, unlike the
/// return-side defect, would have failed to compile even within the single generated crate.
/// `syn::parse_file` proves the whole generated module is still structurally valid Rust
/// after the fix.
#[test]
fn enum_from_json_and_enum_valued_parameter_agree_on_scalar_alef_handle() {
    let api = scalar_handle_enum_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content
            .contains("fn my_lib_verdict_from_json(json: *const c_char) -> AlefHandle"),
        "enum from_json must hand out the same scalar handle type the free function consumes, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("insert_handle(val)"),
        "from_json must register the deserialized enum in the handle registry, got:\n{}",
        lib.content
    );

    let describe = lib
        .content
        .split("fn my_lib_describe_verdict")
        .nth(1)
        .expect("describe_verdict wrapper must be emitted");
    assert!(
        describe.contains("verdict: AlefHandle"),
        "a function taking the enum by value must take the same scalar handle, not a struct pointer, got:\n{describe}"
    );
    assert!(
        !describe.contains("*const my_lib::Verdict") && !describe.contains("*mut my_lib::Verdict"),
        "must not fall back to a raw pointer parameter for the enum, got:\n{describe}"
    );

    syn::parse_file(&lib.content).expect("scalar-handle enum parameter wiring must parse as valid Rust");
}

/// Consumers run clippy at deny level over the generated crate and cannot patch it — the file is
/// emitted with a DO-NOT-EDIT header and its `Cargo.toml` and `poly.toml` are generated too, so no
/// downstream escape hatch exists. The `_free` templates that originally forced this allow now
/// emit a let-chain instead (asserted by
/// `handle_registry::generated_free_functions_guard_remove_handle_with_a_let_chain`), but the
/// allow stays: it covers the whole emitted crate, not one shape, and alef's CI never runs clippy
/// over that crate to discover the next nested `if`. This test only pins the header, so it
/// deliberately makes no claim about which emitted shape needs it. ~keep
#[test]
fn generated_ffi_crate_allows_collapsible_if_over_the_whole_emitted_crate() {
    let api = sample_api();
    let config = sample_config();

    let files = crate::core::backend::Backend::generate_bindings(&FfiBackend, &api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("clippy::collapsible_if"),
        "generated FFI lib.rs must allow clippy::collapsible_if, otherwise any nested `if` it \
         emits fails a consumer's deny-level clippy run, got header:\n{}",
        lib.content.lines().take(30).collect::<Vec<_>>().join("\n")
    );
}

/// Regression for a `cargo check` warning: a void, infallible free function (e.g.
/// `fn clear()`) was unconditionally routed through the `let result = …;` template even
/// though there is nothing to convert or propagate, leaving `result` bound and unused.
#[test]
fn free_function_void_non_error_return_binds_no_result_variable() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![FunctionDef {
            name: "clear".to_string(),
            rust_path: "sample_lib::clear".to_string(),
            return_type: TypeRef::Unit,
            error_type: None,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    // A zero-param, void, infallible free function is exactly the shape that gets inlined into
    // `AssertUnwindSafe(<path>)` rather than wrapped in a closure, so the callee appears as a bare
    // path with no `()`. What this test guards is that the call is still reached at all and that no
    // `result` is bound for it -- both hold in the inlined form. ~keep
    assert!(
        lib.content.contains("std::panic::AssertUnwindSafe(sample_lib::clear))"),
        "expected the call itself to still be emitted, got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("let result = sample_lib::clear()"),
        "a void, infallible free function must not bind an unused `result`, got:\n{}",
        lib.content
    );

    syn::parse_file(&lib.content).expect("void free function body must parse as valid Rust");
}

/// Companion regression: a void but *fallible* free function (`Result<(), E>`) must keep
/// binding `result` — the existing status-code error channel (`error_match_void.jinja`)
/// matches on it to report the error via `set_last_error` and return `-1`.
#[test]
fn free_function_void_error_return_still_binds_result_for_error_channel() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        functions: vec![FunctionDef {
            name: "install".to_string(),
            rust_path: "sample_lib::install".to_string(),
            return_type: TypeRef::Unit,
            error_type: Some("SampleError".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("let result = sample_lib::install();"),
        "a fallible void free function must still bind `result` to route through the error match, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("match result {\n        Ok(()) => 0,"),
        "expected the existing void error-match arm (status-code channel) to fire, got:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("set_last_error(alef_ffi_error_code(&e), &e.to_string());"),
        "the error must still be reported through the existing last-error channel, got:\n{}",
        lib.content
    );

    syn::parse_file(&lib.content).expect("fallible void free function body must parse as valid Rust");
}

/// Same defect, method-shaped: a void, infallible instance method must not bind an unused
/// `result` either. `can_inline` governs the method path the same way `can_inline_fn` governs
/// the free-function path above.
#[test]
fn instance_method_void_non_error_return_binds_no_result_variable() {
    let mut session = TypeDef {
        name: "Session".into(),
        is_opaque: true,
        ..Default::default()
    };
    session.methods.push(MethodDef {
        name: "reset".into(),
        receiver: Some(ReceiverKind::RefMut),
        cfg: None,
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    });
    let api = ApiSurface {
        crate_name: "sample".into(),
        types: vec![session],
        ..Default::default()
    };
    let config = sample_config();

    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn my_lib_session_reset("),
        "expected the reset wrapper to be emitted, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("obj.reset()"),
        "expected the call itself to still be emitted, got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("let result = obj.reset()"),
        "a void, infallible instance method must not bind an unused `result`, got:\n{}",
        lib.content
    );

    syn::parse_file(&lib.content).expect("void instance method body must parse as valid Rust");
}

/// `Optional<Bytes>` must ride the same byte-buffer out-param convention as bare `Bytes`
/// rather than the `*mut c_char` + `_len()` companion convention `Optional<String>` uses:
/// bytes carry no NUL terminator, so a companion has no length to recover. Absence is
/// encoded as a null `*out_ptr` with a zero length, which keeps `-1` the only error signal
/// a C caller has to test and leaves `Some(&[])` — a non-null pointer with a zero length —
/// distinguishable from `None`. ~keep
#[test]
fn optional_bytes_return_uses_bytes_out_params_with_null_ptr_for_none() {
    let api = optional_bytes_api(TypeRef::Optional(Box::new(TypeRef::Bytes)), Some("MyError".to_string()));
    let files = FfiBackend.generate_bindings(&api, &sample_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("out_ptr: *mut *mut u8"),
        "Optional<Bytes> must declare the out_ptr out-param;\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("out_len: *mut usize"),
        "Optional<Bytes> must declare the out_len out-param;\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("out_cap: *mut usize"),
        "Optional<Bytes> must declare the out_cap out-param;\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("fn my_lib_maybe_thumbnail("),
        "Optional<Bytes> function must be emitted under its FFI name;\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("fn my_lib_maybe_thumbnail_len("),
        "Optional<Bytes> must not get a `_len()` companion — bytes have no NUL terminator;\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("fn my_lib_free_bytes("),
        "an Optional<Bytes>-only surface must still emit the free_bytes companion;\n{}",
        lib.content
    );

    assert!(
        lib.content.contains("Ok(Some(val)) => {"),
        "present arm must match `Ok(Some(val))`;\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("unsafe { *out_ptr = ptr; *out_len = len; *out_cap = len; }"),
        "present arm must publish the boxed-slice pointer and its length;\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("Ok(None) => {"),
        "absent arm must match `Ok(None)`;\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("unsafe { *out_ptr = std::ptr::null_mut(); *out_len = 0; *out_cap = 0; }"),
        "absent arm must publish a null pointer with a zero length;\n{}",
        lib.content
    );

    syn::parse_file(&lib.content).expect("Optional<Bytes> wrapper must parse as valid Rust");
}

/// Present-but-empty is the case a length-plus-null scheme most easily conflates with
/// absent. The emitted code must distinguish them structurally: the `Some` arm always runs
/// `Box::into_raw` (non-null even for an empty boxed slice) and never writes a null pointer,
/// so `*out_ptr != NULL && *out_len == 0` can only mean `Some(&[])`. ~keep
#[test]
fn optional_bytes_present_arm_never_writes_null_so_empty_is_not_absent() {
    let api = optional_bytes_api(TypeRef::Optional(Box::new(TypeRef::Bytes)), None);
    let files = FfiBackend.generate_bindings(&api, &sample_config()).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let some_arm_start = lib.content.find("Some(val) => {").expect("infallible present arm");
    let none_arm_offset = lib.content[some_arm_start..]
        .find("None => {")
        .expect("infallible absent arm must follow the present arm");
    let some_arm = &lib.content[some_arm_start..some_arm_start + none_arm_offset];

    assert!(
        some_arm.contains("let buffer = Vec::<u8>::from(val).into_boxed_slice();"),
        "present arm must box the bytes so even an empty value yields a non-null pointer;\n{some_arm}"
    );
    assert!(
        some_arm.contains("let ptr = Box::into_raw(buffer).cast::<u8>();"),
        "present arm must take the boxed-slice pointer;\n{some_arm}"
    );
    assert!(
        !some_arm.contains("std::ptr::null_mut()"),
        "present arm must never write a null pointer — that is the `None` encoding;\n{some_arm}"
    );
    assert!(
        lib.content
            .contains("unsafe { *out_ptr = std::ptr::null_mut(); *out_len = 0; *out_cap = 0; }"),
        "absent arm must be the only writer of a null pointer;\n{}",
        lib.content
    );

    syn::parse_file(&lib.content).expect("infallible Optional<Bytes> wrapper must parse as valid Rust");
}

fn optional_bytes_api(return_type: TypeRef, error_type: Option<String>) -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "maybe_thumbnail".to_string(),
            rust_path: "my_lib::maybe_thumbnail".to_string(),
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
            return_type,
            is_async: false,
            error_type,
            doc: "Render a page thumbnail when one exists.".to_string(),
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
