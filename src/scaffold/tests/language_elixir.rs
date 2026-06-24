use super::*;

#[test]
fn test_scaffold_elixir_cargo_lib_name_no_path() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    // [lib] must NOT have a path pointing to a non-existent -elixir crate.
    // Cargo defaults to src/lib.rs, which is where the generated lib.rs lives.
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
    // Default config has no explicit elixir output and no trait bridges, so the
    // generated tree contains no `lib/` directory and no `checksum-*.exs` files.
    // Hex publish refuses to package a non-existent path, so the emitted
    // `files:` list must not advertise them.
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
    // When the Elixir output lives outside packages/elixir/ (the NIF crate's
    // `[lib] path` points there), the NIF Rust `lib.rs` AND the generated `*.ex`
    // modules both live in that external dir — NOT in `native/<nif>/src`. The
    // `files:` list must therefore (a) NOT advertise the nonexistent
    // `native/<nif>/src` (else `mix hex.build` fails with "Missing files") and
    // (b) list the external source dir itself so the Rust NIF source ships in
    // the tarball for RustlerPrecompiled's source-compile fallback.
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
    // The nonexistent native src dir must NOT be listed (it would hard-fail mix hex.build).
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list the nonexistent native/<nif>/src dir; content: {}",
        mix_exs.content
    );
    // The Rust NIF source dir must ship (not just a *.ex glob), so the
    // source-compile fallback can build standalone from the tarball.
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
    let api = test_api(); // all sync — no async functions or methods
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
    });
    config.trait_bridges = vec![TraitBridgeConfig {
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
    });
    config.trait_bridges = vec![TraitBridgeConfig {
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
                .contains("DemoMarkup.Native.register_ocr_backend(pid, plugin_name)"),
        "register/1 must require Plugin.name/0 and register the started GenServer pid; got:\n{}",
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
    });
    config.trait_bridges = vec![TraitBridgeConfig {
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
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
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

    // Create a Rust NIF directory with only .rs and .toml files (no .ex files).
    std::fs::write(rs_dir.join("lib.rs"), "// Rust NIF source\n").expect("write lib.rs");
    std::fs::write(rs_dir.join("Cargo.toml"), "[package]\n").expect("write Cargo.toml");

    // Build config pointing explicit_output.elixir at the .rs-only directory.
    // Use a TOML literal string (single quotes) so Windows backslash paths like
    // `C:\Users\RUNNER~1\…` aren't interpreted as `\U` unicode escapes by the
    // TOML basic-string parser.
    let explicit_path = rs_dir.to_string_lossy().to_string();
    let config = test_config_from_toml(&format!(
        r#"
[crates.output]
elixir = '{explicit_path}'
"#
    ));
    let api = test_api();

    // Scaffold Elixir and extract the generated mix.exs.
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files
        .iter()
        .find(|f| f.path.ends_with("mix.exs"))
        .expect("mix.exs must be generated");

    // No bare `/*.ex` glob — the whole dir is shipped so the Rust source travels too.
    // (Note: checksum-*.exs contains the substring *.ex, so check the path-glob form.)
    assert!(
        !mix_exs.content.contains("/*.ex)") && !mix_exs.content.contains("/*.ex "),
        "external-output mix.exs must list the whole source dir, not a /*.ex glob; content:\n{}",
        mix_exs.content
    );
    // The nonexistent native src dir must not be advertised (else mix hex.build fails).
    assert!(
        !mix_exs.content.contains("native/my_lib_nif/src"),
        "external-output mix.exs must not list native/<nif>/src; content:\n{}",
        mix_exs.content
    );
    // Verify that standard entries are still present.
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

    // Create a directory with both Elixir source files and the Rust NIF source.
    std::fs::write(ex_dir.join("module.ex"), "defmodule Test do\nend\n").expect("write module.ex");
    std::fs::write(ex_dir.join("helper.exs"), "# helper\n").expect("write helper.exs");
    std::fs::write(ex_dir.join("lib.rs"), "// Rust NIF source\n").expect("write lib.rs");

    // Build config pointing explicit_output.elixir at the .ex-containing directory.
    // Single-quoted TOML literal string keeps Windows `\U`/`\R`/`\T` path segments
    // intact (basic strings would parse them as unicode escapes and panic).
    let explicit_path = ex_dir.to_string_lossy().to_string();
    let config = test_config_from_toml(&format!(
        r#"
[crates.output]
elixir = '{explicit_path}'
"#
    ));
    let api = test_api();

    // Scaffold Elixir and extract the generated mix.exs.
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let mix_exs = files
        .iter()
        .find(|f| f.path.ends_with("mix.exs"))
        .expect("mix.exs must be generated");

    // Listed as a whole directory, not a `/*.ex` glob (which would drop lib.rs).
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

/// When a core crate has no config/download/serde features, the derived default
/// [features] block must not list them, avoiding Cargo "does not have that
/// feature" errors.
#[test]
fn test_scaffold_elixir_cargo_derives_features_from_core_crate() {
    let tmp = tempfile::tempdir().expect("tempdir must be created");
    let ws_root = tmp.path();
    let core_dir = ws_root.join("crates").join("my-lib");
    std::fs::create_dir_all(&core_dir).expect("create core dir");

    // Create a minimal Cargo.toml with only native-http and opendal-cache features.
    let cargo_toml_content = r#"
[package]
name = "my-lib"
version = "0.1.0"
edition = "2024"

[features]
default = ["native-http"]
native-http = []
opendal-cache = []
wasm-http = []
"#;
    std::fs::write(core_dir.join("Cargo.toml"), cargo_toml_content).expect("write Cargo.toml");

    // Build a config that points to this workspace root, matching the core crate name.
    let mut config = test_config();
    config.workspace_root = Some(ws_root.to_path_buf());
    // Ensure the config name matches the crate directory name so feature forwarding works.
    config.name = "my-lib".to_string();
    // Set sources to match the standard layout so core_crate_dir derives correctly.
    config.sources = vec![std::path::PathBuf::from("crates/my-lib/src/lib.rs")];
    let api = test_api();

    // Scaffold Elixir Cargo.toml.
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("Cargo.toml must be generated");

    // The [features] block should NOT list config, download, or serde (which don't exist).
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
        !features_block.contains("config = [\"my-lib/config\"]"),
        "Elixir Cargo.toml must not forward non-existent 'config' feature in [features]; content:\n{}",
        features_block
    );
    assert!(
        !features_block.contains("download = [\"my-lib/download\"]"),
        "Elixir Cargo.toml must not forward non-existent 'download' feature in [features]; content:\n{}",
        features_block
    );
    // Note: serde is a direct dependency, not a feature of the core crate, so it won't be in [features].

    assert!(
        features_block.contains("default = []"),
        "Elixir Cargo.toml must not enable missing legacy defaults; content:\n{}",
        features_block
    );
}
