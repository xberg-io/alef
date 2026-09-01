//! Version checks the shared enumerator otherwise misses: a Ruby `.gemspec`'s literal version
//! and the release-URL version embedded in a repository's root `Package.swift`.
//!
//! ~keep Split into its own module -- not appended to `version_manifests.rs` or
//! `validate_versions.rs` -- because `validate_versions.rs` sits at alef's 1,000-line
//! `file-modularization` cap (1,022, its recorded ratchet ceiling) and must not grow.
//! `collect_checks` extends its `Vec<VersionCheck>` with this module's own [`collect`], the same
//! pattern `version_manifests::collect` already uses for `.csproj`/Dart/Zig/`Cargo.lock`.

use super::validate_versions::VersionCheck;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::core::version::to_rubygems_prerelease;
use std::path::{Path, PathBuf};

/// `.gemspec` and root `Package.swift` version checks, gated on whether this crate enables Ruby
/// / Swift at all. A disabled language contributes nothing -- absence of the language is not a
/// failure. An ENABLED language with none of its expected files present IS a failure: see
/// [`collect_gemspec_checks`] and [`collect_swift_package_check`] for why a check that globs or
/// looks for a file and finds none must not silently read the same as a clean pass.
pub(super) fn collect(config: &ResolvedCrateConfig, workspace_root: &Path, canonical: &str) -> Vec<VersionCheck> {
    let mut checks = Vec::new();
    collect_gemspec_checks(config, workspace_root, canonical, &mut checks);
    collect_swift_package_check(config, workspace_root, canonical, &mut checks);
    checks
}

/// One check per `.gemspec` glob-matched directly under the configured Ruby package directory --
/// never a hardcoded filename, since a consumer names its gem after its own package, not alef's.
///
/// ~keep A crate that enables Ruby but ships zero `.gemspec` files is not "nothing to check":
/// unlike an optional manifest such as `composer.json`, a Ruby package with no gemspec cannot be
/// published to RubyGems at all. This pushes a FAILING check rather than skipping -- the specific
/// trap `prove-the-check-fired` calls out, where a glob that matches nothing must not report the
/// same as a glob that matched and found everything consistent.
fn collect_gemspec_checks(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    canonical: &str,
    checks: &mut Vec<VersionCheck>,
) {
    if !config.targets(Language::Ruby) {
        return;
    }
    let ruby_dir = config.package_dir(Language::Ruby);
    let gemspecs = glob_in_dir(workspace_root, &ruby_dir, "*.gemspec");
    if gemspecs.is_empty() {
        tracing::error!(
            directory = %ruby_dir,
            "Ruby is enabled but no .gemspec was found in the configured package directory"
        );
        checks.push(VersionCheck {
            label: format!("{ruby_dir}/*.gemspec"),
            found: None,
            matches: false,
            blocked_on_publish: None,
        });
        return;
    }
    let expected = to_rubygems_prerelease(canonical);
    for path in gemspecs {
        let label = relative_label(workspace_root, &path);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if let Some(found) = read_gemspec_version(&content) {
                    let matches = found == expected;
                    checks.push(VersionCheck {
                        label,
                        found: Some(found),
                        matches,
                        blocked_on_publish: None,
                    });
                }
                // No `spec.version = ...` line: not managed here, same as any other manifest's
                // `NoVersionField` outcome in `validate_versions.rs`.
            }
            Err(error) => {
                tracing::error!(gemspec = %label, reason = %error, "gemspec exists but could not be read");
                checks.push(VersionCheck {
                    label,
                    found: None,
                    matches: false,
                    blocked_on_publish: None,
                });
            }
        }
    }
}

/// The release-URL version inside a repository's root `Package.swift`
/// (`.binaryTarget(url: "{repository}/releases/download/v{version}/...")`).
///
/// ~keep Only in scope when Swift is enabled AND a repository is configured:
/// `scaffold::scaffold_meta` is the same config `scaffold_swift` reads to decide whether to emit
/// a root `Package.swift` at all (it needs a repository URL for consumers to depend on). A Swift
/// crate with no repository configured has correctly never had one, and this check would
/// otherwise fail every such repo for a file alef itself never scaffolds.
fn collect_swift_package_check(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    canonical: &str,
    checks: &mut Vec<VersionCheck>,
) {
    if !config.targets(Language::Swift) || crate::scaffold::scaffold_meta(config).repository.is_none() {
        return;
    }
    let label = "Package.swift".to_string();
    let path = workspace_root.join(&label);
    if !path.exists() {
        tracing::error!("Swift is enabled with a configured repository but Package.swift is missing");
        checks.push(VersionCheck {
            label,
            found: None,
            matches: false,
            blocked_on_publish: None,
        });
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            if let Some(found) = read_swift_release_version(&content) {
                let matches = found == canonical;
                checks.push(VersionCheck {
                    label,
                    found: Some(found),
                    matches,
                    blocked_on_publish: None,
                });
            }
            // No release-URL version found (e.g. an unresolved `__ALEF_SWIFT_VERSION__`
            // placeholder pre-sync): not managed here, mirrors the gemspec's skip above.
        }
        Err(error) => {
            tracing::error!(reason = %error, "Package.swift exists but could not be read");
            checks.push(VersionCheck {
                label,
                found: None,
                matches: false,
                blocked_on_publish: None,
            });
        }
    }
}

/// Extract `spec.version = "..."` from a `.gemspec`. Plain line-scan, mirroring
/// `validate_versions::read_ruby_version`, rather than pulling in a regex for one substitution.
fn read_gemspec_version(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("spec.version") && trimmed.contains('=') {
            let val = trimmed.split_once('=')?.1.trim();
            return Some(val.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

/// Extract the version from `.../releases/download/v<version>/...` in a root `Package.swift`.
/// Requires the captured text to start with an ASCII digit so an unresolved
/// `__ALEF_SWIFT_VERSION__` placeholder is not misread as a version string.
fn read_swift_release_version(content: &str) -> Option<String> {
    const MARKER: &str = "releases/download/v";
    let start = content.find(MARKER)? + MARKER.len();
    let end = content[start..].find('/')?;
    let candidate = &content[start..start + end];
    candidate
        .starts_with(|character: char| character.is_ascii_digit())
        .then(|| candidate.to_string())
}

fn glob_in_dir(workspace_root: &Path, directory: &str, suffix: &str) -> Vec<PathBuf> {
    let root = glob::Pattern::escape(&workspace_root.to_string_lossy());
    let directory = directory.trim_matches(['/', '\\']);
    let pattern = format!("{root}/{directory}/{suffix}");
    glob::glob(&pattern).into_iter().flatten().flatten().collect()
}

fn relative_label(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
        .replace('\\', "/")
}

#[cfg(test)]
#[path = "ruby_swift_versions/tests.rs"]
mod tests;
