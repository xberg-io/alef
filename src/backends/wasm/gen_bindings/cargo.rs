use super::collect_cfg_features;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use ahash::AHashSet;

/// Generate the `Cargo.toml` for the WASM binding crate.
///
/// This is emitted by [`WasmBackend::generate_bindings`] so that the file is
/// always regenerated on `alef generate` / `alef all` alongside `lib.rs`.
/// Emitting it here (rather than only in `alef-scaffold`) ensures that the
/// `js-sys` dependency required by trait-bridge and visitor-bridge generated
/// code is always present, even in projects whose `Cargo.toml` was created
/// before `js-sys` was added to the scaffold template.
pub(super) fn gen_cargo_toml(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_crate_dir = config.core_crate_for_language(Language::Wasm);
    let crate_name = &config.name;
    let pkg_prefix: String = if config
        .wasm
        .as_ref()
        .and_then(|c| c.core_crate_override.as_deref())
        .is_some()
    {
        crate_name.clone()
    } else {
        core_crate_dir.clone()
    };
    let core_dep_key: String = config
        .wasm
        .as_ref()
        .and_then(|c| c.core_crate_override.clone())
        .unwrap_or_else(|| crate_name.clone());
    let version = &api.version;

    let scaffold = config.scaffold.as_ref();
    let license = scaffold.and_then(|s| s.license.as_deref()).unwrap_or("MIT");
    let description = scaffold
        .and_then(|s| s.description.as_deref())
        .unwrap_or(crate_name.as_str());
    let repository = scaffold.and_then(|s| s.repository.as_deref()).unwrap_or("");
    let authors = crate::scaffold::scaffold_meta(config).authors;
    let authors_line = if authors.is_empty() {
        String::new()
    } else {
        let table = toml::Table::from_iter([(
            "authors".to_string(),
            toml::Value::Array(authors.into_iter().map(toml::Value::String).collect()),
        )]);
        toml::to_string(&table).expect("author metadata serializes to TOML")
    };

    let keywords = scaffold.map(|s| s.keywords.as_slice()).unwrap_or(&[]);
    let keywords_toml = if keywords.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = keywords.iter().map(|k| format!("\"{k}\"")).collect();
        format!("keywords = [{}]\n", quoted.join(", "))
    };
    let package_file_filters = crate::scaffold::render_cargo_package_file_filters(
        config
            .package_metadata
            .as_ref()
            .and_then(|metadata| metadata.cargo.as_ref()),
    );
    let package_file_filters_line = if package_file_filters.is_empty() {
        String::new()
    } else {
        format!("{package_file_filters}\n")
    };

    let features = config.features_for_language(Language::Wasm);
    let features_clause = if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", default-features = false, features = [{}]", quoted.join(", "))
    };

    let extra_deps = config.extra_deps_for_language(Language::Wasm);
    let mut extra_dep_lines: Vec<String> = extra_deps
        .iter()
        .map(|(name, value)| {
            if let Some(s) = value.as_str() {
                format!("{name} = \"{s}\"")
            } else {
                format!("{name} = {value}")
            }
        })
        .collect();
    crate::scaffold::sort_dependency_lines(&mut extra_dep_lines);
    let extra_deps_section = if extra_dep_lines.is_empty() {
        String::new()
    } else {
        format!("\n{}", extra_dep_lines.join("\n"))
    };

    let mut declared_features = collect_cfg_features(api);
    // A configured component's features gate implementation types that the downloaded producer
    // cdylib compiles, not anything this binding wraps. Forwarding them would name a feature
    // this (possibly core-overridden) crate never declares. See
    // `native_wrapper_default_features_for_config`. ~keep
    let component_features = crate::codegen::cfg::component_core_features(config);
    declared_features.retain(|name| !component_features.contains(name));
    if let Some(wasm) = config.wasm.as_ref() {
        declared_features.extend(wasm.extra_features.iter().filter(|name| !name.is_empty()).cloned());
    }
    let features_table = if declared_features.is_empty() {
        String::new()
    } else {
        // Matched against the *expanded* configured list, not the literal one. A configured
        // aggregate (`wasm-target = ["decoder", ...]` in the core crate) is not itself one of
        // the cfg-referenced names in `declared_features`, so a literal intersection yields
        // `default = []` — every passthrough row declared and none enabled, which compiles the
        // gated items out of the wasm crate even though the core dep line turns their core-side
        // features on. The codegen filter in `mod.rs` expands the same list, so both derivations
        // must read from the expansion or the manifest and the emitted source disagree. ~keep
        let expanded_features = crate::codegen::cfg::enabled_features_for_language(config, Language::Wasm);
        let enabled_binding_features: Vec<&str> = expanded_features
            .iter()
            .map(String::as_str)
            .filter(|name| declared_features.contains(*name))
            .collect();
        let mut lines: Vec<String> = Vec::new();
        if !enabled_binding_features.is_empty() {
            let defaults = enabled_binding_features
                .iter()
                .map(|name| format!(r#""{name}""#))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("default = [{defaults}]"));
        }
        lines.extend(
            declared_features
                .iter()
                .map(|name| format!(r#"{name} = ["{core_dep_key}/{name}"]"#)),
        );
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    // `[package.metadata.wasm-pack.profile.release] wasm-opt`: emit the configured ~keep
    // pass args (e.g. `["-Oz"]`) when set, else `false` (wasm-pack skips wasm-opt). ~keep
    let wasm_opt_line = config
        .wasm
        .as_ref()
        .map(|c| c.wasm_opt.as_slice())
        .filter(|args| !args.is_empty())
        .map(|args| {
            let quoted: Vec<String> = args.iter().map(|a| format!("\"{a}\"")).collect();
            format!("wasm-opt = [{}]", quoted.join(", "))
        })
        .unwrap_or_else(|| "wasm-opt = false".to_string());

    // Derived from the same layout that decides where this manifest is written, so the depth of
    // `..` always matches the emitted tree instead of assuming a `crates/` sibling pair.
    // Override-aware: when `[wasm].core_crate_override` is set, `core_crate_root()` (built from
    // `sources`) names the wrong crate entirely -- the override targets an unrelated sibling
    // crate `sources` never describes. ~keep
    let core_dep_path =
        config.core_crate_dep_path_for_language(&super::wasm_output_layout(config).root, Language::Wasm);

    let header = hash::header(CommentStyle::Hash);

    let has_trait_bridges = config.trait_bridges_for(Language::Wasm).next().is_some();

    let mut deps: Vec<(String, String)> = vec![
        (
            core_dep_key.clone(),
            format!(r#"{{ path = "{core_dep_path}"{features_clause} }}"#),
        ),
        ("futures".to_string(), format!(r#""{}""#, tv::cargo::FUTURES)),
        ("futures-util".to_string(), format!(r#""{}""#, tv::cargo::FUTURES_UTIL)),
        ("js-sys".to_string(), format!(r#""{}""#, tv::cargo::JS_SYS)),
        (
            "serde".to_string(),
            r#"{ version = "1", features = ["derive"] }"#.to_string(),
        ),
        (
            "serde-wasm-bindgen".to_string(),
            format!(r#""{}""#, tv::cargo::SERDE_WASM_BINDGEN),
        ),
        ("serde_json".to_string(), r#""1""#.to_string()),
        ("wasm-bindgen".to_string(), format!(r#""{}""#, tv::cargo::WASM_BINDGEN)),
        (
            "wasm-bindgen-futures".to_string(),
            format!(r#""{}""#, tv::cargo::WASM_BINDGEN_FUTURES),
        ),
    ];
    if has_trait_bridges {
        deps.push(("tracing".to_string(), format!(r#""{}""#, tv::cargo::TRACING)));
    }
    let mut extra_parsed: Vec<(String, String)> = Vec::new();
    for line in extra_deps_section.lines() {
        let trimmed = line.trim();
        if let Some((name, value)) = trimmed.split_once('=') {
            extra_parsed.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    let extra_names: AHashSet<&str> = extra_parsed.iter().map(|(name, _)| name.as_str()).collect();
    deps.retain(|(name, _)| !extra_names.contains(name.as_str()));
    deps.extend(extra_parsed);
    deps.sort_by_key(|a| crate::scaffold::dependency_sort_key(&a.0));
    let deps_block = deps
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect::<Vec<_>>()
        .join("\n");

    // Hand-written test files in the binding crate (e.g. `#[wasm_bindgen_test]` ~keep
    // suites) need test-only dependencies the generated manifest must carry. ~keep
    let mut dev_dep_lines: Vec<String> = config
        .wasm
        .as_ref()
        .map(|c| c.extra_dev_dependencies.iter().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|(name, value)| {
            if let Some(v) = value.as_str() {
                format!("{name} = \"{v}\"")
            } else {
                format!("{name} = {value}")
            }
        })
        .collect();
    crate::scaffold::sort_dependency_lines(&mut dev_dep_lines);
    let dev_deps_section = if dev_dep_lines.is_empty() {
        String::new()
    } else {
        format!("\n[dev-dependencies]\n{}\n", dev_dep_lines.join("\n"))
    };
    let tracing_ignored_line = if has_trait_bridges { "    \"tracing\",\n" } else { "" };

    // Same glue contract as the scaffold emitters' `cargo_lints_section`: appended at the
    // very end of the manifest (after `[dev-dependencies]`) because cargo-sort sorts every
    // table absent from its `DEF_TABLE_ORDER` — `lints` among them — after the listed ones.
    // The preceding template text always ends with a newline, so "\n" + block + "\n" yields
    // one blank separator line and the file's trailing newline. ~keep
    let lints_block = config.cargo_lints.render();
    let lints_section = if lints_block.is_empty() {
        String::new()
    } else {
        format!("\n{lints_block}\n")
    };

    format!(
        r#"{header}
[package]
name = "{pkg_prefix}-wasm"
version = "{version}"
edition = "2024"
license = "{license}"
{authors_line}description = "{description}"
repository = "{repository}"
{keywords_toml}
{package_file_filters_line}
[package.metadata.cargo-machete]
ignored = [
    "futures",
    "futures-util",
    "js-sys",
    "wasm-bindgen-futures",
    "serde",
    "serde-wasm-bindgen",
    "serde_json",
{tracing_ignored_line}]

[package.metadata.wasm-pack.profile.release]
{wasm_opt_line}

[lib]
crate-type = ["cdylib"]

{features_table}[dependencies]
{deps_block}

[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = {{ version = "0.4", features = ["wasm_js"] }}
getrandom_02 = {{ package = "getrandom", version = "0.2", features = ["js"] }}
getrandom_03 = {{ package = "getrandom", version = "0.3", features = ["wasm_js"] }}
{dev_deps_section}{lints_section}"#,
        header = header,
        pkg_prefix = pkg_prefix,
        version = version,
        license = license,
        authors_line = authors_line,
        description = description,
        repository = repository,
        keywords_toml = keywords_toml,
        package_file_filters_line = package_file_filters_line,
        lints_section = lints_section,
        wasm_opt_line = wasm_opt_line,
        deps_block = deps_block,
        dev_deps_section = dev_deps_section,
        features_table = features_table,
        tracing_ignored_line = tracing_ignored_line,
    )
}
