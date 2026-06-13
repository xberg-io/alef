use super::*;

#[test]
fn test_type_stubs_documented_field_emits_var_phpdoc_with_description() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ClientConfig".to_string(),
            rust_path: "test_lib::ClientConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field_with_doc(
                    "base_url",
                    TypeRef::Optional(Box::new(TypeRef::String)),
                    true,
                    "Base URL of the remote API endpoint. Defaults to OpenAI's.",
                ),
                make_field_with_doc(
                    "timeout_secs",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
                    true,
                    "Request timeout in seconds.\nDefaults to 30.",
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
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
        ..Default::default()
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // Single-line doc: compact /** @var T Description. */ form.
    assert!(
        content.contains("@var ?string Base URL of the remote API endpoint. Defaults to OpenAI's."),
        "Documented optional string field should have @var ?string with description;\ncontent:\n{content}"
    );

    // Multi-line doc: multi-line block with description + @var tag.
    assert!(
        content.contains("@var ?int"),
        "Documented optional int field should have @var ?int tag;\ncontent:\n{content}"
    );
    assert!(
        content.contains("Request timeout in seconds."),
        "Multi-line doc first line should appear in PHPDoc;\ncontent:\n{content}"
    );
    assert!(
        content.contains("Defaults to 30."),
        "Multi-line doc second line should appear in PHPDoc;\ncontent:\n{content}"
    );
}

#[test]
fn test_type_stubs_undocumented_field_emits_var_phpdoc_type_only() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Options".to_string(),
            rust_path: "test_lib::Options".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false),
                make_field(
                    "max_retries",
                    TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I32))),
                    true,
                ),
            ],
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
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    };

    let config = make_config();
    let files = backend.generate_type_stubs(&api, &config).unwrap();
    let stubs = files.first().unwrap();
    let content = &stubs.content;

    // Type-only compact form for undocumented fields.
    assert!(
        content.contains("/** @var bool */"),
        "Undocumented bool field should have type-only /** @var bool */;\ncontent:\n{content}"
    );
    assert!(
        content.contains("/** @var ?int */"),
        "Undocumented optional int field should have type-only /** @var ?int */;\ncontent:\n{content}"
    );
}

#[test]
fn test_public_api_sanitizes_rust_syntax_from_docstrings() {
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "convert".to_string(),
            rust_path: "test_lib::convert".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "html".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::String,
            error_type: None,
            is_async: false,
            doc: "Convert markup conversion, returning a result.\n\n# Arguments\n\n* `html` - The HTML string to convert.\n\n# Example\n\n```rust\nuse test_lib::convert;\nlet result = convert(html, None).unwrap();\n```"
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
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    ..Default::default()
};

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let facade = files.first().unwrap();
    let content = &facade.content;

    // Verify Rust syntax is NOT in the docstring
    assert!(
        !content.contains("use test_lib::convert;"),
        "Rust 'use' statement must not leak into PHPDoc"
    );
    assert!(!content.contains(".unwrap()"), ".unwrap() must not leak into PHPDoc");
    assert!(!content.contains("```rust"), "Raw Rust fence must not appear in PHPDoc");

    // Verify summary IS present
    assert!(
        content.contains("Convert markup conversion"),
        "Summary must be preserved in PHPDoc"
    );

    // Verify @param/@return tags are present (emitted separately)
    assert!(
        content.contains("@param"),
        "@param tag must be present for documented parameters"
    );
    assert!(content.contains("@return"), "@return tag must be present");
}

