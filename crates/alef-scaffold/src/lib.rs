//! Package scaffolding generator for alef.

use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use std::path::PathBuf;

/// Fields available via `[workspace.package]` inheritance detected from the root `Cargo.toml`.
#[derive(Debug, Default)]
#[allow(dead_code)]
struct WorkspacePackageInheritance {
    /// Whether `[workspace]` exists at all (i.e. this is a Cargo workspace).
    pub is_workspace: bool,
    /// `version` is declared in `[workspace.package]`.
    pub version: bool,
    /// `readme` is declared in `[workspace.package]`.
    pub readme: bool,
    /// `keywords` is declared in `[workspace.package]`.
    pub keywords: bool,
    /// `categories` is declared in `[workspace.package]`.
    pub categories: bool,
    /// `license` is declared in `[workspace.package]`.
    pub license: bool,
}

/// Detect which `[workspace.package]` fields are available in the root `Cargo.toml`.
///
/// Reads `Cargo.toml` from the current working directory. Returns a default
/// (all false) struct if the file is absent or cannot be parsed.
fn detect_workspace_inheritance() -> WorkspacePackageInheritance {
    let Ok(contents) = std::fs::read_to_string("Cargo.toml") else {
        return WorkspacePackageInheritance::default();
    };
    let Ok(doc) = contents.parse::<toml::Value>() else {
        return WorkspacePackageInheritance::default();
    };
    let Some(workspace) = doc.get("workspace") else {
        return WorkspacePackageInheritance::default();
    };
    let pkg = workspace.get("package");
    WorkspacePackageInheritance {
        is_workspace: true,
        version: pkg.map(|p| p.get("version").is_some()).unwrap_or(false),
        readme: pkg.map(|p| p.get("readme").is_some()).unwrap_or(false),
        keywords: pkg.map(|p| p.get("keywords").is_some()).unwrap_or(false),
        categories: pkg.map(|p| p.get("categories").is_some()).unwrap_or(false),
        license: pkg.map(|p| p.get("license").is_some()).unwrap_or(false),
    }
}

/// Build the `[package]` header fields for a binding crate Cargo.toml.
///
/// Uses `*.workspace = true` for any field that is available in `[workspace.package]`,
/// falling back to explicit values otherwise.
fn cargo_package_header(
    name: &str,
    version: &str,
    edition: &str,
    license: &str,
    description: &str,
    keywords: &[String],
    ws: &WorkspacePackageInheritance,
) -> String {
    let version_line = if ws.version {
        "version.workspace = true".to_string()
    } else {
        format!("version = \"{version}\"")
    };
    let edition_line = format!("edition = \"{edition}\"");
    let license_line = if ws.license {
        "license.workspace = true".to_string()
    } else {
        format!("license = \"{license}\"")
    };
    let readme_line = if ws.readme {
        "readme.workspace = true".to_string()
    } else {
        "readme = false".to_string()
    };
    let keywords_line = if ws.keywords {
        "keywords.workspace = true".to_string()
    } else if keywords.is_empty() {
        "keywords = []".to_string()
    } else {
        let quoted: Vec<String> = keywords.iter().map(|k| format!("\"{k}\"")).collect();
        format!("keywords = [{}]", quoted.join(", "))
    };
    let categories_line = if ws.categories {
        "categories.workspace = true".to_string()
    } else {
        "categories = [\"text-processing\"]".to_string()
    };

    let lines = vec![
        "[package]".to_string(),
        format!("name = \"{name}\""),
        version_line,
        edition_line,
        license_line,
        format!("description = \"{description}\""),
        readme_line,
        keywords_line,
        categories_line,
    ];
    lines.join("\n")
}

/// Convert a semver pre-release version to PEP 440 format for Python/PyPI.
/// e.g., "0.1.0-rc.1" -> "0.1.0rc1", "0.1.0-alpha.2" -> "0.1.0a2", "0.1.0-beta.3" -> "0.1.0b3"
/// Non-pre-release versions are returned unchanged.
fn to_pep440(version: &str) -> String {
    if let Some((base, pre)) = version.split_once('-') {
        let pep = pre
            .replace("alpha.", "a")
            .replace("alpha", "a")
            .replace("beta.", "b")
            .replace("beta", "b")
            .replace("rc.", "rc")
            .replace('.', "");
        format!("{base}{pep}")
    } else {
        version.to_string()
    }
}

/// Format the features clause for the core crate dependency in generated Cargo.toml files.
///
/// Checks for per-language feature overrides first, then falls back to `[crate] features`.
/// Returns an empty string if no features are configured, otherwise returns
/// `, features = ["feat1", "feat2"]`.
fn core_dep_features(config: &AlefConfig, lang: Language) -> String {
    let features = config.features_for_language(lang);
    if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    }
}

/// Generate package scaffolding files for the given languages.
pub fn scaffold(api: &ApiSurface, config: &AlefConfig, languages: &[Language]) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![];
    for &lang in languages {
        files.extend(scaffold_language(api, config, lang)?);
    }
    // Project-level files that depend on the full set of configured languages
    files.extend(scaffold_pre_commit_config(config, languages));
    Ok(files)
}

fn scaffold_language(api: &ApiSurface, config: &AlefConfig, lang: Language) -> anyhow::Result<Vec<GeneratedFile>> {
    match lang {
        Language::Python => {
            let mut files = scaffold_python(api, config)?;
            files.extend(scaffold_python_cargo(api, config)?);
            Ok(files)
        }
        Language::Node => {
            let mut files = scaffold_node(api, config)?;
            files.extend(scaffold_node_cargo(api, config)?);
            Ok(files)
        }
        Language::Ffi => scaffold_ffi(api, config),
        Language::Go => scaffold_go(api, config),
        Language::Java => scaffold_java(api, config),
        Language::Csharp => scaffold_csharp(api, config),
        Language::Ruby => {
            let mut files = scaffold_ruby(api, config)?;
            files.extend(scaffold_ruby_cargo(api, config)?);
            Ok(files)
        }
        Language::Php => {
            let mut files = scaffold_php(api, config)?;
            files.extend(scaffold_php_cargo(api, config)?);
            Ok(files)
        }
        Language::Elixir => {
            let mut files = scaffold_elixir(api, config)?;
            files.extend(scaffold_elixir_cargo(api, config)?);
            Ok(files)
        }
        Language::Wasm => scaffold_wasm(api, config),
        Language::R => {
            let mut files = scaffold_r(api, config)?;
            files.extend(scaffold_r_cargo(api, config)?);
            Ok(files)
        }
        Language::Rust => Ok(vec![]), // Rust doesn't need scaffolded binding crates
    }
}

/// Helper to get scaffold metadata with defaults.
struct ScaffoldMeta {
    description: String,
    license: String,
    repository: String,
    homepage: String,
    authors: Vec<String>,
    keywords: Vec<String>,
}

fn scaffold_meta(config: &AlefConfig) -> ScaffoldMeta {
    let scaffold = config.scaffold.as_ref();
    ScaffoldMeta {
        description: scaffold
            .and_then(|s| s.description.clone())
            .unwrap_or_else(|| format!("Bindings for {}", config.crate_config.name)),
        license: scaffold
            .and_then(|s| s.license.clone())
            .unwrap_or_else(|| "MIT".to_string()),
        repository: scaffold
            .and_then(|s| s.repository.clone())
            .unwrap_or_else(|| format!("https://github.com/kreuzberg-dev/{}", config.crate_config.name)),
        homepage: scaffold.and_then(|s| s.homepage.clone()).unwrap_or_default(),
        authors: scaffold.map(|s| s.authors.clone()).unwrap_or_default(),
        keywords: scaffold.map(|s| s.keywords.clone()).unwrap_or_default(),
    }
}

fn scaffold_python_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let module_name = config.python_module_name();
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-py"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}

