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
    /// Stable name referenced by component profiles.
    pub name: String,
    /// Fully-qualified Rust path to the trait that defines the contract.
    pub trait_path: String,
    /// Version of the generated C interface for compatibility negotiation.
    #[serde(default = "default_interface_version")]
    pub interface_version: u32,
}

/// One feature-set-specific native component build.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentProfileConfig {
    /// Stable component name used in manifests and release assets.
    pub name: String,
    /// Name of an entry in `component_contracts`.
    pub contract: String,
    /// Fully-qualified Rust path to the type implementing the contract trait.
    pub implementation: String,
    /// Exact Cargo feature set enabled for this component.
    pub features: Vec<String>,
    /// Whether the core crate's default Cargo features are also enabled.
    #[serde(default)]
    pub default_features: bool,
    /// Rust target triples for which this component is built and published.
    pub targets: Vec<String>,
}

/// Distribution settings shared by every component profile in a crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentDistributionConfig {
    /// URL template used to locate a component artifact or release manifest.
    pub url_template: String,
    /// Trusted public keys accepted when verifying component manifests.
    pub public_keys: BTreeMap<String, String>,
}
