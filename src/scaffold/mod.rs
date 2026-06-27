//! Package scaffolding generator for alef.

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig, ScaffoldCargo, ScaffoldCargoEnvValue};
use crate::core::ir::ApiSurface;

mod languages;
pub(crate) mod naming;
mod template_env;

pub use languages::render_csharp_csproj;

/// Fields available via `[workspace.package]` inheritance detected from the root `Cargo.toml`.
#[derive(Debug, Default)]
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

/// Render a workspace-member core-facade dependency line in DUAL FORM.
///
/// Emits `crate_name = { version = "<version>", path = "<rel_path>"<features> }`.
/// The dual form keeps in-repo dev path builds working (the `path` is always
/// honored when the member crate is present on disk) while letting cargo's
/// package/publish flows (e.g. `maturin sdist`, `cargo package`) strip the
/// `path` and resolve the crate from the registry at `version`.
///
/// `features` is the already-formatted suffix as produced by
/// [`core_dep_features`] — either empty or `, features = ["a", "b"]`. It is
/// appended verbatim so callers control feature selection.
///
/// `version` is the resolved workspace version (the same value used for the
/// generated crate's `[package].version` and by version-sync). The `path` is
/// never altered, so dev builds against the local workspace continue to work.
/// When `version` is empty (no resolvable workspace version, e.g. some unit
/// fixtures), the line falls back to the path-only form so no invalid
/// `version = ""` is emitted.
pub(crate) fn render_core_dep(crate_name: &str, rel_path: &str, features: &str, version: &str) -> String {
    if version.is_empty() {
        format!("{crate_name} = {{ path = \"{rel_path}\"{features} }}")
    } else {
        format!("{crate_name} = {{ version = \"{version}\", path = \"{rel_path}\"{features} }}")
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
/// Workspace members: when an entry is a path-only table (a `path` key, no
/// `version` key) whose crate name resolves to a workspace member, the resolved
/// workspace version is injected so the table becomes
/// `{ path = "../foo", version = "<v>" }` (dual form). This mirrors
/// [`render_core_dep`] for the core facade and lets cargo-package flows strip
/// the path to a registry version-dependency. `alef.toml` entries stay
/// path-only — the version is injected here at scaffold time. Non-member
/// external deps (e.g. `anyhow = "1.0"`) are emitted unchanged.
///
/// Returns an empty string if no extra dependencies are configured.
pub(crate) fn render_extra_deps(config: &ResolvedCrateConfig, lang: Language) -> String {
    let deps = config.extra_deps_for_language(lang);
    if deps.is_empty() {
        return String::new();
    }
    let member_versions = workspace_member_versions(config);
    let mut lines: Vec<String> = deps
        .iter()
        .map(|(name, value)| match value {
            toml::Value::String(version) => format!("{name} = \"{version}\""),
            toml::Value::Table(table) => {
                // Inject the resolved workspace version into path-only member
                // tables so cargo-package flows can resolve them from the
                // registry. Leave non-members and already-versioned tables as-is.
                let needs_version = table.contains_key("path") && !table.contains_key("version");
                if let (true, Some(member_version)) = (needs_version, member_versions.get(name)) {
                    let mut injected = table.clone();
                    injected.insert("version".to_string(), toml::Value::String(member_version.clone()));
                    format!("{name} = {}", toml::Value::Table(injected))
                } else {
                    format!("{name} = {value}")
                }
            }
            other => format!("{name} = {other}"),
        })
        .collect();
    // Sort for deterministic output.
    lines.sort();
    lines.join("\n")
}

/// Resolve the workspace-member crate name → version map for the crate's
/// workspace root.
///
/// Returns an empty map when no workspace root is configured or the root
/// `Cargo.toml` cannot be discovered/parsed — in that case no version is
/// injected and path-only deps are emitted unchanged (matching dev behavior
/// outside a resolvable workspace, e.g. unit tests).
fn workspace_member_versions(config: &ResolvedCrateConfig) -> std::collections::BTreeMap<String, String> {
    let Some(root) = config.workspace_root.as_deref() else {
        return std::collections::BTreeMap::new();
    };
    match crate::publish::workspace::workspace_member_crates(root) {
        Ok(members) => members.versions,
        Err(_) => std::collections::BTreeMap::new(),
    }
}

///
/// Checks for per-language feature overrides first, then falls back to `[crate] features`.
/// Returns an empty string if no features are configured, otherwise returns
/// `, features = ["feat1", "feat2"]`.
pub(crate) fn core_dep_features(config: &ResolvedCrateConfig, lang: Language) -> String {
    let features = config.features_for_language(lang);
    if features.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = features.iter().map(|f| format!("\"{f}\"")).collect();
        format!(", features = [{}]", quoted.join(", "))
    }
}

/// Locate the core crate's `Cargo.toml` for a resolved config.
///
/// Derives the crate directory from the first source path (walking up to the
/// `src/` parent, mirroring [`ResolvedCrateConfig::core_crate_dir`]) and joins
/// it against `workspace_root`. Returns `None` when there is no workspace root
/// (the binding is being scaffolded standalone) or when the path cannot be
/// derived — both cases simply skip the `android-target` aggregate emission.
fn core_crate_manifest_path(config: &ResolvedCrateConfig) -> Option<std::path::PathBuf> {
    let workspace_root = config.workspace_root.as_deref()?;
    let first_source = config.sources.first()?;
    // Walk up from the source file to its enclosing crate directory (the parent
    // of the `src/` directory), e.g. `crates/foo/src/lib.rs` -> `crates/foo`.
    let mut current = std::path::Path::new(first_source).parent();
    while let Some(dir) = current {
        if dir.file_name().is_some_and(|n| n == "src") {
            let crate_dir = dir.parent()?;
            return Some(workspace_root.join(crate_dir).join("Cargo.toml"));
        }
        current = dir.parent();
    }
    None
}

/// Resolve a core-crate aggregate feature to the transitive set of feature-name
/// tokens reachable from it.
///
/// BFS over the core crate's `[features]` map starting at `aggregate`. Every
/// member token that is itself a key in the map is followed; tokens of the
/// `dep:foo` or `crate/feat` form are skipped (they are not binding-side
/// passthrough features). The returned set therefore contains every plain
/// feature name reachable from the aggregate, including the aggregate's own
/// sub-aggregates' leaves. Returns `None` when the core crate has no feature by
/// that name (so callers can skip emission for repos that lack it).
fn resolve_core_aggregate_features(
    features_table: &toml::value::Table,
    aggregate: &str,
) -> Option<std::collections::BTreeSet<String>> {
    let _ = features_table.get(aggregate)?;
    let mut reachable: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut queue: Vec<String> = vec![aggregate.to_string()];
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(name) = queue.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(members) = features_table.get(&name).and_then(|v| v.as_array()) else {
            continue;
        };
        for member in members {
            let Some(token) = member.as_str() else { continue };
            // Skip `dep:foo` activations and `crate/feat` cross-crate forwards —
            // neither is a binding-side passthrough feature name.
            if token.starts_with("dep:") || token.contains('/') {
                continue;
            }
            reachable.insert(token.to_string());
            if features_table.contains_key(token) {
                queue.push(token.to_string());
            }
        }
    }
    Some(reachable)
}

