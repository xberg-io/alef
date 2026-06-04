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
    let core_dep_php = crate::scaffold::render_core_dep(
        &config.name,
        &format!("../{core_crate_dir}"),
        &core_dep_features(config, Language::Php),
        version,
    );
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

    let content = format!(
        r#"{pkg_header}

# `ahash` and `futures-util` are conditionally included but not directly used in PHP code.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[lib]
crate-type = ["cdylib"]

[features]
extension-module = []

[dependencies]
{dep_block}

"#,
        pkg_header = pkg_header,
        dep_block = dep_block,
        machete_ignored_str = machete_ignored_str,
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
    //
    // Note: the former `extra.pie.binary.url-template` block is intentionally
    // omitted. PIE does not read `extra.pie.binary`; it only keys off
    // `extra.php-ext.download-url-method`. Keeping the dead block around misleads
    // maintainers into thinking it does something.
    let render_composer = |autoload_src: &str| -> String {
        format!(
            r#"{{
  "name": "{vendor}/{package_name}",
  "description": "{description}",
  "license": "{license}",
  "type": "php-ext",
  "require": {{
    "php": ">=8.2"
  }},
  "require-dev": {{
    "phpstan/phpstan": "{phpstan}",
    "friendsofphp/php-cs-fixer": "{php_cs_fixer}",
    "phpunit/phpunit": "{phpunit}"
  }},
  "autoload": {{
    "psr-4": {{
      "{php_namespace}\\": "{autoload_src}"
    }}
  }},
  "scripts": {{
    "phpstan": "php -d detect_unicode=0 vendor/bin/phpstan --configuration=phpstan.neon --memory-limit=512M",
    "format": "php vendor/bin/php-cs-fixer fix --quiet",
    "format:check": "php vendor/bin/php-cs-fixer fix --dry-run --quiet",
    "test": "php vendor/bin/phpunit",
    "lint": "@phpstan",
    "lint:fix": "php vendor/bin/php-cs-fixer fix --quiet && php -d detect_unicode=0 vendor/bin/phpstan --configuration=phpstan.neon --memory-limit=512M"
  }},
  "php-ext": {{
    "extension-name": "{ext_name}",
    "support-zts": true,
    "support-nts": true,
    "download-url-method": "pre-packaged-binary"
  }}{keywords}
}}
"#,
            vendor = vendor,
            package_name = package_name,
            description = meta.description,
            license = meta.license,
            php_namespace = php_namespace,
            autoload_src = autoload_src,
            ext_name = ext_name,
            keywords = keywords_json,
            phpstan = tv::packagist::PHPSTAN,
            php_cs_fixer = tv::packagist::PHP_CS_FIXER,
            phpunit = tv::packagist::PHPUNIT,
        )
    };

    let content = render_composer("src/");
    let root_content = render_composer("packages/php/src/");

    let stubs_file = format!("stubs/{ext_name}_extension.php");

    let phpstan_content = format!(
        "includes:\n\
         \x20   - phpstan-baseline.neon\n\
         \n\
         parameters:\n\
         \x20   level: max\n\
         \x20   paths:\n\
         \x20       - src\n\
         \x20   scanFiles:\n\
         \x20       - {stubs_file}\n\
         \x20   treatPhpDocTypesAsCertain: false\n\
         \x20   reportUnmatchedIgnoredErrors: false\n\
         \x20   tmpDir: var/cache/phpstan\n"
    );

    let phpstan_baseline_content = "parameters:\n\tignoreErrors: []\n".to_string();

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
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/phpstan.neon")),
            content: phpstan_content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/phpstan-baseline.neon")),
            content: phpstan_baseline_content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("{pkg_dir}/.php-cs-fixer.dist.php")),
            content: r#"<?php

declare(strict_types=1);

// Stub files declare classes the native extension provides at runtime.
// They contain ext-php-rs-style scaffolding that php-cs-fixer's @PHP82Migration
// rule would otherwise rewrite into constructor-promoted properties, deleting
// the explicit class-level property declarations phpstan needs to see.
// Excluding stubs/ keeps the stub structure intact for static analysis.
$finder = (new PhpCsFixer\Finder())
    ->in(array_filter([
        __DIR__ . '/src',
        is_dir(__DIR__ . '/tests') ? __DIR__ . '/tests' : null,
    ]))
    ->notPath('stubs');

return (new PhpCsFixer\Config())
    ->setUnsupportedPhpVersionAllowed(true)
    ->setRules([
        '@PSR12' => true,
        '@PHP82Migration' => true,
        'array_syntax' => ['syntax' => 'short'],
        'single_quote' => true,
        'trailing_comma_in_multiline' => [
            'elements' => ['arrays', 'arguments', 'parameters'],
        ],
        'declare_strict_types' => true,
        'ordered_imports' => ['sort_algorithm' => 'alpha'],
        'no_unused_imports' => true,
    ])
    ->setFinder($finder)
    ->setRiskyAllowed(true);
"#
            .to_string(),
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