[lib]
name = "{module_name}"
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
pyo3 = {{ version = "0.28", features = ["extension-module"] }}
pyo3-async-runtimes = {{ version = "0.28", features = ["tokio-runtime"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        pkg_header = pkg_header,
        module_name = module_name,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Python),
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-py/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

fn scaffold_python(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let pip_name = config.python_pip_name();
    let version = to_pep440(&api.version);
    let module_name = config.python_module_name();
    let core_crate_dir = config.core_crate_dir();
    let python_package = pip_name.replace('-', "_");

    let authors_toml = if meta.authors.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta
            .authors
            .iter()
            .map(|a| format!("    {{ name = \"{}\" }}", a))
            .collect();
        format!("authors = [\n{}\n]\n", entries.join(",\n"))
    };

    let keywords_toml = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!("keywords = [{}]\n", entries.join(", "))
    };

    let homepage_toml = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!("homepage = \"{}\"\n", meta.homepage)
    };

    let content = format!(
        r#"[build-system]
requires = ["maturin>=1.0,<2.0"]
build-backend = "maturin"

[project]
name = "{pip_name}"
version = "{version}"
description = "{description}"
license = "{license}"
requires-python = ">=3.10"
classifiers = [
  "Programming Language :: Python :: 3 :: Only",
  "Programming Language :: Python :: 3.10",
  "Programming Language :: Python :: 3.11",
  "Programming Language :: Python :: 3.12",
  "Programming Language :: Python :: 3.13",
  "Programming Language :: Python :: 3.14",
]
{authors}{keywords}{homepage}[project.urls]
repository = "{repository}"

[tool.maturin]
module-name = "{python_package}.{module_name}"
manifest-path = "../../crates/{crate_dir}-py/Cargo.toml"
features = ["pyo3/extension-module"]
python-packages = ["{python_package}"]
"#,
        pip_name = pip_name,
        version = version,
        description = meta.description,
        license = meta.license,
        authors = authors_toml,
        keywords = keywords_toml,
        homepage = homepage_toml,
        repository = meta.repository,
        python_package = python_package,
        module_name = module_name,
        crate_dir = core_crate_dir,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/python/pyproject.toml"),
        content,
        generated_header: true,
    }])
}

fn scaffold_node_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-node"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
napi = {{ version = "3", features = ["async"] }}
napi-derive = "3"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"

[build-dependencies]
napi-build = "2"
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Node),
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-node/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

fn scaffold_node(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let package_name = config.node_package_name();
    let name = &config.crate_config.name;
    let version = &api.version;

    let keywords_json = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!(",\n  \"keywords\": [{}]", entries.join(", "))
    };

    let homepage_json = if meta.homepage.is_empty() {
        String::new()
    } else {
        format!(",\n  \"homepage\": \"{}\"", meta.homepage)
    };

    let authors_json = if meta.authors.is_empty() {
        String::new()
    } else {
        format!(",\n  \"author\": \"{}\"", meta.authors.join(", "))
    };

    let content = format!(
        r#"{{
  "name": "{package_name}",
  "version": "{version}",
  "description": "{description}",
  "license": "{license}",
  "main": "index.js",
  "types": "index.d.ts",
  "repository": "{repository}"{homepage}{authors}{keywords},
  "files": [
    "index.js",
    "index.d.ts",
    "**/*.node"
  ],
  "scripts": {{
    "build": "napi build --release",
    "build:debug": "napi build",
    "build:ts": "echo 'No TypeScript wrapper to build'",
    "test": "node -e \"console.log('Add test command')\""
  }},
  "napi": {{
    "name": "{name}",
    "triples": [
      "x86_64-unknown-linux-gnu",
      "x86_64-apple-darwin",
      "aarch64-apple-darwin",
      "x86_64-pc-windows-msvc"
    ]
  }},
  "devDependencies": {{
    "@napi-rs/cli": "^3.0.0"
  }}
}}
"#,
        package_name = package_name,
        version = version,
        description = meta.description,
        license = meta.license,
        repository = meta.repository,
        homepage = homepage_json,
        authors = authors_json,
        keywords = keywords_json,
        name = name,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/typescript/package.json"),
        content,
        generated_header: false,
    }])
}

fn scaffold_ruby_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-rb"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../../../../../crates/{core_crate_dir}"{features} }}
magnus = "0.8"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["rt-multi-thread"] }}
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Ruby),
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!(
            "packages/ruby/ext/{}_rb/native/Cargo.toml",
            core_crate_dir.replace('-', "_")
        )),
        content,
        generated_header: true,
    }])
}

fn scaffold_ruby(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let gem_name = config.ruby_gem_name();
    let core_crate_dir = config.core_crate_dir();
    // The native extension name uses the core crate dir with underscores and _rb suffix,
    // matching the directory generated by scaffold_ruby_cargo: ext/{core_crate_dir}_rb/
    let ext_name = format!("{}_rb", core_crate_dir.replace('-', "_"));
    let version = &api.version;

    let authors_ruby = if meta.authors.is_empty() {
        "[]".to_string()
    } else {
        let entries: Vec<String> = meta.authors.iter().map(|a| format!("'{}'", a)).collect();
        format!("[{}]", entries.join(", "))
    };

    let metadata_ruby = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("'{}'", k)).collect();
        format!("  spec.metadata['keywords'] = [{}].join(',')\n", entries.join(", "))
    };

    let content = format!(
        r#"# frozen_string_literal: true

Gem::Specification.new do |spec|
  spec.name = '{gem_name}'
  spec.version = '{version}'
  spec.authors       = {authors}
  spec.summary       = '{description}'
  spec.description   = '{description}'
  spec.homepage      = '{repository}'
  spec.license       = '{license}'
  spec.required_ruby_version = '>= 3.2.0'
{metadata}  spec.metadata['rubygems_mfa_required'] = 'true'

  spec.files         = Dir.glob(['lib/**/*', 'ext/**/*'])
  spec.require_paths = ['lib']
  spec.extensions    = ['ext/{ext_name}/extconf.rb']

  spec.add_dependency 'rb_sys', '~> 0.9'
end
"#,
        gem_name = gem_name,
        ext_name = ext_name,
        version = version,
        authors = authors_ruby,
        description = meta.description,
        repository = meta.repository,
        license = meta.license,
        metadata = metadata_ruby,
    );

    let rubocop_content = r#"plugins:
  - rubocop-performance
  - rubocop-rspec

AllCops:
  TargetRubyVersion: 3.2
  NewCops: enable
  SuggestExtensions: false
  Exclude:
    - 'vendor/**/*'
    - 'tmp/**/*'
    - 'lib/**/*.bundle'
    - 'ext/**/*'

Style/FrozenStringLiteralComment:
  Enabled: true
  EnforcedStyle: always

Style/StringLiterals:
  Enabled: true
  EnforcedStyle: single_quotes

Style/StringLiteralsInInterpolation:
  Enabled: true
  EnforcedStyle: single_quotes

Style/Documentation:
  Enabled: false

Layout/LineLength:
  Max: 120
  AllowedPatterns:
    - '\A\s*#'
  Exclude:
    - 'spec/**/*'

Metrics/MethodLength:
  Max: 20
  Exclude:
    - 'spec/**/*'

Metrics/BlockLength:
  Enabled: true
  Max: 350
  CountComments: false

Metrics/AbcSize:
  Max: 20
  Exclude:
    - 'spec/**/*'

RSpec/ExampleLength:
  Max: 50

RSpec/MultipleExpectations:
  Max: 25

RSpec/NestedGroups:
  Max: 6
"#
    .to_string();

    let rakefile_content = format!(
        r#"# frozen_string_literal: true

require 'bundler/gem_tasks'
require 'rake/extensiontask'
require 'rspec/core/rake_task'

GEMSPEC = Gem::Specification.load(File.expand_path('{gem_name}.gemspec', __dir__))

Rake::ExtensionTask.new('{ext_name}', GEMSPEC) do |ext|
  ext.lib_dir = 'lib'
  ext.ext_dir = 'ext/{ext_name}'
  ext.cross_compile = true
  ext.cross_platform = %w[
    x86_64-linux
    aarch64-linux
    x86_64-darwin
    arm64-darwin
    x64-mingw32
    x64-mingw-ucrt
  ]
end

RSpec::Core::RakeTask.new(:spec)

task spec: :compile
task default: :spec
"#,
        gem_name = gem_name,
        ext_name = ext_name,
    );

    let lib_content = format!(
        r#"# frozen_string_literal: true

require '{ext_name}'
"#,
        ext_name = ext_name,
    );

    let extconf_content = format!(
        r#"# frozen_string_literal: true

require 'mkmf'
require 'rb_sys/mkmf'

default_profile = ENV.fetch('CARGO_PROFILE', 'release')

create_rust_makefile('{ext_name}') do |config|
  config.profile = default_profile.to_sym
end
"#,
        ext_name = ext_name,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("packages/ruby/{}.gemspec", gem_name)),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/ruby/.rubocop.yml"),
            content: rubocop_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from("packages/ruby/Rakefile"),
            content: rakefile_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/ruby/lib/{}.rb", gem_name)),
            content: lib_content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/ruby/ext/{ext_name}/extconf.rb", ext_name = ext_name)),
            content: extconf_content,
            generated_header: true,
        },
    ])
}

