//! Cross-ecosystem version-format conversions.
//!
//! Cargo and the polyglot package registries disagree on prerelease syntax;
//! these helpers normalize between them so each binding manifest receives a
//! version string that its tooling will accept.

/// Convert a semver pre-release version to R / CRAN-compatible version format.
///
/// R's `package_version()` rejects SemVer dash-form prereleases like
/// `4.10.0-rc.15`. CRAN convention is to encode development versions as a
/// fourth component with a high value (9000+). This helper maps:
///   `4.10.0`      → `4.10.0`        (unchanged)
///   `4.10.0-rc.1` → `4.10.0.9001`
///   `4.10.0-rc.15`→ `4.10.0.9015`
///
/// The numeric suffix preserves ordering: rc.1 < rc.15 < release.
///
/// # Examples
///
/// ```
/// use crate::core::version::to_r_version;
/// assert_eq!(to_r_version("1.8.0"), "1.8.0");
/// assert_eq!(to_r_version("4.10.0-rc.1"), "4.10.0.9001");
/// assert_eq!(to_r_version("4.10.0-rc.15"), "4.10.0.9015");
/// assert_eq!(to_r_version("0.1.0-alpha.2"), "0.1.0.9000");
/// ```
pub fn to_r_version(version: &str) -> String {
    let Some((base, pre)) = version.split_once('-') else {
        return version.to_string();
    };

    // For rc (release candidate) prereleases, encode the RC number as an offset
    // from 9000 so that ordering is preserved within a series (rc.1 → 9001, rc.15 → 9015).
    // All other prerelease identifiers (alpha, beta, dev, …) map to the base value 9000.
    let numeric_offset: u32 = if pre.starts_with("rc") {
        pre.split('.')
            .filter_map(|part| part.parse::<u32>().ok())
            .next_back()
            .unwrap_or(0)
    } else {
        0
    };

    format!("{base}.{}", 9000 + numeric_offset)
}

/// Convert a semver pre-release version to RubyGems canonical prerelease format.
///
/// RubyGems rejects the dash-form prerelease syntax that cargo uses
/// (`Gem::Version.new("1.8.0-rc.2")` raises) and requires the `.pre.` form.
///
/// # Examples
///
/// ```
/// use crate::core::version::to_rubygems_prerelease;
/// assert_eq!(to_rubygems_prerelease("1.8.0"), "1.8.0");
/// assert_eq!(to_rubygems_prerelease("1.8.0-rc.2"), "1.8.0.pre.rc.2");
/// assert_eq!(to_rubygems_prerelease("0.1.0-alpha.2"), "0.1.0.pre.alpha.2");
/// ```
pub fn to_rubygems_prerelease(version: &str) -> String {
    if let Some((base, pre)) = version.split_once('-') {
        let normalized_pre = pre.replace(['-', '_'], ".");
        format!("{base}.pre.{normalized_pre}")
    } else {
        version.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_version_is_unchanged() {
        assert_eq!(to_rubygems_prerelease("1.8.0"), "1.8.0");
        assert_eq!(to_rubygems_prerelease("0.1.0"), "0.1.0");
    }

    #[test]
    fn rc_prerelease_uses_pre_dot_form() {
        assert_eq!(to_rubygems_prerelease("1.8.0-rc.2"), "1.8.0.pre.rc.2");
    }

    #[test]
    fn alpha_and_beta_prereleases_normalize() {
        assert_eq!(to_rubygems_prerelease("0.1.0-alpha.2"), "0.1.0.pre.alpha.2");
        assert_eq!(to_rubygems_prerelease("0.1.0-beta.3"), "0.1.0.pre.beta.3");
    }

    #[test]
    fn dashes_in_prerelease_become_dots() {
        assert_eq!(to_rubygems_prerelease("1.0.0-pre-rc-2"), "1.0.0.pre.pre.rc.2");
    }

    // --- to_r_version tests ---

    #[test]
    fn r_release_version_is_unchanged() {
        assert_eq!(to_r_version("1.8.0"), "1.8.0");
        assert_eq!(to_r_version("0.1.0"), "0.1.0");
        assert_eq!(to_r_version("4.10.0"), "4.10.0");
    }

    #[test]
    fn r_rc_prerelease_gets_9000_offset() {
        assert_eq!(to_r_version("4.10.0-rc.1"), "4.10.0.9001");
        assert_eq!(to_r_version("4.10.0-rc.15"), "4.10.0.9015");
        assert_eq!(to_r_version("1.8.0-rc.2"), "1.8.0.9002");
    }

    #[test]
    fn r_alpha_without_number_gets_9000() {
        assert_eq!(to_r_version("0.1.0-alpha"), "0.1.0.9000");
    }

    #[test]
    fn r_alpha_with_number_uses_offset() {
        assert_eq!(to_r_version("0.1.0-alpha.2"), "0.1.0.9000");
    }
}