/// Regression test: a Duration field on a Default struct is stored as `Option<i64>` in the
/// binding (via the `option_duration_on_defaults` path in the struct emitter). The getter
/// return type must mirror the storage type and also be `Option<i64>`, not bare `i64`.
///
/// Before the fix the getter was emitted as `pub fn get_ttl(&self) -> i64 { self.ttl.clone() }`,
/// which caused E0308 because `self.ttl` is `Option<i64>`.
#[test]
fn test_duration_field_on_default_struct_getter_returns_option() {
    let backend = PhpBackend;

    // Simulate a struct like `CacheConfig { ttl: Duration }` with `has_default = true`.
    // The IR uses TypeRef::Duration for the field and has `optional = false`.
    // The struct emitter wraps it in Option<i64> when option_duration_on_defaults is enabled;
    // the getter must match.
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "CacheConfig".to_string(),
            rust_path: "test_lib::CacheConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("max_entries", TypeRef::Primitive(PrimitiveType::I64), false),
                make_field("ttl", TypeRef::Duration, false),
            ],
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
            doc: "Cache configuration".to_string(),
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
        ..Default::default()
    };

    let config = make_config();
    let result = backend.generate_bindings(&api, &config);
    assert!(result.is_ok(), "generation must succeed: {:?}", result.err());

    let files = result.unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("lib.rs"))
        .unwrap();
    let content = &lib_rs.content;

    // The struct field must be Option<i64> (Duration → i64 ms, wrapped in Option).
    assert!(
        content.contains("pub ttl: Option<i64>"),
        "Duration field on Default struct must be stored as Option<i64>; got:\n{content}"
    );

    // The getter must return Option<i64>, not bare i64, to match the storage type.
    assert!(
        content.contains("fn get_ttl") && content.contains("-> Option<i64>"),
        "getter for Duration field on Default struct must return Option<i64>; got:\n{content}"
    );

    // Must NOT emit the wrong bare return type.
    assert!(
        !content.contains("fn get_ttl(&self) -> i64"),
        "getter must not return bare i64 for a Duration-on-Default field; got:\n{content}"
    );
}

/// Regression test for the `has_default` + `#[serde(default)]` defaults bug.
///
/// When a core struct has a custom `impl Default` (e.g. `max_redirects: 10`), the binding's
/// struct-level `#[serde(default)]` previously caused missing JSON fields to fall back to
/// the derived `Default`, which uses Rust's primitive zeros. The zeros were then propagated
/// to the core type via `From<BindingType>`, silently clobbering the core's semantic defaults.
///
/// The fix suppresses the auto `#[derive(Default)]` and emits a delegating
/// `impl Default for BindingType { fn default() -> Self { <core::Type as Default>::default().into() } }`
/// so that `serde(default)` and `unwrap_or_default()` honour the core's custom values.
///
/// This test exercises the shared struct generator directly with the same configuration
/// shape PHP uses when serde is available, so it does not depend on filesystem-based
/// Cargo.toml serde detection.
#[test]
fn has_default_struct_emits_delegating_impl_not_derived_default() {
    use alef::codegen::generators::{AsyncPattern, RustBindingConfig, gen_struct_with_per_field_attrs};
    use alef::core::ir::FieldDef;

    let typ = TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "test_lib::CrawlConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            // Mirrors the real-world bug: core sets `max_redirects: 10` in its custom
            // Default, but the binding's derived Default uses `0` for i64, which then
            // overrides the core's value on round-trip through `From<BindingType>`.
            make_field("max_redirects", TypeRef::Primitive(PrimitiveType::I64), false),
            make_field("respect_robots_txt", TypeRef::Primitive(PrimitiveType::Bool), false),
        ],
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
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        version: Default::default(),
    };

    struct StubMapper;
    impl alef::codegen::type_mapper::TypeMapper for StubMapper {
        fn error_wrapper(&self) -> &str {
            "Result"
        }
    }
    let mapper = StubMapper;

    // PHP-shaped config with `emit_delegating_default_impl: true` and serde derives.
    // Mirrors `gen_php_struct`'s `cfg.has_serde == true` branch but without depending on
    // crate-private symbols.
    let struct_attrs: &[&str] = &["php_class", "serde(default, rename_all = \"camelCase\")"];
    let struct_derives: &[&str] = &["Clone", "serde::Serialize", "serde::Deserialize"];
    let cfg = RustBindingConfig {
        struct_attrs,
        field_attrs: &[],
        struct_derives,
        method_block_attr: Some("php_impl"),
        constructor_attr: "",
        static_attr: None,
        function_attr: "#[php_function]",
        enum_attrs: &[],
        enum_derives: &[],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "test_lib",
        async_pattern: AsyncPattern::TokioBlockOn,
        has_serde: true,
        type_name_prefix: "",
        option_duration_on_defaults: true,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: true,
        skip_methods_when_not_delegatable: false,
    };

    let content = gen_struct_with_per_field_attrs(&typ, &mapper, &cfg, |_: &FieldDef| vec![]);

    // The struct must NOT derive Default — that would override the core's custom defaults.
    let struct_start = content
        .find("pub struct CrawlConfig")
        .expect("CrawlConfig struct must be emitted");
    let derive_window = &content[..struct_start];
    assert!(
        !derive_window.contains("Default"),
        "CrawlConfig must NOT derive Default — that would emit zeros instead of \
         delegating to the core's custom Default. Derive block:\n{derive_window}"
    );

    // The delegating `impl Default` must be emitted and delegate to the core type's Default.
    assert!(
        content.contains("impl Default for CrawlConfig"),
        "delegating impl Default must be emitted for has_default types; got:\n{content}"
    );
    assert!(
        content.contains("<test_lib::CrawlConfig as Default>::default().into()"),
        "impl Default must delegate to the core type's Default via `.into()`; got:\n{content}"
    );

    // The struct should still carry struct-level `#[serde(default)]` for from_json to accept
    // partial JSON — this is the path that previously surfaced the bug.
    assert!(
        content.contains("serde(default"),
        "struct must still carry struct-level `#[serde(default)]`; got:\n{content}"
    );

    // Serde Serialize/Deserialize derives must remain — only Default is suppressed.
    assert!(
        content.contains("serde::Serialize"),
        "struct must still derive serde::Serialize; got:\n{content}"
    );
    assert!(
        content.contains("serde::Deserialize"),
        "struct must still derive serde::Deserialize; got:\n{content}"
    );
}

