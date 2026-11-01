use crate::core::version::{to_r_version, to_rubygems_prerelease};
use anyhow::Context as _;

use super::version_core::to_pep440;

/// `~>`, `^`, `v`) and the version number.  This function:
///
/// 1. Strips the known prefix from `existing_version`.
/// 2. Re-renders the bare version using the appropriate per-language formatter
///    (`to_pep440` for Python, `to_rubygems_prerelease` for Ruby,
///    `to_r_version` for R, identity for everything else).
/// 3. Re-attaches the original prefix.
///
/// Returns `None` when the rendered version is already current (no write needed).
pub(crate) fn render_registry_version(lang: &str, workspace_version: &str, existing_version: &str) -> Option<String> {
    let prefix_len = existing_version.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
    let prefix = &existing_version[..prefix_len];

    let rendered_core: String = match lang {
        "python" => to_pep440(workspace_version),
        "ruby" => to_rubygems_prerelease(workspace_version),
        "r" => to_r_version(workspace_version),
        _ => workspace_version.to_string(),
    };

    let new_version = format!("{prefix}{rendered_core}");
    if new_version == existing_version {
        None
    } else {
        Some(new_version)
    }
}

// ~keep Modern Zig package IDs end in 44 URL-safe base64 characters; '-' can occur inside that suffix.
const ZIG_CONTENT_ID_LENGTH: usize = 44;
const STALE_ZIG_CONTENT_ID: &str = "STALE_HASH_REGENERATE";

fn extract_zig_hash_version(hash: &str, package_name: Option<&str>) -> Option<String> {
    let content_start = hash.len().checked_sub(ZIG_CONTENT_ID_LENGTH)?;
    let content = hash.get(content_start..)?;
    if !content
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    let prefix = hash.get(..content_start)?.strip_suffix('-')?;
    if let Some(version) = package_name.and_then(|name| prefix.strip_prefix(name)?.strip_prefix('-')) {
        return semver::Version::parse(version).ok().map(|_| version.to_string());
    }
    let mut versions = prefix.match_indices('-').filter_map(|(index, _)| {
        let version = &prefix[index + 1..];
        (index > 0 && semver::Version::parse(version).is_ok()).then_some(version)
    });
    let version = versions.next()?;
    versions.next().is_none().then(|| version.to_string())
}

fn zig_package_hash_version(package: &dyn toml_edit::TableLike, workspace_version: &str) -> Option<String> {
    let name = package.get("name").and_then(toml_edit::Item::as_str);
    let generic = package.get("hash").and_then(toml_edit::Item::as_str).into_iter();
    let platforms = package.get("platform_hashes").and_then(toml_edit::Item::as_table_like);
    let hashes = generic.chain(
        platforms
            .into_iter()
            .flat_map(|table| table.iter().filter_map(|(_, item)| item.as_str())),
    );
    let mut current = None;
    for version in hashes.filter_map(|hash| extract_zig_hash_version(hash, name)) {
        if version != workspace_version {
            return Some(version);
        }
        current = Some(version);
    }
    current
}

fn invalidate_zig_package_hashes(package: &mut dyn toml_edit::TableLike) -> bool {
    let mut changed = package.remove("hash").is_some();
    // ~keep Platform keys select per-platform asset URLs; dropping the map would select a generic asset instead.
    if let Some(platforms) = package
        .get_mut("platform_hashes")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        for (_, hash) in platforms.iter_mut() {
            *hash = toml_edit::value(STALE_ZIG_CONTENT_ID);
            changed = true;
        }
    }
    changed
}

