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
/// use alef::core::version::to_r_version;
/// assert_eq!(to_r_version("1.8.0"), "1.8.0");
/// assert_eq!(to_r_version("4.10.0-rc.1"), "4.10.0.9001");
/// assert_eq!(to_r_version("4.10.0-rc.15"), "4.10.0.9015");
/// assert_eq!(to_r_version("0.1.0-alpha.2"), "0.1.0.9000");
/// ```
pub fn to_r_version(version: &str) -> String {
    let Some((base, pre)) = version.split_once('-') else {
        return version.to_string();
    };

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

/// Convert a semver pre-release version to PEP 440 format for Python/PyPI.
///
/// Maps SemVer pre-release identifiers to the PEP 440 canonical short forms:
///   `alpha` → `a`, `beta` → `b`, `rc` → `rc`
/// and strips any remaining dots from the numeric suffix.
///
/// # Examples
///
/// ```
/// use alef::core::version::to_pep440;
/// assert_eq!(to_pep440("1.2.3"), "1.2.3");
/// assert_eq!(to_pep440("3.6.0-rc.1"), "3.6.0rc1");
/// assert_eq!(to_pep440("1.0.0-alpha.2"), "1.0.0a2");
/// assert_eq!(to_pep440("1.0.0-beta.3"), "1.0.0b3");
/// ```
pub fn to_pep440(version: &str) -> String {
    let Some((base, pre)) = version.split_once('-') else {
        return version.to_string();
    };
    let pep = pre
        .replace("alpha.", "a")
        .replace("alpha", "a")
        .replace("beta.", "b")
        .replace("beta", "b")
        .replace("rc.", "rc")
        .replace('.', "");
    format!("{base}{pep}")
}

/// Convert a semver pre-release version to RubyGems canonical prerelease format.
///
/// RubyGems rejects the dash-form prerelease syntax that cargo uses
/// (`Gem::Version.new("1.8.0-rc.2")` raises) and requires the `.pre.` form.
///
/// # Examples
///
/// ```
/// use alef::core::version::to_rubygems_prerelease;
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

/// Convert a semver version to a .NET-compatible 4-component assembly version
/// (`MAJOR.MINOR.PATCH.REVISION`).
///
/// .NET's `AssemblyVersion` and `AssemblyFileVersion` attributes require a strict
/// 4-component numeric form. SemVer pre-release suffixes (`1.9.0-rc.48`) are
/// rejected by the compiler, so the prerelease is stripped and the revision is
/// set to `0` — pre-releases in the same `MAJOR.MINOR.PATCH` series all stamp
/// the same assembly version, which matches the .NET convention for in-series
/// binary compatibility. The full SemVer is still preserved on the NuGet
/// `<Version>` and `<InformationalVersion>` properties.
///
/// # Examples
///
/// ```
/// use alef::core::version::to_dotnet_assembly_version;
/// assert_eq!(to_dotnet_assembly_version("1.9.0"), "1.9.0.0");
/// assert_eq!(to_dotnet_assembly_version("1.9.0-rc.48"), "1.9.0.0");
/// assert_eq!(to_dotnet_assembly_version("0.1.0-alpha.2"), "0.1.0.0");
/// ```
pub fn to_dotnet_assembly_version(version: &str) -> String {
    let base = version.split_once('-').map_or(version, |(b, _)| b);
    let mut parts: Vec<&str> = base.split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.truncate(3);
    format!("{}.0", parts.join("."))
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

    #[test]
    fn pep440_release_version_is_unchanged() {
        assert_eq!(to_pep440("1.2.3"), "1.2.3");
        assert_eq!(to_pep440("0.1.0"), "0.1.0");
    }

    #[test]
    fn pep440_rc_prerelease_canonical_form() {
        assert_eq!(to_pep440("3.6.0-rc.1"), "3.6.0rc1");
        assert_eq!(to_pep440("4.10.0-rc.9"), "4.10.0rc9");
        assert_eq!(to_pep440("0.1.0-rc.1"), "0.1.0rc1");
    }

    #[test]
    fn pep440_alpha_beta_prereleases() {
        assert_eq!(to_pep440("1.0.0-alpha.2"), "1.0.0a2");
        assert_eq!(to_pep440("1.0.0-beta.3"), "1.0.0b3");
    }

    #[test]
    fn dotnet_release_version_pads_to_four_components() {
        assert_eq!(to_dotnet_assembly_version("1.9.0"), "1.9.0.0");
        assert_eq!(to_dotnet_assembly_version("0.1.0"), "0.1.0.0");
    }

    #[test]
    fn dotnet_strips_prerelease_suffix() {
        assert_eq!(to_dotnet_assembly_version("1.9.0-rc.48"), "1.9.0.0");
        assert_eq!(to_dotnet_assembly_version("0.1.0-alpha.2"), "0.1.0.0");
        assert_eq!(to_dotnet_assembly_version("0.1.0-beta.3"), "0.1.0.0");
    }

    #[test]
    fn dotnet_short_versions_pad_zero_components() {
        assert_eq!(to_dotnet_assembly_version("1"), "1.0.0.0");
        assert_eq!(to_dotnet_assembly_version("1.2"), "1.2.0.0");
    }
}