/// Compute the binding-crate `android-target` aggregate feature line, if applicable.
///
/// The consuming repo's core crate may define an `android-target` aggregate (a
/// curated ORT-free, libheif-free feature set) so it can be cross-compiled for
/// Android via `cargo ndk ... --no-default-features --features android-target`.
/// The binding crate's own FFI exports are gated by its passthrough features, so
/// it must expose a matching `android-target` that enables the passthrough
/// features that are members of the core aggregate — not merely forward to the
/// core dep.
///
/// `passthrough_feature_names` is the binding crate's own forwarding feature set
/// (the names that appear in its `[features]` passthrough block, e.g. `pdf`,
/// `ocr`), excluding the `full` umbrella. Returns the emitted line:
///
/// ```text
/// android-target = ["<core>/android-target", <sorted passthrough ∩ core aggregate>]
/// ```
///
/// Returns `None` when the core crate has no `android-target` feature (so other
/// consuming repos are unaffected) or when its manifest cannot be read.
pub(crate) fn android_target_feature_line(
    config: &ResolvedCrateConfig,
    passthrough_feature_names: &[&str],
) -> Option<String> {
    android_target_feature_line_for_dep(config, &config.name, passthrough_feature_names)
}

/// Variant of [`android_target_feature_line`] that takes the core-crate cargo
/// dep key explicitly.
///
/// The FFI crate forwards via the cargo package name (`config.name`), whereas
/// the dart bridge crate forwards via the rust-ident dep key (e.g. `sample_lib`)
/// to match its other passthrough entries. Both share the same resolution logic.
pub(crate) fn android_target_feature_line_for_dep(
    config: &ResolvedCrateConfig,
    core_dep_key: &str,
    passthrough_feature_names: &[&str],
) -> Option<String> {
    let manifest_path = core_crate_manifest_path(config)?;
    let contents = std::fs::read_to_string(&manifest_path).ok()?;
    let doc = toml::from_str::<toml::Value>(&contents).ok()?;
    let features_table = doc.get("features")?.as_table()?;
    let aggregate_members = resolve_core_aggregate_features(features_table, "android-target")?;

    // Intersect the binding's passthrough features with the core aggregate's
    // reachable members. `full` is never a member (it is the umbrella the
    // android target deliberately excludes) and is filtered defensively.
    let mut selected: Vec<&str> = passthrough_feature_names
        .iter()
        .copied()
        .filter(|name| *name != "full" && aggregate_members.contains(*name))
        .collect();
    selected.sort_unstable();
    selected.dedup();

    let mut tokens: Vec<String> = vec![format!("{core_dep_key}/android-target")];
    tokens.extend(selected.iter().map(|name| (*name).to_string()));
    let list = tokens.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ");
    Some(format!("android-target = [{list}]"))
}

