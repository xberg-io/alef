/// Derive a reverse-DNS package name from a repository URL.
///
/// Recognises `https?://<host>/<org>/<rest>` and produces `<reversed-host>.<org>`,
/// where the host is split into labels and reversed (so `github.com` → `com.github`),
/// the org's hyphens become underscores (Java identifier rules), and the trailing
/// path is ignored. Returns `None` when the URL is missing a host or path segment.
///
/// Examples:
/// - `https://github.com/kreuzberg-dev/kreuzberg` → `Some("com.github.kreuzberg_dev")`
/// - `https://github.com/spikard-rs/spikard`     → `Some("com.github.spikard_rs")`
/// - `https://gitlab.com/foo/bar`                → `Some("com.gitlab.foo")`
/// - `https://example.invalid/x`                 → `Some("invalid.example.x")`
/// - `https://github.com/`                       → `None` (no org segment)
pub fn derive_reverse_dns_package(repo_url: &str) -> Option<String> {
    let after_scheme = repo_url.split_once("://").map(|(_, rest)| rest).unwrap_or(repo_url);
    let mut parts = after_scheme.split('/').filter(|s| !s.is_empty());
    let host = parts.next()?;
    let org = parts.next()?;

    let host_reversed: Vec<String> = host
        .split('.')
        .filter(|s| !s.is_empty())
        .rev()
        .map(|s| s.replace('-', "_"))
        .collect();
    if host_reversed.is_empty() {
        return None;
    }

    let mut pkg = host_reversed.join(".");
    pkg.push('.');
    pkg.push_str(&org.replace('-', "_"));
    Some(pkg)
}

/// Derive a Go module path from a repository URL.
///
/// Strips the `https?://` scheme and any trailing slash. Returns `None` when
/// the URL has no host or no path segment beyond the host.
///
/// Examples:
/// - `https://github.com/kreuzberg-dev/kreuzberg` → `Some("github.com/kreuzberg-dev/kreuzberg")`
/// - `https://github.com/foo/bar/` → `Some("github.com/foo/bar")`
/// - `https://github.com/` → `None`
pub fn derive_go_module_from_repo(repo_url: &str) -> Option<String> {
    let after_scheme = repo_url.split_once("://").map(|(_, rest)| rest).unwrap_or(repo_url);
    let trimmed = after_scheme.trim_end_matches('/');
    let mut parts = trimmed.split('/');
    let host = parts.next().filter(|s| !s.is_empty())?;
    let org = parts.next().filter(|s| !s.is_empty())?;
    let repo_segment = parts.next().filter(|s| !s.is_empty());

    let mut module = format!("{host}/{org}");
    if let Some(repo) = repo_segment {
        module.push('/');
        module.push_str(repo);
    }
    Some(module)
}

/// Extract the org segment from a repository URL.
///
/// Recognises `https?://<host>/<org>/<rest>` and returns `<org>` verbatim
/// (no case or punctuation transformation). Returns `None` when the URL is
/// missing a host or org segment.
///
/// Examples:
/// - `https://github.com/kreuzberg-dev/kreuzberg` → `Some("kreuzberg-dev")`
/// - `https://github.com/`                       → `None`
pub fn derive_repo_org(repo_url: &str) -> Option<String> {
    let after_scheme = repo_url.split_once("://").map(|(_, rest)| rest).unwrap_or(repo_url);
    let mut parts = after_scheme.split('/').filter(|s| !s.is_empty());
    let _host = parts.next()?;
    let org = parts.next()?;
    Some(org.to_string())
}

#[cfg(test)]
mod tests {
    use super::derive_reverse_dns_package;

    #[test]
    fn github_org_with_hyphen_underscores_in_package() {
        assert_eq!(
            derive_reverse_dns_package("https://github.com/kreuzberg-dev/kreuzberg"),
            Some("com.github.kreuzberg_dev".to_string())
        );
    }

    #[test]
    fn other_host_reverses_correctly() {
        assert_eq!(
            derive_reverse_dns_package("https://gitlab.com/foo/bar"),
            Some("com.gitlab.foo".to_string())
        );
    }

    #[test]
    fn missing_org_returns_none() {
        assert_eq!(derive_reverse_dns_package("https://github.com/"), None);
        assert_eq!(derive_reverse_dns_package("https://github.com"), None);
    }

    #[test]
    fn no_scheme_still_parses() {
        assert_eq!(
            derive_reverse_dns_package("github.com/foo/bar"),
            Some("com.github.foo".to_string())
        );
    }

    #[test]
    fn placeholder_url_derives_predictably() {
        assert_eq!(
            derive_reverse_dns_package("https://example.invalid/my-lib"),
            Some("invalid.example.my_lib".to_string())
        );
    }
}
