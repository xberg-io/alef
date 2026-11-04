//! Downloadable native-component configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Desktop/server targets a component is built for by default.
///
/// This is the default matrix only: a component's explicit `targets` list may name any other
/// well-formed Rust target triple (see [`is_valid_target_triple`]), e.g. `aarch64-pc-windows-msvc`.
pub const SUPPORTED_COMPONENT_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

/// Whether `triple` is a well-formed Rust target triple: three to five `-`-separated segments of
/// lowercase ASCII letters, digits, `_` or `.`.
///
/// This is a syntax check only; it does not know which triples `rustc` can build.
#[must_use]
pub fn is_valid_target_triple(triple: &str) -> bool {
    let segments: Vec<&str> = triple.split('-').collect();
    (3..=5).contains(&segments.len())
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.'))
        })
}

/// Placeholders accepted by [`ComponentDistributionConfig::artifact_name_template`].
pub const ARTIFACT_NAME_PLACEHOLDERS: &[&str] = &["crate", "component", "version", "target", "fh16", "ch16"];

/// Artifact file name used when no `artifact_name_template` is configured.
pub const DEFAULT_ARTIFACT_NAME_TEMPLATE: &str = "{crate}-{component}-{version}-{fh16}-{ch16}-{target}.tar.gz";

fn default_interface_version() -> u32 {
    1
}

/// A stable component interface authored as a Rust trait.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentContractConfig {
    /// Stable name referenced by components.
    pub name: String,
    /// Fully-qualified Rust path to the trait that defines the contract.
    pub trait_path: String,
    /// Version of the generated C interface for compatibility negotiation.
    #[serde(default = "default_interface_version")]
    pub interface_version: u32,
}

/// One contract a component implements, and the Rust type that implements it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentProvidesConfig {
    /// Name of an entry in `component_contracts`.
    pub contract: String,
    /// Fully-qualified Rust implementation type, or plain Go constructor identifier.
    pub implementation: String,
}

/// Language used to build a component producer.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ComponentProducerLanguage {
    /// Rust cdylib producer.
    #[default]
    Rust,
    /// Go c-shared producer using the native C ABI.
    Go,
}

impl ComponentProducerLanguage {
    fn is_rust(&self) -> bool {
        *self == Self::Rust
    }
}