pub fn scaffold(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![];
    for &lang in languages {
        files.extend(scaffold_language(api, config, lang)?);
    }
    // Project-level files that depend on the full set of configured languages
    files.extend(scaffold_pre_commit_config(config, languages));

    // LICENSE sync — copy the workspace-root LICENSE into every per-language
    // package directory so ecosystems like pub.dev (Dart) that require a LICENSE
    // in the package root can publish successfully. Skips gracefully when no
    // LICENSE file is present at the workspace root.
    files.extend(scaffold_license_files(config, languages));

    // rust-toolchain.toml — pin Rust version, include wasm32 target when wasm is configured
    if !std::path::Path::new("rust-toolchain.toml").exists() {
        let targets = if languages.contains(&Language::Wasm) {
            "\ntargets = [\"wasm32-unknown-unknown\"]\n"
        } else {
            "\n"
        };
        files.push(GeneratedFile {
            path: std::path::PathBuf::from("rust-toolchain.toml"),
            content: format!(
                "[toolchain]\nchannel = \"1.95\"\ncomponents = [\"rust-src\", \"rustfmt\", \"clippy\"]\n{targets}"
            ),
            generated_header: false,
        });
    }

    // .cargo/config.toml
    //
    // Two modes, gated by `[scaffold.cargo]` in alef.toml:
    //
    //   * Opted in (`[scaffold.cargo]` present): alef writes the full canonical file
    //     with hash-based drift detection. Includes macOS dynamic_lookup rustflag
    //     (required for PyO3/ext-php-rs cdylibs to link on macOS), Windows MSVC
    //     rust-lld, aarch64-linux-gnu cross-gcc, x86_64-linux-musl, and the wasm32
    //     bulk-memory + getrandom_backend cfg. Per-target opt-out via
    //     `[scaffold.cargo.targets]`; repo-specific `[env]` via `[scaffold.cargo.env]`.
    //
    //   * Legacy (no `[scaffold.cargo]`): create-if-missing, wasm32-only block. No
    //     hash, no overwrite. Matches behavior prior to alef 0.13.6.
    if let Some(cargo) = config.scaffold.as_ref().and_then(|s| s.cargo.as_ref()) {
        files.push(GeneratedFile {
            path: std::path::PathBuf::from(".cargo/config.toml"),
            content: render_cargo_config(cargo),
            generated_header: true,
        });
    } else if languages.contains(&Language::Wasm) && !std::path::Path::new(".cargo/config.toml").exists() {
        files.push(GeneratedFile {
            path: std::path::PathBuf::from(".cargo/config.toml"),
            content: "[build]\nincremental = true\n\n[target.wasm32-unknown-unknown]\nrustflags = [\"-C\", \"target-feature=+bulk-memory\", \"--cfg\", \"getrandom_backend=\\\"wasm_js\\\"\"]\n\n[net]\ngit-fetch-with-cli = true\n\n[registries.crates-io]\nprotocol = \"sparse\"\n".to_string(),
            generated_header: false,
        });
    }

    // Typos configuration (spell checker)
    if !std::path::Path::new(".typos.toml").exists() {
        files.push(GeneratedFile {
            path: std::path::PathBuf::from(".typos.toml"),
            content: "[files]\nextend-exclude = [\"target/\", \".alef/\", \"*.lock\", \"*.min.js\"]\n\n[default.extend-words]\n# Add project-specific words here\n# crate_name = \"crate_name\"\n".to_string(),
            generated_header: false,
        });
    }

    // .gitattributes — mark all generated output directories as linguist-generated
    // so GitHub collapses them in PR diffs. create-once seed; skipped if the file
    // already exists so hand-added entries (e.g. `* text=auto`) are preserved.
    files.extend(scaffold_gitattributes(config, languages));

    Ok(files)
}