fn scaffold_php_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-php"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
ext-php-rs = "0.15"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Php),
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-php/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }])
}

fn scaffold_php(_api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let ext_name = config.php_extension_name();
    let name = &config.crate_config.name;
    // PSR-4 namespace derived from the extension name (e.g. html_to_markdown_rs -> Html\To\Markdown\Rs).
    // Double backslashes for JSON string literal output.
    let php_namespace = config.php_autoload_namespace().replace('\\', "\\\\");

    let keywords_json = if meta.keywords.is_empty() {
        String::new()
    } else {
        let entries: Vec<String> = meta.keywords.iter().map(|k| format!("\"{}\"", k)).collect();
        format!(",\n  \"keywords\": [{}]", entries.join(", "))
    };

    let content = format!(
        r#"{{
  "name": "kreuzberg-dev/{name}",
  "description": "{description}",
  "license": "{license}",
  "type": "php-ext",
  "require": {{
    "php": ">=8.2"
  }},
  "require-dev": {{
    "phpstan/phpstan": "^2.1",
    "friendsofphp/php-cs-fixer": "^3.95",
    "phpunit/phpunit": "^13.1"
  }},
  "autoload": {{
    "psr-4": {{
      "{php_namespace}\\\\": "src/"
    }}
  }},
  "scripts": {{
    "phpstan": "php -d detect_unicode=0 vendor/bin/phpstan --configuration=phpstan.neon --memory-limit=512M",
    "format": "PHP_CS_FIXER_IGNORE_ENV=1 php vendor/bin/php-cs-fixer fix --config php-cs-fixer.php src tests",
    "format:check": "PHP_CS_FIXER_IGNORE_ENV=1 php vendor/bin/php-cs-fixer fix --config php-cs-fixer.php --dry-run src tests",
    "test": "php vendor/bin/phpunit",
    "lint": "@phpstan",
    "lint:fix": "PHP_CS_FIXER_IGNORE_ENV=1 php vendor/bin/php-cs-fixer fix --config php-cs-fixer.php src tests && php -d detect_unicode=0 vendor/bin/phpstan --configuration=phpstan.neon --memory-limit=512M"
  }},
  "extra": {{
    "ext-name": "{ext_name}"
  }}{keywords}
}}
"#,
        name = name,
        description = meta.description,
        license = meta.license,
        php_namespace = php_namespace,
        ext_name = ext_name,
        keywords = keywords_json,
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
            path: PathBuf::from("packages/php/composer.json"),
            content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/php/phpstan.neon"),
            content: phpstan_content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/php/phpstan-baseline.neon"),
            content: phpstan_baseline_content,
            generated_header: false,
        },
    ])
}

fn scaffold_elixir_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let app_name = config.elixir_app_name();
    let nif_name = format!("{app_name}_rustler");
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &nif_name,
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../../../../crates/{core_crate_dir}"{features} }}
rustler = "0.37"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Elixir),
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("packages/elixir/native/{nif_name}/Cargo.toml")),
        content,
        generated_header: true,
    }])
}

fn scaffold_elixir(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let app_name = config.elixir_app_name();
    let version = &api.version;

    let content = format!(
        r#"defmodule {module}.MixProject do
  use Mix.Project

  def project do
    [
      app: :{app_name},
      version: "{version}",
      elixir: "~> 1.14",
      rustler_crates: [{nif_atom}: [mode: :release]],
      description: "{description}",
      package: package(),
      deps: deps()
    ]
  end

  defp package do
    [
      licenses: ["{license}"],
      links: %{{"GitHub" => "{repository}"}},
      files: ~w(lib native .formatter.exs mix.exs README* checksum-*.exs)
    ]
  end

  defp deps do
    [
      {{:rustler, "~> 0.37.0", optional: true, runtime: false}},
      {{:rustler_precompiled, "~> 0.9"}},
      {{:credo, "~> 1.7", only: [:dev, :test], runtime: false}},
      {{:ex_doc, "~> 0.40", only: :dev, runtime: false}}
    ]
  end
end
"#,
        module = capitalize_first(&app_name),
        app_name = app_name,
        nif_atom = format_args!("{app_name}_rustler"),
        version = version,
        description = meta.description,
        license = meta.license,
        repository = meta.repository,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/elixir/mix.exs"),
        content,
        generated_header: true,
    }])
}

fn scaffold_go(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let go_module = config.go_module();
    let version = &api.version;
    let _ = version; // go.mod doesn't embed the package version

    let content = format!("module {module}\n\ngo 1.26\n", module = go_module,);

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/go/go.mod"),
        content,
        generated_header: false,
    }])
}

