//! Registry version existence checker.
//!
//! For each supported package registry, performs a lightweight HTTP lookup to
//! determine whether a specific version of a package is published.
//!
//! Replaces:
//! - `actions/check-registry/action.yml`
//! - `sample_core/scripts/publish/check_*.sh`

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
    Scoop,
    GithubRelease,
    /// pub.dev (Dart packages).
    Pub,
    /// Zig: no central registry, checks GitHub release tag existence.
    Zig,
    /// Swift Package Index: no central registry, checks GitHub release tag existence
    /// (SPI auto-discovers new tags from Git).
    Swift,
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
            Registry::Scoop => write!(f, "scoop"),
            Registry::GithubRelease => write!(f, "github-release"),
            Registry::Pub => write!(f, "pub"),
            Registry::Zig => write!(f, "zig"),
            Registry::Swift => write!(f, "swift"),
        }
    }
}

/// Check whether `package@version` exists in `registry`.
///
/// `extra` carries registry-specific parameters:
/// - Maven: `package` is `groupId:artifactId` (colon-separated).
/// - NuGet: `source_url` override (defaults to `https://api.nuget.org`).
/// - Homebrew: `tap_repo` in `owner/repo` form (e.g. `Homebrew/homebrew-core`).
/// - Scoop: `tap_repo` names the bucket repository in `owner/repo` form (e.g.
///   `xberg-io/scoop-bucket`); required, since Scoop has no default/central bucket.
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
        Registry::Scoop => check_scoop(package, version, extra.tap_repo.as_deref())?,
        Registry::GithubRelease => {
            let exists = check_github_release(
                package,
                version,
                extra.repo.as_deref(),
                extra.asset_prefix.as_deref(),
                &extra.required_assets,
            )?;
            if exists && !has_asset_filter(extra.asset_prefix.as_deref(), &extra.required_assets) {
                // ~keep A CI-sourced --required-assets/--asset-prefix value that expands to
                // nothing looks identical, at the CLI layer, to the flags never being passed.
                // Warn so "the release exists" is not silently read as "all artifacts are
                // attached" — Zig/Swift below intentionally never supply these filters and
                // must not trigger this warning.
                tracing::warn!(
                    "check-registry --registry github-release: no --asset-prefix or \
                     --required-assets given; verified only that the release exists, not which \
                     artifacts are attached to it"
                );
            }
            exists
        }
        Registry::Pub => check_pub(package, version)?,
        Registry::Zig | Registry::Swift => check_github_release(package, version, extra.repo.as_deref(), None, &[])?,
    };

    if output_json {
        let out = json!({
            "registry": registry.to_string(),
            "package": package,
            "version": version,
            "exists": exists,
        });
        crate::bin_cli::output::payload(serde_json::to_string_pretty(&out)?);
    } else {
        crate::bin_cli::output::line(format!("exists={}", if exists { "true" } else { "false" }));
    }

    Ok(exists)
}

/// Extra parameters for registry-specific checks.
#[derive(Debug, Default)]
pub struct ExtraParams {
    /// NuGet source URL override.
    pub nuget_source: Option<String>,
    /// Homebrew tap repository, or Scoop bucket repository (`owner/repo`).
    pub tap_repo: Option<String>,
    /// GitHub repository (`owner/repo`) for GitHub Release check.
    pub repo: Option<String>,
    /// Asset name prefix (github-release): require at least one asset whose
    /// name starts with this prefix to consider the release "exists".
    pub asset_prefix: Option<String>,
    /// Required asset names (github-release): all must be present.
    pub required_assets: Vec<String>,
}

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
    let response = agent.get(&url).header("User-Agent", "alef-publish/1.0").call();
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

fn check_pub(package: &str, version: &str) -> Result<bool> {
    let url = format!("https://pub.dev/api/packages/{package}/versions/{version}");
    http_get_ok(&url)
}