/// Render the canonical workspace `.cargo/config.toml` from a `[scaffold.cargo]`
/// configuration block.
///
/// The output is deterministic (same config → byte-identical output) and includes
/// the `auto-generated by alef` marker so `finalize_hashes` will stamp the
/// `alef:hash:` line during the scaffold pipeline.
///
/// Section order is fixed: header comment → `[build]` → `[net]` →
/// `[registries.crates-io]` → `[target.*]` blocks (in declaration order:
/// macOS dynamic_lookup, Windows MSVC x64+i686, aarch64-linux-gnu, x86_64-linux-musl,
/// wasm32) → optional `[env]`. `inject_hash_line` will insert the hash comment
/// directly after the marker line.
pub fn render_cargo_config(cargo: &ScaffoldCargo) -> String {
    let mut out = String::new();
    out.push_str("# This file is auto-generated by alef. DO NOT EDIT.\n");
    out.push_str("# Re-generate with: alef scaffold\n");
    out.push('\n');
    out.push_str("[build]\nincremental = true\n");
    if cargo.build_jobs > 0 {
        out.push_str(&format!("jobs = {}\n", cargo.build_jobs));
    }
    if let Some(wrapper) = cargo.rustc_wrapper.as_deref() {
        out.push_str(&format!("rustc-wrapper = \"{}\"\n", escape_toml_string(wrapper)));
    }
    out.push('\n');
    out.push_str("[net]\ngit-fetch-with-cli = true\n\n");
    out.push_str("[registries.crates-io]\nprotocol = \"sparse\"\n");

    let t = &cargo.targets;
    if t.macos_dynamic_lookup {
        out.push_str(
            "\n# Required for PyO3 / ext-php-rs cdylibs: Python and Zend C-API symbols are\n\
             # resolved at runtime when the host loads the extension, not at link time.\n\
             # macOS ld is strict and rejects unresolved symbols by default.\n\
             [target.'cfg(target_os = \"macos\")']\n\
             rustflags = [\"-C\", \"link-arg=-Wl,-undefined,dynamic_lookup\"]\n",
        );
    }
    if t.x86_64_pc_windows_msvc {
        out.push_str("\n[target.x86_64-pc-windows-msvc]\nlinker = \"rust-lld\"\n");
    }
    if t.i686_pc_windows_msvc {
        out.push_str("\n[target.i686-pc-windows-msvc]\nlinker = \"rust-lld\"\n");
    }
    if t.aarch64_unknown_linux_gnu {
        out.push_str("\n[target.aarch64-unknown-linux-gnu]\nlinker = \"aarch64-linux-gnu-gcc\"\n");
    }
    if t.x86_64_unknown_linux_musl {
        out.push_str("\n[target.x86_64-unknown-linux-musl]\nlinker = \"musl-gcc\"\n");
    }
    if t.wasm32_unknown_unknown {
        out.push_str(
            "\n[target.wasm32-unknown-unknown]\n\
             rustflags = [\"-C\", \"target-feature=+bulk-memory\", \"--cfg\", \"getrandom_backend=\\\"wasm_js\\\"\"]\n",
        );
    }

    if !cargo.env.is_empty() {
        out.push_str("\n[env]\n");
        let mut keys: Vec<&String> = cargo.env.keys().collect();
        keys.sort();
        for key in keys {
            let value = &cargo.env[key];
            match value {
                ScaffoldCargoEnvValue::Plain(s) => {
                    out.push_str(&template_env::render(
                        "cargo_env_plain.jinja",
                        minijinja::context! { key => key, value => escape_toml_string(s) },
                    ));
                }
                ScaffoldCargoEnvValue::Structured { value, relative } => {
                    out.push_str(&template_env::render(
                        "cargo_env_structured.jinja",
                        minijinja::context! {
                            key => key,
                            value => escape_toml_string(value),
                            relative => relative,
                        },
                    ));
                }
            }
        }
    }

    out
}

