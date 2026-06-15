/// Return the public docs label for a SemVer-like version.
///
/// API reference pages keep their package release badge intact, but item-level
/// feature labels should be stable across patch and prerelease churn.
pub(super) fn major_minor(version: &str) -> String {
    let version = version.trim().trim_start_matches('v');
    let core = version
        .split_once('-')
        .map_or(version, |(core, _)| core)
        .split_once('+')
        .map_or(version, |(core, _)| core);

    let mut parts = core.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) if !major.is_empty() && !minor.is_empty() => format!("{major}.{minor}"),
        _ => version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::major_minor;

    #[test]
    fn strips_patch_version() {
        assert_eq!(major_minor("5.0.0"), "5.0");
    }

    #[test]
    fn strips_prerelease_and_patch_version() {
        assert_eq!(major_minor("v1.6.0-rc.1"), "1.6");
    }

    #[test]
    fn preserves_unexpected_shape() {
        assert_eq!(major_minor("next"), "next");
    }
}
