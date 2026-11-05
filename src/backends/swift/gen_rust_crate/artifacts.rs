use super::*;
use std::path::{Path, PathBuf};

pub fn emit(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let deduped_api = api.with_deduped_functions();
    let api = &deduped_api;
    let package_root = package_root(config);
    let base = package_root.join("rust");
    let core_crate_dir = config.core_crate_for_language(Language::Swift);
    let features = feature_gate::configured_swift_features(config, &core_crate_dir, api);
    let excluded = excluded_items(api, config);
    let configured_features: HashSet<&str> = features.iter().map(String::as_str).collect();
    for typ in api.types.iter().filter(|typ| !excluded.types.contains(&typ.name)) {
        extern_block::validate_legacy_constructor_fields(typ, &excluded.fields, &configured_features)?;
    }
    let manifest = cargo_manifest(api, config, &features);
    let effective_features = feature_gate::effective_swift_codegen_features(api, config, &core_crate_dir);
    let configured_features: HashSet<&str> = effective_features.iter().map(String::as_str).collect();
    // Derive features before filtering methods, matching the Swift facade. ~keep
    let filtered_api = feature_gate::with_cfg_filtered_methods(api, &configured_features);
    let (lib_rs, owned_vec_accessors) = emit_lib_rs(
        &filtered_api,
        config,
        &api.crate_name,
        &excluded.functions,
        &excluded.types,
        &excluded.fields,
        &configured_features,
    )?;
    let mut files = base_files(&base, manifest, lib_rs);
    if let Some(callback_file) = callback_files::emit_callback_registration_file(&filtered_api, &base) {
        files.push(callback_file);
    }
    let sources = package_root.join("Sources").join("RustBridge");
    if let Some(inputs) = crate::backends::swift::owned_vector_inputs::swift_file(&owned_vec_accessors.inputs, &sources)
    {
        files.push(inputs);
    }
    if let Some(accessors) = owned_vec::emit_swift_file(&owned_vec_accessors, &sources) {
        files.push(accessors);
    }
    Ok(files)
}

fn package_root(config: &ResolvedCrateConfig) -> PathBuf {
    // The Rust crate and facade share the configured package root, including explicit overrides. ~keep
    let base_dir = resolve_output_dir(config.output_paths.get("swift"), &config.name, "packages/swift");
    swift_package_root(&base_dir, config.explicit_output.swift.is_some())
}

struct ExcludedItems {
    functions: HashSet<String>,
    types: HashSet<String>,
    fields: HashSet<String>,
}

fn excluded_items(api: &ApiSurface, config: &ResolvedCrateConfig) -> ExcludedItems {
    let mut functions: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();
    let mut types: HashSet<String> = config
        .swift
        .as_ref()
        .map(|c| c.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(ffi) = &config.ffi {
        functions.extend(ffi.exclude_functions.iter().cloned());
        types.extend(ffi.exclude_types.iter().cloned());
    }
    types.extend(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.clone()));
    types.extend(api.enums.iter().filter(|e| e.binding_excluded).map(|e| e.name.clone()));
    for contract in &api.handler_contracts {
        if let Some(adapter) = contract.response_adapter.as_deref()
            && let Some(short) = adapter.rsplit("::").next()
        {
            functions.insert(short.to_string());
        }
    }
    let fields = config
        .swift
        .as_ref()
        .map(|c| c.exclude_fields.iter().cloned().collect())
        .unwrap_or_default();
    ExcludedItems {
        functions,
        types,
        fields,
    }
}

struct CoreDependency {
    directory: String,
    key: String,
    path: String,
}

fn core_dependency(api: &ApiSurface, config: &ResolvedCrateConfig) -> CoreDependency {
    let directory = config.core_crate_for_language(Language::Swift);
    let override_name = config.swift.as_ref().and_then(|c| c.core_crate_override.as_deref());
    let same_as_workspace = override_name.is_none() && directory == api.crate_name && config.workspace_root.is_none();
    let path = if same_as_workspace {
        "../../..".to_string()
    } else {
        format!("../../../crates/{directory}")
    };
    let key = override_name
        .map(str::to_owned)
        .unwrap_or_else(|| api.crate_name.replace('-', "_"));
    CoreDependency { directory, key, path }
}