/// Escape a string for TOML basic-string syntax: backslash + double-quote only.
/// (Tabs/newlines are preserved as-is — typical Cargo config values don't contain them.)
fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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

/// Escape special characters for XML text content.
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Parse an author string like `"Name <email>"` into `(name, email)`.
/// If no angle brackets are found, returns `(input, "")`.
pub fn parse_author(s: &str) -> (&str, &str) {
    if let Some(start) = s.find('<') {
        if let Some(end) = s.find('>') {
            let name = s[..start].trim();
            let email = &s[start + 1..end];
            return (name, email);
        }
    }
    (s.trim(), "")
}

pub(crate) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Copy the workspace-root `LICENSE` file into each per-language package directory.
///
/// Reads `<workspace_root>/LICENSE` (falling back to `./LICENSE` when no workspace root is
/// configured). When the file is absent, this function warns and returns an empty list so
/// the caller can continue without error.
///
/// Emits one `GeneratedFile` per unique package directory that the languages list would
/// populate. Files with `generated_header: false` so they are create-once seeds —
/// `write_scaffold_files` skips them if they already exist, which keeps the copy
/// idempotent and `alef verify` happy (the file carries no `alef:hash:` marker).
///
/// Languages that do not produce a publishable package directory (Rust, C, FFI, JNI)
/// are skipped.
fn scaffold_license_files(config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    // Determine the path of the root LICENSE file.
    let license_path = config
        .workspace_root
        .as_deref()
        .map(|r| r.join("LICENSE"))
        .unwrap_or_else(|| std::path::PathBuf::from("LICENSE"));

    let license_content = match std::fs::read_to_string(&license_path) {
        Ok(content) => content,
        Err(_) => {
            tracing::warn!(
                "No LICENSE file found at {} — skipping LICENSE sync into package directories",
                license_path.display()
            );
            return vec![];
        }
    };

    // Collect unique package directories from publishable languages.
    // Languages without a real package output (Rust, C, FFI, JNI) are excluded.
    let mut seen = std::collections::BTreeSet::new();
    let mut files = vec![];

    for &lang in languages {
        match lang {
            // These languages do not produce a standalone publishable package directory.
            Language::Rust | Language::C | Language::Ffi | Language::Jni => continue,
            _ => {}
        }

        let pkg_dir = config.package_dir(lang);
        if seen.insert(pkg_dir.clone()) {
            files.push(GeneratedFile {
                path: std::path::PathBuf::from(format!("{pkg_dir}/LICENSE")),
                content: license_content.clone(),
                generated_header: false,
            });
        }
    }

    files
}

