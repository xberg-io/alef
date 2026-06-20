use super::*;

pub(super) fn detect_stale_zig_hash(hash: &str, current_version: &str, pkg_name: &str) -> bool {
    // Hash format: `{pkg_name}-{version}-{multihash}`
    // Example: `demo_client-1.4.0-rc.50-Jfgk_NcsAQBpkv3XrckgE9vZmwDERDOandv0Ud6LXpHH`
    let prefix = format!("{pkg_name}-");
    if !hash.starts_with(&prefix) {
        return false;
    }

    // Remove the crate name prefix and split the rest by dashes.
    let rest = &hash[prefix.len()..];
    let parts: Vec<&str> = rest.split('-').collect();

    // Reconstruct the version by iterating through parts until we hit
    // the hash-like segment (long alphanumeric or underscore string).
    let mut version_parts: Vec<&str> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        // Last part is always the hash; don't include it.
        if i == parts.len() - 1 {
            break;
        }

        version_parts.push(part);

        // Heuristic: if this part looks like a hash (>20 chars or contains underscores/alphanumerics),
        // and we've accumulated at least one version part, stop here.
        if part.len() > 20 || (part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && i > 0) {
            // This is likely the hash segment; remove it from version_parts.
            version_parts.pop();
            break;
        }
    }

    let embedded_version = version_parts.join("-");

    if embedded_version != current_version {
        tracing::warn!(
            "zig package hash mismatch: hash contains version '{}', but current version is '{}'; \
             regenerate with `alef sync-versions`",
            embedded_version,
            current_version
        );
        return true;
    }

    false
}

/// Path to the on-disk hash cache: `~/.cache/alef/zig-hashes.json` on Unix /
/// `%LOCALAPPDATA%\alef\zig-hashes.json` on Windows.
///
/// Returns `None` when the home / local-app-data environment variable is unset.
fn zig_hash_cache_path() -> Option<std::path::PathBuf> {
    // XDG_CACHE_HOME takes precedence on Linux.
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Some(std::path::PathBuf::from(xdg).join("alef").join("zig-hashes.json"));
        }
    }
    // macOS and Linux: $HOME/.cache/alef/zig-hashes.json
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(
                std::path::PathBuf::from(home)
                    .join(".cache")
                    .join("alef")
                    .join("zig-hashes.json"),
            );
        }
    }
    // Windows: %LOCALAPPDATA%\alef\zig-hashes.json
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        if !local_app.is_empty() {
            return Some(std::path::PathBuf::from(local_app).join("alef").join("zig-hashes.json"));
        }
    }
    None
}

