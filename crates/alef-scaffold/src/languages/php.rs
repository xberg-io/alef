use crate::{cargo_package_header, core_dep_features, detect_workspace_inheritance, render_extra_deps, scaffold_meta};
use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use alef_core::template_versions as tv;
use std::path::PathBuf;

pub(crate) fn scaffold_php_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.crate_config.workspace_root.as_deref());
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
    let extra_deps_section = if extra_deps.is_empty() {
        String::new()
    } else {
        format!("\n{extra_deps}")
    };
    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
ext-php-rs = "{ext_php_rs}"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}{extra_deps_section}

[features]
extension-module = []

[lints]
workspace = true
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Php),
        ext_php_rs = tv::cargo::EXT_PHP_RS,
        extra_deps_section = extra_deps_section,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-php/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

pub(crate) fn scaffold_php(_api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let ext_name = config.php_extension_name();
    let name = &config.crate_config.name;
    let pkg_dir = config.package_dir(Language::Php);
    // PSR-4 namespace derived from the extension name (e.g. html_to_markdown_rs -> Html\To\Markdown\Rs).
    // Double backslashes for JSON string literal output.
    let php_namespace = config.php_autoload_namespace().replace('\\', "\\\\");

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
    let vendor = meta
        .repository
        .strip_prefix("https://github.com/")
        .or_else(|| meta.repository.strip_prefix("http://github.com/"))
        .and_then(|rest| rest.split('/').next())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| name.clone());

    let content = format!(
        r#"{{
  "name": "{vendor}/{name}",
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
      "{php_namespace}\\\\": "src/"
    }}
  }},
  "scripts": {{
    "phpstan": "php -d detect_unicode=0 vendor/bin/phpstan --configuration=phpstan.neon --memory-limit=512M",
    "format": "PHP_CS_FIXER_IGNORE_ENV=1 php vendor/bin/php-cs-fixer fix --config php-cs-fixer.php src",
    "format:check": "PHP_CS_FIXER_IGNORE_ENV=1 php vendor/bin/php-cs-fixer fix --config php-cs-fixer.php --dry-run src",
    "test": "php vendor/bin/phpunit",
    "lint": "@phpstan",
    "lint:fix": "PHP_CS_FIXER_IGNORE_ENV=1 php vendor/bin/php-cs-fixer fix --config php-cs-fixer.php src && php -d detect_unicode=0 vendor/bin/phpstan --configuration=phpstan.neon --memory-limit=512M"
  }},
  "php-ext": {{
    "extension-name": "{ext_name}",
    "support-zts": true,
    "support-nts": true
  }}{keywords}
}}
"#,
        name = name,
        description = meta.description,
        license = meta.license,
        php_namespace = php_namespace,
        ext_name = ext_name,
        keywords = keywords_json,
        phpstan = tv::packagist::PHPSTAN,
        php_cs_fixer = tv::packagist::PHP_CS_FIXER,
        phpunit = tv::packagist::PHPUNIT,
    );

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
            path: PathBuf::from(format!("{pkg_dir}/php-cs-fixer.php")),
            content: r#"<?php

$finder = (new PhpCsFixer\Finder())
    ->in(__DIR__ . '/src')
    ->in(__DIR__ . '/tests');

return (new PhpCsFixer\Config())
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
