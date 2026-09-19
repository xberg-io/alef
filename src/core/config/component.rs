//! Downloadable native-component configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Desktop/server targets supported by the v1 native component loader.
pub const SUPPORTED_COMPONENT_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

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
    /// Fully-qualified Rust path to the type implementing the contract trait.
    pub implementation: String,
}

/// One deployable unit: a fixed Cargo feature set built into a single signed
/// prebuilt artifact per target, providing one or more [`ComponentContractConfig`]s.
///
/// Modelled on Python extras but language-neutral: the core crate ships small,
/// and a component is the thing a consumer opts into downloading.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentConfig {
    /// Stable component name used in manifests and release assets.
    pub name: String,
    /// Contracts this component implements. Must declare at least one entry;
    /// a component provides each contract at most once, though the same
    /// contract may be provided by more than one component under different
    /// feature sets.
    pub provides: Vec<ComponentProvidesConfig>,
    /// Exact Cargo feature set enabled for this component.
    pub features: Vec<String>,
    /// Whether the core crate's default Cargo features are also enabled.
    #[serde(default)]
    pub default_features: bool,
    /// Rust target triples for which this component is built and published.
    ///
    /// Defaults to every target in [`SUPPORTED_COMPONENT_TARGETS`] enabled by
    /// the crate's `[targets]` table, minus anything listed in `bundled_on`.
    /// An explicit list must be a subset of [`SUPPORTED_COMPONENT_TARGETS`]
    /// and must not overlap `bundled_on`.
    #[serde(default)]
    pub targets: Option<Vec<String>>,
    /// Targets/platforms for which downloading component code is unsupported
    /// (e.g. mobile/wasm hosts) and the component must instead be linked
    /// directly into the binding. Not implemented by any backend yet -- this
    /// only shapes config, validation, and the lock file's per-target
    /// `mode` so backends have a typed switch to read later.
    #[serde(default)]
    pub bundled_on: Vec<String>,
}

/// Distribution settings shared by every component in a crate.
///
/// May be declared at the workspace level (`[component_distribution]`), the
/// crate level (`[crates.component_distribution]`), or both; see [`Self::merge`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentDistributionConfig {
    /// URL template used to locate a component artifact or release manifest.
    #[serde(default)]
    pub url_template: String,
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
        Some(Self {
            url_template,
            public_keys,
        })
    }
}