fn scaffold_java(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let name = &config.crate_config.name;
    let version = &api.version;

    // Derive SCM URLs from repository URL
    let repo_url = &meta.repository;
    let repo_path = repo_url
        .strip_prefix("https://github.com/")
        .or_else(|| repo_url.strip_prefix("http://github.com/"))
        .unwrap_or(repo_url.trim_start_matches("https://"));

    let group_id = config.java_group_id();

    // Build developers XML from authors
    let developers_xml = if meta.authors.is_empty() {
        String::new()
    } else {
        let devs: Vec<String> = meta
            .authors
            .iter()
            .map(|a| {
                format!(
                    "        <developer>\n            <name>{a}</name>\n            <email>noreply@kreuzberg.dev</email>\n        </developer>"
                )
            })
            .collect();
        format!("\n    <developers>\n{}\n    </developers>\n", devs.join("\n"))
    };

    // License URL mapping
    let license_url = match meta.license.as_str() {
        "Elastic-2.0" => "https://www.elastic.co/licensing/elastic-license",
        "MIT" => "https://opensource.org/licenses/MIT",
        "Apache-2.0" => "https://www.apache.org/licenses/LICENSE-2.0",
        _ => "",
    };
    let license_url_xml = if license_url.is_empty() {
        String::new()
    } else {
        format!("\n            <url>{license_url}</url>")
    };

    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <groupId>{group_id}</groupId>
    <artifactId>{name}</artifactId>
    <version>{version}</version>
    <packaging>jar</packaging>

    <name>{name}</name>
    <description>{description}</description>
    <url>{repository}</url>

    <licenses>
        <license>
            <name>{license}</name>{license_url}
        </license>
    </licenses>
{developers}
    <scm>
        <connection>scm:git:git://github.com/{repo_path}.git</connection>
        <developerConnection>scm:git:ssh://github.com:{repo_path}.git</developerConnection>
        <url>{repository}</url>
    </scm>

    <properties>
        <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
        <maven.compiler.release>25</maven.compiler.release>
        <junit.version>5.11.4</junit.version>
        <maven-compiler-plugin.version>3.14.0</maven-compiler-plugin.version>
        <maven-surefire-plugin.version>3.5.5</maven-surefire-plugin.version>
        <maven-source-plugin.version>3.3.1</maven-source-plugin.version>
        <maven-javadoc-plugin.version>3.12.0</maven-javadoc-plugin.version>
        <maven-gpg-plugin.version>3.2.8</maven-gpg-plugin.version>
        <maven-clean-plugin.version>3.4.1</maven-clean-plugin.version>
        <maven-resources-plugin.version>3.3.1</maven-resources-plugin.version>
        <maven-jar-plugin.version>3.4.2</maven-jar-plugin.version>
        <maven-install-plugin.version>3.1.3</maven-install-plugin.version>
        <maven-deploy-plugin.version>3.1.3</maven-deploy-plugin.version>
        <maven-site-plugin.version>3.21.0</maven-site-plugin.version>
        <central-publishing-plugin.version>0.10.0</central-publishing-plugin.version>
        <spotless-maven-plugin.version>3.4.0</spotless-maven-plugin.version>
        <gpg.skip>true</gpg.skip>
    </properties>

    <dependencies>
        <dependency>
            <groupId>com.fasterxml.jackson.core</groupId>
            <artifactId>jackson-databind</artifactId>
            <version>2.21.2</version>
        </dependency>
        <dependency>
            <groupId>com.fasterxml.jackson.datatype</groupId>
            <artifactId>jackson-datatype-jdk8</artifactId>
            <version>2.21.2</version>
        </dependency>
        <dependency>
            <groupId>org.junit.jupiter</groupId>
            <artifactId>junit-jupiter</artifactId>
            <version>${{junit.version}}</version>
            <scope>test</scope>
        </dependency>
    </dependencies>

    <build>
        <resources>
            <resource>
                <directory>src/main/resources</directory>
            </resource>
        </resources>
        <pluginManagement>
            <plugins>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-clean-plugin</artifactId>
                    <version>${{maven-clean-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-resources-plugin</artifactId>
                    <version>${{maven-resources-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-jar-plugin</artifactId>
                    <version>${{maven-jar-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-install-plugin</artifactId>
                    <version>${{maven-install-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-deploy-plugin</artifactId>
                    <version>${{maven-deploy-plugin.version}}</version>
                </plugin>
                <plugin>
                    <groupId>org.apache.maven.plugins</groupId>
                    <artifactId>maven-site-plugin</artifactId>
                    <version>${{maven-site-plugin.version}}</version>
                </plugin>
            </plugins>
        </pluginManagement>
        <plugins>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-compiler-plugin</artifactId>
                <version>${{maven-compiler-plugin.version}}</version>
                <configuration>
                    <release>25</release>
                    <compilerArgs>
                        <arg>--enable-preview</arg>
                    </compilerArgs>
                </configuration>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-surefire-plugin</artifactId>
                <version>${{maven-surefire-plugin.version}}</version>
                <configuration>
                    <argLine>@{{argLine}} --enable-native-access=ALL-UNNAMED --enable-preview -Djava.library.path=${{project.basedir}}/../../target/release</argLine>
                </configuration>
            </plugin>
            <plugin>
                <groupId>com.diffplug.spotless</groupId>
                <artifactId>spotless-maven-plugin</artifactId>
                <version>${{spotless-maven-plugin.version}}</version>
                <configuration>
                    <java>
                        <eclipse>
                            <version>4.31</version>
                        </eclipse>
                    </java>
                </configuration>
                <executions>
                    <execution>
                        <goals>
                            <goal>apply</goal>
                        </goals>
                        <phase>process-sources</phase>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-source-plugin</artifactId>
                <version>${{maven-source-plugin.version}}</version>
                <executions>
                    <execution>
                        <id>attach-sources</id>
                        <goals>
                            <goal>jar-no-fork</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-javadoc-plugin</artifactId>
                <version>${{maven-javadoc-plugin.version}}</version>
                <configuration>
                    <doclint>none</doclint>
                    <show>protected</show>
                    <additionalOptions>--enable-preview</additionalOptions>
                    <sourcepath>${{project.basedir}}/src/main/java</sourcepath>
                </configuration>
                <executions>
                    <execution>
                        <id>attach-javadocs</id>
                        <goals>
                            <goal>jar</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
            <plugin>
                <groupId>org.apache.maven.plugins</groupId>
                <artifactId>maven-gpg-plugin</artifactId>
                <version>${{maven-gpg-plugin.version}}</version>
                <executions>
                    <execution>
                        <id>sign-artifacts</id>
                        <phase>verify</phase>
                        <goals>
                            <goal>sign</goal>
                        </goals>
                    </execution>
                </executions>
            </plugin>
        </plugins>
    </build>

    <profiles>
        <profile>
            <id>publish</id>
            <properties>
                <gpg.skip>false</gpg.skip>
            </properties>
            <build>
                <plugins>
                    <plugin>
                        <groupId>org.apache.maven.plugins</groupId>
                        <artifactId>maven-deploy-plugin</artifactId>
                        <configuration>
                            <skip>true</skip>
                        </configuration>
                    </plugin>
                    <plugin>
                        <groupId>org.apache.maven.plugins</groupId>
                        <artifactId>maven-gpg-plugin</artifactId>
                        <version>${{maven-gpg-plugin.version}}</version>
                        <executions>
                            <execution>
                                <id>sign-artifacts</id>
                                <phase>verify</phase>
                                <goals>
                                    <goal>sign</goal>
                                </goals>
                                <configuration>
                                    <passphraseEnvName>MAVEN_GPG_PASSPHRASE</passphraseEnvName>
                                    <gpgArguments>
                                        <arg>--batch</arg>
                                        <arg>--yes</arg>
                                        <arg>--pinentry-mode=loopback</arg>
                                    </gpgArguments>
                                </configuration>
                            </execution>
                        </executions>
                    </plugin>
                    <plugin>
                        <groupId>org.sonatype.central</groupId>
                        <artifactId>central-publishing-maven-plugin</artifactId>
                        <version>${{central-publishing-plugin.version}}</version>
                        <extensions>true</extensions>
                        <configuration>
                            <publishingServerId>ossrh</publishingServerId>
                            <autoPublish>true</autoPublish>
                            <waitUntil>published</waitUntil>
                            <waitMaxTime>7200</waitMaxTime>
                        </configuration>
                    </plugin>
                </plugins>
            </build>
        </profile>
    </profiles>
</project>
"#,
        group_id = group_id,
        name = name,
        version = version,
        description = meta.description,
        repository = repo_url,
        license = meta.license,
        license_url = license_url_xml,
        developers = developers_xml,
        repo_path = repo_path,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/java/pom.xml"),
        content,
        generated_header: true,
    }])
}

fn scaffold_csharp(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let namespace = config.csharp_namespace();
    let version = &api.version;

    let target_framework = config
        .csharp
        .as_ref()
        .and_then(|c| c.target_framework.clone())
        .unwrap_or_else(|| "net8.0".to_string());

    let authors_csproj = if meta.authors.is_empty() {
        String::new()
    } else {
        format!("    <Authors>{}</Authors>\n", meta.authors.join(";"))
    };

    let content = format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <RootNamespace>{namespace}</RootNamespace>
    <PackageId>{namespace}</PackageId>
    <Version>{version}</Version>
    <Description>{description}</Description>
    <PackageLicenseFile>LICENSE</PackageLicenseFile>
    <RepositoryUrl>{repository}</RepositoryUrl>
{authors}    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
  </PropertyGroup>

  <ItemGroup>
    <None Include="../../../LICENSE" Pack="true" PackagePath="/" />
    <None Include="runtimes/**" Pack="true" PackagePath="runtimes/" CopyToOutputDirectory="PreserveNewest" />
  </ItemGroup>
</Project>
"#,
        target_framework = target_framework,
        namespace = namespace,
        version = version,
        description = meta.description,
        repository = meta.repository,
        authors = authors_csproj,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("packages/csharp/{}.csproj", namespace)),
        content,
        generated_header: true,
    }])
}

fn scaffold_ffi(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-ffi"),
        version,
        "2021",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}
repository = "{repository}"

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}

[features]
default = []

