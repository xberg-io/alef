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
    use crate::codegen::naming::go_package_name_from_module;

    assert_eq!(go_package_name_from_module("github.com/org/my-lib"), "mylib");
    assert_eq!(go_package_name_from_module("binding"), "binding");
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert!(!files.is_empty());
    assert!(files[0].path.to_string_lossy().contains("binding.go"));

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

/// Regression test for the dropped `exclude_functions` config key on `[crates.go]`: today
/// the Go backend only honours `[crates.ffi].exclude_functions`, which would also strip the
/// function's C symbol from every other binding. A per-language `[crates.go].exclude_functions`
/// must hide a function from Go's generated `binding.go` while leaving the FFI-level list (and
/// hence the C ABI, and other bindings) untouched — mirrors `CSharpConfig::exclude_functions`.
#[test]
fn test_generate_bindings_unions_go_exclude_functions_with_ffi_exclude_functions() {
    use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};

    let config = resolved_one(
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
exclude_functions = ["embed_sparse_async"]
"#,
    );

    let make_fn = |name: &str| FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![make_fn("embed_sparse_async"), make_fn("other_func")],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        !binding.content.contains("EmbedSparseAsync"),
        "GoConfig::exclude_functions must drop the function from binding.go:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("OtherFunc"),
        "a function not named in exclude_functions must still be generated:\n{}",
        binding.content
    );
}

/// `GoConfig::exclude_functions` must UNION with `[crates.ffi].exclude_functions`, not
/// replace it: a function named only at the FFI level must still be dropped from Go's
/// `binding.go`, alongside a function named only at the Go level, while a function named in
/// neither list survives.
#[test]
fn test_generate_bindings_go_exclude_functions_unions_rather_than_replaces_ffi_list() {
    use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};

    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "test"
exclude_functions = ["ffi_only_excluded"]
[crates.go]
module = "github.com/test/test-lib"
exclude_functions = ["go_only_excluded"]
"#,
    );

    let make_fn = |name: &str| FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            make_fn("ffi_only_excluded"),
            make_fn("go_only_excluded"),
            make_fn("kept_everywhere"),
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        !binding.content.contains("FfiOnlyExcluded"),
        "a function excluded only at the FFI level must still be dropped from Go:\n{}",
        binding.content
    );
    assert!(
        !binding.content.contains("GoOnlyExcluded"),
        "a function excluded only at the Go level must be dropped from Go:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("KeptEverywhere"),
        "a function excluded in neither list must survive:\n{}",
        binding.content
    );
}

