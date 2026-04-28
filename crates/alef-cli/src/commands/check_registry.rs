//! Registry version existence checker.
//!
//! For each supported package registry, performs a lightweight HTTP lookup to
//! determine whether a specific version of a package is published.
//!
//! Replaces:
//! - `actions/check-registry/action.yml`
//! - `kreuzberg/scripts/publish/check_*.sh`

use anyhow::{Context, Result};
use serde_json::json;
use std::time::Duration;

/// Supported registries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Registry {
    Pypi,
    Npm,
    Wasm,
    Rubygems,
    Maven,
    Nuget,
    Packagist,
    Cratesio,
    Hex,
    Homebrew,
    GithubRelease,
}

impl std::fmt::Display for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Registry::Pypi => write!(f, "pypi"),
            Registry::Npm | Registry::Wasm => write!(f, "npm"),
            Registry::Rubygems => write!(f, "rubygems"),
            Registry::Maven => write!(f, "maven"),
            Registry::Nuget => write!(f, "nuget"),
            Registry::Packagist => write!(f, "packagist"),
            Registry::Cratesio => write!(f, "cratesio"),
            Registry::Hex => write!(f, "hex"),
            Registry::Homebrew => write!(f, "homebrew"),
            Registry::GithubRelease => write!(f, "github-release"),
        }
    }
}