/// Rewrite `version` fields inside a `[….packages.<lang>]` table (either the base
/// `[crates.<name>.e2e.packages.<lang>]` block or the registry-mode
/// `[crates.<name>.e2e.registry.packages.<lang>]` block — both hold the same
/// per-language package shape) to track the current workspace version.
///
/// Existing versions are updated; Zig content identities are invalidated when their version changes.
/// Hash-only Zig references can infer a version, but never gain a new `version` field.
fn patch_packages_table(packages: &mut dyn toml_edit::TableLike, workspace_version: &str) -> bool {
    let lang_keys: Vec<String> = packages.iter().map(|(k, _)| k.to_string()).collect();
    let mut any = false;
    for lang in &lang_keys {
        let pkg = match packages.get_mut(lang.as_str()).and_then(|i| i.as_table_like_mut()) {
            Some(t) => t,
            None => continue,
        };
        let existing_version_opt = pkg.get("version").and_then(|i| i.as_str()).map(|s| s.to_string());

        let existing_version = match existing_version_opt.clone() {
            Some(v) => v,
            None if lang == "zig" => match zig_package_hash_version(pkg, workspace_version) {
                Some(version) => version,
                None => continue,
            },
            None => continue,
        };
        if let Some(new_ver) = render_registry_version(lang, workspace_version, &existing_version) {
            if existing_version_opt.is_some()
                && let Some(ver_item) = pkg.get_mut("version")
            {
                *ver_item = toml_edit::value(new_ver.clone());
                any = true;
            }

            if lang == "zig" {
                any |= invalidate_zig_package_hashes(pkg);
            }
        }
    }
    any
}

/// Rewrite `version` fields under both `[crates.<name>.e2e.packages.<lang>]` and
/// `[crates.<name>.e2e.registry.packages.<lang>]` in `alef.toml` to track the current
/// workspace version.
///
/// Uses `toml_edit` for format-preserving surgery: comments, blank lines, and
/// key ordering are preserved. Zig content hashes are invalidated on version changes,
/// including recognizable hash-only references. No new `version` field is inserted.
///
/// `[crates.<name>.e2e.packages.<lang>]` is the base per-language package reference
/// (`E2eConfig::packages`) that local-mode e2e generation and `effective_package_for`
/// both read; `[crates.<name>.e2e.registry.packages.<lang>]` is the registry-mode override
/// layered on top of it. Both carry an independent `version` field, so both need a native
/// sync — before this, only the registry block was kept current, leaving the base block to
/// drift unless a consumer worked around it with a `sync.text_replacements` rule (which
/// `catch_all_rewrite_is_permitted` refuses without an alef provenance marker on `alef.toml`).
///
/// Returns `true` when at least one field was rewritten.
pub(crate) fn sync_registry_package_versions(
    config_path: &std::path::Path,
    workspace_version: &str,
) -> anyhow::Result<bool> {
    use toml_edit::{DocumentMut, Item};

    let content =
        std::fs::read_to_string(config_path).with_context(|| format!("failed to read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse {} as TOML", config_path.display()))?;

    let mut changed = false;

    let crate_keys: Vec<String> = doc.iter().map(|(k, _)| k.to_string()).collect();
    for key in &crate_keys {
        if key != "crates" {
            continue;
        }
        let crates_item = match doc.get_mut(key.as_str()) {
            Some(item) => item,
            None => continue,
        };

        fn patch_crate_table(crate_table: &mut dyn toml_edit::TableLike, workspace_version: &str) -> bool {
            let Some(e2e) = crate_table.get_mut("e2e").and_then(|i| i.as_table_like_mut()) else {
                return false;
            };
            let mut any = false;
            if let Some(packages) = e2e.get_mut("packages").and_then(|i| i.as_table_like_mut()) {
                any |= patch_packages_table(packages, workspace_version);
            }
            if let Some(registry) = e2e.get_mut("registry").and_then(|i| i.as_table_like_mut())
                && let Some(packages) = registry.get_mut("packages").and_then(|i| i.as_table_like_mut())
            {
                any |= patch_packages_table(packages, workspace_version);
            }
            any
        }

        if let Some(arr) = crates_item.as_array_of_tables_mut() {
            for crate_table in arr.iter_mut() {
                if patch_crate_table(crate_table, workspace_version) {
                    changed = true;
                }
            }
        } else if let Item::Table(tbl) = crates_item
            && patch_crate_table(tbl as &mut dyn toml_edit::TableLike, workspace_version)
        {
            changed = true;
        }
    }

    if changed {
        let new_content = doc.to_string();
        std::fs::write(config_path, &new_content)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }

    Ok(changed)
}

#[cfg(test)]
#[path = "version_registry/zig_hash_invalidation_tests.rs"]
mod zig_hash_invalidation_tests;
