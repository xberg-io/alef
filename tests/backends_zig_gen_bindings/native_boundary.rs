use super::*;

#[test]
fn from_json_params_check_null_and_defer_handle_cleanup() {
    let config_type = TypeDef {
        name: "Config".into(),
        rust_path: "demo::Config".into(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        serde_container_default: false,
        serde_deserialization: Default::default(),
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![config_type],
        functions: vec![FunctionDef {
            name: "configure".into(),
            rust_path: "demo::configure".into(),
            original_rust_path: String::new(),
            params: vec![make_param("config", TypeRef::Named("Config".into()))],
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &make_config()).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains(
            "pub fn configure(config: []const u8) error{OutOfMemory,InvalidJson,UnknownFfiError,Cancelled}![]u8"
        ),
        "from_json failure must be part of the generated error union: {content}"
    );
    assert!(
        content.contains("const config_handle = c.demo_config_from_json(config_z);"),
        "must create a handle via _from_json: {content}"
    );
    // Handles are a scalar `u64` under the scalar handle ABI (commit
    // 03109fc52, "fix(zig)!: adopt scalar handle ABI"), so `0` is the "none"
    // sentinel checked here, not a `null` pointer comparison. ~keep
    assert!(
        content.contains("if (config_handle == 0) return error.InvalidJson;"),
        "must check _from_json handle creation before the primary call: {content}"
    );
    assert!(
        content.contains("defer c.demo_config_free(config_handle);"),
        "non-null _from_json handles must be cleaned up with defer: {content}"
    );
}

#[test]
fn client_constructors_emits_create_function() {
    let toml = r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[workspace.client_constructors.DefaultClient]
body = "demo::DefaultClient::new(api_key)"
error_type = "String"

[[workspace.client_constructors.DefaultClient.params]]
name = "api_key"
type = "*const std::ffi::c_char"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    let config = cfg.resolve().expect("test config must resolve").remove(0);

    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "demo::DefaultClient".to_string(),
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
            serde_deserialization: Default::default(),
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = ZigBackend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub fn create_default_client("),
        "should emit create_default_client function: {content}"
    );
    assert!(
        content.contains("api_key: []const u8"),
        "string param should map to []const u8: {content}"
    );
    assert!(
        content.contains("c.demo_default_client_new("),
        "should call FFI constructor: {content}"
    );
    assert!(
        content.contains("if (_handle == 0) return _error_with_message(anyerror);"),
        "should read the native error on null handle: {content}"
    );
    assert!(
        !content.contains("_first_error"),
        "a null handle substantiates no specific variant, so the first-declared-variant \
         fallback must not be emitted: {content}"
    );
}