/// Whether a constructor can be emitted as a Go function name without an expression.
#[must_use]
pub(crate) fn is_valid_go_constructor_identifier(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "_",
        "init",
        "main",
        "break",
        "default",
        "func",
        "interface",
        "select",
        "case",
        "defer",
        "go",
        "map",
        "struct",
        "chan",
        "else",
        "goto",
        "package",
        "switch",
        "const",
        "fallthrough",
        "if",
        "range",
        "type",
        "continue",
        "for",
        "import",
        "return",
        "var",
    ];
    let mut bytes = name.bytes();
    !RESERVED.contains(&name)
        && bytes
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// One deployable unit: a fixed Cargo feature set built into a single signed
/// prebuilt artifact per target, providing one or more [`ComponentContractConfig`]s.
///
/// Modelled on Python extras but language-neutral: the core crate ships small,
/// and a component is the thing a consumer opts into downloading.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentConfig {
    /// Language used by the generated component producer; defaults to Rust.
    #[serde(default, skip_serializing_if = "ComponentProducerLanguage::is_rust")]
    pub producer_language: ComponentProducerLanguage,
    /// Stable component name used in manifests and release assets.
    pub name: String,
    /// Contracts this component implements. Must declare at least one entry;
    /// a component provides each contract at most once, though the same
    /// contract may be provided by more than one component under different
    /// feature sets.
    pub provides: Vec<ComponentProvidesConfig>,
    /// Exact Cargo feature set enabled for this component. Required and non-empty unless the
    /// component is a Go producer or names its own `source_manifest` crate.
    #[serde(default)]
    pub features: Vec<String>,
    /// Whether the core crate's default Cargo features are also enabled.
    #[serde(default)]
    pub default_features: bool,
    /// Rust target triples for which this component is built and published.
    ///
    /// Defaults to every target in [`SUPPORTED_COMPONENT_TARGETS`] enabled by
    /// the crate's `[targets]` table, minus anything listed in `bundled_on`.
    /// An explicit list may name any well-formed Rust target triple (see
    /// [`is_valid_target_triple`]), including ones outside the default matrix such as
    /// `aarch64-pc-windows-msvc`, and must not overlap `bundled_on`.
    #[serde(default)]
    pub targets: Option<Vec<String>>,
    /// Targets/platforms for which downloading component code is unsupported
    /// (e.g. mobile/wasm hosts) and the component must instead be linked
    /// directly into the binding. No backend generates the bundled delivery
    /// itself: this shapes config, validation, and the lock file's per-target
    /// `mode`. For Rust hosts the supported fallback is a manual static link
    /// (the host depends on the implementation crate behind a cargo feature
    /// and calls it directly, since musl targets cannot `dlopen`); see
    /// "Bundled (static-link) fallback" in `docs/design/components.md`.
    #[serde(default)]
    pub bundled_on: Vec<String>,
    /// Extra Cargo dependencies for this component's generated producer crate, keyed by crate name.
    ///
    /// Each value is a Cargo dependency spec: a version string (`"1"`) or an inline table
    /// (`{ version = "1", features = ["x"] }`, `{ path = "../x" }`, `{ git = "..." }`), emitted
    /// verbatim into the producer `Cargo.toml`. Lets an implementation type live in a crate other
    /// than the core crate. Cannot override `alef-component-abi`, `futures` or the core crate.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub extra_dependencies: BTreeMap<String, serde_json::Value>,
    /// Path to the `Cargo.toml` of the crate that holds this component's `implementation` types.
    ///
    /// Defaults to the `[[crates]]` core crate. When set, the generated producer keeps its path
    /// dependency on the core crate (the contract trait home) and additionally depends on this
    /// crate, named by its `[package] name`. The component's `features` go on this source crate
    /// (so they must be defined there) and `default_features` applies to both dependencies; the
    /// core crate gets no features. When the manifest resolves to the core crate itself, a single
    /// dependency carrying the features is emitted as usual. Rust producers only. Relative to the
    /// directory containing `alef.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_manifest: Option<String>,
    /// Directory where the producer crate is generated and built from
    /// (`cargo build --manifest-path <producer_dir>/Cargo.toml`).
    ///
    /// Defaults to `crates/<core>-<name>-component`. Relative to the directory containing
    /// `alef.toml`; must not contain `..`. For Go producers it is the Go module directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_dir: Option<String>,
}

impl ComponentConfig {
    /// Re-root `source_manifest` and `producer_dir` onto `config_dir`, the directory of the
    /// `alef.toml` in use, so they do not depend on the process working directory.
    ///
    /// An empty `config_dir` (a bare `alef.toml` in the working directory) leaves both untouched.
    pub fn rebase_paths(&mut self, config_dir: &std::path::Path) {
        if config_dir.as_os_str().is_empty() {
            return;
        }
        for path in [&mut self.source_manifest, &mut self.producer_dir]
            .into_iter()
            .flatten()
        {
            *path = config_dir.join(&*path).to_string_lossy().replace('\\', "/");
        }
    }
}

/// Directory of the `alef.toml` at `config_path` as a lexically clean path relative to `cwd`.
///
/// The result is the same whether `config_path` is spelled `alef.toml`, `./alef.toml`, an absolute
/// path or `sub/alef.toml`: empty when the config sits in `cwd`, `sub` for a config in `cwd/sub`,
/// `..` for one in the parent, and so on. A relative spelling is taken to be relative to `cwd`.
#[must_use]
pub fn config_dir_relative_to(config_path: &std::path::Path, cwd: &std::path::Path) -> std::path::PathBuf {
    let parent = config_path.parent().unwrap_or(std::path::Path::new(""));
    let cwd = lexically_clean(cwd);
    let dir = lexically_clean(&cwd.join(parent));
    match dir.strip_prefix(&cwd) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => match crate::core::config::resolve_helpers::relative_slash_path(&cwd, &dir).as_str() {
            "." => std::path::PathBuf::new(),
            relative => std::path::PathBuf::from(relative),
        },
    }
}

