use super::*;

fn config_with_extra_deps() -> ResolvedCrateConfig {
    let mut config = test_config();
    config
        .extra_dependencies
        .insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));
    config.extra_dependencies.insert(
        "tracing".to_string(),
        toml::Value::Table({
            let mut t = toml::map::Map::new();
            t.insert("version".to_string(), toml::Value::String("0.1".to_string()));
            t.insert(
                "features".to_string(),
                toml::Value::Array(vec![toml::Value::String("log".to_string())]),
            );
            t
        }),
    );
    config
}

#[test]
fn test_render_extra_deps_empty() {
    let config = test_config();
    assert_eq!(render_extra_deps(&config, Language::Python), "");
}

#[test]
fn test_render_extra_deps_string_version() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    assert!(rendered.contains("anyhow = \"1.0\""), "got: {rendered}");
}

#[test]
fn test_render_extra_deps_table_value() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    assert!(rendered.contains("tracing = "), "got: {rendered}");
    assert!(rendered.contains("\"log\""), "got: {rendered}");
}

#[test]
fn test_render_extra_deps_sorted() {
    let config = config_with_extra_deps();
    let rendered = render_extra_deps(&config, Language::Python);
    let anyhow_pos = rendered.find("anyhow").expect("anyhow missing");
    let tracing_pos = rendered.find("tracing").expect("tracing missing");
    assert!(anyhow_pos < tracing_pos, "deps should be sorted alphabetically");
}

#[test]
fn test_scaffold_python_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
    assert!(
        cargo_toml.content.contains("tracing"),
        "content: {}",
        cargo_toml.content
    );
    // Extra deps should appear inside the [dependencies] section (which follows [features]).
    let deps_pos = cargo_toml.content.find("[dependencies]").unwrap();
    let anyhow_pos = cargo_toml.content.find("anyhow").unwrap();
    assert!(anyhow_pos > deps_pos, "anyhow should appear inside [dependencies]");
}

#[test]
fn test_scaffold_node_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_ruby_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_php_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Php]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_elixir_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_r_cargo_extra_deps() {
    let config = config_with_extra_deps();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files
        .iter()
        .find(|f| f.path.ends_with("packages/r/src/rust/Cargo.toml"))
        .unwrap();
    assert!(
        cargo_toml.content.contains("anyhow = \"1.0\""),
        "content: {}",
        cargo_toml.content
    );
}

#[test]
fn test_scaffold_language_level_extra_deps_override_crate_level() {
    let mut config = test_config();
    // Crate-level dep with version "1.0"
    config
        .extra_dependencies
        .insert("shared-dep".to_string(), toml::Value::String("1.0".to_string()));
    // Python-level override with a different version; inject via extra_deps_for_language
    // by inserting directly into a Python extra_dependencies map.
    let mut python_extra: std::collections::HashMap<String, toml::Value> = std::collections::HashMap::new();
    python_extra.insert("shared-dep".to_string(), toml::Value::String("2.0".to_string()));
    config.python = Some(PythonConfig {
        module_name: None,
        async_runtime: None,
        stubs: None,
        pip_name: None,
        features: None,
        serde_rename_all: None,
        capsule_types: std::collections::HashMap::new(),
        release_gil: false,
        exclude_functions: vec![],
        exclude_types: vec![],
        extra_dependencies: python_extra,
        pip_dependencies: Vec::new(),
        sdist_include: Vec::new(),
        scaffold_output: None,
        rename_fields: Default::default(),
        run_wrapper: None,
        extra_lint_paths: Vec::new(),
        extra_init_imports: std::collections::BTreeMap::new(),
        reexported_types: Vec::new(),
        target_dep_overrides: Vec::new(),
    });
    let rendered = render_extra_deps(&config, Language::Python);
    // Python-level "2.0" should win over crate-level "1.0"
    assert!(rendered.contains("shared-dep = \"2.0\""), "got: {rendered}");
    assert!(
        !rendered.contains("1.0"),
        "crate-level version should be overridden, got: {rendered}"
    );
}

/// Helper: extract the [dependencies] key order from a Cargo.toml string.
///
/// Returns the dependency keys in the order they appear, so tests can assert
/// that the emitted file is already cargo-sort canonical (alphabetical order).
fn dep_keys_in_order(cargo_toml: &str) -> Vec<&str> {
    let mut in_deps = false;
    let mut keys = Vec::new();
    for line in cargo_toml.lines() {
        if line.trim_start().starts_with('[') {
            in_deps = line.trim() == "[dependencies]";
            continue;
        }
        if in_deps {
            if let Some(key) = line.split('=').next() {
                let key = key.trim();
                if !key.is_empty() && !key.starts_with('#') {
                    keys.push(key);
                }
            }
        }
    }
    keys
}

