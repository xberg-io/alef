//! Crate manifest header rendering: workspace-package-inheritance detection, scaffold
//! metadata, and the `[package]` header line assembly that consumes both.

use crate::core::config::ResolvedCrateConfig;

/// Fields available via `[workspace.package]` inheritance detected from the root `Cargo.toml`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkspacePackageInheritance {
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

/// The `[workspace.package]` inheritance fields declared by the `Cargo.toml` at `dir`,
/// or `None` when the file is missing/unparseable, has no `[workspace]` table at all, or
/// has a `[workspace]` table with no `[workspace.package]` (an empty self-hosted
/// workspace root — a generated crate may declare a bare `[workspace]` to isolate itself
/// from an outer workspace's resolver without ever declaring its own inheritable fields;
/// that root still cannot satisfy `<field>.workspace = true`). ~keep
fn read_workspace_package_fields(dir: &std::path::Path) -> Option<WorkspacePackageInheritance> {
    let contents = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
    let doc = toml::from_str::<toml::Value>(&contents).ok()?;
    let pkg = doc.get("workspace")?.get("package")?;
    Some(WorkspacePackageInheritance {
        version: pkg.get("version").is_some(),
        readme: pkg.get("readme").is_some(),
        keywords: pkg.get("keywords").is_some(),
        categories: pkg.get("categories").is_some(),
        license: pkg.get("license").is_some(),
    })
}

/// True when `crate_relative_dir` (forward-slash, relative to the workspace root) is
/// named in `root_doc`'s `[workspace] exclude` list — an exact match or a match against
/// an ancestor directory entry. This is not a full implementation of Cargo's
/// gitignore-style workspace-exclude glob syntax; it is sufficient for the literal
/// directory paths alef's own `exclude` generators, and every observed consumer
/// `Cargo.toml`, actually write to this list (no wildcards). ~keep
fn crate_dir_is_excluded(root_doc: &toml::Value, crate_relative_dir: &str) -> bool {
    let Some(excludes) = root_doc
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(|e| e.as_array())
    else {
        return false;
    };
    let normalized = crate_relative_dir.trim_matches('/');
    excludes.iter().filter_map(|entry| entry.as_str()).any(|pattern| {
        let pattern = pattern.trim_matches('/');
        normalized == pattern || normalized.starts_with(&format!("{pattern}/"))
    })
}

/// Detect which `[workspace.package]` fields a *specific* generated crate can actually
/// reach. This is the only detector: a caller must name the crate, because inheritance is
/// a property of the crate's membership, never of the workspace alone.
///
/// A crate can inherit a field only if it can reach a `[workspace.package]` that defines
/// it:
/// - `crate_relative_dir` is a member of the workspace rooted at `workspace_root` — i.e.
///   not named in that root's `[workspace] exclude` — and that root declares the field
///   under `[workspace.package]`; or
/// - the crate's own pre-existing manifest at `<workspace_root>/<crate_relative_dir>/Cargo.toml`
///   self-hosts a `[workspace.package]` that defines the field (it declares its own
///   `[workspace]` table, making it its own workspace root).
///
/// Neither holding means every field is reported absent, so [`cargo_package_header`]
/// falls back to literals — the alternative, blindly trusting the root's
/// `[workspace.package]` regardless of exclusion, emits `<field>.workspace = true` into a
/// manifest that can never resolve it, which fails `cargo metadata` outright for any
/// crate excluded from the root workspace (Elixir NIF / Ruby native-extension crates are
/// excluded so their own toolchain, not the root workspace's resolver, builds them). ~keep
pub(crate) fn detect_workspace_inheritance_for_crate(
    workspace_root: Option<&std::path::Path>,
    crate_relative_dir: &str,
) -> WorkspacePackageInheritance {
    let Some(root) = workspace_root else {
        return WorkspacePackageInheritance::default();
    };
    let root_reaches = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|contents| toml::from_str::<toml::Value>(&contents).ok())
        .filter(|doc| doc.get("workspace").is_some())
        .is_some_and(|doc| !crate_dir_is_excluded(&doc, crate_relative_dir));
    if root_reaches && let Some(inheritance) = read_workspace_package_fields(root) {
        return inheritance;
    }
    read_workspace_package_fields(&root.join(crate_relative_dir)).unwrap_or_default()
}

