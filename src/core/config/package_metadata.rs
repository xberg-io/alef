//! Centralized package metadata for generated binding manifests.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Shared package metadata used by language package manifests.
///
/// Values can be set at `[workspace.package_metadata]` and overridden per
/// `[[crates]]` entry with `[crates.package_metadata]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadataConfig {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    /// Extra licenses shipped inside the generated package, declared after the
    /// project license in a generated Maven POM's `<licenses>` block (for
    /// example the license of a bundled native library).
    #[serde(default)]
    pub additional_licenses: Vec<AdditionalLicenseConfig>,
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
    #[serde(default)]
    pub cargo: Option<CargoPackageFilesConfig>,
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
            additional_licenses: non_empty_or(&krate.additional_licenses, &workspace.additional_licenses),
            repository: krate.repository.clone().or_else(|| workspace.repository.clone()),
            homepage: krate.homepage.clone().or_else(|| workspace.homepage.clone()),
            documentation: krate.documentation.clone().or_else(|| workspace.documentation.clone()),
            issues: krate.issues.clone().or_else(|| workspace.issues.clone()),
            funding: krate.funding.clone().or_else(|| workspace.funding.clone()),
            authors: non_empty_or(&krate.authors, &workspace.authors),
            keywords: non_empty_or(&krate.keywords, &workspace.keywords),
            categories: non_empty_or(&krate.categories, &workspace.categories),
            cargo: krate.cargo.clone().or_else(|| workspace.cargo.clone()),
            truncate_registry_lists: krate.truncate_registry_lists || workspace.truncate_registry_lists,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CargoPackageFilesConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// One extra `<license>` entry in a generated Maven POM.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdditionalLicenseConfig {
    /// SPDX expression or license name, e.g. `"LGPL-2.1-or-later"`.
    pub name: String,
    /// URL of the license text.
    #[serde(default)]
    pub url: Option<String>,
    /// Free-form note shown in the POM, e.g. which bundled component the
    /// license covers.
    #[serde(default)]
    pub comments: Option<String>,
}

fn non_empty_or<T: Clone>(primary: &[T], fallback: &[T]) -> Vec<T> {
    if primary.is_empty() {
        fallback.to_vec()
    } else {
        primary.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filters(include: &[&str], exclude: &[&str]) -> CargoPackageFilesConfig {
        CargoPackageFilesConfig {
            include: include.iter().map(|value| (*value).to_string()).collect(),
            exclude: exclude.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    #[test]
    fn crate_cargo_filters_replace_workspace_filters() {
        let workspace = PackageMetadataConfig {
            cargo: Some(filters(&["src/**", "build.rs"], &[])),
            ..Default::default()
        };
        let krate = PackageMetadataConfig {
            cargo: Some(filters(&[], &["tests/**", "benches/**"])),
            ..Default::default()
        };

        let merged = PackageMetadataConfig::merge(Some(&workspace), Some(&krate)).expect("merged metadata");

        assert_eq!(merged.cargo, krate.cargo);
    }

    #[test]
    fn empty_crate_cargo_table_clears_workspace_filters() {
        let workspace = PackageMetadataConfig {
            cargo: Some(filters(&["src/**"], &[])),
            ..Default::default()
        };
        let krate = PackageMetadataConfig {
            cargo: Some(CargoPackageFilesConfig::default()),
            ..Default::default()
        };

        let merged = PackageMetadataConfig::merge(Some(&workspace), Some(&krate)).expect("merged metadata");

        assert_eq!(merged.cargo, Some(CargoPackageFilesConfig::default()));
    }

    #[test]
    fn workspace_cargo_filters_are_inherited_without_crate_override() {
        let workspace = PackageMetadataConfig {
            cargo: Some(filters(&["src/**", "!src/**/tests.rs"], &[])),
            ..Default::default()
        };

        let merged = PackageMetadataConfig::merge(Some(&workspace), None).expect("merged metadata");

        assert_eq!(merged.cargo, workspace.cargo);
    }

    fn extra_license(name: &str, url: Option<&str>, comments: Option<&str>) -> AdditionalLicenseConfig {
        AdditionalLicenseConfig {
            name: name.to_string(),
            url: url.map(str::to_string),
            comments: comments.map(str::to_string),
        }
    }

    #[test]
    fn additional_licenses_round_trip_through_toml() {
        let toml_str = r#"
license = "MIT"
additional_licenses = [
  { name = "LGPL-2.1-or-later", url = "https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html", comments = "Bundled libheif" },
  { name = "GPL-3.0-only", comments = "Windows build" },
]
"#;

        let parsed: PackageMetadataConfig = toml::from_str(toml_str).expect("additional_licenses parses");
        assert_eq!(
            parsed.additional_licenses,
            vec![
                extra_license(
                    "LGPL-2.1-or-later",
                    Some("https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html"),
                    Some("Bundled libheif")
                ),
                extra_license("GPL-3.0-only", None, Some("Windows build")),
            ]
        );

        let round_tripped = toml::to_string(&parsed).expect("metadata serializes");
        let reparsed: PackageMetadataConfig = toml::from_str(&round_tripped).expect("serialized metadata parses");
        assert_eq!(reparsed, parsed);
    }

    #[test]
    fn crate_additional_licenses_replace_workspace_entries() {
        let workspace = PackageMetadataConfig {
            additional_licenses: vec![extra_license("LGPL-2.1-or-later", None, Some("workspace"))],
            ..Default::default()
        };
        let krate = PackageMetadataConfig {
            additional_licenses: vec![extra_license("GPL-3.0-only", None, Some("crate"))],
            ..Default::default()
        };

        let merged = PackageMetadataConfig::merge(Some(&workspace), Some(&krate)).expect("merged metadata");
        assert_eq!(merged.additional_licenses, krate.additional_licenses);

        let inherited = PackageMetadataConfig::merge(Some(&workspace), Some(&PackageMetadataConfig::default()))
            .expect("merged metadata");
        assert_eq!(
            inherited.additional_licenses, workspace.additional_licenses,
            "a crate that does not configure additional licenses inherits the workspace list"
        );
    }

    #[test]
    fn unknown_additional_license_fields_are_rejected() {
        let error = toml::from_str::<PackageMetadataConfig>(
            r#"
additional_licenses = [{ name = "MIT", url_text = "https://opensource.org/licenses/MIT" }]
"#,
        )
        .expect_err("a typo inside an additional license entry must be rejected");

        assert!(
            error.to_string().contains("url_text"),
            "the error should name the unknown key; got: {error}"
        );
    }
}
