//! Centralized package metadata for generated binding manifests.

use serde::{Deserialize, Serialize};

/// Shared package metadata used by language package manifests.
///
/// Values can be set at `[workspace.package_metadata]` and overridden per
/// `[[crates]]` entry with `[crates.package_metadata]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadataConfig {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub documentation: Option<String>,
    #[serde(default)]
    pub issues: Option<String>,
    #[serde(default)]
    pub funding: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    /// If true, registry-specific keyword/category limits are truncated
    /// deterministically instead of failing validation.
    #[serde(default)]
    pub truncate_registry_lists: bool,
}

impl PackageMetadataConfig {
    pub fn merge(workspace: Option<&Self>, krate: Option<&Self>) -> Option<Self> {
        if workspace.is_none() && krate.is_none() {
            return None;
        }
        let default = Self::default();
        let workspace = workspace.unwrap_or(&default);
        let krate = krate.unwrap_or(&default);
        Some(Self {
            description: krate.description.clone().or_else(|| workspace.description.clone()),
            license: krate.license.clone().or_else(|| workspace.license.clone()),
            repository: krate.repository.clone().or_else(|| workspace.repository.clone()),
            homepage: krate.homepage.clone().or_else(|| workspace.homepage.clone()),
            documentation: krate.documentation.clone().or_else(|| workspace.documentation.clone()),
            issues: krate.issues.clone().or_else(|| workspace.issues.clone()),
            funding: krate.funding.clone().or_else(|| workspace.funding.clone()),
            authors: non_empty_or(&krate.authors, &workspace.authors),
            keywords: non_empty_or(&krate.keywords, &workspace.keywords),
            categories: non_empty_or(&krate.categories, &workspace.categories),
            truncate_registry_lists: krate.truncate_registry_lists || workspace.truncate_registry_lists,
        })
    }
}

fn non_empty_or(primary: &[String], fallback: &[String]) -> Vec<String> {
    if primary.is_empty() {
        fallback.to_vec()
    } else {
        primary.to_vec()
    }
}
