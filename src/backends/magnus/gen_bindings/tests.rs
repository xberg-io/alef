use super::*;
use crate::core::backend::Backend;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::*;
use tracing_test::traced_test;

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
                version: Default::default(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
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
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
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
fn ruby_public_wrapper_keeps_generated_constants_namespaced() {
    let backend = MagnusBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let main_file = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("test_lib.rb"))
        .expect("main Ruby wrapper must exist");

    assert!(
        !main_file.content.contains("Object.const_set"),
        "the generated Ruby wrapper must not export constants globally"
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
    let backend = MagnusBackend;

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
                cfg: None,
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
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        serde_content: None,
        serde_tag: None,
        serde_rename_all: None,
        rename_all_fields: None,
        serde_untagged: false,
        is_copy: false,
        has_serde: false,
        has_default: false,
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

    assert!(
        !native_file.content.contains("const_get(:Status)"),
        "enum types must not be in re-export list"
    );
    assert!(
        native_file.content.contains("const_get(:Config)"),
        "struct types must be in re-export list via const_get"
    );
}

/// A field accessor and a same-named inherent method both mint `fn providers(&self)` in the
/// single `#[magnus::wrap]` impl block, which rustc rejects with `E0592`, and both register
/// `define_method("providers", ...)`. The accessor is emitted first and must win.
#[test]
fn gen_struct_methods_skips_method_wrapper_when_field_accessor_already_emitted() {
    let backend = MagnusBackend;
    let config = make_config();
    let mut api = make_api_surface();
    api.types = vec![TypeDef {
        name: "LlmConfig".to_string(),
        rust_path: "test_lib::LlmConfig".to_string(),
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
    }];

    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let definitions = content.matches("fn providers(&self)").count();
    assert_eq!(
        definitions, 1,
        "`providers` must be defined exactly once, found {definitions} in:\n{content}"
    );
    let registrations = content.matches(r#"define_method("providers""#).count();
    assert_eq!(
        registrations, 1,
        "`providers` must be registered exactly once, found {registrations} in:\n{content}"
    );
}

/// End-to-end reproduction of the liter-llm incident through the real `generate_bindings` entry
/// point: a `#[cfg(feature = "tokenizer")]`-gated function reaches the generated `lib.rs`, but the
/// scaffolded `native/Cargo.toml` on disk (deliberately stale here, as `alef scaffold` output
/// legitimately goes once a core crate gains a cfg-gated item after the last scaffold run) never
/// declares `tokenizer`. `alef build` must surface that with a warning instead of staying silent.
#[traced_test]
#[test]
fn generate_bindings_warns_when_the_scaffolded_manifest_is_missing_a_referenced_feature() {
    let backend = MagnusBackend;
    let mut config = make_config();
    let workspace = tempfile::tempdir().expect("tempdir");
    config.workspace_root = Some(workspace.path().to_path_buf());

    // Same formula `MagnusBackend::generate_bindings` itself reads the manifest back from, so
    // this test does not depend on guessing the scaffold's directory layout.
    let manifest_path = workspace
        .path()
        .join(crate::scaffold::ruby_native_manifest_path(&config));
    std::fs::create_dir_all(manifest_path.parent().expect("manifest has a parent dir"))
        .expect("create native crate dir");
    std::fs::write(
        &manifest_path,
        "[package]\nname = \"test_lib_rb\"\n\n[features]\ndefault = []\n",
    )
    .expect("write stale manifest");

    let mut api = make_api_surface();
    api.functions.push(FunctionDef {
        name: "count_tokens".to_string(),
        rust_path: "test_lib::count_tokens".to_string(),
        cfg: Some(r#"feature = "tokenizer""#.to_string()),
        ..Default::default()
    });

    let _ = backend.generate_bindings(&api, &config).unwrap();

    assert!(
        logs_contain("does not enable by default"),
        "a stale scaffolded manifest missing a referenced feature must warn during `alef build`"
    );
}

/// The same manifest, but declaring `tokenizer` -- the normal, up-to-date case -- must stay
/// silent.
#[traced_test]
#[test]
fn generate_bindings_stays_silent_when_the_scaffolded_manifest_declares_the_feature() {
    let backend = MagnusBackend;
    let mut config = make_config();
    let workspace = tempfile::tempdir().expect("tempdir");
    config.workspace_root = Some(workspace.path().to_path_buf());

    let manifest_path = workspace
        .path()
        .join(crate::scaffold::ruby_native_manifest_path(&config));
    std::fs::create_dir_all(manifest_path.parent().expect("manifest has a parent dir"))
        .expect("create native crate dir");
    std::fs::write(
        &manifest_path,
        "[package]\nname = \"test_lib_rb\"\n\n[features]\ndefault = [\"tokenizer\"]\ntokenizer = [\"test-lib/tokenizer\"]\n",
    )
    .expect("write up-to-date manifest");

    let mut api = make_api_surface();
    api.functions.push(FunctionDef {
        name: "count_tokens".to_string(),
        rust_path: "test_lib::count_tokens".to_string(),
        cfg: Some(r#"feature = "tokenizer""#.to_string()),
        ..Default::default()
    });

    let _ = backend.generate_bindings(&api, &config).unwrap();

    assert!(
        !logs_contain("does not enable by default"),
        "an up-to-date manifest must not warn"
    );
}