/// Read the (URL → hash) cache. Returns an empty map on any I/O error.
fn read_zig_hash_cache() -> std::collections::HashMap<String, String> {
    let Some(path) = zig_hash_cache_path() else {
        return std::collections::HashMap::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return std::collections::HashMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Persist a single (url → hash) entry into the cache.
fn write_zig_hash_cache_entry(url: &str, hash: &str) {
    let Some(path) = zig_hash_cache_path() else {
        return;
    };
    let mut map = read_zig_hash_cache();
    map.insert(url.to_string(), hash.to_string());
    let Ok(json) = serde_json::to_string_pretty(&map) else {
        return;
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, json).ok();
}

/// Fetch the content multihash for a Zig package tarball URL by shelling out
/// to `zig fetch <url>` from a scratch directory.
///
/// Returns the hash string (printed by `zig fetch` on stdout) on success, or
/// `None` when `zig fetch` is unavailable / returns a non-zero exit code /
/// produces no recognisable hash output.
fn fetch_zig_hash_from_network(url: &str) -> Option<String> {
    let tmp = tempfile::tempdir().ok()?;
    // Write a minimal stub build.zig.zon so `zig fetch` has a valid package
    // context to operate from. Without it, older zig versions refuse to run.
    let stub = r#".{
    .name = .zig_hash_fetch_stub,
    .version = "0.0.0",
    .fingerprint = 0x0000000000000001,
    .dependencies = .{},
    .paths = .{"build.zig.zon"},
}
"#;
    std::fs::write(tmp.path().join("build.zig.zon"), stub).ok()?;
    // `zig fetch <url>` (hash-only, no `--save`) still aborts with "no build.zig
    // file found" unless a build.zig exists in the directory tree, so write a
    // no-op one alongside the manifest.
    std::fs::write(
        tmp.path().join("build.zig"),
        "pub fn build(b: *@import(\"std\").Build) void {\n    _ = b;\n}\n",
    )
    .ok()?;

    let output = std::process::Command::new("zig")
        .arg("fetch")
        .arg(url)
        .current_dir(tmp.path())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // `zig fetch` prints the content multihash on stdout as a single line.
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .map(|s| s.to_string())
}

/// Resolve the content multihash for a Zig registry tarball URL.
///
/// Resolution order:
/// 1. `explicit` — a `hash` value set directly in `alef.toml` under
///    `[crates.e2e.registry.packages.zig]`. Takes precedence over everything.
/// 2. Cache — `~/.cache/alef/zig-hashes.json` keyed by URL.
/// 3. Network — shells out to `zig fetch <url>`, parses the printed hash,
///    writes the result back to the cache, and returns it.
/// 4. Fallback — logs a warning and returns `None`. Registry generation
///    requires an explicit hash before calling this helper, so this path is
///    only available to tests and non-publishable dry-run callers.
pub(super) fn resolve_zig_hash(explicit: Option<&str>, url: &str) -> Option<String> {
    // 1. Explicit override wins.
    if let Some(h) = explicit {
        return Some(h.to_string());
    }

    // 2. On-disk cache.
    let cache = read_zig_hash_cache();
    if let Some(h) = cache.get(url) {
        return Some(h.clone());
    }

    // 3. Network fetch.
    match fetch_zig_hash_from_network(url) {
        Some(h) => {
            write_zig_hash_cache_entry(url, &h);
            Some(h)
        }
        None => {
            tracing::warn!(
                "zig hash skipped — asset {} not yet published; regen after release",
                url
            );
            None
        }
    }
}

pub(super) fn supported_zig_platforms() -> &'static [&'static str] {
    &[
        "aarch64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ]
}

pub(super) fn uses_platform_registry_deps(platform_hashes: &BTreeMap<String, (String, Option<String>)>) -> bool {
    platform_hashes.keys().any(|platform| platform != "generic")
}

#[cfg(test)]
mod zig_hash_tests {
    use super::resolve_zig_hash;
    use crate::e2e::codegen::zig::build::render_build_zig_zon;
    use crate::e2e::config::DependencyMode;

    /// When an explicit hash is supplied via alef.toml it must be emitted
    /// verbatim — no network fetch, no cache lookup.
    #[test]
    fn explicit_hash_override_is_used_verbatim() {
        let url = "https://example.invalid/example-org/demo-client/releases/download/v1.4.0/demo-client-zig-v1.4.0-linux-x86_64.tar.gz";
        let pinned = "1220abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";
        let result = resolve_zig_hash(Some(pinned), url);
        assert_eq!(
            result.as_deref(),
            Some(pinned),
            "explicit hash must be returned unchanged; got: {result:?}"
        );
    }

    /// When the explicit hash is used it must be emitted in build.zig.zon (single generic tarball).
    #[test]
    fn build_zig_zon_emits_explicit_hash() {
        let hash = "12208badf00d";
        let mut platform_hashes = std::collections::BTreeMap::new();
        let url =
            "https://example.invalid/example-org/demo-client/releases/download/v1.4.0-rc.32/demo-client-zig-v1.4.0-rc.32.tar.gz"
                .to_string();
        platform_hashes.insert("generic".to_string(), (url, Some(hash.to_string())));
        let content = render_build_zig_zon(
            "demo_client",
            "../../packages/zig",
            DependencyMode::Registry,
            "1.4.0-rc.32",
            &platform_hashes,
            false,
            &[],
        );
        assert!(
            content.contains(&format!(".hash = \"{hash}\"")),
            "build.zig.zon must embed the explicit hash, got:\n{content}"
        );
        assert!(
            !content.contains(".hash = \"PLACEHOLDER\""),
            "build.zig.zon must not emit a placeholder hash when hash is provided, got:\n{content}"
        );
        // Verify the single generic (no-suffix) URL is present.
        assert!(
            content.contains("demo-client-zig-v1.4.0-rc.32.tar.gz"),
            "build.zig.zon must emit the generic source tarball URL (no platform suffix), got:\n{content}"
        );
    }

    #[test]
    fn build_zig_zon_emits_platform_hashes_as_lazy_dependencies() {
        let mut platform_hashes = std::collections::BTreeMap::new();
        platform_hashes.insert(
            "x86_64-unknown-linux-gnu".to_string(),
            (
                "https://example.invalid/example-org/sample-lib/releases/download/v1.2.3/sample-lib-zig-v1.2.3-x86_64-unknown-linux-gnu.tar.gz"
                    .to_string(),
                Some("1220linux".to_string()),
            ),
        );
        platform_hashes.insert(
            "aarch64-apple-darwin".to_string(),
            (
                "https://example.invalid/example-org/sample-lib/releases/download/v1.2.3/sample-lib-zig-v1.2.3-aarch64-apple-darwin.tar.gz"
                    .to_string(),
                Some("1220macos".to_string()),
            ),
        );

        let content = render_build_zig_zon(
            "sample_lib",
            "../../packages/zig",
            DependencyMode::Registry,
            "1.2.3",
            &platform_hashes,
            false,
            &[],
        );

        assert!(content.contains(".sample_lib_x86_64_unknown_linux_gnu"));
        assert!(content.contains(".sample_lib_aarch64_apple_darwin"));
        assert!(content.contains(".lazy = true"));
        assert!(content.contains(".hash = \"1220linux\""));
        assert!(content.contains(".hash = \"1220macos\""));
        assert!(
            !content.contains(".sample_lib = .{"),
            "platform-specific registry mode must not also emit a generic dependency: {content}"
        );
    }

    /// When no hash is available (None), no fake hash may be emitted for the single generic tarball entry.
    #[test]
    fn build_zig_zon_omits_hash_when_no_hash() {
        let mut platform_hashes = std::collections::BTreeMap::new();
        let url =
            "https://example.invalid/example-org/demo-client/releases/download/v1.4.0-rc.32/demo-client-zig-v1.4.0-rc.32.tar.gz"
                .to_string();
        platform_hashes.insert("generic".to_string(), (url, None));
        let content = render_build_zig_zon(
            "demo_client",
            "../../packages/zig",
            DependencyMode::Registry,
            "1.4.0-rc.32",
            &platform_hashes,
            false,
            &[],
        );
        assert!(
            !content.contains(".hash"),
            "build.zig.zon must omit fake hash metadata when no hash is available, got:\n{content}"
        );
    }

    /// Regression test for the malformed asset URL bug: the rendered URL must
    /// include the repo segment (`<org>/<repo>/releases/...`).  Previously the
    /// codegen defaulted `github_repo` to `https://github.com/<org>` (no
    /// repo), producing `https://github.com/<org>/releases/...` which 404s.
    /// Now the URL is a single generic (no platform suffix) source tarball.
    #[test]
    fn build_zig_zon_emits_full_release_url_with_repo_segment_and_platform_suffix() {
        let mut platform_hashes = std::collections::BTreeMap::new();
        let url =
            "https://example.invalid/example-org/demo-markup/releases/download/v3.5.1/demo-markup-rs-zig-v3.5.1.tar.gz"
                .to_string();
        platform_hashes.insert("generic".to_string(), (url, None));
        let content = render_build_zig_zon(
            "demo_markup",
            "../../packages/zig",
            DependencyMode::Registry,
            "3.5.1",
            &platform_hashes,
            false,
            &[],
        );
        // Verify the generic (no-suffix) URL is present with proper repo segment.
        let expected_url =
            "https://example.invalid/example-org/demo-markup/releases/download/v3.5.1/demo-markup-rs-zig-v3.5.1.tar.gz";
        assert!(
            content.contains(expected_url),
            "build.zig.zon must emit the generic source tarball URL with proper repo segment; got:\n{content}"
        );
    }
}

#[cfg(test)]
mod detect_stale_zig_hash_tests {
    use crate::core::config::e2e::DependencyMode;
    use crate::e2e::codegen::zig::build::render_build_zig_zon;

    use super::detect_stale_zig_hash;
    use super::supported_zig_platforms;

    /// Stale hash detection: hash contains rc.50, current version is rc.57 → true (stale).
    #[test]
    fn detects_stale_hash_with_older_rc_version() {
        let result = detect_stale_zig_hash(
            "demo_client-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0-rc.57",
            "demo_client",
        );
        assert!(result, "expected stale hash detection (rc.50 vs rc.57), but got false");
    }

    /// Matching hash and version: hash contains rc.57, current version is rc.57 → false (fresh).
    #[test]
    fn accepts_matching_version_in_hash() {
        let result = detect_stale_zig_hash(
            "demo_client-1.4.0-rc.57-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0-rc.57",
            "demo_client",
        );
        assert!(!result, "expected fresh hash (rc.57 matches), but got true (stale)");
    }

    /// Matching stable version: hash contains 1.4.0, current version is 1.4.0 → false (fresh).
    #[test]
    fn accepts_matching_stable_version() {
        let result = detect_stale_zig_hash(
            "demo_client-1.4.0-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0",
            "demo_client",
        );
        assert!(
            !result,
            "expected fresh hash (1.4.0 matches stable), but got true (stale)"
        );
    }

    /// Malformed hash (wrong pkg_name prefix) → false (no prefix match, silent fail).
    #[test]
    fn returns_false_for_wrong_pkg_name_prefix() {
        let result = detect_stale_zig_hash(
            "wrong_pkg-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9",
            "1.4.0-rc.57",
            "demo_client",
        );
        assert!(
            !result,
            "expected no detection for mismatched pkg_name prefix, but got true"
        );
    }

    /// Regression test: zig platform URLs must use Rust target triples to match
    /// publish-zig action asset naming (e.g., aarch64-unknown-linux-gnu, not linux-aarch64).
    #[test]
    fn build_zig_zon_emits_rust_triple_platform_suffixes() {
        let mut platform_hashes = std::collections::BTreeMap::new();
        for platform in supported_zig_platforms() {
            let url = format!(
                "https://github.com/example/releases/download/v1.0.0/mylib-zig-v1.0.0-{}.tar.gz",
                platform
            );
            platform_hashes.insert(platform.to_string(), (url, None));
        }

        let content = render_build_zig_zon(
            "mylib",
            "../../packages/zig",
            DependencyMode::Registry,
            "1.0.0",
            &platform_hashes,
            false,
            &[],
        );

        // Verify all Rust target triples are present in the emitted URLs
        assert!(
            content.contains("aarch64-unknown-linux-gnu"),
            "URL must include aarch64-unknown-linux-gnu triple: {content}"
        );
        assert!(
            content.contains("aarch64-apple-darwin"),
            "URL must include aarch64-apple-darwin triple: {content}"
        );
        assert!(
            content.contains("x86_64-unknown-linux-gnu"),
            "URL must include x86_64-unknown-linux-gnu triple: {content}"
        );
        assert!(
            content.contains("x86_64-apple-darwin"),
            "URL must include x86_64-apple-darwin triple: {content}"
        );
        assert!(
            content.contains("x86_64-pc-windows-msvc"),
            "URL must include x86_64-pc-windows-msvc triple: {content}"
        );

        // Verify old simple platform names are NOT present
        assert!(
            !content.contains("linux-x86_64"),
            "URL must NOT use simple platform name linux-x86_64: {content}"
        );
        assert!(
            !content.contains("linux-aarch64"),
            "URL must NOT use simple platform name linux-aarch64: {content}"
        );
        assert!(
            !content.contains("macos-arm64"),
            "URL must NOT use simple platform name macos-arm64: {content}"
        );
        assert!(
            !content.contains("macos-x86_64"),
            "URL must NOT use simple platform name macos-x86_64: {content}"
        );
        assert!(
            !content.contains("windows-x86_64"),
            "URL must NOT use simple platform name windows-x86_64: {content}"
        );
    }
}
