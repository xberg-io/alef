use super::*;
use crate::core::backend::Backend;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::*;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
    )
}

fn make_api_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Config".to_string(),
            rust_path: "test_lib::Config".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: "timeout".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
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
        functions: vec![FunctionDef {
            name: "process".to_string(),
            rust_path: "test_lib::process".to_string(),
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
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
        ..Default::default()
    }
}

#[test]
fn generates_magnus_module_init() {
    let backend = MagnusBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    assert!(content.contains("#[magnus::init]"), "must emit #[magnus::init]");
}

#[test]
fn generates_struct_with_magnus_wrap() {
    let backend = MagnusBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("magnus::wrap"),
        "structs must have magnus::wrap attribute"
    );
    assert!(content.contains("struct Config"), "Config struct must be generated");
}

#[test]
fn generate_public_api_emits_gem_files() {
    let backend = MagnusBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    assert_eq!(files.len(), 3, "must generate main rb file + native.rb + version file");
    let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with("test_lib.rb")),
        "must have main gem file"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("native.rb")),
        "must have native.rb file"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("version.rb")),
        "must have version file"
    );
}

#[test]
fn output_path_defaults_to_packages_ruby() {
    let backend = MagnusBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert!(
        files[0].path.to_string_lossy().contains("lib.rs"),
        "output must be lib.rs"
    );
}

#[test]
fn test_explicit_re_export_list_filters_internal_types() {
    // Verify that generate_public_api includes only struct types in the re-export list,
    // filtering out enums (which are not registered on the native module).
    let backend = MagnusBackend;

    // Create a custom config where module_name != native_module_name
    // (so the template emits the re-export block).
    // api.crate_name "my_lib" → native_module_name "MyLib"
    // gem_name "my_gem" → module_name "MyGem" (different!)
    let cfg_str = r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "my_lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "my_gem"
"#;
    let cfg: NewAlefConfig = toml::from_str(cfg_str).unwrap();
    let config = cfg.resolve().unwrap().remove(0);

    let mut api = make_api_surface();
    api.crate_name = "my_lib".to_string();
    // Add an enum to the API surface
    api.enums.push(EnumDef {
        name: "Status".to_string(),
        rust_path: "sample_markdown::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Active".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                version: Default::default(),
            },
            EnumVariant {
                name: "Inactive".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                version: Default::default(),
            },
        ],
        doc: String::new(),
        serde_tag: None,
        serde_rename_all: None,
        serde_untagged: false,
        is_copy: false,
        has_serde: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        cfg: None,
        version: Default::default(),
    });

    let files = backend.generate_public_api(&api, &config).unwrap();
    let native_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("native.rb"))
        .expect("native.rb must exist");

    // Verify that the enum (Status) is NOT in the re-export list via const_get
    assert!(
        !native_file.content.contains("const_get(:Status)"),
        "enum types must not be in re-export list"
    );
    // Verify that the struct type (Config) IS in the re-export list via const_get
    assert!(
        native_file.content.contains("const_get(:Config)"),
        "struct types must be in re-export list via const_get"
    );
}