[build-dependencies]
cbindgen = "0.29"
"#,
        pkg_header = pkg_header,
        repository = meta.repository,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Ffi),
    );

    let ffi_name = format!("{core_crate_dir}-ffi");
    let header_name = config.ffi_header_name();
    let lib_name = config.ffi_lib_name();
    let ffi_name_under = ffi_name.replace('-', "_");

    let cmake_content = format!(
        r#"# {ffi_name} CMake config-mode find module
#
# Defines the imported target:
#   {ffi_name}::{ffi_name}
#
# Usage:
#   find_package({ffi_name} REQUIRED)
#   target_link_libraries(myapp PRIVATE {ffi_name}::{ffi_name})

if(TARGET {ffi_name}::{ffi_name})
  return()
endif()

get_filename_component(_FFI_CMAKE_DIR "${{CMAKE_CURRENT_LIST_FILE}}" PATH)
get_filename_component(_FFI_PREFIX "${{_FFI_CMAKE_DIR}}/.." ABSOLUTE)

find_library(_FFI_LIBRARY
  NAMES {lib_name} lib{lib_name}
  PATHS "${{_FFI_PREFIX}}/lib"
  NO_DEFAULT_PATH
)
if(NOT _FFI_LIBRARY)
  find_library(_FFI_LIBRARY NAMES {lib_name} lib{lib_name})
endif()

find_path(_FFI_INCLUDE_DIR
  NAMES {header_name}
  PATHS "${{_FFI_PREFIX}}/include"
  NO_DEFAULT_PATH
)
if(NOT _FFI_INCLUDE_DIR)
  find_path(_FFI_INCLUDE_DIR NAMES {header_name})
endif()

include(FindPackageHandleStandardArgs)
find_package_handle_standard_args({ffi_name}
  REQUIRED_VARS _FFI_LIBRARY _FFI_INCLUDE_DIR
)

if({ffi_name_under}_FOUND)
  set(_FFI_LIB_TYPE UNKNOWN)
  if(_FFI_LIBRARY MATCHES "\\.(dylib|so)$" OR _FFI_LIBRARY MATCHES "\\.so\\.")
    set(_FFI_LIB_TYPE SHARED)
  elseif(_FFI_LIBRARY MATCHES "\\.dll$")
    set(_FFI_LIB_TYPE SHARED)
  elseif(_FFI_LIBRARY MATCHES "\\.(a|lib)$")
    set(_FFI_LIB_TYPE STATIC)
  endif()

  add_library({ffi_name}::{ffi_name} ${{_FFI_LIB_TYPE}} IMPORTED)
  set_target_properties({ffi_name}::{ffi_name} PROPERTIES
    IMPORTED_LOCATION "${{_FFI_LIBRARY}}"
    INTERFACE_INCLUDE_DIRECTORIES "${{_FFI_INCLUDE_DIR}}"
  )

  if(WIN32 AND _FFI_LIB_TYPE STREQUAL "SHARED")
    find_file(_FFI_DLL
      NAMES {lib_name}.dll lib{lib_name}.dll
      PATHS "${{_FFI_PREFIX}}/bin" "${{_FFI_PREFIX}}/lib"
      NO_DEFAULT_PATH
    )
    if(_FFI_DLL)
      set_target_properties({ffi_name}::{ffi_name} PROPERTIES
        IMPORTED_LOCATION "${{_FFI_DLL}}"
        IMPORTED_IMPLIB "${{_FFI_LIBRARY}}"
      )
    endif()
    unset(_FFI_DLL CACHE)
  endif()

  if(APPLE)
    set_property(TARGET {ffi_name}::{ffi_name} APPEND PROPERTY
      INTERFACE_LINK_LIBRARIES "-framework CoreFoundation" "-framework Security" pthread)
  elseif(UNIX)
    set_property(TARGET {ffi_name}::{ffi_name} APPEND PROPERTY
      INTERFACE_LINK_LIBRARIES pthread dl m)
  elseif(WIN32)
    set_property(TARGET {ffi_name}::{ffi_name} APPEND PROPERTY
      INTERFACE_LINK_LIBRARIES ws2_32 userenv bcrypt)
  endif()

  unset(_FFI_LIB_TYPE)
endif()

mark_as_advanced(_FFI_LIBRARY _FFI_INCLUDE_DIR)
unset(_FFI_CMAKE_DIR)
unset(_FFI_PREFIX)
"#,
        ffi_name = ffi_name,
        ffi_name_under = ffi_name_under,
        lib_name = lib_name,
        header_name = header_name,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(format!("crates/{}-ffi/Cargo.toml", core_crate_dir)),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!(
                "crates/{}-ffi/cmake/{}-ffi-config.cmake",
                core_crate_dir, core_crate_dir
            )),
            content: cmake_content,
            generated_header: true,
        },
    ])
}

fn scaffold_wasm(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-wasm"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}
repository = "{repository}"

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
serde-wasm-bindgen = "0.6"

[package.metadata.wasm-pack.profile.release]
wasm-opt = false

[package.metadata.cargo-machete]
ignored = ["wasm-bindgen-futures"]
"#,
        pkg_header = pkg_header,
        repository = meta.repository,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Wasm),
    );

    let mut files = vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{}-wasm/Cargo.toml", core_crate_dir)),
        content,
        generated_header: true,
    }];

    // Generate package.json for npm publishing.
    // Uses the node package name with -wasm suffix for the npm scope.
    let node_pkg = config.node_package_name();
    let wasm_pkg_name = format!("{node_pkg}-wasm");
    let pkg_json = format!(
        r#"{{
  "name": "{wasm_pkg_name}",
  "version": "{version}",
  "private": false,
  "description": "{description}",
  "license": "{license}",
  "repository": {{
    "type": "git",
    "url": "{repository}",
    "directory": "crates/{core_crate_dir}-wasm"
  }},
  "type": "module",
  "files": [
    "pkg",
    "*.wasm",
    "*.d.ts",
    "README.md"
  ],
  "main": "pkg/nodejs/{core_crate_dir}_wasm.js",
  "module": "pkg/web/{core_crate_dir}_wasm.js",
  "types": "pkg/nodejs/{core_crate_dir}_wasm.d.ts",
  "scripts": {{
    "build": "wasm-pack build --target nodejs --out-dir pkg/nodejs",
    "build:ci": "wasm-pack build --release --target nodejs --out-dir pkg/nodejs",
    "build:wasm:web": "wasm-pack build --release --target web --out-dir pkg/web",
    "build:wasm:bundler": "wasm-pack build --release --target bundler --out-dir pkg/bundler",
    "build:wasm:nodejs": "wasm-pack build --release --target nodejs --out-dir pkg/nodejs",
    "build:wasm:deno": "wasm-pack build --release --target deno --out-dir pkg/deno",
    "build:all": "npm run build:wasm:web && npm run build:wasm:bundler && npm run build:wasm:nodejs && npm run build:wasm:deno && find pkg -name .gitignore -delete",
    "test": "vitest run",
    "test:watch": "vitest watch",
    "test:coverage": "vitest run --coverage",
    "clean": "rm -rf pkg dist"
  }}
}}
"#,
        wasm_pkg_name = wasm_pkg_name,
        version = version,
        description = meta.description,
        license = meta.license,
        repository = meta.repository,
        core_crate_dir = core_crate_dir,
    );

    files.push(GeneratedFile {
        path: PathBuf::from(format!("crates/{}-wasm/package.json", core_crate_dir)),
        content: pkg_json,
        generated_header: false,
    });

    Ok(files)
}

/// Generate a `.pre-commit-config.yaml` file based on configured languages.
///
/// This is a create-only scaffold: if the file already exists on disk, an empty
/// vec is returned so it won't be overwritten.
fn scaffold_pre_commit_config(config: &AlefConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    if std::path::Path::new(".pre-commit-config.yaml").exists() {
        return vec![];
    }
    generate_pre_commit_config(config, languages)
}