/// Build the `[package]` header fields for a binding crate Cargo.toml.
///
/// Uses `*.workspace = true` for any field that is available in `[workspace.package]`,
/// falling back to explicit values otherwise.
pub(crate) fn cargo_package_header(
    name: &str,
    version: &str,
    edition: &str,
    meta: &ScaffoldMeta,
    ws: &WorkspacePackageInheritance,
) -> String {
    let version_line = if ws.version {
        "version.workspace = true".to_string()
    } else {
        format!("version = \"{version}\"")
    };
    let edition_line = format!("edition = \"{edition}\"");
    let license_line = if ws.license {
        Some("license.workspace = true".to_string())
    } else {
        meta.license.as_ref().map(|license| format!("license = \"{license}\""))
    };
    let readme_line = if ws.readme {
        "readme.workspace = true".to_string()
    } else {
        "readme = false".to_string()
    };
    let keywords_line = if ws.keywords {
        "keywords.workspace = true".to_string()
    } else if meta.keywords.is_empty() {
        "keywords = []".to_string()
    } else {
        let quoted: Vec<String> = meta.keywords.iter().map(|k| format!("\"{k}\"")).collect();
        format!("keywords = [{}]", quoted.join(", "))
    };
    let categories_line = if ws.categories {
        "categories.workspace = true".to_string()
    } else if meta.categories.is_empty() {
        "categories = []".to_string()
    } else {
        let quoted: Vec<String> = meta.categories.iter().map(|k| format!("\"{k}\"")).collect();
        format!("categories = [{}]", quoted.join(", "))
    };

    let mut lines = vec![
        "[package]".to_string(),
        format!("name = \"{name}\""),
        version_line,
        edition_line,
        format!("description = \"{}\"", meta.description),
        readme_line,
        keywords_line,
        categories_line,
    ];
    if let Some(license_line) = license_line {
        lines.insert(4, license_line);
    }
    lines.join("\n")
}

pub struct ScaffoldMeta {
    pub description: String,
    pub license: Option<String>,
    pub repository: Option<String>,
    pub configured_repository: Option<String>,
    pub homepage: String,
    pub documentation: String,
    pub issues: String,
    pub funding: String,
    pub authors: Vec<String>,
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
}

pub fn scaffold_meta(config: &ResolvedCrateConfig) -> ScaffoldMeta {
    let scaffold = config.scaffold.as_ref();
    let package = config.package_metadata.as_ref();
    let truncate = package.map(|p| p.truncate_registry_lists).unwrap_or(false);
    let configured_repository = package
        .and_then(|p| p.repository.clone())
        .or_else(|| scaffold.and_then(|s| s.repository.clone()));
    let mut keywords = package
        .filter(|p| !p.keywords.is_empty())
        .map(|p| p.keywords.clone())
        .or_else(|| scaffold.map(|s| s.keywords.clone()))
        .unwrap_or_default();
    let mut categories = package.map(|p| p.categories.clone()).unwrap_or_default();
    keywords.sort();
    categories.sort();
    if truncate {
        keywords.truncate(5);
        categories.truncate(5);
    }
    ScaffoldMeta {
        description: package
            .and_then(|p| p.description.clone())
            .or_else(|| scaffold.and_then(|s| s.description.clone()))
            .unwrap_or_else(|| format!("Bindings for {}", config.name)),
        license: package
            .and_then(|p| p.license.clone())
            .or_else(|| scaffold.and_then(|s| s.license.clone())),
        repository: configured_repository.clone(),
        configured_repository,
        homepage: package
            .and_then(|p| p.homepage.clone())
            .or_else(|| scaffold.and_then(|s| s.homepage.clone()))
            .unwrap_or_default(),
        documentation: package.and_then(|p| p.documentation.clone()).unwrap_or_default(),
        issues: package.and_then(|p| p.issues.clone()).unwrap_or_default(),
        funding: package.and_then(|p| p.funding.clone()).unwrap_or_default(),
        authors: package
            .filter(|p| !p.authors.is_empty())
            .map(|p| p.authors.clone())
            .or_else(|| scaffold.map(|s| s.authors.clone()))
            .unwrap_or_default(),
        keywords,
        categories,
    }
}

/// Returns true when `crates.readme.languages.<lang_code>` is configured for this
/// crate, meaning the README module in [`crate::readme`] owns `packages/<lang>/README.md`
/// end-to-end (badges, "What This Package Provides", Quick Start, feature/OCR
/// sections, snippets).
///
/// A handful of scaffold language modules (currently Swift, Dart, Zig) also emit a
/// minimal placeholder `README.md` alongside their package skeleton, predating the
/// languages having any `[crates.readme.languages.*]` entry at all. That placeholder
/// is a second, independent writer for the exact output path the README module
/// targets: `alef all --clean` always has the README stage overwrite it afterwards,
/// but any run that only performs scaffolding (`alef scaffold`, a `--lang`-scoped
/// scaffold-only pass, or a run that errors out before reaching the README stage)
/// leaves the placeholder as the final, committed content — silently discarding
/// every section the crate's `alef.toml` configured, with no error and no diff
/// signal (#555). Scaffold modules must call this and skip emitting their own
/// `README.md` once the language has real README config, so there is only ever one
/// writer for that path and the file is either the fully rendered template or (for
/// an as-yet-unconfigured language) the historical placeholder — never a silent mix
/// of the two depending on which command happened to run last. ~keep
pub(crate) fn readme_language_configured(config: &ResolvedCrateConfig, lang_code: &str) -> bool {
    config
        .readme
        .as_ref()
        .is_some_and(|readme| readme.languages.contains_key(lang_code))
}