#[test]
fn test_generate_bindings_emits_cmd_setup_and_native_setup_sentinel() {
    use crate::core::ir::ApiSurface;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0-rc.38".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(
        !files
            .iter()
            .any(|f| f.path.to_string_lossy().ends_with("cmd/download_ffi/main.go")),
        "the old cmd/download_ffi tool must no longer be emitted"
    );

    let setup = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("cmd/setup/main.go"))
        .expect("cmd/setup/main.go must be generated");
    assert!(
        setup.content.contains(r#"moduleVersion     = "1.0.0-rc.38""#),
        "cmd/setup/main.go must embed the crate version:\n{}",
        setup.content
    );
    assert!(
        setup.content.contains(r#"versionIdent      = "1_0_0_rc_38""#),
        "cmd/setup must embed the version-matched sentinel identifier:\n{}",
        setup.content
    );
    assert!(
        setup.content.contains("RequireNativeSetup_%s"),
        "cmd/setup's shim writer must build the RequireNativeSetup_<versionIdent> reference:\n{}",
        setup.content
    );

    let native_setup = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("native_setup.go"))
        .expect("native_setup.go must be generated");
    assert!(
        native_setup
            .content
            .contains(r#"const RequireNativeSetup_1_0_0_rc_38 = "1.0.0-rc.38""#),
        "native_setup.go must declare the version-skew sentinel:\n{}",
        native_setup.content
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
into_raw_type = "my_crate::ffi::MyLang"
c_return_type = "MyLang"
[crates.go]
module = "github.com/test/sample-capsule"
[crates.go.capsule_types.Language]
host_type = "*my_pkg.Language"
package = "github.com/example/go-my-lib"
package_version = "v1.0.0"
construct_expr = "my_pkg.NewLanguage(unsafe.Pointer({ptr}))"
pointer_ownership = "borrowed_static"
abi_compatible = true
host_destructor = "none"
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
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "A grammar.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
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
            .contains("func GetLanguage(name string) *my_pkg.Language"),
        "capsule wrapper must return host *my_pkg.Language. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("my_pkg.NewLanguage(unsafe.Pointer(cLang))"),
        "capsule wrapper must construct via my_pkg.NewLanguage. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("github.com/example/go-my-lib"),
        "binding.go must import the configured capsule package"
    );
}

/// A free function whose Go PascalCase name collides with a struct type of the same name
/// (e.g. Rust's `fn model_info(...)` and `struct ModelInfo`) must not produce two `ModelInfo`
/// package-level declarations. The type keeps the plain name; the function is renamed
/// `GetModelInfo`. A non-colliding function in the same package is unaffected.
#[test]
fn free_function_colliding_with_type_name_is_renamed_get_prefixed() {
    use crate::core::ir::*;

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ModelInfo".to_string(),
            rust_path: "test_lib::ModelInfo".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: true,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "Model metadata.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![
            FunctionDef {
                name: "model_info".to_string(),
                rust_path: "test_lib::model_info".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "model".to_string(),
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
                return_type: TypeRef::Optional(Box::new(TypeRef::Named("ModelInfo".to_string()))),
                is_async: false,
                error_type: None,
                doc: "Look up model metadata by name.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            FunctionDef {
                name: "list_models".to_string(),
                rust_path: "test_lib::list_models".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "List known model names.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding.content.contains("type ModelInfo struct"),
        "struct type must keep its plain name. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("func GetModelInfo(model string)"),
        "colliding free function must be renamed to GetModelInfo. Got:\n{}",
        binding.content
    );
    assert!(
        !binding.content.contains("func ModelInfo("),
        "colliding free function must not keep the bare type name. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("func ListModels()"),
        "non-colliding function must be unaffected. Got:\n{}",
        binding.content
    );
}

/// Go rejects a struct that carries both a field and a method named `Providers`
/// (`field and method with the same name`). A core type with a public `providers` field and
/// an inherent `providers()` method feeds both the struct emitter and the method-wrapper
/// emitter, so the wrapper must be dropped and the field kept.
#[test]
fn generate_bindings_skips_method_wrapper_when_struct_field_has_same_name() {
    use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "LlmConfig".to_string(),
            rust_path: "test_lib::LlmConfig".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "providers".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            methods: vec![MethodDef {
                name: "providers".to_string(),
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..Default::default()
            }],
            ..Default::default()
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

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding.content.contains("Providers "),
        "the struct field must still be emitted. Got:\n{}",
        binding.content
    );
    let wrappers = binding.content.matches("func (r *LlmConfig) Providers(").count();
    assert_eq!(
        wrappers, 0,
        "the same-named method wrapper must be skipped, found {wrappers} in:\n{}",
        binding.content
    );
}

/// Regression (Defect 1): a `Duration`-typed struct field pulls in the package-level
/// `DurationMillis` wire helper (and, transitively, the `encoding/json` import it needs)
/// even when the crate has no sync functions or non-static methods — the only prior
/// triggers for `encoding/json`.
#[test]
fn generate_bindings_emits_duration_millis_helper_when_a_duration_field_exists() {
    use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "RateLimitConfig".to_string(),
            rust_path: "test_lib::RateLimitConfig".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "window".to_string(),
                ty: TypeRef::Duration,
                ..Default::default()
            }],
            ..Default::default()
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

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding.content.contains("type DurationMillis uint64"),
        "expected the DurationMillis wire helper. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("\"encoding/json\""),
        "DurationMillis's Marshal/UnmarshalJSON need encoding/json imported. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("Window DurationMillis `json:\"window\"`"),
        "expected the field itself to use the wire-safe type. Got:\n{}",
        binding.content
    );
}

/// Counterpart of the above: a crate with no `Duration` field anywhere must not carry the
/// unused `DurationMillis` helper.
#[test]
fn generate_bindings_omits_duration_millis_helper_without_a_duration_field() {
    use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "PlainConfig".to_string(),
            rust_path: "test_lib::PlainConfig".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
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

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        !binding.content.contains("DurationMillis"),
        "no Duration field exists, so the helper must not be emitted. Got:\n{}",
        binding.content
    );
}

#[cfg(test)]
mod pipeline_checks;