/// Generate the `.pre-commit-config.yaml` content based on configured languages.
///
/// Separated from `scaffold_pre_commit_config` for testability.
fn generate_pre_commit_config(config: &AlefConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    let has = |lang: Language| languages.contains(&lang);
    let crate_dir = config.core_crate_dir();

    // Build clippy --exclude args for binding crates that use incompatible compilation targets
    let clippy_excludes = {
        let suffixes: &[(&str, Language)] = &[
            ("-py", Language::Python),
            ("-node", Language::Node),
            ("-php", Language::Php),
            ("-wasm", Language::Wasm),
            ("-rb", Language::Ruby),
            ("-r", Language::R),
        ];
        let mut excludes = String::new();
        for (suffix, lang) in suffixes {
            if has(*lang) {
                excludes.push_str(&format!("            \"--exclude={crate_dir}{suffix}\",\n"));
            }
        }
        excludes
    };

    let mut yaml = String::new();

    // Header
    yaml.push_str(
        "# Generated by alef scaffold. Customize as needed.\n\
         default_install_hook_types:\n  - pre-commit\n  - commit-msg\n\
         exclude: ^target/|\\.alef/\n\n\
         repos:\n",
    );

    // Commit message linting
    yaml.push_str(
        "  # Commit message linting\n\
         \x20 - repo: https://github.com/Goldziher/gitfluff\n\
         \x20   rev: v0.8.0\n\
         \x20   hooks:\n\
         \x20     - id: gitfluff-lint\n\
         \x20       args: [\"--write\"]\n\
         \x20       stages: [commit-msg]\n\n",
    );

    // General file checks
    yaml.push_str(
        "  # General file checks\n\
         \x20 - repo: https://github.com/pre-commit/pre-commit-hooks\n\
         \x20   rev: v6.0.0\n\
         \x20   hooks:\n\
         \x20     - id: trailing-whitespace\n\
         \x20     - id: end-of-file-fixer\n\
         \x20     - id: check-merge-conflict\n\
         \x20     - id: check-added-large-files\n\
         \x20     - id: detect-private-key\n\
         \x20     - id: check-json\n\
         \x20     - id: check-yaml\n\
         \x20       args: [\"--allow-multiple-documents\", \"--unsafe\"]\n\
         \x20     - id: check-toml\n\
         \x20     - id: check-case-conflict\n\n",
    );

    // TOML formatting
    if has(Language::Python) {
        yaml.push_str(
            "  - repo: https://github.com/tox-dev/pyproject-fmt\n\
             \x20   rev: \"v2.21.1\"\n\
             \x20   hooks:\n\
             \x20     - id: pyproject-fmt\n\n",
        );
    }

    yaml.push_str(
        "  - repo: https://github.com/DevinR528/cargo-sort\n\
         \x20   rev: \"v2.1.3\"\n\
         \x20   hooks:\n\
         \x20     - id: cargo-sort\n\
         \x20       args: [-w]\n\n\
         \x20 - repo: https://github.com/ComPWA/taplo-pre-commit\n\
         \x20   rev: v0.9.3\n\
         \x20   hooks:\n\
         \x20     - id: taplo-format\n\
         \x20       exclude: \"Cargo.toml\"\n\n",
    );

    // Python: ruff
    if has(Language::Python) {
        yaml.push_str(
            "  # Python: ruff (linting + formatting)\n\
             \x20 - repo: https://github.com/astral-sh/ruff-pre-commit\n\
             \x20   rev: v0.15.10\n\
             \x20   hooks:\n\
             \x20     - id: ruff\n\
             \x20       args: [\"--fix\"]\n\
             \x20     - id: ruff-format\n\n",
        );
    }

    // Rust: formatting, linting, unused deps, license/advisory
    yaml.push_str("  # Rust: formatting, linting, unused deps, license/advisory\n");
    yaml.push_str(
        "  - repo: https://github.com/AndrejOrsula/pre-commit-cargo\n\
         \x20   rev: 0.5.0\n\
         \x20   hooks:\n\
         \x20     - id: cargo-fmt\n\
         \x20       args: [\"--all\"]\n\
         \x20     - id: cargo-clippy\n\
         \x20       args:\n\
         \x20         [\n\
         \x20           \"--fix\",\n\
         \x20           \"--allow-dirty\",\n\
         \x20           \"--allow-staged\",\n\
         \x20           \"--workspace\",\n",
    );
    yaml.push_str(&clippy_excludes);
    yaml.push_str(
        "            \"--all-targets\",\n\
         \x20           \"--\",\n\
         \x20           \"-D\",\n\
         \x20           \"warnings\",\n\
         \x20         ]\n\n",
    );

    yaml.push_str(
        "  - repo: https://github.com/bnjbvr/cargo-machete\n\
         \x20   rev: v0.9.2\n\
         \x20   hooks:\n\
         \x20     - id: cargo-machete\n\n\
         \x20 - repo: https://github.com/EmbarkStudios/cargo-deny\n\
         \x20   rev: 0.19.4\n\
         \x20   hooks:\n\
         \x20     - id: cargo-deny\n\
         \x20       args: [\"check\"]\n\n",
    );

    // JavaScript/TypeScript: biome + oxlint
    if has(Language::Node) || has(Language::Wasm) {
        yaml.push_str(
            "  # JavaScript/TypeScript: biome (formatting + linting)\n\
             \x20 - repo: https://github.com/biomejs/pre-commit\n\
             \x20   rev: v2.4.12\n\
             \x20   hooks:\n\
             \x20     - id: biome-format\n\
             \x20     - id: biome-lint\n\n\
             \x20 - repo: https://github.com/oxc-project/mirrors-oxlint\n\
             \x20   rev: v1.60.0\n\
             \x20   hooks:\n\
             \x20     - id: oxlint\n\
             \x20       args: [\"--fix\"]\n\n",
        );
    }

    // C/C++ (for FFI tests)
    if has(Language::Ffi) {
        yaml.push_str(&format!(
            "  # C/C++: formatting and linting (FFI tests)\n\
             \x20 - repo: https://github.com/pocc/pre-commit-hooks\n\
             \x20   rev: v1.3.5\n\
             \x20   hooks:\n\
             \x20     - id: clang-format\n\
             \x20       args: [--style=file]\n\
             \x20       files: ^crates/{crate_dir}-ffi/tests/c/\n\
             \x20     - id: cppcheck\n\
             \x20       args:\n\
             \x20         [\n\
             \x20           \"--std=c11\",\n\
             \x20           \"--enable=warning,style,performance\",\n\
             \x20           \"--suppress=missingIncludeSystem\",\n\
             \x20           \"--suppress=unusedStructMember\",\n\
             \x20         ]\n\
             \x20       files: ^crates/{crate_dir}-ffi/tests/c/\n\n",
        ));
    }

    // Shell scripts
    yaml.push_str(
        "  # Shell scripts: formatting and linting\n\
         \x20 - repo: https://github.com/scop/pre-commit-shfmt\n\
         \x20   rev: v3.13.1-1\n\
         \x20   hooks:\n\
         \x20     - id: shfmt\n\
         \x20       args: [\"-w\", \"-i\", \"2\"]\n\n\
         \x20 - repo: https://github.com/koalaman/shellcheck-precommit\n\
         \x20   rev: v0.11.0\n\
         \x20   hooks:\n\
         \x20     - id: shellcheck\n\n",
    );

    // Markdown
    yaml.push_str(
        "  # Markdown\n\
         \x20 - repo: https://github.com/rvben/rumdl-pre-commit\n\
         \x20   rev: \"v0.1.72\"\n\
         \x20   hooks:\n\
         \x20     - id: rumdl-fmt\n\n",
    );

    // GitHub Actions
    yaml.push_str(
        "  # GitHub Actions\n\
         \x20 - repo: https://github.com/rhysd/actionlint\n\
         \x20   rev: v1.7.12\n\
         \x20   hooks:\n\
         \x20     - id: actionlint\n\n",
    );

    // Java: copy-paste detection and style checking
    if has(Language::Java) {
        yaml.push_str(
            "  # Java: copy-paste detection and style checking\n\
             \x20 - repo: https://github.com/gherynos/pre-commit-java\n\
             \x20   rev: v0.6.37\n\
             \x20   hooks:\n\
             \x20     - id: cpd\n\
             \x20     - id: checkstyle\n\
             \x20       args:\n\
             \x20         [\n\
             \x20           \"-c\",\n\
             \x20           \"packages/java/checkstyle.xml\",\n\
             \x20           \"-p\",\n\
             \x20           \"packages/java/checkstyle.properties\",\n\
             \x20         ]\n\n",
        );
    }

    // Local hooks for language toolchains
    let mut local_hooks: Vec<String> = Vec::new();

    if has(Language::Go) {
        local_hooks.push(
            "      - id: golangci-lint\n\
             \x20       name: golangci-lint\n\
             \x20       entry: bash -c 'cd packages/go && golangci-lint run ./...'\n\
             \x20       language: system\n\
             \x20       files: \\.go$\n\
             \x20       pass_filenames: false\n"
                .to_string(),
        );
    }

    if has(Language::Ruby) {
        local_hooks.push(
            "      - id: rbs-validate\n\
             \x20       name: rbs validate\n\
             \x20       entry: task ruby:rbs-validate\n\
             \x20       language: system\n\
             \x20       files: \\.(rb|rbs)$\n\
             \x20       pass_filenames: false\n\
             \x20       require_serial: true\n"
                .to_string(),
        );
        local_hooks.push(
            "      - id: steep-check\n\
             \x20       name: steep check\n\
             \x20       entry: task ruby:typecheck\n\
             \x20       language: system\n\
             \x20       files: \\.(rb|rbs)$\n\
             \x20       pass_filenames: false\n\
             \x20       require_serial: true\n"
                .to_string(),
        );
    }

    if has(Language::Php) {
        local_hooks.push(
            "      - id: php-lint\n\
             \x20       name: php lint (cs-fixer + phpstan)\n\
             \x20       entry: bash -c 'cd packages/php && composer install --no-interaction --no-progress && composer run lint:fix'\n\
             \x20       language: system\n\
             \x20       files: ^packages/php/\n\
             \x20       pass_filenames: false\n"
                .to_string(),
        );
    }

    if has(Language::Csharp) {
        let namespace = config.csharp_namespace();
        local_hooks.push(format!(
            "      - id: dotnet-format\n\
             \x20       name: dotnet format\n\
             \x20       entry: bash -c 'dotnet format packages/csharp/{namespace}/{namespace}.csproj'\n\
             \x20       language: system\n\
             \x20       files: ^packages/csharp/\n\
             \x20       pass_filenames: false\n",
        ));
    }

    if has(Language::Elixir) {
        local_hooks.push(
            "      - id: mix-credo\n\
             \x20       name: mix credo\n\
             \x20       entry: bash -c 'cd packages/elixir && MIX_ENV=dev mix credo --strict'\n\
             \x20       language: system\n\
             \x20       files: ^packages/elixir/\n\
             \x20       pass_filenames: false\n"
                .to_string(),
        );
        local_hooks.push(
            "      - id: mix-format\n\
             \x20       name: mix format\n\
             \x20       entry: bash -c 'cd packages/elixir && MIX_ENV=dev mix format --check-formatted'\n\
             \x20       language: system\n\
             \x20       files: ^packages/elixir/\n\
             \x20       pass_filenames: false\n"
                .to_string(),
        );
    }

    if has(Language::R) {
        local_hooks.push(
            "      - id: r-lintr\n\
             \x20       name: lintr (R)\n\
             \x20       entry: bash -c 'cd packages/r && Rscript -e \"lints <- lintr::lint_package(); if (length(lints) > 0) { print(lints); quit(status=1) }\"'\n\
             \x20       language: system\n\
             \x20       files: ^packages/r/\n\
             \x20       pass_filenames: false\n"
                .to_string(),
        );
        local_hooks.push(
            "      - id: r-styler\n\
             \x20       name: styler format check (R)\n\
             \x20       entry: bash -c 'cd packages/r && Rscript -e \"out <- styler::style_pkg(dry=\\\"on\\\"); if (!all(out\\$changed == FALSE)) quit(status=1)\"'\n\
             \x20       language: system\n\
             \x20       files: ^packages/r/\n\
             \x20       pass_filenames: false\n"
                .to_string(),
        );
    }

    if !local_hooks.is_empty() {
        yaml.push_str("  # Local hooks for language toolchains\n  - repo: local\n    hooks:\n");
        for hook in &local_hooks {
            yaml.push_str(hook);
        }
    }

    vec![GeneratedFile {
        path: PathBuf::from(".pre-commit-config.yaml"),
        content: yaml,
        generated_header: false,
    }]
}