fn cargo_manifest(api: &ApiSurface, config: &ResolvedCrateConfig, features: &[String]) -> String {
    let core = core_dependency(api, config);
    let swift = config.swift.clone().unwrap_or_default();
    let license = config
        .scaffold
        .as_ref()
        .and_then(|s| s.license.as_deref())
        .unwrap_or("MIT");
    let streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming));
    let mut dependencies = crate::scaffold::render_extra_deps(config, Language::Swift);
    if !config.components.is_empty() {
        append_component_dependencies(&mut dependencies);
    }
    cargo::emit_cargo_toml(
        &api.crate_name,
        &core.key,
        &core.directory,
        &api.version,
        &crate::backends::swift::naming::swift_bridge_version(config),
        template_versions::cargo::SWIFT_BRIDGE_BUILD,
        &core.path,
        features,
        &dependencies,
        license,
        streaming,
        &swift.target_dep_overrides,
        api,
        &swift.excluded_default_features,
        &config.ffi_crate_package_name(),
        &config.ffi_crate_path_from_swift_rust(),
        &swift.ffi_features,
        &swift.ffi_target_dep_overrides,
        &package_metadata(config),
        &config.cargo_lints,
    )
}

fn package_metadata(config: &ResolvedCrateConfig) -> String {
    let filters = crate::scaffold::render_cargo_package_file_filters(
        config
            .package_metadata
            .as_ref()
            .and_then(|metadata| metadata.cargo.as_ref()),
    );
    let authors = crate::scaffold::scaffold_meta(config).authors;
    if authors.is_empty() {
        return filters;
    }
    let table = toml::Table::from_iter([(
        "authors".to_string(),
        toml::Value::Array(authors.into_iter().map(toml::Value::String).collect()),
    )]);
    let authors = toml::to_string(&table).expect("author metadata serializes to TOML");
    format!("{authors}{filters}")
}

fn base_files(base: &Path, manifest: String, source: String) -> Vec<GeneratedFile> {
    [
        ("Cargo.toml", manifest),
        ("src/lib.rs", source),
        ("build.rs", cargo::emit_build_rs()),
    ]
    .into_iter()
    .map(|(name, content)| GeneratedFile {
        path: base.join(name),
        content,
        generated_header: false,
    })
    .collect()
}

fn append_component_dependencies(extra_deps: &mut String) {
    let alef_version = env!("CARGO_PKG_VERSION");
    for (name, line) in [
        ("alef-component-abi", format!("alef-component-abi = \"{alef_version}\"")),
        (
            "alef-component-runtime",
            format!("alef-component-runtime = \"{alef_version}\""),
        ),
        ("directories", "directories = \"6\"".to_string()),
    ] {
        let configured = extra_deps
            .lines()
            .filter_map(|line| line.split_once('=').map(|(key, _)| key.trim()))
            .any(|key| key == name);
        if configured {
            continue;
        }
        if !extra_deps.is_empty() && !extra_deps.ends_with('\n') {
            extra_deps.push('\n');
        }
        extra_deps.push_str(&line);
    }
}

#[cfg(test)]
mod component_dependency_tests {
    use super::append_component_dependencies;

    #[test]
    fn configured_component_dependencies_are_not_duplicated() {
        let mut dependencies = "alef-component-runtime = { path = \"../../runtime\" }\ndirectories = \"5\"".to_string();
        append_component_dependencies(&mut dependencies);

        assert_eq!(dependencies.matches("alef-component-runtime =").count(), 1);
        assert_eq!(dependencies.matches("directories =").count(), 1);
        assert_eq!(dependencies.matches("alef-component-abi =").count(), 1);
    }
}