#[test]
fn test_scaffold_elixir_cargo_deps_are_alphabetically_sorted() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.languages = vec![Language::Elixir];
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
        bind_via: crate::core::config::BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    // With a trait bridge, async-trait and tokio must be present.
    assert!(
        keys.contains(&"async-trait"),
        "async-trait must appear when trait bridges are configured; keys: {keys:?}"
    );
    assert!(
        keys.contains(&"tokio"),
        "tokio must appear when trait bridges are configured; keys: {keys:?}"
    );
    // All keys must be in sorted order.
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "elixir Cargo.toml [dependencies] must be alphabetically sorted; got: {keys:?}"
    );
}

#[test]
fn test_scaffold_ruby_cargo_deps_are_alphabetically_sorted() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.languages = vec![Language::Ruby];
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
        bind_via: crate::core::config::BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    // With a trait bridge, async-trait and tokio must be present.
    assert!(
        keys.contains(&"async-trait"),
        "async-trait must appear when trait bridges are configured; keys: {keys:?}"
    );
    assert!(
        keys.contains(&"tokio"),
        "tokio must appear when trait bridges are configured; keys: {keys:?}"
    );
    // A synchronous trait bridge emits async-trait and tokio but the generated NIF
    // never imports them, so both must join rb-sys in the cargo-machete ignored
    // list — otherwise cargo-machete fails `poly lint` downstream.
    let ignored_line = cargo_toml
        .content
        .lines()
        .find(|l| l.trim_start().starts_with("ignored ="))
        .expect("ruby Cargo.toml must have a cargo-machete ignored line");
    for dep in ["async-trait", "rb-sys", "tokio"] {
        assert!(
            ignored_line.contains(dep),
            "cargo-machete ignored list must contain {dep}; got: {ignored_line}"
        );
    }
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "ruby Cargo.toml [dependencies] must be alphabetically sorted; got: {keys:?}"
    );
}

#[test]
fn test_scaffold_r_cargo_deps_are_alphabetically_sorted() {
    use crate::core::config::TraitBridgeConfig;

    let mut config = test_config();
    config.languages = vec![Language::R];
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
        bind_via: crate::core::config::BridgeBinding::OptionsField,
        options_type: Some("ConversionOptions".to_string()),
        options_field: None,
        context_type: None,
        result_type: None,
    }];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    // With a trait bridge, async-trait must be present.
    assert!(
        keys.contains(&"async-trait"),
        "async-trait must appear when trait bridges are configured; keys: {keys:?}"
    );
    // async-trait is declared for the async impl macro but the extendr shim never
    // imports it, so the R crate must emit a cargo-machete ignore stanza for it.
    let ignored_line = cargo_toml
        .content
        .lines()
        .find(|l| l.trim_start().starts_with("ignored ="))
        .expect("R Cargo.toml must have a cargo-machete ignored line when trait bridges are configured");
    assert!(
        ignored_line.contains("async-trait"),
        "R cargo-machete ignored list must contain async-trait; got: {ignored_line}"
    );
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "r Cargo.toml [dependencies] must be alphabetically sorted; got: {keys:?}"
    );
}

#[test]
fn test_scaffold_elixir_cargo_deps_sorted_no_trait_bridges() {
    // Even without trait bridges, the basic deps must be in sorted order.
    let mut config = test_config();
    config.languages = vec![Language::Elixir];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "elixir Cargo.toml [dependencies] must be alphabetically sorted (sync-only); got: {keys:?}"
    );
}

#[test]
fn test_scaffold_r_cargo_deps_sorted_no_trait_bridges() {
    // Without trait bridges, the basic R deps must still be in sorted order.
    let mut config = test_config();
    config.languages = vec![Language::R];
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::R]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let keys = dep_keys_in_order(&cargo_toml.content);
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "r Cargo.toml [dependencies] must be alphabetically sorted (no trait bridges); got: {keys:?}"
    );
}

/// Helper: extract TOML section headers in the order they appear, skipping
/// inline sub-tables (lines that don't start with `[`).
fn section_headers_in_order(cargo_toml: &str) -> Vec<&str> {
    cargo_toml
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.starts_with('[') && !t.starts_with("[[") {
                Some(t)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn test_scaffold_elixir_cargo_section_order_is_cargo_sort_canonical() {
    // cargo-sort canonical order for a NIF crate: [package] → [workspace] → [lib] → [dependencies]
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Elixir]).unwrap();
    let files = language_files(&all_files);
    let cargo_toml = files.iter().find(|f| f.path.ends_with("Cargo.toml")).unwrap();

    let headers = section_headers_in_order(&cargo_toml.content);
    // [workspace] must appear before [lib], which must appear before [dependencies].
    let workspace_pos = headers.iter().position(|h| *h == "[workspace]");
    let lib_pos = headers.iter().position(|h| *h == "[lib]");
    let deps_pos = headers.iter().position(|h| *h == "[dependencies]");

    assert!(
        workspace_pos.is_some(),
        "Elixir NIF Cargo.toml must contain a [workspace] section; headers: {headers:?}"
    );
    assert!(
        lib_pos.is_some(),
        "Elixir NIF Cargo.toml must contain a [lib] section; headers: {headers:?}"
    );
    assert!(
        deps_pos.is_some(),
        "Elixir NIF Cargo.toml must contain a [dependencies] section; headers: {headers:?}"
    );

    assert!(
        workspace_pos < lib_pos,
        "[workspace] must come before [lib] (cargo-sort canonical); headers: {headers:?}"
    );
    assert!(
        lib_pos < deps_pos,
        "[lib] must come before [dependencies] (cargo-sort canonical); headers: {headers:?}"
    );
}

