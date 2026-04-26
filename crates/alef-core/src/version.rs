//! Cross-ecosystem version-format conversions.
//!
//! Cargo and the polyglot package registries disagree on prerelease syntax;
//! these helpers normalize between them so each binding manifest receives a
//! version string that its tooling will accept.

/// Convert a semver pre-release version to RubyGems canonical prerelease format.
///
/// RubyGems rejects the dash-form prerelease syntax that cargo uses
/// (`Gem::Version.new("1.8.0-rc.2")` raises) and requires the `.pre.` form.
///
/// # Examples
///
/// ```
/// use alef_core::version::to_rubygems_prerelease;
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
}
