use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

#[test]
fn test_generates_lib_rs() {
    let api = sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    assert!(files.iter().any(|f| f.path.ends_with("lib.rs")));

    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    assert!(lib.content.contains("extern \"C\""));
    assert!(lib.content.contains("my_lib_last_error_code"));
    assert!(lib.content.contains("my_lib_config_from_json"));
    assert!(lib.content.contains("my_lib_config_free"));
    assert!(lib.content.contains("my_lib_config_timeout"));
    assert!(lib.content.contains("my_lib_config_name"));
    assert!(lib.content.contains("my_lib_free_string"));
    assert!(lib.content.contains("my_lib_version"));
    assert!(lib.content.contains("my_lib_extract"));
    assert!(lib.content.contains("my_lib_output_format_from_i32"));
    assert!(lib.content.contains("my_lib_output_format_from_str"));
}

/// Build an `ApiSurface` whose only function returns the unit-variant enum
/// `Color` (`has_serde: true`) by pointer. Used to exercise emission of
/// `_to_string` accessors alongside `_free` / `_to_json`.
fn enum_return_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "current_color".to_string(),
            rust_path: "my_lib::current_color".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Named("Color".to_string()),
            is_async: false,
            error_type: None,
            doc: "Currently selected color.".to_string(),
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
            name: "Color".to_string(),
            rust_path: "my_lib::Color".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Red".to_string(),
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
                },
                EnumVariant {
                    name: "Green".to_string(),
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
                },
            ],
            methods: vec![],
            doc: "Colors.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn test_emits_enum_to_string_for_pointer_return_enum() {
    let api = enum_return_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    // Sanity: pointer-return enum lifecycle helpers are emitted.
    assert!(
        lib.content.contains("my_lib_color_free"),
        "expected my_lib_color_free in emitted lib.rs"
    );
    assert!(
        lib.content.contains("my_lib_color_to_json"),
        "expected my_lib_color_to_json in emitted lib.rs"
    );

    // The new accessor: takes *const Color, returns *mut c_char.
    assert!(
        lib.content
            .contains("pub unsafe extern \"C\" fn my_lib_color_to_string("),
        "expected pub unsafe extern \"C\" fn my_lib_color_to_string in emitted lib.rs"
    );
    assert!(
        lib.content.contains("ptr: *const my_lib::Color)"),
        "to_string should accept *const Color"
    );
    assert!(
        lib.content.contains("-> *mut c_char"),
        "to_string should return *mut c_char"
    );
    // Body should extract the unit-variant name via serde, not via JSON-with-quotes.
    assert!(
        lib.content.contains("serde_json::to_value(val)"),
        "to_string should use serde_json::to_value"
    );
    assert!(
        lib.content.contains(".as_str()"),
        "to_string should call .as_str() to strip JSON quotes"
    );
}

#[test]
fn test_omits_enum_to_string_when_enum_not_returned() {
    // The default sample_api() uses `OutputFormat` only as a non-return enum
    // (no function returns it, no struct field has it), so neither _free nor
    // _to_string should be emitted.
    let api = sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        !lib.content.contains("my_lib_output_format_to_string"),
        "expected NO my_lib_output_format_to_string when enum is not returned by pointer"
    );
    assert!(
        !lib.content.contains("my_lib_output_format_free"),
        "expected NO my_lib_output_format_free when enum is not returned by pointer"
    );
}

#[test]
fn test_generates_cbindgen_toml() {
    let api = sample_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();
    assert!(cbindgen.content.contains("MY_LIB_H"));
    assert!(cbindgen.content.contains("language = \"C\""));
    assert!(cbindgen.content.contains("style = \"both\""));
}

#[test]
fn test_cbindgen_toml_omits_excluded_forward_declarations() {
    let mut api = sample_api();

    let mut hidden_config = api.types[0].clone();
    hidden_config.name = "HiddenConfig".to_string();
    hidden_config.rust_path = "my_lib::HiddenConfig".to_string();
    hidden_config.doc = "Hidden config.".to_string();
    api.types.push(hidden_config);

    let mut binding_excluded_config = api.types[0].clone();
    binding_excluded_config.name = "BindingExcludedConfig".to_string();
    binding_excluded_config.rust_path = "my_lib::BindingExcludedConfig".to_string();
    binding_excluded_config.binding_excluded = true;
    binding_excluded_config.binding_exclusion_reason = Some("alef skip".to_string());
    api.types.push(binding_excluded_config);

    let mut hidden_enum = api.enums[0].clone();
    hidden_enum.name = "HiddenEnum".to_string();
    hidden_enum.rust_path = "my_lib::HiddenEnum".to_string();
    hidden_enum.doc = "Hidden enum.".to_string();
    api.enums.push(hidden_enum);

    let mut binding_excluded_enum = api.enums[0].clone();
    binding_excluded_enum.name = "BindingExcludedEnum".to_string();
    binding_excluded_enum.rust_path = "my_lib::BindingExcludedEnum".to_string();
    binding_excluded_enum.binding_excluded = true;
    binding_excluded_enum.binding_exclusion_reason = Some("alef skip".to_string());
    api.enums.push(binding_excluded_enum);

    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
exclude_types = ["HiddenConfig", "HiddenEnum"]
"#,
    );
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(cbindgen.content.contains("typedef struct MY_LIBConfig MY_LIBConfig;"));
    assert!(
        cbindgen
            .content
            .contains("typedef struct MY_LIBOutputFormat MY_LIBOutputFormat;")
    );
    assert!(!cbindgen.content.contains("MY_LIBHiddenConfig"));
    assert!(!cbindgen.content.contains("MY_LIBHiddenEnum"));
    assert!(!cbindgen.content.contains("MY_LIBBindingExcludedConfig"));
    assert!(!cbindgen.content.contains("MY_LIBBindingExcludedEnum"));
}

#[test]
fn test_cbindgen_toml_honors_ffi_exclude_types() {
    let mut api = sample_api();
    api.types.push(TypeDef {
        name: "HiddenOptions".to_string(),
        rust_path: "my_lib::internal::HiddenOptions".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        doc: "Rust-only helper options.".to_string(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    });
    api.enums.push(EnumDef {
        name: "HiddenStatus".to_string(),
        rust_path: "my_lib::internal::HiddenStatus".to_string(),
        original_rust_path: String::new(),
        variants: vec![],
        methods: vec![],
        doc: "Rust-only helper status.".to_string(),
        has_serde: true,
        cfg: None,
        is_copy: false,
        has_default: false,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    });
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "my_lib"
exclude_types = ["HiddenOptions", "my_lib::internal::HiddenStatus"]
"#,
    );
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let cbindgen = files.iter().find(|f| f.path.ends_with("cbindgen.toml")).unwrap();

    assert!(
        cbindgen
            .content
            .contains(r#"exclude = ["HiddenOptions", "HiddenStatus"]"#),
        "expected cbindgen export excludes for bare type names, got:\n{}",
        cbindgen.content
    );
    assert!(
        !cbindgen
            .content
            .contains("typedef struct MY_LIBHiddenOptions MY_LIBHiddenOptions;"),
        "excluded struct must not be forward-declared, got:\n{}",
        cbindgen.content
    );
    assert!(
        !cbindgen
            .content
            .contains("typedef struct MY_LIBHiddenStatus MY_LIBHiddenStatus;"),
        "excluded enum must not be forward-declared, got:\n{}",
        cbindgen.content
    );
}