/// Emit a root-level `.gitattributes` that marks all generated output directories as
/// `linguist-generated=true`, causing GitHub to collapse them in PR diffs.
///
/// Covers three path categories:
/// - `packages/{lang}/` — language-native packages (Python, Ruby, PHP, Go, Java, …)
/// - `crates/{name}-{suffix}/` — Rust binding crates (pyo3, napi, php, ffi, jni)
/// - `e2e/` — cross-language test suites generated by `alef e2e generate`
///
/// The file uses `generated_header: false` (create-once seed). `write_scaffold_files`
/// skips it when `.gitattributes` already exists. Note: `alef scaffold --clean` passes
/// `overwrite=true` which DOES overwrite `generated_header: false` files — delete the
/// file beforehand if you want a fresh regeneration without `--clean`.
fn scaffold_gitattributes(config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    // Use BTreeSet so the output is stable and alphabetically sorted.
    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for &lang in languages {
        match lang {
            // Rust and C are source languages, not binding output directories.
            Language::Rust | Language::C => {}
            // FFI and JNI write only to their binding crates; package_dir() would return
            // a non-existent `packages/ffi` or `packages/jni` placeholder, so handle
            // them explicitly.
            Language::Ffi => {
                dirs.insert(format!("crates/{}-ffi", config.name));
            }
            Language::Jni => {
                dirs.insert(format!("crates/{}-jni", config.name));
            }
            // Python and PHP each have a language-native package directory AND a
            // separate Rust binding crate; include both.
            Language::Python => {
                dirs.insert(config.package_dir(lang));
                dirs.insert(format!("crates/{}-py", config.name));
            }
            Language::Php => {
                dirs.insert(config.package_dir(lang));
                dirs.insert(format!("crates/{}-php", config.name));
            }
            // Kotlin has three scaffold variants with distinct output directories:
            //   - JVM (default): packages/kotlin/
            //   - Native:        packages/kotlin-native/
            //   - Multiplatform: packages/kotlin-mpp/
            // package_dir() always returns packages/kotlin (JVM fallback), so we must
            // resolve the actual output directory from the configured target here.
            Language::Kotlin => {
                let dir = if let Some(k) = config.kotlin.as_ref() {
                    if k.mode.as_deref() == Some("kmp") || k.target == crate::core::config::KotlinTarget::Multiplatform
                    {
                        "packages/kotlin-mpp".to_string()
                    } else if k.target == crate::core::config::KotlinTarget::Native {
                        "packages/kotlin-native".to_string()
                    } else {
                        config.package_dir(lang)
                    }
                } else {
                    config.package_dir(lang)
                };
                dirs.insert(dir);
            }
            // Node: package_dir() checks scaffold_output first, which is unrelated to where
            // scaffold_node and the napi backend actually write files. Those always use the
            // crate dir (crate_dir override or crates/{name}-node default). Bypass package_dir
            // to avoid emitting a wrong path when scaffold_output is set.
            Language::Node => {
                let dir = config
                    .node
                    .as_ref()
                    .and_then(|c| c.crate_dir.as_ref())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("crates/{}-node", config.name));
                dirs.insert(dir);
            }
            // Wasm: package_dir() uses only crate_dir/formula (no scaffold_output risk).
            _ => {
                dirs.insert(config.package_dir(lang));
            }
        }
    }

    // e2e output dir is configurable via [e2e] output = "..." (default "e2e").
    let e2e_dir = config.e2e.as_ref().map(|e| e.output.as_str()).unwrap_or("e2e");
    dirs.insert(e2e_dir.to_string());

    // test_apps output dir is configurable via [e2e.registry] output = "..."
    // (default "test_apps"). Registry-mode test_apps are emitted by
    // `alef test-apps generate`, mirroring the e2e local-mode flow.
    let test_apps_dir = config
        .e2e
        .as_ref()
        .map(|e| e.registry.output.as_str())
        .unwrap_or("test_apps");
    dirs.insert(test_apps_dir.to_string());

    let mut content = String::from("# Generated by alef scaffold.\n");
    for dir in dirs {
        let dir = dir.trim_end_matches('/');
        content.push_str(&format!("{dir}/** linguist-generated=true\n"));
    }

    vec![GeneratedFile {
        path: std::path::PathBuf::from(".gitattributes"),
        content,
        generated_header: false,
    }]
}

use languages::{
    scaffold_csharp, scaffold_dart, scaffold_elixir, scaffold_elixir_cargo, scaffold_ffi, scaffold_gleam, scaffold_go,
    scaffold_java, scaffold_jni, scaffold_kotlin, scaffold_node, scaffold_node_cargo, scaffold_php, scaffold_php_cargo,
    scaffold_pre_commit_config, scaffold_python, scaffold_python_cargo, scaffold_r, scaffold_r_cargo, scaffold_ruby,
    scaffold_ruby_cargo, scaffold_swift, scaffold_wasm, scaffold_zig,
};

fn scaffold_language(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
) -> anyhow::Result<Vec<GeneratedFile>> {
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
        Language::Rust | Language::C => Ok(vec![]), // Rust/C don't need scaffolded binding crates
        Language::Jni => scaffold_jni(api, config),
        Language::Kotlin => scaffold_kotlin(api, config),
        // KotlinAndroid emission is fully handled by the dedicated backend
        // crate (`alef-backend-kotlin-android`); no scaffold step needed.
        Language::KotlinAndroid => Ok(vec![]),
        Language::Gleam => scaffold_gleam(api, config),
        Language::Zig => scaffold_zig(api, config),
        Language::Dart => scaffold_dart(api, config),
        Language::Swift => scaffold_swift(api, config),
    }
}

#[cfg(test)]
mod tests;
