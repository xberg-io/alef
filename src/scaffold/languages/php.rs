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
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-php"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let extra_deps = render_extra_deps(config, Language::Php);

    let has_trait_bridges = !config.trait_bridges.is_empty();
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    // ahash is needed when any function takes an AHashMap<Cow, _> param — the generated
    // PHP wrapper emits a `let __<name>_ahash: ahash::AHashMap<...>` pre-call binding.
    let needs_ahash = api
        .functions
        .iter()
        .any(|f| f.params.iter().any(|p| p.map_is_ahash));
    let mut all_deps = extra_deps;
    if needs_ahash && !all_deps.contains("ahash") {
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
    // Build the cargo-machete ignored list. `serde_json` and `tokio` are
    // emitted unconditionally above so they are always ignored. Conditional
    // deps (`async-trait` for trait bridges, `futures-util` for streaming)
    // are appended only when the scaffold actually adds them to
    // `[dependencies]`, so cargo-machete doesn't flap on umbrellas whose
    // API surface doesn't exercise the trait-bridge / streaming codepath.
    let mut machete_ignored: Vec<&str> = vec!["serde_json", "tokio"];
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
    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{core_dep}
ext-php-rs = "{ext_php_rs}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}{extra_deps_section}

# `serde_json` and `tokio` are emitted unconditionally above so the manifest
# is stable across regens, but for umbrella crates with no async fns and no
# JSON-marshalled return types they are genuinely unused. The conditional
# `async-trait` / `futures-util` deps are similarly flagged when the
# umbrella has trait-bridge / streaming adapters configured but no actual
# async-trait callsite in the generated PHP shim.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[features]
extension-module = []

"#,
        pkg_header = pkg_header,
        core_dep = crate::scaffold::render_core_dep(
            &config.name,
            &format!("../{core_crate_dir}"),
            &core_dep_features(config, Language::Php),
            version,
        ),
        ext_php_rs = tv::cargo::EXT_PHP_RS,
        extra_deps_section = extra_deps_section,
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
    let name = &config.name;
    let pkg_dir = config.package_dir(Language::Php);
    // PSR-4 namespace derived from the extension name (e.g. html_to_markdown_rs -> Html\To\Markdown\Rs).
    // Double backslashes for JSON string literal output.
    let php_namespace = php_autoload_namespace(config).replace('\\', "\\\\");

    let keywords_json = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!(",\n  \"keywords\": [{}]", entries.join(", "))
    };

    // Derive vendor from the GitHub owner in the repository URL.
    // e.g. "https://github.com/Acme/my-lib" -> "acme" (composer requires the
    // vendor to be all-lowercase per the package-name regex; mixed-case orgs
    // like `Goldziher` get folded down here).
    let (vendor, package_name) = {
        let repo = meta
            .repository
            .strip_prefix("https://github.com/")
            .or_else(|| meta.repository.strip_prefix("http://github.com/"))
            .filter(|s| !s.is_empty())
            .unwrap_or(&meta.repository);

        let parts: Vec<&str> = repo.split('/').collect();
        match parts.as_slice() {
            [owner, repo_name, ..] => {
                let vendor = owner.to_lowercase();
                // Use the repo name (e.g. html-to-markdown) for the package name,
                // falling back to the crate name if the repo name can't be extracted.
                let pkg_name = repo_name.to_lowercase();
                (vendor, pkg_name)
            }
            _ => (name.clone(), name.to_lowercase()),
        }
    };

    // Derive the GitHub owner/repo from the repository URL for the binary URL template.
    // e.g. "https://github.com/kreuzberg-dev/html-to-markdown" -> "kreuzberg-dev/html-to-markdown"
    let repo_path = meta
        .repository
        .strip_prefix("https://github.com/")
        .or_else(|| meta.repository.strip_prefix("http://github.com/"))
        .filter(|s| !s.is_empty())
        .unwrap_or("");

    let pie_binary_block = if !repo_path.is_empty() {
        format!(
            r#",
  "extra": {{
    "pie": {{
      "binary": {{
        "url-template": "https://github.com/{repo_path}/releases/download/v{{Version}}/php_{ext_name}-{{Version}}_php{{PhpVersion}}-{{Arch}}-{{OS}}-{{Libc}}-{{TSMode}}.tgz"
      }}
    }}
  }}"#,
            repo_path = repo_path,
            ext_name = ext_name,
        )
    } else {
        String::new()
    };

    // Composer manifests are emitted twice with one structural difference: the
    // PSR-4 autoload src path. The package manifest at `{pkg_dir}/composer.json`
    // is the dev manifest used by phpstan/phpunit/php-cs-fixer inside the
    // package directory and points at `src/`. The root manifest at
    // `composer.json` is the one Packagist indexes and PIE installs read — it
    // must point at `{pkg_dir}/src/` so the same PSR-4 classes resolve from
    // the repo root. Everything else (name, php-ext block, extra.pie binary
    // url-template, require/require-dev, scripts) is byte-identical so the two
    // manifests stay in sync without drift.
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
    "download-url-method": ["pre-packaged-binary", "composer-default"]
  }}{keywords}{pie_binary_block}
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
            pie_binary_block = pie_binary_block,
            phpstan = tv::packagist::PHPSTAN,
            php_cs_fixer = tv::packagist::PHP_CS_FIXER,
            phpunit = tv::packagist::PHPUNIT,
        )
    };

    let content = render_composer("src/");
    let root_content = render_composer(&format!("{pkg_dir}/src/"));

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
        // Root composer.json is the Packagist/PIE manifest — Packagist indexes
        // it from the repo root and PIE reads `extra.pie.binary.url-template`
        // from it to download prebuilt extension binaries from GitHub Releases.
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