/// `path` with `.` segments dropped and `..` segments folded into the preceding segment.
pub(crate) fn lexically_clean(path: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The non-empty, non-`.` segments of a `/`-separated config path, so `a/b`, `a/b/` and `a//b`
/// compare equal.
#[must_use]
pub fn path_segments(value: &str) -> Vec<&str> {
    value
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect()
}

/// Cargo package name of a component's generated producer crate.
#[must_use]
pub fn component_producer_package(core_crate_dir: &str, component: &str) -> String {
    format!("{core_crate_dir}-{}-component", component.replace('_', "-"))
}

/// Directory the producer crate of `component` is generated into and built from: the configured
/// `producer_dir`, else `crates/<core>-<name>-component`.
#[must_use]
pub fn component_producer_dir(core_crate_dir: &str, component: &str, producer_dir: Option<&str>) -> std::path::PathBuf {
    producer_dir.map_or_else(
        || std::path::Path::new("crates").join(component_producer_package(core_crate_dir, component)),
        std::path::PathBuf::from,
    )
}

/// Check a component `source_manifest` value: a relative `/`-separated path ending in `Cargo.toml`.
///
/// # Errors
///
/// Returns a human-readable reason when the value is invalid.
pub fn validate_source_manifest(value: &str) -> Result<(), String> {
    validate_component_path("source_manifest", value)?;
    if std::path::Path::new(value).file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return Err(format!("source_manifest `{value}` must point at a `Cargo.toml` file"));
    }
    Ok(())
}

/// Check a component `producer_dir` value: a relative `/`-separated path without `.` or `..`
/// segments that does not normalise to nothing.
///
/// # Errors
///
/// Returns a human-readable reason when the value is invalid.
pub fn validate_producer_dir(value: &str) -> Result<(), String> {
    validate_component_path("producer_dir", value)?;
    if value.split('/').any(|segment| segment == "..") {
        return Err(format!("producer_dir `{value}` must not contain `..` segments"));
    }
    if value.split('/').any(|segment| segment == ".") {
        return Err(format!("producer_dir `{value}` must not contain `.` segments"));
    }
    if path_segments(value).is_empty() {
        return Err(format!("producer_dir `{value}` must name a directory"));
    }
    Ok(())
}

fn validate_component_path(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.starts_with('/')
        || value.contains('\\')
        || value.contains('"')
        || value.chars().any(char::is_control)
        || value.as_bytes().get(1) == Some(&b':')
    {
        return Err(format!(
            "{label} `{value}` must be a relative path using `/` separators and no quotes"
        ));
    }
    Ok(())
}

/// Dependencies of the generated producer crate that `extra_dependencies` must not override.
pub const PRODUCER_BUILTIN_DEPENDENCIES: &[&str] = &["alef-component-abi", "futures"];

/// Whether `name` is a valid Cargo package name: ASCII letters, digits, `-` and `_`, starting
/// with a letter or `_`.
#[must_use]
pub fn is_valid_cargo_package_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Check one `extra_dependencies` entry: a valid, non-builtin crate name and a spec that is a
/// non-empty version string or a table of strings, booleans and string arrays.
///
/// # Errors
///
/// Returns a human-readable reason when the entry is invalid.
pub fn validate_extra_dependency(core_crate: &str, name: &str, spec: &serde_json::Value) -> Result<(), String> {
    use serde_json::Value;
    if !is_valid_cargo_package_name(name) {
        return Err(format!(
            "extra dependency name `{name}` must be a Cargo package name (ASCII letters, digits, `-`, `_`)"
        ));
    }
    let normalized = name.replace('_', "-");
    let builtin = PRODUCER_BUILTIN_DEPENDENCIES
        .iter()
        .copied()
        .chain([core_crate])
        .any(|builtin| builtin.replace('_', "-") == normalized);
    if builtin {
        return Err(format!(
            "extra dependency `{name}` would override a dependency of the generated producer crate"
        ));
    }
    let invalid = || format!("extra dependency `{name}` must be a version string or a table of Cargo dependency keys");
    match spec {
        Value::String(version) if !version.trim().is_empty() => Ok(()),
        Value::Object(table) if !table.is_empty() => {
            let scalar_ok = |value: &Value| match value {
                Value::String(_) | Value::Bool(_) => true,
                Value::Array(items) => items.iter().all(Value::is_string),
                _ => false,
            };
            if table.values().all(scalar_ok) {
                Ok(())
            } else {
                Err(invalid())
            }
        }
        _ => Err(invalid()),
    }
}

