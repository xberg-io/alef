use crate::backends::php::naming::php_autoload_namespace;
use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::{
    scaffold::cargo_package_header, scaffold::core_dep_features, scaffold::detect_workspace_inheritance,
    scaffold::render_extra_deps, scaffold::scaffold_meta,
};
use std::path::PathBuf;

pub(crate) fn scaffold_php_cargo(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-php"), version, "2024", &meta, &ws);

    let extra_deps = render_extra_deps(config, Language::Php);

    let has_trait_bridges = !config.trait_bridges.is_empty();
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    // ahash is needed when any function takes an AHashMap<Cow, _> param — the generated
    // PHP wrapper emits a `let __<name>_ahash: ahash::AHashMap<...>` pre-call binding.
    let needs_ahash = api.functions.iter().any(|f| f.params.iter().any(|p| p.map_is_ahash));
    let mut all_deps = extra_deps;
    if needs_ahash {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("ahash = \"0.8\"");
    }
    if has_trait_bridges && !all_deps.contains("async-trait") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("async-trait = \"0.1\"");
    }
    if has_streaming && !all_deps.contains("futures-util = ") && !all_deps.contains("futures-util =\"") {
        if !all_deps.is_empty() {
            all_deps.push('\n');
        }
        all_deps.push_str("futures-util = \"0.3\"");
    }

    let extra_deps_section = if all_deps.is_empty() {
        String::new()
    } else {
        format!("\n{all_deps}")
    };
    // Build the cargo-machete ignored list. `tokio` is added to the
    // dependency block unconditionally to keep the manifest layout stable
    // across generated PHP crates, but consumers whose PHP surface exposes
    // no async functions never reference it — list it as ignored. Same for
    // `async-trait`, which is added when the umbrella declares
    // `trait_bridges` but goes unreferenced when the resulting trait shim
    // does not use `#[async_trait]` after JSON-bridging. `ahash` is added when
    // any parameter uses AHashMap<Cow, _>, but the PHP wrapper never directly
    // uses ahash—it's used only in the Rust core for type field marshalling.
    let mut machete_ignored: Vec<&str> = vec!["tokio", "ahash"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
    }
    if has_streaming {
        machete_ignored.push("futures-util");
    }
    let machete_ignored_str = machete_ignored
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");
    // Build [dependencies] block alphabetically sorted to match cargo-sort.
    // Order: async-trait?, ext-php-rs, futures-util?, <core-crate>,
    // serde, serde_json, tokio.
    let core_overrides = config
        .php
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let (core_dep_php, core_target_blocks) = crate::scaffold::render_core_dep_with_overrides(
        &config.name,
        &format!("../{core_crate_dir}"),
        &core_dep_features(config, Language::Php),
        version,
        core_overrides,
    );
    let core_target_blocks_section = if core_target_blocks.is_empty() {
        String::new()
    } else {
        format!("\n{core_target_blocks}")
    };
    let mut dep_entries: Vec<String> = vec![
        format!("ext-php-rs = \"{}\"", tv::cargo::EXT_PHP_RS),
        "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
        "serde_json = \"1\"".to_string(),
        "tokio = { version = \"1\", features = [\"full\"] }".to_string(),
    ];
    if !core_dep_php.is_empty() {
        dep_entries.push(core_dep_php.clone());
    }
    if !all_deps.is_empty() {
        for line in all_deps.lines() {
            if !line.is_empty() {
                dep_entries.push(line.to_string());
            }
        }
    }
    dep_entries.sort();
    let dep_block = dep_entries.join("\n");
    let _ = extra_deps_section;

    // Collect every feature name referenced by a cfg attribute on any type, field,
    // enum variant, or function in the API surface and emit forwarding entries so
    // the binding crate can re-export them to the core dep. Without this,
    // `#[cfg(feature = "X")]` arms emitted by the codegen produce
    // `error: unexpected cfg condition value: X` and, when the cfg'd item is a
    // method inside a `#[php_impl]` block, a fatal E0599. Enable them all by
    // default so `#[cfg(feature = "X")]` arms compile unconditionally.
    let core_dep_name = &config.name; // e.g. "sample-core" — the Cargo dep key
    let cfg_forwarding: String = {
        let features = crate::codegen::cfg::collect_cfg_features(api);
        if features.is_empty() {
            String::new()
        } else {
            let mut lines: Vec<String> = Vec::with_capacity(features.len() + 1);
            let default_list: Vec<String> = features.iter().map(|name| format!("\"{name}\"")).collect();
            lines.push(format!("default = [{}]", default_list.join(", ")));
            for name in &features {
                lines.push(format!(r#"{name} = ["{core_dep_name}/{name}"]"#));
            }
            format!("{}\n", lines.join("\n"))
        }
    };

    let content = format!(
        r#"{pkg_header}

# `ahash` and `futures-util` are conditionally included but not directly used in PHP code.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[lib]
crate-type = ["cdylib"]

[features]
extension-module = []
{cfg_forwarding}
[dependencies]
{dep_block}
{core_target_blocks_section}
"#,
        pkg_header = pkg_header,
        dep_block = dep_block,
        core_target_blocks_section = core_target_blocks_section,
        machete_ignored_str = machete_ignored_str,
        cfg_forwarding = cfg_forwarding,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-php/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

pub(crate) fn scaffold_php(_api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let ext_name = config.php_extension_name();
    let pkg_dir = config.package_dir(Language::Php);
    // PSR-4 namespace derived from the extension name.
    // Double backslashes for JSON string literal output.
    let php_namespace = php_autoload_namespace(config).replace('\\', "\\\\");

    let keywords_json = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!(",\n  \"keywords\": [{}]", entries.join(", "))
    };

    let (vendor, package_name) = if let Some(pkg) = config.php.as_ref().and_then(|p| p.composer_package.as_ref()) {
        let parts: Vec<&str> = pkg.split('/').collect();
        match parts.as_slice() {
            [v, p] => (v.to_string(), p.to_string()),
            _ => composer_package_name(config, &meta),
        }
    } else {
        composer_package_name(config, &meta)
    };

    // Composer manifests are emitted twice with one structural difference: the
    // PSR-4 autoload src path. The package manifest at `{pkg_dir}/composer.json`
    // is the dev manifest used by phpstan/phpunit/php-cs-fixer inside the
    // package directory and points at `src/`. The root manifest at
    // `composer.json` is the one Packagist indexes and PIE installs read — it
    // must point at `{pkg_dir}/src/` so the same PSR-4 classes resolve from
    // the repo root. Everything else (name, php-ext block, require/require-dev,
    // scripts) is byte-identical so the two manifests stay in sync without drift.
    let render_composer = |autoload_src: &str| -> String {
        let license_json = meta
            .license
            .as_deref()
            .map(|license| format!("  \"license\": \"{license}\",\n"))
            .unwrap_or_default();

        // PIE (PHP Installation Extension) uses extra.pie.binary.url-template to locate
        // pre-packaged extension archives. Template tokens: {Version}, {PhpVersion}, {Arch},
        // {OS}, {Libc}, {TSMode} (zts or nts). When no repository is configured, emit a
        // placeholder; the URL is typically overridden by the package maintainer.
        // PIE 1.4+ substitutes {Version} with `$package->version()` from Composer, which
        // preserves the leading `v` from the source tag (e.g. `v0.3.0-rc.45`). PIE's {OS}
        // placeholder resolves to PHP's PHP_OS_FAMILY (uppercase: Linux, Darwin, Windows),
        // but published release assets use lowercase (linux, darwin). Use the {OSLower}
        // placeholder (PIE 1.5+) to match actual GitHub Release asset names.
        let pie_binary_block = if let Some(repo_url) = meta.configured_repository.as_deref() {
            format!(
                ",\n  \"extra\": {{\n    \"pie\": {{\n      \"binary\": {{\n        \"url-template\": \"{repo_url}/releases/download/{{Version}}/php_{ext_name}-{{Version}}_php{{PhpVersion}}-{{Arch}}-{{OSLower}}-{{Libc}}-{{TSMode}}.tgz\"\n      }}\n    }}\n  }}"
            )
        } else {
            String::new()
        };

        format!(
            r#"{{
  "name": "{vendor}/{package_name}",
  "description": "{description}",
{license_json}  "type": "php-ext",
  "require": {{
    "php": ">=8.2"
  }},
  "require-dev": {{
    "phpunit/phpunit": "{phpunit}"
  }},
  "autoload": {{
    "psr-4": {{
      "{php_namespace}\\": "{autoload_src}"
    }}
  }},
  "scripts": {{
    "format": "poly fmt --fix",
    "format:check": "poly fmt --check",
    "test": "php vendor/bin/phpunit",
    "lint": "poly lint",
    "lint:fix": "poly lint --fix && poly fmt --fix"
  }},
  "php-ext": {{
    "extension-name": "{ext_name}",
    "support-zts": true,
    "support-nts": true,
    "download-url-method": "pre-packaged-binary"
  }}{keywords}{pie_binary}
}}
"#,
            vendor = vendor,
            package_name = package_name,
            description = meta.description,
            license_json = license_json,
            php_namespace = php_namespace,
            autoload_src = autoload_src,
            ext_name = ext_name,
            keywords = keywords_json,
            pie_binary = pie_binary_block,
            phpunit = tv::packagist::PHPUNIT,
        )
    };

    let content = render_composer("src/");
    let root_content = render_composer("packages/php/src/");

    // PHP linting + formatting are poly-native via mago (no PHP runtime): no
    // phpstan.neon / phpstan-baseline.neon / .php-cs-fixer.dist.php is emitted.
    // The mago ruleset lives in the repo-root poly.toml ([lint.php.mago]).
    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/composer.json")),
            content,
            generated_header: false,
        },
        // Root composer.json is the Packagist/PIE manifest. Packagist indexes
        // it from the repo root, so the autoload path must point at the package
        // source directory.
        GeneratedFile {
            path: PathBuf::from("composer.json"),
            content: root_content,
            generated_header: false,
        },
    ])
}

fn composer_package_name(config: &ResolvedCrateConfig, meta: &crate::scaffold::ScaffoldMeta) -> (String, String) {
    let Some(repository) = meta.configured_repository.as_deref() else {
        return ("unconfigured".to_string(), config.name.to_lowercase());
    };
    let repo = repository
        .strip_prefix("https://github.com/")
        .or_else(|| repository.strip_prefix("http://github.com/"))
        .filter(|s| !s.is_empty())
        .unwrap_or(repository);

    let parts: Vec<&str> = repo.split('/').collect();
    match parts.as_slice() {
        [owner, repo_name, ..] => (owner.to_lowercase(), repo_name.to_lowercase()),
        _ => ("unconfigured".to_string(), config.name.to_lowercase()),
    }
}
