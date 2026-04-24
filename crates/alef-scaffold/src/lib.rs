//! Package scaffolding generator for alef.

use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;

mod languages;

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
pub(crate) fn detect_workspace_inheritance(workspace_root: Option<&std::path::Path>) -> WorkspacePackageInheritance {
    let cargo_toml_path = workspace_root
        .map(|r| r.join("Cargo.toml"))
        .unwrap_or_else(|| std::path::PathBuf::from("Cargo.toml"));
    let Ok(contents) = std::fs::read_to_string(&cargo_toml_path) else {
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
pub(crate) fn cargo_package_header(
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

/// e.g., "0.1.0-rc.1" -> "0.1.0rc1", "0.1.0-alpha.2" -> "0.1.0a2", "0.1.0-beta.3" -> "0.1.0b3"
/// Non-pre-release versions are returned unchanged.
pub(crate) fn to_pep440(version: &str) -> String {
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

///
/// Merges crate-level `extra_dependencies` with per-language overrides via
/// `extra_deps_for_language`, then serializes each entry as a TOML line suitable
/// for appending to a `[dependencies]` section.
///
/// Each value is either:
/// - A string (version only): `cratename = "1.0"`
/// - A TOML table (with path/features/etc.): `cratename = { path = "../foo", features = ["bar"] }`
///
/// Returns an empty string if no extra dependencies are configured.
pub(crate) fn render_extra_deps(config: &AlefConfig, lang: Language) -> String {
    let deps = config.extra_deps_for_language(lang);
    if deps.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = deps
        .iter()
        .map(|(name, value)| match value {
            toml::Value::String(version) => format!("{name} = \"{version}\""),
            other => {
                // Serialize as inline TOML table. toml::to_string wraps in a [table] header,
                // so we use the Display of the Value directly which gives the inline form.
                format!("{name} = {other}")
            }
        })
        .collect();
    // Sort for deterministic output.
    lines.sort();
    lines.join("\n")
}

///
/// Checks for per-language feature overrides first, then falls back to `[crate] features`.
/// Returns an empty string if no features are configured, otherwise returns
/// `, features = ["feat1", "feat2"]`.
pub(crate) fn core_dep_features(config: &AlefConfig, lang: Language) -> String {
    let features = config.features_for_language(lang);
    if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    }
}

pub fn scaffold(api: &ApiSurface, config: &AlefConfig, languages: &[Language]) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![];
    for &lang in languages {
        files.extend(scaffold_language(api, config, lang)?);
    }
    // Project-level files that depend on the full set of configured languages
    files.extend(scaffold_pre_commit_config(config, languages));

    // Typos configuration (spell checker)
    if !std::path::Path::new(".typos.toml").exists() {
        files.push(GeneratedFile {
            path: std::path::PathBuf::from(".typos.toml"),
            content: "[files]\nextend-exclude = [\"target/\", \".alef/\", \"*.lock\", \"*.min.js\"]\n\n[default.extend-words]\n# Add project-specific words here\n# crate_name = \"crate_name\"\n".to_string(),
            generated_header: false,
        });
    }
    Ok(files)
}

pub(crate) struct ScaffoldMeta {
    description: String,
    license: String,
    repository: String,
    homepage: String,
    authors: Vec<String>,
    keywords: Vec<String>,
}

pub(crate) fn scaffold_meta(config: &AlefConfig) -> ScaffoldMeta {
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

pub(crate) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

use languages::{
    scaffold_csharp, scaffold_elixir, scaffold_elixir_cargo, scaffold_ffi, scaffold_go, scaffold_java, scaffold_node,
    scaffold_node_cargo, scaffold_php, scaffold_php_cargo, scaffold_pre_commit_config, scaffold_python,
    scaffold_python_cargo, scaffold_r, scaffold_r_cargo, scaffold_ruby, scaffold_ruby_cargo, scaffold_wasm,
};

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

#[cfg(test)]
mod tests;