/// Render an `extra_dependencies` spec as the right-hand side of a `[dependencies]` line.
///
/// # Errors
///
/// Returns an error when the value cannot be represented in TOML.
pub fn render_dependency_spec(spec: &serde_json::Value) -> Result<String, String> {
    let value = toml::Value::try_from(spec).map_err(|error| error.to_string())?;
    match value {
        toml::Value::Table(table) => {
            let entries = table
                .iter()
                .map(|(key, value)| format!("{} = {value}", toml::Value::String(key.clone())))
                .collect::<Vec<_>>();
            Ok(format!("{{ {} }}", entries.join(", ")))
        }
        other => Ok(other.to_string()),
    }
}

/// Distribution settings shared by every component in a crate.
///
/// May be declared at the workspace level (`[component_distribution]`), the
/// crate level (`[crates.component_distribution]`), or both; see [`Self::merge`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentDistributionConfig {
    /// URL template used to locate a component artifact or release manifest.
    ///
    /// Must start with `https://` (or `file://` for local dev locks) and contain `{artifact}`,
    /// which expands to the artifact file name. `{crate}`, `{component}`, `{version}`,
    /// `{target}`, `{feature_hash}` and `{contract_hash}` are optional.
    #[serde(default)]
    pub url_template: String,
    /// Optional template for artifact file names, replacing the default
    /// [`DEFAULT_ARTIFACT_NAME_TEMPLATE`].
    ///
    /// Placeholders: `{crate}`, `{component}`, `{version}`, `{target}`, `{fh16}` and `{ch16}`
    /// (the first 16 hex characters of the feature and contract hashes). Must contain
    /// `{component}` and `{target}`; the rendered name may only use `[A-Za-z0-9._-]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_name_template: Option<String>,
    /// Trusted public keys accepted when verifying component manifests.
    #[serde(default)]
    pub public_keys: BTreeMap<String, String>,
}