/// Check whether `package@version` exists in `registry`.
///
/// `extra` carries registry-specific parameters:
/// - Maven: `package` is `groupId:artifactId` (colon-separated).
/// - NuGet: `source_url` override (defaults to `https://api.nuget.org`).
/// - Homebrew: `tap_repo` in `owner/repo` form (e.g. `Homebrew/homebrew-core`).
/// - GitHub Release: `repo` in `owner/repo` form.
pub fn check(registry: Registry, package: &str, version: &str, extra: &ExtraParams, output_json: bool) -> Result<bool> {
    let exists = match registry {
        Registry::Pypi => check_pypi(package, version)?,
        Registry::Npm | Registry::Wasm => check_npm(package, version)?,
        Registry::Rubygems => check_rubygems(package, version)?,
        Registry::Maven => check_maven(package, version)?,
        Registry::Nuget => check_nuget(package, version, extra.nuget_source.as_deref())?,
        Registry::Packagist => check_packagist(package, version)?,
        Registry::Cratesio => check_cratesio(package, version)?,
        Registry::Hex => check_hex(package, version)?,
        Registry::Homebrew => check_homebrew(package, version, extra.tap_repo.as_deref())?,
        Registry::GithubRelease => check_github_release(
            package,
            version,
            extra.repo.as_deref(),
            extra.asset_prefix.as_deref(),
            &extra.required_assets,
        )?,
    };

    if output_json {
        let out = json!({
            "registry": registry.to_string(),
            "package": package,
            "version": version,
            "exists": exists,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("exists={}", if exists { "true" } else { "false" });
    }

    Ok(exists)
}

/// Extra parameters for registry-specific checks.
#[derive(Debug, Default)]
pub struct ExtraParams {
    /// NuGet source URL override.
    pub nuget_source: Option<String>,
    /// Homebrew tap repository (`owner/repo`).
    pub tap_repo: Option<String>,
    /// GitHub repository (`owner/repo`) for GitHub Release check.
    pub repo: Option<String>,
    /// Asset name prefix (github-release): require at least one asset whose
    /// name starts with this prefix to consider the release "exists".
    pub asset_prefix: Option<String>,
    /// Required asset names (github-release): all must be present.
    pub required_assets: Vec<String>,
}

// ---- HTTP helper ----

/// Build a configured ureq agent with a 30-second global timeout.
fn build_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent()
}

/// Map a ureq v3 error to the boolean "exists" semantic. Returns `Ok(false)` on
/// 404, `Ok(true)` on any 2xx, and propagates other failures.
fn classify(result: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<HttpOutcome> {
    match result {
        Ok(resp) => Ok(HttpOutcome::Ok(resp)),
        Err(ureq::Error::StatusCode(404)) => Ok(HttpOutcome::NotFound),
        Err(e) => Err(anyhow::anyhow!("HTTP request failed: {e}")),
    }
}

enum HttpOutcome {
    Ok(ureq::http::Response<ureq::Body>),
    NotFound,
}

/// GET `url` and return true if the response is 2xx, false if 404, error otherwise.
fn http_get_ok(url: &str) -> Result<bool> {
    let agent = build_agent();
    let response = agent.get(url).header("User-Agent", "alef-publish/1.0").call();
    match classify(response).with_context(|| format!("HTTP GET {url}"))? {
        HttpOutcome::Ok(_) => Ok(true),
        HttpOutcome::NotFound => Ok(false),
    }
}

/// GET `url`, parse the response as JSON. Returns None on 404.
fn http_get_json(url: &str) -> Result<Option<serde_json::Value>> {
    let agent = build_agent();
    let response = agent
        .get(url)
        .header("User-Agent", "alef-publish/1.0")
        .header("Accept", "application/json")
        .call();
    match classify(response).with_context(|| format!("HTTP GET {url}"))? {
        HttpOutcome::Ok(resp) => {
            let text = resp
                .into_body()
                .read_to_string()
                .with_context(|| format!("reading body from {url}"))?;
            let val: serde_json::Value =
                serde_json::from_str(&text).with_context(|| format!("parsing JSON from {url}"))?;
            Ok(Some(val))
        }
        HttpOutcome::NotFound => Ok(None),
    }
}

// ---- per-registry checks ----

fn check_pypi(package: &str, version: &str) -> Result<bool> {
    let url = format!("https://pypi.org/pypi/{package}/{version}/json");
    http_get_ok(&url)
}

fn check_npm(package: &str, version: &str) -> Result<bool> {
    let url = format!("https://registry.npmjs.org/{package}/{version}");
    http_get_ok(&url)
}

fn check_cratesio(package: &str, version: &str) -> Result<bool> {
    let url = format!("https://crates.io/api/v1/crates/{package}/{version}");
    let agent = build_agent();
    let response = agent
        .get(&url)
        // crates.io requires a descriptive User-Agent.
        .header("User-Agent", "alef-publish/1.0 (https://github.com/kreuzberg-dev/alef)")
        .call();
    match classify(response).with_context(|| format!("HTTP GET {url}"))? {
        HttpOutcome::Ok(_) => Ok(true),
        HttpOutcome::NotFound => Ok(false),
    }
}

fn check_rubygems(package: &str, version: &str) -> Result<bool> {
    let url = format!("https://rubygems.org/api/v1/versions/{package}.json");
    match http_get_json(&url)? {
        None => Ok(false),
        Some(val) => {
            if let Some(versions) = val.as_array() {
                for v in versions {
                    if v["number"].as_str() == Some(version) {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
    }
}

fn check_hex(package: &str, version: &str) -> Result<bool> {
    let url = format!("https://hex.pm/api/packages/{package}/releases/{version}");
    http_get_ok(&url)
}

fn check_maven(package: &str, version: &str) -> Result<bool> {
    // package format: groupId:artifactId
    let (group_id, artifact_id) = if let Some(colon) = package.find(':') {
        (&package[..colon], &package[colon + 1..])
    } else {
        anyhow::bail!("Maven package must be 'groupId:artifactId', got: {package}");
    };
    let group_path = group_id.replace('.', "/");
    let url = format!("https://repo1.maven.org/maven2/{group_path}/{artifact_id}/{version}/");
    http_get_ok(&url)
}

fn check_nuget(package: &str, version: &str, source: Option<&str>) -> Result<bool> {
    let base = source.unwrap_or("https://api.nuget.org");
    let pkg_lower = package.to_lowercase();
    let url = format!("{base}/v3/registration5-gz-semver2/{pkg_lower}/{version}.json");
    http_get_ok(&url)
}

fn check_packagist(package: &str, version: &str) -> Result<bool> {
    let url = format!("https://repo.packagist.org/p2/{package}.json");
    match http_get_json(&url)? {
        None => Ok(false),
        Some(val) => {
            if let Some(packages) = val["packages"][package].as_array() {
                for pkg in packages {
                    if pkg["version"].as_str() == Some(version) || pkg["version_normalized"].as_str() == Some(version) {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
    }
}

fn check_homebrew(package: &str, _version: &str, tap_repo: Option<&str>) -> Result<bool> {
    // Use the Homebrew formula API: https://formulae.brew.sh/api/formula/{name}.json
    let repo = tap_repo.unwrap_or("Homebrew/homebrew-core");
    if repo == "Homebrew/homebrew-core" {
        let url = format!("https://formulae.brew.sh/api/formula/{package}.json");
        return http_get_ok(&url);
    }
    // For third-party taps: use GitHub API to check formula existence.
    let url = format!("https://raw.githubusercontent.com/{repo}/HEAD/Formula/{package}.rb");
    http_get_ok(&url)
}

fn check_github_release(
    package: &str,
    version: &str,
    repo: Option<&str>,
    asset_prefix: Option<&str>,
    required_assets: &[String],
) -> Result<bool> {
    let repo = repo
        .filter(|r| !r.is_empty())
        .with_context(|| format!("--repo is required for github-release check of {package}"))?;
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
    let agent = build_agent();
    let response = agent
        .get(&url)
        .header("User-Agent", "alef-publish/1.0")
        .header("Accept", "application/vnd.github+json")
        .call();
    let resp = match classify(response).with_context(|| format!("GitHub API GET {url}"))? {
        HttpOutcome::Ok(resp) => resp,
        HttpOutcome::NotFound => return Ok(false),
    };

    let asset_prefix = asset_prefix.filter(|s| !s.is_empty());
    let has_asset_filter = asset_prefix.is_some() || !required_assets.is_empty();
    if !has_asset_filter {
        return Ok(true);
    }

    let body = resp
        .into_body()
        .read_to_string()
        .with_context(|| format!("reading body from {url}"))?;
    let json: serde_json::Value = serde_json::from_str(&body).with_context(|| format!("parsing JSON from {url}"))?;
    let asset_names: Vec<&str> = json["assets"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    if let Some(prefix) = asset_prefix
        && !asset_names.iter().any(|n| n.starts_with(prefix))
    {
        return Ok(false);
    }
    for required in required_assets {
        if !asset_names.iter().any(|n| *n == required) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_display() {
        assert_eq!(Registry::Pypi.to_string(), "pypi");
        assert_eq!(Registry::Npm.to_string(), "npm");
        assert_eq!(Registry::GithubRelease.to_string(), "github-release");
    }

    #[test]
    fn maven_package_parse_colon() {
        // Just verify the URL construction doesn't panic.
        let result = check_maven("com.example:my-lib", "1.0.0");
        // Network unavailable in CI — we just check it doesn't crash with wrong format.
        // It will fail with a network error, not a parse error.
        let _ = result; // ignore network errors in unit tests
    }

    #[test]
    fn maven_package_no_colon_errors() {
        let result = check_maven("invalid-package-name", "1.0.0");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("groupId:artifactId"));
    }

    #[test]
    fn extra_params_default() {
        let extra = ExtraParams::default();
        assert!(extra.nuget_source.is_none());
        assert!(extra.tap_repo.is_none());
        assert!(extra.repo.is_none());
        assert!(extra.asset_prefix.is_none());
        assert!(extra.required_assets.is_empty());
    }

    #[test]
    fn github_release_requires_repo() {
        let result = check_github_release("alef", "1.0.0", None, None, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("--repo"));
    }
}
