use super::binding_file::{is_ffi_enum_type, strip_trailing_whitespace};
use super::constructors::gen_go_opaque_constructor;
use super::*;
use crate::core::config::NewAlefConfig;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "test"
[crates.go]
module = "github.com/test/test-lib"
"#,
    )
}

#[test]
fn test_package_name_extracts_last_segment() {
    assert_eq!(GoBackend::package_name("github.com/org/my-lib"), "mylib");
    assert_eq!(GoBackend::package_name("binding"), "binding");
}

#[test]
fn test_strip_trailing_whitespace_normalizes_lines() {
    let input = "line one   \nline two\n";
    let result = strip_trailing_whitespace(input);
    assert_eq!(result, "line one\nline two\n");
}

#[test]
fn test_is_ffi_enum_type_returns_true_for_known_enum() {
    let mut enum_names = HashSet::new();
    enum_names.insert("Status".to_string());
    assert!(is_ffi_enum_type("Status", &enum_names));
    assert!(!is_ffi_enum_type("Config", &enum_names));
}

#[test]
fn test_generate_bindings_produces_binding_go_file() {
    use crate::core::ir::ApiSurface;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
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
    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert!(!files.is_empty());
    assert!(files[0].path.to_string_lossy().contains("binding.go"));

    // embed_ffi.go must declare the same package as binding.go, never a
    // hardcoded foreign package name.
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");
    let pkg_line = binding
        .content
        .lines()
        .find(|l| l.starts_with("package "))
        .expect("binding.go declares a package");
    let embed = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("embed_ffi.go"))
        .expect("embed_ffi.go present");
    assert!(
        !embed.content.contains("package samplepack"),
        "embed_ffi.go must not hardcode the samplepack package name"
    );
    assert!(
        embed.content.contains(pkg_line),
        "embed_ffi.go package must match binding.go ({pkg_line})"
    );
}

#[test]
fn test_gen_go_opaque_constructor_emits_new_function() {
    use crate::core::config::workspace::{ClientConstructorConfig, ConstructorParam};
    use crate::core::ir::TypeDef;

    let typ = TypeDef {
        name: "TestClient".to_string(),
        rust_path: "test_lib::TestClient".to_string(),
        original_rust_path: "test_lib::TestClient".to_string(),
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
    };
    let ctor = ClientConstructorConfig {
        params: vec![ConstructorParam {
            name: "api_key".to_string(),
            ty: "*const std::ffi::c_char".to_string(),
        }],
        body: "TestClient::new(api_key)".to_string(),
        error_type: None,
    };
    let output = gen_go_opaque_constructor(&typ, "test", &ctor);
    assert!(
        output.contains("func NewTestClient("),
        "should contain func NewTestClient"
    );
    assert!(output.contains("api_key string"), "should contain api_key string param");
    assert!(
        output.contains("C.CString(api_key)"),
        "should use C.CString for c_char param"
    );
    assert!(
        output.contains("C.free(unsafe.Pointer("),
        "should defer-free the C string"
    );
    assert!(
        output.contains("C.test_test_client_new("),
        "should call FFI constructor"
    );
    assert!(output.contains("return nil, fmt.Errorf"), "should return error on nil");
    assert!(
        output.contains("return &TestClient{ptr:"),
        "should return handle on success"
    );
}