impl ComponentDistributionConfig {
    /// Merge a crate-level override onto a workspace-level default, field by field.
    ///
    /// A non-empty crate-level `url_template` replaces the workspace one entirely; a
    /// crate-level `public_keys` entry overrides a workspace key with the same
    /// ID, and workspace-only keys are kept. Returns `None` when neither level
    /// declares a distribution config.
    #[must_use]
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        workspace.or(krate)?;
        let mut public_keys = workspace.map(|config| config.public_keys.clone()).unwrap_or_default();
        if let Some(krate) = krate {
            public_keys.extend(krate.public_keys.clone());
        }
        let url_template = krate
            .map(|config| config.url_template.clone())
            .filter(|url| !url.is_empty())
            .or_else(|| workspace.map(|config| config.url_template.clone()))
            .unwrap_or_default();
        let artifact_name_template = krate
            .and_then(|config| config.artifact_name_template.clone())
            .or_else(|| workspace.and_then(|config| config.artifact_name_template.clone()));
        Some(Self {
            url_template,
            artifact_name_template,
            public_keys,
        })
    }

    /// Check a `url_template`: it must use `https://` (or `file://`, for local dev locks), have
    /// a non-empty remainder without whitespace, and contain `{artifact}`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason when the template is invalid.
    pub fn validate_url_template(template: &str) -> Result<(), String> {
        let template = template.trim();
        if let Some(rest) = template.strip_prefix("file://") {
            if rest.is_empty() || rest.chars().any(char::is_whitespace) {
                return Err("component distribution url_template must be a non-empty file:// URL".into());
            }
        } else {
            let authority = template
                .strip_prefix("https://")
                .and_then(|remainder| remainder.split('/').next());
            if authority.is_none_or(|value| value.is_empty() || value.chars().any(char::is_whitespace)) {
                return Err("component distribution url_template must use HTTPS (or file:// for dev locks)".into());
            }
        }
        if !template.contains("{artifact}") {
            return Err("component distribution url_template must contain `{artifact}`".into());
        }
        Ok(())
    }

    /// Check an `artifact_name_template`: known placeholders only, `{component}` and `{target}`
    /// present, and a literal part limited to `[A-Za-z0-9._-]`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason when the template is invalid.
    pub fn validate_artifact_name_template(template: &str) -> Result<(), String> {
        let mut rest = template;
        let mut seen = Vec::new();
        let mut literal = String::new();
        while let Some(open) = rest.find(['{', '}']) {
            literal.push_str(&rest[..open]);
            if !rest[open..].starts_with('{') {
                return Err(format!("artifact_name_template `{template}` has an unmatched `}}`"));
            }
            let Some(close) = rest[open..].find('}') else {
                return Err(format!("artifact_name_template `{template}` has an unmatched `{{`"));
            };
            let name = &rest[open + 1..open + close];
            if !ARTIFACT_NAME_PLACEHOLDERS.contains(&name) {
                return Err(format!(
                    "artifact_name_template uses unknown placeholder `{{{name}}}`; allowed: {}",
                    ARTIFACT_NAME_PLACEHOLDERS
                        .iter()
                        .map(|placeholder| format!("{{{placeholder}}}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            seen.push(name);
            rest = &rest[open + close + 1..];
        }
        literal.push_str(rest);
        for required in ["component", "target"] {
            if !seen.contains(&required) {
                return Err(format!("artifact_name_template must contain `{{{required}}}`"));
            }
        }
        if !literal
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(format!(
                "artifact_name_template `{template}` may only contain [A-Za-z0-9._-] outside placeholders"
            ));
        }
        Ok(())
    }
}

/// Default `lock_path` of a [`ComponentHostConfig`]: the lock sits next to the generated module.
pub const DEFAULT_COMPONENT_HOST_LOCK_PATH: &str = "components.lock.json";

/// A standalone Rust host module for hosts that load components without an ffi/pyo3 binding.
///
/// `alef generate` writes one Rust file at `output` holding the contract tables, proxies,
/// entry-symbol constants and a `component_manager` constructor over the embedded lock.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentHostConfig {
    /// Component IDs loaded as child processes by this standalone Rust host.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stdio_components: Vec<String>,
    /// Path of the generated Rust module, relative to the project root. Must end in `.rs`.
    pub output: String,
    /// Path of `components.lock.json` as written in the module's `include_str!`, relative to the
    /// generated module's directory. Defaults to [`DEFAULT_COMPONENT_HOST_LOCK_PATH`] (next to
    /// the module). `alef component lock` stages the lock at this location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_path: Option<String>,
}

impl ComponentHostConfig {
    /// The `lock_path` in effect, applying the default.
    #[must_use]
    pub fn effective_lock_path(&self) -> &str {
        self.lock_path.as_deref().unwrap_or(DEFAULT_COMPONENT_HOST_LOCK_PATH)
    }

    /// Project-root-relative location of the lock file the generated module embeds.
    #[must_use]
    pub fn lock_file_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.output)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join(self.effective_lock_path())
    }

    /// Check both paths: non-empty, relative, free of control characters and backslashes, and
    /// `output` ending in `.rs`.
    ///
    /// # Errors
    ///
    /// Returns a human-readable reason when a path is invalid.
    pub fn validate(&self) -> Result<(), String> {
        validate_host_path("component_host output", &self.output)?;
        if !self.output.ends_with(".rs") {
            return Err(format!("component_host output `{}` must end in `.rs`", self.output));
        }
        if let Some(lock_path) = &self.lock_path {
            validate_host_path("component_host lock_path", lock_path)?;
        }
        Ok(())
    }
}