/// Companion test: when `emit_delegating_default_impl` is false (default for non-PHP backends),
/// the auto `Default` derive is preserved and no delegating impl is emitted.
#[test]
fn has_default_struct_keeps_derived_default_when_delegation_disabled() {
    use alef::codegen::generators::{AsyncPattern, RustBindingConfig, gen_struct_with_per_field_attrs};
    use alef::core::ir::FieldDef;

    let typ = TypeDef {
        name: "PlainConfig".to_string(),
        rust_path: "test_lib::PlainConfig".to_string(),
        original_rust_path: String::new(),
        fields: vec![make_field("count", TypeRef::Primitive(PrimitiveType::I64), false)],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: true,
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

    struct StubMapper;
    impl alef::codegen::type_mapper::TypeMapper for StubMapper {
        fn error_wrapper(&self) -> &str {
            "Result"
        }
    }
    let mapper = StubMapper;

    let cfg = RustBindingConfig {
        struct_attrs: &[],
        field_attrs: &[],
        struct_derives: &["Clone"],
        method_block_attr: None,
        constructor_attr: "",
        static_attr: None,
        function_attr: "",
        enum_attrs: &[],
        enum_derives: &[],
        needs_signature: false,
        signature_prefix: "",
        signature_suffix: "",
        core_import: "test_lib",
        async_pattern: AsyncPattern::None,
        has_serde: false,
        type_name_prefix: "",
        option_duration_on_defaults: false,
        opaque_type_names: &[],
        skip_impl_constructor: false,
        cast_uints_to_i32: false,
        cast_large_ints_to_f64: false,
        named_non_opaque_params_by_ref: false,
        lossy_skip_types: &[],
        serializable_opaque_type_names: &[],
        never_skip_cfg_field_names: &[],
        emit_delegating_default_impl: false,
        skip_methods_when_not_delegatable: false,
    };

    let content = gen_struct_with_per_field_attrs(&typ, &mapper, &cfg, |_: &FieldDef| vec![]);

    assert!(
        content.contains("Default"),
        "Default must still be derived when emit_delegating_default_impl is false; got:\n{content}"
    );
    assert!(
        !content.contains("impl Default for PlainConfig"),
        "no delegating impl Default should be emitted when the flag is disabled; got:\n{content}"
    );
}
