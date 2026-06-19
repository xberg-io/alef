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

fn capsule_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-capsule"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "tsp"
[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
[crates.go]
module = "github.com/test/sample-capsule"
[crates.go.capsule_types.Language]
host_type = "*tree_sitter.Language"
package = "github.com/tree-sitter/go-tree-sitter"
package_version = "v0.25.0"
"#,
    )
}

fn capsule_api() -> crate::core::ir::ApiSurface {
    use crate::core::ir::*;
    ApiSurface {
        crate_name: "sample-capsule".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Language".to_string(),
            rust_path: "sample_capsule::Language".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "A grammar.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "get_language".to_string(),
            rust_path: "sample_capsule::get_language".to_string(),
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
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: None,
            doc: "Look up a grammar.".to_string(),
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

#[test]
fn capsule_function_constructs_host_language_and_imports_package() {
    let config = capsule_config();
    let api = capsule_api();
    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding
            .content
            .contains("func GetLanguage(name string) *tree_sitter.Language"),
        "capsule wrapper must return host *tree_sitter.Language. Got:\n{}",
        binding.content
    );
    assert!(
        binding
            .content
            .contains("tree_sitter.NewLanguage(unsafe.Pointer(cLang))"),
        "capsule wrapper must construct via tree_sitter.NewLanguage. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("github.com/tree-sitter/go-tree-sitter"),
        "binding.go must import the go-tree-sitter package"
    );
}