// ---- LICENSE sync tests -----------------------------------------------

/// When a LICENSE file exists at the workspace root, alef must copy it into
/// every per-language package directory so ecosystems like pub.dev that require
/// a LICENSE can publish successfully.

#[test]
fn test_render_extra_deps_injects_version_for_workspace_member() {
    use std::fs;
    use tempfile::TempDir;

    // Build a real temp workspace whose root Cargo.toml declares a member
    // `my-lib-http` at version 2.5.0 (via workspace inheritance).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib-http"]

[workspace.package]
version = "2.5.0"
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/my-lib-http/src")).unwrap();
    fs::write(root.join("crates/my-lib-http/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/my-lib-http/Cargo.toml"),
        "[package]\nname = \"my-lib-http\"\nversion.workspace = true\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // A path-only workspace-member dep + a non-member external dep.
    config.extra_dependencies.insert(
        "my-lib-http".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-http".to_string()),
        )])),
    );
    config
        .extra_dependencies
        .insert("anyhow".to_string(), toml::Value::String("1.0".to_string()));

    let rendered = render_extra_deps(&config, Language::Python);
    // Member: version injected (toml inline tables sort keys: path before version).
    assert!(
        rendered.contains(r#"my-lib-http = { path = "../my-lib-http", version = "2.5.0" }"#),
        "workspace member should get the resolved workspace version injected; got:\n{rendered}"
    );
    // Non-member external dep unchanged.
    assert!(
        rendered.contains(r#"anyhow = "1.0""#),
        "non-member external dep must be emitted unchanged; got:\n{rendered}"
    );
}

#[test]
fn test_render_extra_deps_leaves_non_member_path_dep_unchanged() {
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = []\n[workspace.package]\nversion = \"2.5.0\"\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // `vendored-thing` is NOT a workspace member, so its path-only table must
    // stay path-only (no version injection).
    config.extra_dependencies.insert(
        "vendored-thing".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "path".to_string(),
            toml::Value::String("../../vendor/thing".to_string()),
        )])),
    );

    let rendered = render_extra_deps(&config, Language::Python);
    assert!(
        rendered.contains(r#"vendored-thing = { path = "../../vendor/thing" }"#),
        "non-member path dep must remain path-only; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("version ="),
        "no version should be injected for a non-member; got:\n{rendered}"
    );
}

#[test]
fn test_render_extra_deps_does_not_double_inject_version() {
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/my-lib-http\"]\n[workspace.package]\nversion = \"2.5.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/my-lib-http/src")).unwrap();
    fs::write(root.join("crates/my-lib-http/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/my-lib-http/Cargo.toml"),
        "[package]\nname = \"my-lib-http\"\nversion = \"9.9.9\"\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // Already carries an explicit version — must be left untouched.
    config.extra_dependencies.insert(
        "my-lib-http".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([
            ("path".to_string(), toml::Value::String("../my-lib-http".to_string())),
            ("version".to_string(), toml::Value::String("1.0".to_string())),
        ])),
    );

    let rendered = render_extra_deps(&config, Language::Python);
    assert!(
        rendered.contains(r#"version = "1.0""#),
        "pre-existing version must be preserved; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("9.9.9") && !rendered.contains("2.5.0"),
        "must not overwrite or append a second version; got:\n{rendered}"
    );
}

#[test]
fn test_render_extra_deps_swift_injects_version_for_workspace_member() {
    use std::fs;
    use tempfile::TempDir;

    // Build a real temp workspace whose root Cargo.toml declares a member
    // `my-lib-http` at version 3.1.0 (via workspace inheritance).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"
[workspace]
resolver = "2"
members = ["crates/my-lib-http"]

[workspace.package]
version = "3.1.0"
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/my-lib-http/src")).unwrap();
    fs::write(root.join("crates/my-lib-http/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/my-lib-http/Cargo.toml"),
        "[package]\nname = \"my-lib-http\"\nversion.workspace = true\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(root.to_path_buf());
    // Path-only workspace-member dep configured under swift extra_dependencies.
    config.extra_dependencies.insert(
        "my-lib-http".to_string(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "path".to_string(),
            toml::Value::String("../my-lib-http".to_string()),
        )])),
    );

    // Calling with Language::Swift exercises the same code path as the swift
    // gen_rust_crate backend, which now delegates to this shared function.
    let rendered = render_extra_deps(&config, Language::Swift);
    assert!(
        rendered.contains(r#"version = "3.1.0""#),
        "swift backend: workspace member must get version injected; got:\n{rendered}"
    );
    assert!(
        rendered.contains(r#"path = "../my-lib-http""#),
        "swift backend: path must be preserved alongside injected version; got:\n{rendered}"
    );
}