/// Capitalize the first character of a string (for Elixir module names).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

fn scaffold_r(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let package_name = config.r_package_name();

    let mut description = meta.description.clone();
    if description.ends_with('.') {
        description.pop();
    }

    let authors_r = if meta.authors.is_empty() {
        r#"Authors@R: person("Author", "Name", email = "author@example.com", role = c("aut", "cre"))"#.to_string()
    } else {
        format!(
            "Authors@R: person(\"{}\", email = \"author@example.com\", role = c(\"aut\", \"cre\"))",
            meta.authors.first().unwrap_or(&"Author Name".to_string())
        )
    };

    let content = format!(
        r#"Package: {package}
Title: {title}
Version: {version}
{authors}
Description: {description}
    Rust bindings generated with extendr.
URL: {repository}
BugReports: {repository}/issues
License: {license}
Depends: R (>= 4.2)
Imports: jsonlite
Suggests:
    testthat (>= 3.0.0),
    withr,
    roxygen2
SystemRequirements: Cargo (Rust's package manager), rustc (>= 1.91)
Config/rextendr/version: 0.4.2
Encoding: UTF-8
Roxygen: list(markdown = TRUE)
RoxygenNote: 7.3.3
Config/testthat/edition: 3
"#,
        package = package_name,
        title = meta.description,
        version = version,
        authors = authors_r,
        description = description,
        repository = meta.repository,
        license = meta.license,
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/r/DESCRIPTION"),
        content,
        generated_header: true,
    }])
}

