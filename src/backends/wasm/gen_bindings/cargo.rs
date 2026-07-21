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

    let keywords = scaffold.map(|s| s.keywords.as_slice()).unwrap_or(&[]);
    let keywords_toml = if keywords.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = keywords.iter().map(|k| format!("\"{k}\"")).collect();
        format!("keywords = [{}]\n", quoted.join(", "))
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
    extra_dep_lines.sort();
    let extra_deps_section = if extra_dep_lines.is_empty() {
        String::new()
    } else {
        format!("\n{}", extra_dep_lines.join("\n"))
    };

    // `#[cfg(feature = X)]` on the binding crate intentionally evaluate false
    let _ = features;
    let cfg_features = collect_cfg_features(api);
    let features_table = if cfg_features.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = cfg_features
            .iter()
            .map(|name| format!(r#"{name} = ["{core_dep_key}/{name}"]"#))
            .collect();
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

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

    let header = hash::header(CommentStyle::Hash);

    let mut deps: Vec<(String, String)> = vec![
        (
            core_dep_key.clone(),
            format!(r#"{{ path = "../{core_crate_dir}"{features_clause} }}"#),
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
    deps.sort_by(|a, b| a.0.cmp(&b.0));
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
    dev_dep_lines.sort();
    let dev_deps_section = if dev_dep_lines.is_empty() {
        String::new()
    } else {
        format!("\n[dev-dependencies]\n{}\n", dev_dep_lines.join("\n"))
    };

    format!(
        r#"{header}
[package]
name = "{pkg_prefix}-wasm"
version = "{version}"
edition = "2024"
license = "{license}"
description = "{description}"
repository = "{repository}"
{keywords_toml}
[package.metadata.cargo-machete]
ignored = [
    "futures",
    "futures-util",
    "js-sys",
    "wasm-bindgen-futures",
    "serde",
    "serde_json",
]

[package.metadata.wasm-pack.profile.release]
{wasm_opt_line}

[lib]
crate-type = ["cdylib"]

{features_table}[dependencies]
{deps_block}
{dev_deps_section}
[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = {{ version = "0.4", features = ["wasm_js"] }}
getrandom_02 = {{ package = "getrandom", version = "0.2", features = ["js"] }}
getrandom_03 = {{ package = "getrandom", version = "0.3", features = ["wasm_js"] }}
"#,
        header = header,
        pkg_prefix = pkg_prefix,
        version = version,
        license = license,
        description = description,
        repository = repository,
        keywords_toml = keywords_toml,
        wasm_opt_line = wasm_opt_line,
        deps_block = deps_block,
        dev_deps_section = dev_deps_section,
        features_table = features_table,
    )
}