fn validate_host_path(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.starts_with('/') || value.contains('\\') || value.chars().any(char::is_control) || value.contains('"') {
        return Err(format!(
            "{label} `{value}` must be a relative path using `/` separators and no quotes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod host_tests {
    use super::*;

    fn host(output: &str, lock_path: Option<&str>) -> ComponentHostConfig {
        ComponentHostConfig {
            stdio_components: Vec::new(),
            output: output.into(),
            lock_path: lock_path.map(str::to_string),
        }
    }

    #[test]
    fn lock_path_defaults_next_to_the_module() {
        let config = host("crates/x/src/components_gen.rs", None);
        assert_eq!(config.effective_lock_path(), "components.lock.json");
        assert_eq!(
            config.lock_file_path(),
            std::path::PathBuf::from("crates/x/src/components.lock.json")
        );
    }

    #[test]
    fn explicit_lock_path_is_relative_to_the_module_directory() {
        let config = host("crates/x/src/gen.rs", Some("../components.lock.json"));
        assert_eq!(
            config.lock_file_path(),
            std::path::PathBuf::from("crates/x/src/../components.lock.json")
        );
    }

    #[test]
    fn validation_rejects_bad_paths() {
        assert!(host("a/b.rs", None).validate().is_ok());
        assert!(host("", None).validate().is_err());
        assert!(host("a/b.txt", None).validate().is_err());
        assert!(host("/abs/b.rs", None).validate().is_err());
        assert!(host("a/b.rs", Some("")).validate().is_err());
        assert!(host("a/b.rs", Some("x\"y")).validate().is_err());
    }

    #[test]
    fn parses_from_toml_and_rejects_unknown_keys() {
        let parsed: ComponentHostConfig = toml::from_str("output = \"g.rs\"\nlock_path = \"l.json\"").unwrap();
        assert_eq!(parsed, host("g.rs", Some("l.json")));
        assert!(toml::from_str::<ComponentHostConfig>("output = \"g.rs\"\ncache_root = \"x\"").is_err());
    }
}

#[cfg(test)]
mod extra_dependency_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_version_strings_and_tables() {
        assert!(validate_extra_dependency("core", "serde_json", &json!("1")).is_ok());
        assert!(validate_extra_dependency("core", "x", &json!({ "version": "1", "features": ["a"] })).is_ok());
        assert!(validate_extra_dependency("core", "x", &json!({ "path": "../x", "optional": true })).is_ok());
    }

    #[test]
    fn rejects_bad_names_specs_and_builtins() {
        assert!(validate_extra_dependency("core", "bad name", &json!("1")).is_err());
        assert!(validate_extra_dependency("core", "1x", &json!("1")).is_err());
        assert!(validate_extra_dependency("core", "x", &json!("")).is_err());
        assert!(validate_extra_dependency("core", "x", &json!(1)).is_err());
        assert!(validate_extra_dependency("core", "x", &json!({})).is_err());
        assert!(validate_extra_dependency("core", "x", &json!({ "features": [1] })).is_err());
        assert!(validate_extra_dependency("core", "futures", &json!("1")).is_err());
        assert!(validate_extra_dependency("core", "alef_component_abi", &json!("1")).is_err());
        assert!(validate_extra_dependency("my-core", "my_core", &json!("1")).is_err());
    }

    #[test]
    fn renders_strings_and_inline_tables() {
        assert_eq!(render_dependency_spec(&json!("1.2")).unwrap(), "\"1.2\"");
        let rendered =
            render_dependency_spec(&json!({ "version": "1", "features": ["a", "b"], "default-features": false }))
                .unwrap();
        assert!(rendered.starts_with("{ ") && rendered.ends_with(" }"), "{rendered}");
        for part in [
            "\"version\" = \"1\"",
            "\"features\" = [\"a\", \"b\"]",
            "\"default-features\" = false",
        ] {
            assert!(rendered.contains(part), "{rendered} missing {part}");
        }
    }
}

#[cfg(test)]
mod producer_language_tests {
    use super::*;

    #[test]
    fn producer_language_defaults_to_rust_without_changing_legacy_serialization() {
        let mut value = serde_json::json!({ "name": "codec", "provides": [], "features": [] });
        let config: ComponentConfig = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(config.producer_language, ComponentProducerLanguage::Rust);
        assert!(
            serde_json::to_value(&config)
                .unwrap()
                .get("producer_language")
                .is_none()
        );
        value["producer_language"] = serde_json::json!("rust");
        assert_eq!(serde_json::from_value::<ComponentConfig>(value).unwrap(), config);
    }

    #[test]
    fn producer_language_accepts_go_and_rejects_unknown_values() {
        let mut value =
            serde_json::json!({ "name": "codec", "provides": [], "features": [], "producer_language": "go" });
        let config: ComponentConfig = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(config.producer_language, ComponentProducerLanguage::Go);
        assert_eq!(serde_json::to_value(config).unwrap()["producer_language"], "go");
        value["producer_language"] = serde_json::json!("python");
        let error = serde_json::from_value::<ComponentConfig>(value)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown variant `python`"), "{error}");
    }
}