fn scaffold_r_cargo(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance();
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-r"),
        version,
        "2024",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    let content = format!(
        r#"{pkg_header}

[lib]
crate-type = ["cdylib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
extendr-api = {{ version = "0.7", features = ["use-precompiled-bindings"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
        pkg_header = pkg_header,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::R),
    );

    Ok(vec![GeneratedFile {
        path: PathBuf::from("packages/r/src/rust/Cargo.toml".to_string()),
        content,
        generated_header: true,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::*;

    fn test_config() -> AlefConfig {
        AlefConfig {
            crate_config: CrateConfig {
                name: "my-lib".to_string(),
                sources: vec![],
                version_from: "Cargo.toml".to_string(),
                core_import: None,
                workspace_root: None,
                skip_core_import: false,
                features: vec![],
                path_mappings: std::collections::HashMap::new(),
            },
            languages: vec![Language::Python, Language::Node],
            exclude: ExcludeConfig::default(),
            include: IncludeConfig::default(),
            output: OutputConfig::default(),
            python: None,
            node: None,
            ruby: None,
            php: None,
            elixir: None,
            wasm: None,
            ffi: None,
            go: None,
            java: None,
            csharp: None,
            r: None,
            scaffold: Some(ScaffoldConfig {
                description: Some("Test library".to_string()),
                license: Some("MIT".to_string()),
                repository: Some("https://github.com/test/my-lib".to_string()),
                homepage: None,
                authors: vec!["Alice".to_string()],
                keywords: vec!["test".to_string()],
            }),
            readme: None,
            lint: None,
            custom_files: None,
            adapters: vec![],
            custom_modules: CustomModulesConfig::default(),
            custom_registrations: CustomRegistrationsConfig::default(),
            opaque_types: std::collections::HashMap::new(),
            generate: GenerateConfig::default(),
            generate_overrides: std::collections::HashMap::new(),
            dto: Default::default(),
            sync: None,
            test: None,
            e2e: None,
        trait_bridges: vec![],
        }
    }

    fn test_api() -> ApiSurface {
        ApiSurface {
            crate_name: "my-lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        }
    }

    /// Filter out project-level scaffold files (like .pre-commit-config.yaml)
    /// to isolate language-specific scaffold tests.
    fn language_files(files: &[GeneratedFile]) -> Vec<&GeneratedFile> {
        files
            .iter()
            .filter(|f| !f.path.ends_with(".pre-commit-config.yaml"))
            .collect()
    }

    #[test]
    fn test_scaffold_python() {
        let config = test_config();
        let api = test_api();
        let all_files = scaffold(&api, &config, &[Language::Python]).unwrap();
        let files = language_files(&all_files);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("packages/python/pyproject.toml"));
        assert!(files[0].content.contains("maturin"));
        assert!(files[0].content.contains("my-lib"));
        assert_eq!(files[1].path, PathBuf::from("crates/my-lib-py/Cargo.toml"));
        assert!(files[1].content.contains("pyo3"));
    }

    #[test]
    fn test_scaffold_node() {
        let config = test_config();
        let api = test_api();
        let all_files = scaffold(&api, &config, &[Language::Node]).unwrap();
        let files = language_files(&all_files);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("packages/typescript/package.json"));
        assert!(files[0].content.contains("napi"));
        assert_eq!(files[1].path, PathBuf::from("crates/my-lib-node/Cargo.toml"));
        assert!(files[1].content.contains("napi-derive"));
    }

    #[test]
    fn test_scaffold_multiple() {
        let config = test_config();
        let api = test_api();
        let all_files = scaffold(&api, &config, &[Language::Python, Language::Node]).unwrap();
        let files = language_files(&all_files);
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn test_scaffold_python_production_features() {
        let config = test_config();
        let api = test_api();
        let files = scaffold(&api, &config, &[Language::Python]).unwrap();
        let content = &files[0].content;
        assert!(content.contains("[project.urls]"));
        assert!(content.contains("repository ="));
        // Linter config (ruff, mypy) is NOT generated — consumers configure in root pyproject.toml
        assert!(!content.contains("[tool.ruff]"));
    }

    #[test]
    fn test_scaffold_node_production_features() {
        let config = test_config();
        let api = test_api();
        let files = scaffold(&api, &config, &[Language::Node]).unwrap();
        let content = &files[0].content;
        assert!(content.contains("\"scripts\""));
        assert!(content.contains("\"build\""));
        assert!(content.contains("\"files\""));
        assert!(content.contains("\"devDependencies\""));
        assert!(content.contains("@napi-rs/cli"));
        assert!(content.contains("\"triples\""));
    }

    #[test]
    fn test_scaffold_ffi_with_core_import() {
        let config = test_config();
        let api = test_api();
        let all_files = scaffold(&api, &config, &[Language::Ffi]).unwrap();
        let files = language_files(&all_files);
        assert_eq!(files.len(), 2);
        let cargo_toml = &files[0].content;
        assert!(cargo_toml.contains("serde"));
        assert!(cargo_toml.contains("serde_json"));
        // Should have core_import as dependency
        assert!(cargo_toml.contains("my-lib ="));
        // Should generate cmake config
        let cmake = &files[1].content;
        assert!(cmake.contains("find_package"));
        assert!(cmake.contains("my-lib-ffi::my-lib-ffi"));
    }

    #[test]
    fn test_scaffold_go_production_format() {
        let config = test_config();
        let api = test_api();
        let all_files = scaffold(&api, &config, &[Language::Go]).unwrap();
        let files = language_files(&all_files);
        assert_eq!(files.len(), 1);
        let content = &files[0].content;
        assert!(content.contains("go 1.26"));
        assert!(!content.contains("require ("));
    }

    #[test]
    fn test_scaffold_java_production_features() {
        let config = test_config();
        let api = test_api();
        let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
        let files = language_files(&all_files);
        assert_eq!(files.len(), 1);
        let content = &files[0].content;
        assert!(content.contains("<properties>"));
        assert!(content.contains("<project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>"));
        assert!(content.contains("<dependencies>"));
        assert!(content.contains("<build>"));
        assert!(content.contains("maven-compiler-plugin"));
        assert!(content.contains("maven-surefire-plugin"));
        assert!(content.contains("--enable-native-access=ALL-UNNAMED"));
        assert!(content.contains("-Djava.library.path=${project.basedir}/../../target/release"));
    }

    #[test]
    fn test_scaffold_ruby_production_features() {
        let config = test_config();
        let api = test_api();
        let all_files = scaffold(&api, &config, &[Language::Ruby]).unwrap();
        let files = language_files(&all_files);
        assert_eq!(files.len(), 6);
        let content = &files[0].content;
        assert!(content.contains("spec.required_ruby_version"));
        assert!(content.contains("spec.extensions"));
        assert!(content.contains("spec.metadata['keywords']"));
        assert!(content.contains("frozen_string_literal: true"));
        assert!(content.contains("spec.metadata['rubygems_mfa_required'] = 'true'"));
        // Check for .rubocop.yml generation
        assert_eq!(files[1].path, PathBuf::from("packages/ruby/.rubocop.yml"));
        // Check for Rakefile generation
        assert_eq!(files[2].path, PathBuf::from("packages/ruby/Rakefile"));
        assert!(files[2].content.contains("Rake::ExtensionTask"));
        assert!(files[2].content.contains("my_lib_rb"));
        // Check for lib entry point generation
        assert_eq!(files[3].path, PathBuf::from("packages/ruby/lib/my_lib.rb"));
        assert!(files[3].content.contains("require 'my_lib_rb'"));
        // Check for extconf.rb generation
        assert_eq!(files[4].path, PathBuf::from("packages/ruby/ext/my_lib_rb/extconf.rb"));
        assert!(files[4].content.contains("create_rust_makefile"));
        assert!(files[4].content.contains("rb_sys/mkmf"));
        // Check for Cargo.toml generation
        assert_eq!(
            files[5].path,
            PathBuf::from("packages/ruby/ext/my-lib_rb/native/Cargo.toml")
        );
        assert!(files[5].content.contains("magnus"));
    }

    #[test]
    fn test_pre_commit_config_python_node() {
        let config = test_config();
        let files = generate_pre_commit_config(&config, &[Language::Python, Language::Node]);
        assert_eq!(files.len(), 1);
        let content = &files[0].content;
        // Common hooks always present
        assert!(content.contains("cargo-fmt"));
        assert!(content.contains("cargo-clippy"));
        assert!(content.contains("trailing-whitespace"));
        assert!(content.contains("cargo-deny"));
        // Python-specific
        assert!(content.contains("ruff-pre-commit"));
        assert!(content.contains("ruff-format"));
        assert!(content.contains("pyproject-fmt"));
        // Node-specific
        assert!(content.contains("biome-format"));
        assert!(content.contains("biome-lint"));
        assert!(content.contains("oxlint"));
        // Should NOT have PHP/Ruby/Go/etc hooks
        assert!(!content.contains("php-lint"));
        assert!(!content.contains("golangci-lint"));
        assert!(!content.contains("mix-credo"));
        assert!(!content.contains("rbs-validate"));
    }

    #[test]
    fn test_pre_commit_config_ffi_only() {
        let config = test_config();
        let files = generate_pre_commit_config(&config, &[Language::Ffi]);
        assert_eq!(files.len(), 1);
        let content = &files[0].content;
        // Common + Rust hooks
        assert!(content.contains("cargo-fmt"));
        assert!(content.contains("cargo-clippy"));
        // FFI-specific: C/C++ hooks
        assert!(content.contains("clang-format"));
        assert!(content.contains("cppcheck"));
        // No Python/Node hooks
        assert!(!content.contains("ruff"));
        assert!(!content.contains("biome"));
    }

    #[test]
    fn test_pre_commit_config_clippy_excludes() {
        let config = test_config();
        let files = generate_pre_commit_config(
            &config,
            &[Language::Python, Language::Node, Language::Php, Language::Wasm],
        );
        let content = &files[0].content;
        assert!(content.contains("--exclude=my-lib-py"));
        assert!(content.contains("--exclude=my-lib-node"));
        assert!(content.contains("--exclude=my-lib-php"));
        assert!(content.contains("--exclude=my-lib-wasm"));
        // Ruby not in languages, should not be excluded
        assert!(!content.contains("--exclude=my-lib-rb"));
    }

    #[test]
    fn test_pre_commit_config_all_languages() {
        let config = test_config();
        let files = generate_pre_commit_config(
            &config,
            &[
                Language::Python,
                Language::Node,
                Language::Ruby,
                Language::Php,
                Language::Ffi,
                Language::Go,
                Language::Java,
                Language::Csharp,
                Language::Elixir,
                Language::R,
            ],
        );
        let content = &files[0].content;
        // All language hooks should be present
        assert!(content.contains("ruff"));
        assert!(content.contains("biome"));
        assert!(content.contains("clang-format"));
        assert!(content.contains("golangci-lint"));
        assert!(content.contains("cpd")); // Java
        assert!(content.contains("dotnet-format"));
        assert!(content.contains("mix-credo"));
        assert!(content.contains("rbs-validate"));
        assert!(content.contains("php-lint"));
        assert!(content.contains("r-lintr"));
        assert!(content.contains("r-styler"));
    }
}
