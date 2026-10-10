use super::*;

#[test]
fn test_scaffold_elixir_preserves_configured_maintainers() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>", "Grace Hopper <grace@example.com>"]
"#,
    );
    let all_files = scaffold(&test_api(), &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix = files
        .iter()
        .find(|file| file.path == Path::new("packages/elixir/mix.exs"))
        .expect("Elixir mix.exs must be emitted");
    assert!(
        mix.content
            .contains(r#"maintainers: ["Ada Lovelace <ada@example.com>", "Grace Hopper <grace@example.com>"]"#),
        "mix.exs must preserve configured maintainers; got:\n{}",
        mix.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_lib_name_no_path() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        !cargo_toml.content.contains("-elixir/src/lib.rs"),
        "Elixir Cargo.toml [lib] must NOT point to a non-existent -elixir crate; content: {}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("name = \"my_lib_nif\""),
        "Elixir Cargo.toml [lib] must set name to {{app_name}}_nif; content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_lib_path_for_external_output() {
    let config = test_config_from_toml(
        r#"
[crates.output]
elixir = "crates/my-lib-elixir/src/"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    assert!(
        cargo_toml
            .content
            .contains(r#"path = "../../../../crates/my-lib-elixir/src/lib.rs""#),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_elixirc_paths_normalizes_leading_slash() {
    let config = test_config_from_toml(
        r#"
[crates.output]
elixir = "/crates/my-lib-elixir/src/"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs
            .content
            .contains(r#"elixirc_paths: ["lib", Path.expand("../../crates/my-lib-elixir/src", __DIR__)],"#),
        "content: {}",
        mix_exs.content
    );
    assert!(
        !mix_exs.content.contains("../..//crates"),
        "content: {}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_elixir_mix_exs_files_list_omits_nonexistent_lib_and_checksum() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs
            .content
            .contains("files: ~w(.formatter.exs mix.exs README* checksum-*.exs native/my_lib_nif/Cargo.toml native/my_lib_nif/Cargo.lock)"),
        "content: {}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_elixir_mix_exs_uses_configured_nif_targets() {
    let config = test_config_from_toml(
        r#"
[crates.elixir]
nif_targets = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs.content.contains("rustler_crates: [\n")
            && mix_exs.content.contains("my_lib_nif: [")
            && mix_exs.content.contains("\"aarch64-apple-darwin\",")
            && mix_exs.content.contains("\"x86_64-unknown-linux-gnu\""),
        "mix.exs must wire configured nif_targets into rustler_crates as a multi-line list; content:\n{}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_elixir_mix_exs_files_list_includes_external_source_dir() {
    let config = test_config_from_toml(
        r#"
[crates.output]
elixir = "crates/my-lib-elixir/src/"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files.iter().find(|f| f.path.ends_with("mix.exs")).unwrap();

    assert!(
        mix_exs.content.contains(
            "files:\n        ~w(.formatter.exs mix.exs README* checksum-*.exs native/my_lib_nif/Cargo.toml native/my_lib_nif/Cargo.lock ../../crates/my-lib-elixir/src)"
        ),
        "content: {}",
        mix_exs.content
    );
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list the nonexistent native/<nif>/src dir; content: {}",
        mix_exs.content
    );
    assert!(
        !mix_exs.content.contains("/*.ex)") && !mix_exs.content.contains("/*.ex "),
        "external-output mix.exs must ship the whole source dir, not just *.ex; content: {}",
        mix_exs.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_no_tokio_when_sync_only() {
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        !cargo_toml.content.contains("tokio"),
        "sync-only API must not include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        !cargo_toml.content.contains("async-trait"),
        "sync-only API without trait bridges must not include async-trait; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_ruby_cargo_no_tokio_when_sync_only() {
    let mut config = test_config();
    config.languages = vec![Language::Ruby];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        !cargo_toml.content.contains("tokio"),
        "sync-only Ruby API must not include tokio; content:\n{}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_tokio_when_async_function() {
    use crate::core::ir::{FunctionDef, TypeRef};
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let mut api = test_api();
    api.functions.push(FunctionDef {
        name: "do_work".to_string(),
        rust_path: "my_lib::do_work".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: true,
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
    });
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("tokio"),
        "async function API must include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("rt-multi-thread"),
        "tokio dep must include rt-multi-thread feature; content:\n{}",
        cargo_toml.content
    );
}

/// Trait bridge module names must use PascalCase for hyphenated crate names.
///
/// When the source crate name contains hyphens (e.g., `demo-markup`), the
/// Elixir trait bridge module name must be `DemoMarkupHtmlVisitorBridge`, not
/// `Demo_markupHtmlVisitorBridge` (which is what `capitalize_first` produces).
#[test]
fn test_scaffold_elixir_trait_bridge_module_name_is_pascal_case_for_hyphenated_crate() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "demo-markup".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(crate::core::config::ElixirConfig {
        dependency_versions: Default::default(),
        app_name: Some("demo_markup".to_string()),
        features: None,
        nif_features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
        nif_targets: Vec::new(),
        target_dep_overrides: Vec::new(),
        excluded_default_features: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "HtmlVisitor".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("html_visitor_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file.content.contains("defmodule DemoMarkupHtmlVisitorBridge do"),
        "trait bridge module name must be PascalCase for hyphenated crate names; got:\n{}",
        bridge_file.content
    );
    assert!(
        !bridge_file.content.contains("Demo_markup"),
        "trait bridge module name must not contain capitalize_first artifact 'Demo_markup'; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_trait_bridge_registers_genserver_pid_and_plugin_name() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "demo-markup".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(crate::core::config::ElixirConfig {
        dependency_versions: Default::default(),
        app_name: Some("demo_markup".to_string()),
        features: None,
        nif_features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
        nif_targets: Vec::new(),
        target_dep_overrides: Vec::new(),
        excluded_default_features: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "OcrBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        registry_getter: Some("demo_markup::get_registry".to_string()),
        register_fn: Some("register_ocr_backend".to_string()),
        unregister_fn: None,
        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("ocr_backend_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file.content.contains("plugin_name = impl_module.name()")
            && bridge_file
                .content
                .contains("DemoMarkup.Native.register_ocr_backend(pid, plugin_name, implemented_methods)"),
        "register/1 must require Plugin.name/0 and register the started GenServer pid; got:\n{}",
        bridge_file.content
    );
    assert!(
        bridge_file.content.contains("impl_module.__info__(:functions)"),
        "register/1 must pass the implementation module's exported function names so \
         Rust-defaulted trait methods outside the set keep their Rust default; got:\n{}",
        bridge_file.content
    );
    assert!(
        !bridge_file
            .content
            .contains("register_ocr_backend(self(), Atom.to_string(impl_module))"),
        "register/1 must not register the caller pid or fallback module string name; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_trait_bridge_module_name_is_pascal_case_for_multi_word_crate() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.name = "sample-language-pack".to_string();
    config.languages = vec![Language::Elixir];
    config.elixir = Some(crate::core::config::ElixirConfig {
        dependency_versions: Default::default(),
        app_name: Some("sample_language_pack".to_string()),
        features: None,
        nif_features: None,
        serde_rename_all: None,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: Default::default(),
        scaffold_output: Default::default(),
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        cpu_bound_functions: Vec::new(),
        nif_targets: Vec::new(),
        target_dep_overrides: Vec::new(),
        excluded_default_features: Vec::new(),
    });
    config.trait_bridges = vec![TraitBridgeConfig {
        exclude_functions: Vec::new(),
        trait_name: "Parser".to_string(),
        super_trait: None,
        registry_getter: None,
        register_fn: None,

        unregister_fn: None,

        clear_fn: None,
        type_alias: None,
        param_name: None,
        register_extra_args: None,
        exclude_languages: vec![],
        ffi_skip_methods: Vec::new(),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        options_type: None,
        options_field: None,
        context_type: None,
        result_type: None,
    }];

    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let bridge_file = all_files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("parser_bridge.ex"))
        .expect("Elixir scaffold must produce a trait bridge .ex file");

    assert!(
        bridge_file
            .content
            .contains("defmodule SampleLanguagePackParserBridge do"),
        "trait bridge module name must be full PascalCase; got:\n{}",
        bridge_file.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_tokio_when_async_method() {
    use crate::core::ir::{MethodDef, TypeDef, TypeRef};
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let mut api = test_api();
    api.types.push(TypeDef {
        name: "Worker".to_string(),
        rust_path: "my_lib::Worker".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![MethodDef {
            name: "run".to_string(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: true,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: None,
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
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    });
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("tokio"),
        "async method API must include tokio; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("rt-multi-thread"),
        "tokio dep must include rt-multi-thread feature; content:\n{}",
        cargo_toml.content
    );
}

/// When explicit_output.elixir points at an external source directory (where the
/// NIF crate's `[lib] path` resolves), the generated mix.exs `files:` list must
/// list that directory as a self-contained dir entry — shipping the Rust NIF
/// `lib.rs` and any `*.rs`/`*.ex` together — instead of a bare `/*.ex` glob that
/// leaves the Rust source out of the tarball.
#[test]
fn test_scaffold_elixir_mix_exs_external_dir_is_listed_as_whole_dir() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let rs_dir = tmp.path();

    std::fs::write(rs_dir.join("lib.rs"), "// Rust NIF source\n").expect("write lib.rs");
    std::fs::write(rs_dir.join("Cargo.toml"), "[package]\n").expect("write Cargo.toml");

    let explicit_path = rs_dir.to_string_lossy().to_string();
    let config = test_config_from_toml(&format!(
        r#"
[crates.output]
elixir = '{explicit_path}'
"#
    ));
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files
        .iter()
        .find(|f| f.path.ends_with("mix.exs"))
        .expect("mix.exs must be generated");

    assert!(
        !mix_exs.content.contains("/*.ex)") && !mix_exs.content.contains("/*.ex "),
        "external-output mix.exs must list the whole source dir, not a /*.ex glob; content:\n{}",
        mix_exs.content
    );
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list native/<nif>/src; content:\n{}",
        mix_exs.content
    );
    assert!(
        mix_exs.content.contains(".formatter.exs"),
        "mix.exs should contain .formatter.exs"
    );
    assert!(
        mix_exs.content.contains("native/my_lib_nif/Cargo.toml"),
        "mix.exs should still ship the NIF Cargo.toml"
    );
}

/// Even when the external Elixir output directory contains `.ex`/`.exs` modules,
/// it is still listed as a single self-contained directory entry (covering both
/// the Elixir modules and the co-located Rust NIF source), not a `/*.ex` glob.
#[test]
fn test_scaffold_elixir_mix_exs_external_dir_with_ex_sources_listed_as_dir() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let ex_dir = tmp.path();

    std::fs::write(ex_dir.join("module.ex"), "defmodule Test do\nend\n").expect("write module.ex");
    std::fs::write(ex_dir.join("helper.exs"), "# helper\n").expect("write helper.exs");
    std::fs::write(ex_dir.join("lib.rs"), "// Rust NIF source\n").expect("write lib.rs");

    let explicit_path = ex_dir.to_string_lossy().to_string();
    let config = test_config_from_toml(&format!(
        r#"
[crates.output]
elixir = '{explicit_path}'
"#
    ));
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files
        .iter()
        .find(|f| f.path.ends_with("mix.exs"))
        .expect("mix.exs must be generated");

    assert!(
        !mix_exs.content.contains("/*.ex)") && !mix_exs.content.contains("/*.ex "),
        "external-output mix.exs must list the whole source dir, not a /*.ex glob; content:\n{}",
        mix_exs.content
    );
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list native/<nif>/src; content:\n{}",
        mix_exs.content
    );
}

/// The derived default `[features]` block must mirror the core crate's own declared
/// `[features] default = [...]` list -- not a fixed alef-side name list. This fixture's core
/// crate declares defaults (`turbo-cache`) that share no name with any historical alef default
/// (`download`/`serde`/`config`); a hardcoded name list would produce `default = []` here even
/// though the core crate itself opts `turbo-cache` in by default, silently changing what the
/// generated NIF builds with by default.
#[test]
fn test_scaffold_elixir_cargo_derives_features_from_core_crate() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let ws_root = tmp.path();
    let core_dir = ws_root.join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core dir");

    let cargo_toml_content = r#"
[package]
name = "my-lib"
version = "0.1.0"
edition = "2024"

[features]
default = ["turbo-cache"]
turbo-cache = []
opendal-cache = []
wasm-http = []
"#;
    std::fs::write(core_dir.join("Cargo.toml"), cargo_toml_content).expect("write Cargo.toml");

    let mut config = test_config();
    config.workspace_root = Some(ws_root.to_path_buf());
    config.name = "my-lib".to_string();
    config.sources = vec![std::path::PathBuf::from("crates/my-lib/src/lib.rs")];
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Cargo.toml must be generated");

    let features_start = cargo_toml
        .content
        .find("[features]")
        .expect("must have [features] block");
    let deps_start = cargo_toml
        .content
        .find("[dependencies]")
        .expect("must have [dependencies] block");
    let features_block = &cargo_toml.content[features_start..deps_start];

    assert!(
        !features_block.contains("opendal-cache = [\"my-lib/opendal-cache\"]"),
        "Elixir Cargo.toml must not forward a core feature the core crate does not list under \
         its own default; content:\n{}",
        features_block
    );
    assert!(
        !features_block.contains("wasm-http = [\"my-lib/wasm-http\"]"),
        "Elixir Cargo.toml must not forward a core feature the core crate does not list under \
         its own default; content:\n{}",
        features_block
    );

    assert!(
        features_block.contains("default = [\"turbo-cache\"]"),
        "Elixir Cargo.toml default array must mirror the core crate's own declared \
         [features] default list, not a fixed alef-side name list; content:\n{}",
        features_block
    );
    assert!(
        features_block.contains("turbo-cache = [\"my-lib/turbo-cache\"]"),
        "the core crate's declared default feature must be forwarded to the core dependency; \
         content:\n{}",
        features_block
    );
}

/// Regression for alef-task #375: the "no explicit `nif_features`" fallback must derive
/// defaults from the core crate the same way whether or not `[crates.elixir]` is present at
/// all -- the two prior branches (config present without `nif_features`, and config absent
/// entirely) computed the identical fallback and must keep doing so now that they are one
/// expression.
#[test]
fn test_scaffold_elixir_cargo_derives_features_from_core_crate_when_elixir_config_present() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let ws_root = tmp.path();
    let core_dir = ws_root.join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core dir");

    let cargo_toml_content = r#"
[package]
name = "my-lib"
version = "0.1.0"
edition = "2024"

[features]
default = ["turbo-cache"]
turbo-cache = []
"#;
    std::fs::write(core_dir.join("Cargo.toml"), cargo_toml_content).expect("write Cargo.toml");

    let mut config = test_config_from_toml(
        r#"
[crates.elixir]
cpu_bound_functions = ["parse"]
"#,
    );
    config.workspace_root = Some(ws_root.to_path_buf());
    config.name = "my-lib".to_string();
    config.sources = vec![std::path::PathBuf::from("crates/my-lib/src/lib.rs")];
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Cargo.toml must be generated");

    let default_line = cargo_toml
        .content
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default array present");
    assert_eq!(
        default_line, "default = [\"turbo-cache\"]",
        "a present [crates.elixir] table without nif_features must still derive defaults from \
         the core crate; content:\n{}",
        cargo_toml.content
    );
}

/// An explicit `nif_features` override must win over the core crate's own declared defaults,
/// even when they disagree.
#[test]
fn test_scaffold_elixir_cargo_explicit_nif_features_overrides_core_default() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let ws_root = tmp.path();
    let core_dir = ws_root.join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core dir");

    let cargo_toml_content = r#"
[package]
name = "my-lib"
version = "0.1.0"
edition = "2024"

[features]
default = ["turbo-cache"]
turbo-cache = []
zen-mode = []
"#;
    std::fs::write(core_dir.join("Cargo.toml"), cargo_toml_content).expect("write Cargo.toml");

    let mut config = test_config_from_toml(
        r#"
[crates.elixir]
nif_features = ["zen-mode"]
"#,
    );
    config.workspace_root = Some(ws_root.to_path_buf());
    config.name = "my-lib".to_string();
    config.sources = vec![std::path::PathBuf::from("crates/my-lib/src/lib.rs")];
    let api = test_api();

    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Cargo.toml must be generated");

    let default_line = cargo_toml
        .content
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default array present");
    assert_eq!(
        default_line, "default = [\"zen-mode\"]",
        "an explicit nif_features override must win over the core crate's own default list; \
         content:\n{}",
        cargo_toml.content
    );
}

/// The elixir scaffold already emits its own `unexpected_cfgs` check-cfg allowlist
/// into `[lints.rust]`. A configured `[crates.cargo_lints]` table must compose with
/// that single table -- not open a second `[lints.rust]` header, which Cargo
/// rejects as a duplicate table -- and the builtin `unexpected_cfgs` entry must
/// survive even if the user also sets that key.
#[test]
fn test_scaffold_elixir_cargo_lints_merges_with_builtin_unexpected_cfgs() {
    let config = test_config_from_toml(
        r#"
[crates.cargo_lints.rust]
unexpected_cfgs = "warn"
unused_must_use = "deny"

[crates.cargo_lints.clippy]
print_stdout = "deny"
"#,
    );
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Cargo.toml must be generated");

    assert_eq!(
        cargo_toml.content.matches("[lints.rust]").count(),
        1,
        "must not emit a second [lints.rust] table; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml
            .content
            .contains("unexpected_cfgs = { level = \"warn\", check-cfg ="),
        "the builtin unexpected_cfgs entry must survive the user's colliding key; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("unused_must_use = \"deny\""),
        "non-colliding configured rust lints must be spliced in; content:\n{}",
        cargo_toml.content
    );
    assert!(
        cargo_toml
            .content
            .contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "configured clippy lints must merge with the builtin deny defaults; content:\n{}",
        cargo_toml.content
    );
    toml::from_str::<toml::Value>(&cargo_toml.content).expect("generated Cargo.toml must be valid TOML");
}

/// Absence of `[crates.cargo_lints]` must leave the pre-existing builtin `[lints.rust]`
/// block (only the `unexpected_cfgs` entry) exactly as it was, and must still emit the
/// builtin `[lints.clippy]` deny block right after it.
#[test]
fn test_scaffold_elixir_cargo_lints_unset_emits_builtin_clippy_denies() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Cargo.toml must be generated");

    let lints_start = cargo_toml
        .content
        .find("[lints.rust]")
        .expect("builtin [lints.rust] block must still be emitted");
    let lints_block = &cargo_toml.content[lints_start..];
    assert_eq!(
        lints_block.matches("unexpected_cfgs").count(),
        1,
        "content:\n{}",
        cargo_toml.content
    );
    assert!(
        lints_block.contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "the builtin [lints.clippy] deny block must be emitted even when cargo_lints is unset; content:\n{}",
        cargo_toml.content
    );
    toml::from_str::<toml::Value>(&cargo_toml.content).expect("generated Cargo.toml must be valid TOML");
}

/// Regression for alef-task #320: `scaffold_elixir_cargo` unconditionally forwarded every
/// `collect_cfg_features` name into the wrapper's own `default = [...]` array and every
/// `[crates.elixir].features` name into the core dependency's own explicit `features = [...]`
/// line, re-enabling a feature a `target_dep_overrides` entry excluded for one cfg target -- the
/// same defect `RubyConfig::excluded_default_features` fixed for the Magnus crate, generalized
/// here. Asserts both directions on both surfaces: the excluded name is never defaulted or
/// forwarded, and a name nobody excluded still is.
#[test]
fn test_scaffold_elixir_cargo_excludes_named_feature_from_default_but_keeps_others() {
    let config = test_config_from_toml(
        r#"
[crates.elixir]
features = ["native-http", "wasm-http"]
excluded_default_features = ["native-http"]
[[crates.elixir.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["wasm-http"]
default_features = false
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            crate::core::ir::TypeDef {
                name: "NativeOnly".to_string(),
                rust_path: "my_lib::NativeOnly".to_string(),
                cfg: Some(r#"feature = "native-http""#.to_string()),
                ..Default::default()
            },
            crate::core::ir::TypeDef {
                name: "WasmOnly".to_string(),
                rust_path: "my_lib::WasmOnly".to_string(),
                cfg: Some(r#"feature = "wasm-http""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Cargo.toml must be generated");

    let default_line = cargo_toml
        .content
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default array present");
    assert!(
        !default_line.contains("native-http"),
        "excluded_default_features must drop the name from the wrapper's own default array:\n{default_line}"
    );
    assert!(
        default_line.contains("wasm-http"),
        "a feature nobody excluded must still be forwarded into default:\n{default_line}"
    );
    assert!(
        cargo_toml.content.contains(r#"native-http = ["my-lib/native-http"]"#),
        "the excluded feature stays declared (so `cargo build --features native-http` still \
         works), just not defaulted:\n{}",
        cargo_toml.content
    );

    let default_target_block = cargo_toml
        .content
        .split("[target.'cfg(not(target_os")
        .nth(1)
        .expect("default target block present");
    let default_block_dep_line = default_target_block
        .lines()
        .find(|line| line.trim_start().starts_with("my-lib ="))
        .expect("core dependency line present in default target block");
    assert!(
        !default_block_dep_line.contains("native-http"),
        "excluded_default_features must also drop the name from the core dependency's own \
         explicit features = [...] line, not just the wrapper's default array:\n{default_block_dep_line}"
    );
}
