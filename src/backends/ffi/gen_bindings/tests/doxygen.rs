use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

// -----------------------------------------------------------------------

fn doxygen_sample_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Handle".to_string(),
            rust_path: "my_lib::Handle".to_string(),
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
            doc: "An opaque handle that wraps the underlying resource.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "lookup".to_string(),
            rust_path: "my_lib::lookup".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
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
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            is_async: false,
            error_type: Some("MyError".to_string()),
            doc: "Look up the registry index for a name.\n\n\
                      # Arguments\n\n\
                      * `name` - The unique key to search.\n\n\
                      # Returns\n\n\
                      A non-zero index when found; zero on lookup miss.\n\n\
                      # Errors\n\n\
                      Returns the last-error code when the registry is poisoned."
                .to_string(),
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
        enums: vec![EnumDef {
            name: "Severity".to_string(),
            rust_path: "my_lib::Severity".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Warn".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: "Diagnostic severity level.".to_string(),
            cfg: None,
            is_copy: true,
            has_serde: false,
            has_default: false,
            serde_content: None,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            rename_all_fields: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn test_extern_fn_emits_doxygen_param_return_note_markers() {
    let api = doxygen_sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("/// \\param name The unique key to search."),
        "expected \\param marker for `name`, got:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("/// \\return A non-zero index when found; zero on lookup miss."),
        "expected \\return marker, got:\n{}",
        lib.content
    );
    assert!(
        lib.content
            .contains("/// \\note Returns the last-error code when the registry is poisoned."),
        "expected \\note marker for # Errors, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("/// \\note SAFETY:"),
        "expected \\note SAFETY: marker derived from synthetic safety clause, got:\n{}",
        lib.content
    );
}

#[test]
fn test_opaque_typedef_carries_doxygen_block_in_cbindgen_toml() {
    let api = doxygen_sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
    toml::from_str::<toml::Value>(&cbindgen.content).expect("cbindgen.toml must be valid TOML");

    assert!(
        cbindgen.content.contains("/**"),
        "expected /** doxygen opener, got:\n{}",
        cbindgen.content
    );
    assert!(
        cbindgen
            .content
            .contains("* An opaque handle that wraps the underlying resource."),
        "expected typedef doc body, got:\n{}",
        cbindgen.content
    );
    assert!(
        cbindgen.content.contains("typedef uint64_t MY_LIBHandle;"),
        "expected scalar generational handle typedef, got:\n{}",
        cbindgen.content
    );
}

#[test]
fn test_cbindgen_toml_escapes_doxygen_backslashes() {
    let mut api = doxygen_sample_api();
    api.types[0].doc = r##"Has an example.

# Example

```rust
let value = "triple """ quote";
```"##
        .to_string();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
    let parsed = toml::from_str::<toml::Value>(&cbindgen.content).expect("cbindgen.toml must parse");
    let after_includes = parsed
        .get("after_includes")
        .and_then(toml::Value::as_str)
        .expect("after_includes must be a string");

    assert!(
        after_includes.contains("\\code") && after_includes.contains("\\endcode"),
        "Doxygen markers must survive TOML parsing: {after_includes}"
    );
    assert!(
        after_includes.contains("triple \"\"\" quote"),
        "triple quotes must round-trip through TOML parsing: {after_includes}"
    );
}

#[test]
fn test_enum_opaque_typedef_carries_doxygen_block() {
    let api = doxygen_sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(
        cbindgen.content.contains("* Diagnostic severity level."),
        "expected enum typedef doc body, got:\n{}",
        cbindgen.content
    );
    assert!(
        cbindgen
            .content
            .contains("typedef struct MY_LIBSeverity MY_LIBSeverity;"),
        "expected prefixed enum typedef, got:\n{}",
        cbindgen.content
    );
}

/// Every error type whose accessor functions are emitted must also have a
/// forward `typedef struct` in the cbindgen.toml `after_includes` block.
/// Without it cbindgen produces an "unknown type name" compile error because
/// the accessor signature references `*const ErrorType` but no opaque struct
/// is declared in the header.
#[test]
fn test_error_type_with_methods_gets_opaque_typedef_in_cbindgen_toml() {
    let mut api = sample_api();
    api.errors.push(ErrorDef {
        name: "GraphQLError".to_string(),
        rust_path: "my_lib::GraphQLError".to_string(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: "GraphQL execution error.".to_string(),
        methods: vec![MethodDef {
            name: "status_code".to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "HTTP status code for the error.".to_string(),
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let config = sample_config();
    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(
        cbindgen
            .content
            .contains("typedef struct MY_LIBGraphQLError MY_LIBGraphQLError;"),
        "expected opaque typedef for error type with methods, got:\n{}",
        cbindgen.content
    );

    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    assert!(
        lib.content.contains("my_lib_graph_ql_error_status_code"),
        "expected accessor fn for error type, got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("graph_q_l_error"),
        "the accessor must not use the per-uppercase-letter split that the error_gen-local \
         snake-caser produced; the type component goes through `pascal_to_snake` like every \
         other C symbol component:\n{}",
        lib.content
    );
}

/// The error accessor's type component must be spelled by the same derivation as every other
/// C symbol in the header — `c_consumer::method_symbol`, i.e. acronym-aware `pascal_to_snake`.
///
/// `error_gen` carried its own snake-caser that split before every uppercase letter, so
/// `GraphQLError` produced `graph_q_l_error_status_code` while a method on a type of the same
/// name produced `graph_ql_error_*`. A C symbol has no namespace, so two spellings for one type
/// in one header is a silent link-time break for any hand-written or generated consumer that
/// derives the name rather than copying it out of the header.
#[test]
fn test_error_accessor_symbol_matches_c_consumer_method_symbol() {
    let mut api = sample_api();
    api.errors.push(ErrorDef {
        name: "GraphQLError".to_string(),
        rust_path: "my_lib::GraphQLError".to_string(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: "GraphQL execution error.".to_string(),
        methods: vec![
            MethodDef {
                name: "status_code".to_string(),
                return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
                doc: "HTTP status code for the error.".to_string(),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..MethodDef::default()
            },
            MethodDef {
                name: "is_transient".to_string(),
                return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                doc: "Whether a retry may succeed.".to_string(),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..MethodDef::default()
            },
            MethodDef {
                name: "error_type".to_string(),
                return_type: TypeRef::String,
                doc: "Machine-readable error category.".to_string(),
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..MethodDef::default()
            },
        ],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let config = sample_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    let prefix = config.ffi_prefix();
    for method in ["status_code", "is_transient", "error_type"] {
        let expected = crate::codegen::c_consumer::method_symbol(&prefix, "GraphQLError", method);
        assert!(
            lib.content.contains(&expected),
            "expected accessor symbol `{expected}` for `{method}`, got:\n{}",
            lib.content
        );
    }
    assert!(
        lib.content.contains(&format!(
            "{}_free",
            crate::codegen::c_consumer::method_symbol(&prefix, "GraphQLError", "error_type")
        )),
        "the `error_type` companion free fn must share the corrected stem, got:\n{}",
        lib.content
    );
}

/// Control for `test_error_accessor_symbol_matches_c_consumer_method_symbol`: an error type
/// with no consecutive-uppercase run is the shape both casers already agreed on, and must be
/// spelled exactly as before the fix.
#[test]
fn test_error_accessor_symbol_unchanged_for_non_acronym_error_name() {
    let mut api = sample_api();
    api.errors.push(ErrorDef {
        name: "ConversionError".to_string(),
        rust_path: "my_lib::ConversionError".to_string(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: "Conversion error.".to_string(),
        methods: vec![MethodDef {
            name: "status_code".to_string(),
            return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
            doc: "HTTP status code for the error.".to_string(),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        }],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let config = sample_config();
    let files = FfiBackend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("my_lib_conversion_error_status_code"),
        "expected the pre-existing spelling to be unchanged, got:\n{}",
        lib.content
    );
}

/// Error types without any whitelisted methods must NOT produce a spurious
/// typedef — the accessor function is not emitted so there is nothing to
/// declare.
#[test]
fn test_error_type_without_methods_does_not_get_typedef_in_cbindgen_toml() {
    let mut api = sample_api();
    api.errors.push(ErrorDef {
        name: "SilentError".to_string(),
        rust_path: "my_lib::SilentError".to_string(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    });

    let config = sample_config();
    let backend = FfiBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(
        !cbindgen.content.contains("SilentError"),
        "error type with no methods must not appear in cbindgen.toml, got:\n{}",
        cbindgen.content
    );
}

#[test]
fn test_generates_build_rs() {
    let api = sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();
    assert!(build.content.contains("cbindgen::generate"));
    assert!(build.content.contains("my_lib.h"));
}

#[test]
fn test_build_rs_sets_macos_install_name_and_loader_rpath() {
    let api = sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();
    assert!(
        build.content.contains("-Wl,-install_name,@rpath/"),
        "build.rs must set @rpath-relative install_name on macOS:\n{}",
        build.content
    );
    assert!(
        build.content.contains("-Wl,-rpath,@loader_path"),
        "build.rs must add @loader_path LC_RPATH on macOS so sibling dylibs resolve:\n{}",
        build.content
    );
}

#[test]
fn test_build_rs_rewrites_prefixed_capsule_return_type() {
    use crate::core::config::FfiCapsuleTypeConfig;
    let mut capsule_types = std::collections::HashMap::new();
    capsule_types.insert(
        "Language".to_string(),
        FfiCapsuleTypeConfig {
            into_raw_type: "tree_sitter::ffi::TSLanguage".to_string(),
            c_return_type: "TSLanguage".to_string(),
            package: None,
            package_version: None,
        },
    );
    let build = super::super::helpers::gen_build_rs(
        "ts_pack.h",
        "libts_pack_core_ffi",
        "crates/ts-pack-ffi",
        None,
        "ts_pack",
        &capsule_types,
    )
    .expect("valid build.rs paths");
    assert!(
        build.contains(r#"header.replace("TS_PACKTSLanguage", "TSLanguage")"#),
        "build.rs must rewrite the prefixed capsule pointee back to the unprefixed prelude name:\n{build}"
    );
    assert!(
        !build.contains("for (prefixed, bare)"),
        "build.rs must not emit a single-element loop for the capsule header fixup:\n{build}"
    );
}

#[test]
fn test_build_rs_omits_capsule_fixup_when_no_capsule_types() {
    let build = super::super::helpers::gen_build_rs(
        "ts_pack.h",
        "libts_pack_core_ffi",
        "crates/ts-pack-ffi",
        None,
        "ts_pack",
        &std::collections::HashMap::new(),
    )
    .expect("valid build.rs paths");
    assert!(
        !build.contains("header.replace"),
        "build.rs must not emit a capsule fixup when no capsule types are configured:\n{build}"
    );
}

/// `ts_pack` (the only tree that configures capsule types today) is already
/// underscored, so `to_uppercase()` and `to_shouty_snake_case()` agree on it and
/// `test_build_rs_rewrites_prefixed_capsule_return_type` above cannot tell the two
/// derivations apart. `SampleCore` carries an internal capital with no separator,
/// where the two diverge (`SAMPLECORE` vs `SAMPLE_CORE`) -- this is what actually
/// exercises the fix.
#[test]
fn test_build_rs_capsule_fixup_uses_shouty_snake_case_prefix() {
    use crate::core::config::FfiCapsuleTypeConfig;
    let mut capsule_types = std::collections::HashMap::new();
    capsule_types.insert(
        "Language".to_string(),
        FfiCapsuleTypeConfig {
            into_raw_type: "tree_sitter::ffi::TSLanguage".to_string(),
            c_return_type: "TSLanguage".to_string(),
            package: None,
            package_version: None,
        },
    );
    let build = super::super::helpers::gen_build_rs(
        "sample.h",
        "libsample_ffi",
        "crates/sample-ffi",
        None,
        "SampleCore",
        &capsule_types,
    )
    .expect("valid build.rs paths");
    assert!(
        build.contains(r#"header.replace("SAMPLE_CORETSLanguage", "TSLanguage")"#),
        "build.rs must rewrite the shouty-snake-prefixed capsule pointee, matching the header cbindgen \
         actually writes, back to the unprefixed prelude name:\n{build}"
    );
    assert!(
        !build.contains("SAMPLECORETSLanguage"),
        "build.rs must not fall back to a bare-uppercase prefix for the capsule fixup:\n{build}"
    );
}

#[test]
fn test_custom_prefix() {
    let api = sample_api();
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"
header_name = "mylib.h"
"#,
    );
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    assert!(lib.content.contains("ml_last_error_code"));
    assert!(lib.content.contains("ml_config_from_json"));

    let build = files.iter().find(|f| f.path.ends_with("build.rs")).unwrap();
    assert!(build.content.contains("mylib.h"));
}

/// Every prefix configured across the actual consumer trees (`literllm`, `ts_pack`,
/// `htm`, `cberg`, `xberg`) is lowercase, single-word, or already underscored, so
/// `to_uppercase()` and `to_shouty_snake_case()` return identical strings for all of
/// them -- a test built on any of those prefixes cannot distinguish the two
/// derivations and would pass unchanged before and after the fix. `SampleCore`
/// carries an internal capital with no separator, which is exactly where the two
/// derivations diverge (`SAMPLECORE` vs `SAMPLE_CORE`); cbindgen's `[export] prefix`
/// is applied verbatim to every exported type name, so this is what the real header
/// says once written into cbindgen.toml.
#[test]
fn test_cbindgen_export_prefix_uses_shouty_snake_case_for_internal_capital() {
    let api = sample_api();
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "SampleCore"
header_name = "sample.h"
"#,
    );
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(
        cbindgen.content.contains(r#"prefix = "SAMPLE_CORE""#),
        "expected shouty-snake `[export] prefix`, got:\n{}",
        cbindgen.content
    );
    assert!(
        !cbindgen.content.contains("SAMPLECORE"),
        "must not fall back to a bare-uppercase prefix, got:\n{}",
        cbindgen.content
    );
}