fn check_maven(package: &str, version: &str) -> Result<bool> {
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

fn check_homebrew(package: &str, version: &str, tap_repo: Option<&str>) -> Result<bool> {
    let repo = tap_repo.unwrap_or("Homebrew/homebrew-core");
    if repo == "Homebrew/homebrew-core" {
        let url = format!("https://formulae.brew.sh/api/formula/{package}.json");
        let Some(json) = http_get_json(&url)? else {
            return Ok(false);
        };
        let formula_version = json
            .get("versions")
            .and_then(|v| v.get("stable"))
            .and_then(|v| v.as_str());
        return Ok(formula_version == Some(version));
    }
    let url = format!("https://raw.githubusercontent.com/{repo}/HEAD/Formula/{package}.rb");
    let agent = build_agent();
    let response = agent.get(&url).header("User-Agent", "alef-publish/1.0").call();
    let resp = match classify(response).with_context(|| format!("HTTP GET {url}"))? {
        HttpOutcome::Ok(resp) => resp,
        HttpOutcome::NotFound => return Ok(false),
    };
    let body = resp
        .into_body()
        .read_to_string()
        .with_context(|| format!("reading body from {url}"))?;

    if body.contains(&format!("version \"{version}\"")) || body.contains(&format!("version '{version}'")) {
        return Ok(true);
    }
    for line in body.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("url ") && !trimmed.starts_with("url(") {
            continue;
        }
        if trimmed.contains(&format!("/v{version}.tar.gz"))
            || trimmed.contains(&format!("/v{version}.zip"))
            || trimmed.contains(&format!("/{version}.tar.gz"))
            || trimmed.contains(&format!("/{version}.zip"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Check whether `version` is published as a Scoop manifest in `tap_repo`'s bucket.
///
/// Scoop has no default/central bucket (unlike Homebrew's `homebrew-core`), so `tap_repo`
/// is required. Manifests live under the bucket repository's `bucket/` directory per the
/// Scoop manifest convention: `bucket/<package>.json` with a top-level `"version"` field.
fn check_scoop(package: &str, version: &str, tap_repo: Option<&str>) -> Result<bool> {
    let repo = tap_repo
        .filter(|r| !r.is_empty())
        .with_context(|| format!("--tap-repo is required for scoop check of {package} (no default bucket)"))?;
    let url = format!("https://raw.githubusercontent.com/{repo}/HEAD/bucket/{package}.json");
    let Some(json) = http_get_json(&url)? else {
        return Ok(false);
    };
    Ok(json.get("version").and_then(|v| v.as_str()) == Some(version))
}

/// Pick the GitHub token to authenticate with, given the two candidate
/// environment variables read by [`github_auth_token`]. Kept separate from
/// the env lookup so the precedence/empty-string rules are unit-testable
/// without mutating real process environment state. An empty value is treated
/// as absent rather than as a token, so a workflow that exports an empty
/// `GITHUB_TOKEN` still falls through to `GH_TOKEN` instead of going anonymous.
fn resolve_github_token(github_token: Option<String>, gh_token: Option<String>) -> Option<String> {
    github_token.into_iter().chain(gh_token).find(|token| !token.is_empty())
}

/// Read a GitHub token from the environment, if one is available.
///
/// Checks `GITHUB_TOKEN` (set on every GitHub Actions job by default) then
/// `GH_TOKEN` (the `gh` CLI convention). Authenticated requests to
/// `api.github.com` get a 5,000 req/hour rate limit instead of the 60
/// req/hour applied to anonymous requests from a shared runner IP -- the
/// unauthenticated limit is what produced HTTP 403s on hosted runners.
fn github_auth_token() -> Option<String> {
    resolve_github_token(std::env::var("GITHUB_TOKEN").ok(), std::env::var("GH_TOKEN").ok())
}

/// Whether `asset_prefix`/`required_assets` request an asset-level check, versus
/// only checking that the release itself exists. An asset prefix of only
/// whitespace-empty content does not count as a filter.
fn has_asset_filter(asset_prefix: Option<&str>, required_assets: &[String]) -> bool {
    asset_prefix.is_some_and(|s| !s.is_empty()) || !required_assets.is_empty()
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
    let mut request = agent
        .get(&url)
        .header("User-Agent", "alef-publish/1.0")
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = github_auth_token() {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = request.call();
    let resp = match classify(response).with_context(|| format!("GitHub API GET {url}"))? {
        HttpOutcome::Ok(resp) => resp,
        HttpOutcome::NotFound => return Ok(false),
    };

    if !has_asset_filter(asset_prefix, required_assets) {
        return Ok(true);
    }
    let asset_prefix = asset_prefix.filter(|s| !s.is_empty());

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
        assert_eq!(Registry::Homebrew.to_string(), "homebrew");
        assert_eq!(Registry::Scoop.to_string(), "scoop");
        assert_eq!(Registry::GithubRelease.to_string(), "github-release");
        assert_eq!(Registry::Pub.to_string(), "pub");
        assert_eq!(Registry::Zig.to_string(), "zig");
        assert_eq!(Registry::Swift.to_string(), "swift");
    }

    #[test]
    fn scoop_requires_tap_repo() {
        let result = check_scoop("alef", "1.0.0", None);
        assert!(
            result.is_err(),
            "scoop has no default bucket and must require --tap-repo"
        );
        assert!(result.unwrap_err().to_string().contains("--tap-repo"));
    }

    #[test]
    fn scoop_requires_non_empty_tap_repo() {
        let result = check_scoop("alef", "1.0.0", Some(""));
        assert!(result.is_err(), "an empty --tap-repo must not be treated as present");
    }

    #[test]
    fn zig_swift_require_repo() {
        let extra = ExtraParams::default();
        for registry in [Registry::Zig, Registry::Swift] {
            let result = check(registry, "alef", "1.0.0", &extra, false);
            assert!(
                result.is_err(),
                "{registry} should fail without a --repo because it delegates to github-release"
            );
        }
    }

    #[test]
    fn maven_package_parse_colon() {
        let result = check_maven("com.example:my-lib", "1.0.0");
        let _ = result;
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

    #[test]
    fn resolve_github_token_prefers_github_token() {
        let resolved = resolve_github_token(Some("actions-token".to_string()), Some("gh-cli-token".to_string()));
        assert_eq!(resolved.as_deref(), Some("actions-token"));
    }

    #[test]
    fn resolve_github_token_falls_back_to_gh_token() {
        let resolved = resolve_github_token(None, Some("gh-cli-token".to_string()));
        assert_eq!(resolved.as_deref(), Some("gh-cli-token"));
    }

    #[test]
    fn resolve_github_token_none_when_both_absent() {
        assert_eq!(resolve_github_token(None, None), None);
    }

    #[test]
    fn resolve_github_token_treats_empty_string_as_absent() {
        assert_eq!(
            resolve_github_token(Some(String::new()), Some("gh-cli-token".to_string())).as_deref(),
            Some("gh-cli-token"),
            "an empty GITHUB_TOKEN must not suppress a usable GH_TOKEN"
        );
        assert_eq!(resolve_github_token(Some(String::new()), None), None);
        assert_eq!(resolve_github_token(None, Some(String::new())), None);
    }

    #[test]
    fn has_asset_filter_false_when_both_absent() {
        assert!(!has_asset_filter(None, &[]));
    }

    #[test]
    fn has_asset_filter_false_when_prefix_is_empty_string() {
        // A `--asset-prefix ""` (e.g. an unset CI variable expanded and quoted) must not
        // count as a real filter, the same as omitting the flag entirely.
        assert!(!has_asset_filter(Some(""), &[]));
    }

    #[test]
    fn has_asset_filter_true_with_prefix() {
        assert!(has_asset_filter(Some("alef-"), &[]));
    }

    #[test]
    fn has_asset_filter_true_with_required_assets() {
        assert!(has_asset_filter(None, &["alef-linux-x64.tar.gz".to_string()]));
    }
}
