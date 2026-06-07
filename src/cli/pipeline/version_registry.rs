use crate::core::version::{to_r_version, to_rubygems_prerelease};
use anyhow::Context as _;

use super::version::to_pep440;

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
    // Extract the prefix: any leading non-alphanumeric, non-dot characters
    // (e.g., ">=", "~> ", "^", "v") that precede the semver digits.
    let prefix_len = existing_version.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
    let prefix = &existing_version[..prefix_len];

    // Render the bare version core using the per-language formatter.
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

/// Extract the embedded semver version from a zig package hash of the form
/// `<pkg-name>-<version>-<base64sha>`. Used when `[crates.e2e.registry.packages.zig]`
/// carries only a `hash` field (no separate `version`), so version-sync can still
/// refresh the hash's version component on workspace bumps.
///
/// Returns `None` when the hash is malformed or the base64 segment is unidentifiable.
pub(super) fn extract_zig_hash_version(hash: &str) -> Option<String> {
    let parts: Vec<&str> = hash.split('-').collect();
    if parts.len() < 3 {
        return None;
    }
    let base64_part = parts[parts.len() - 1];
    let is_base64 = base64_part.contains('_') || base64_part.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    if !is_base64 {
        return None;
    }
    let middle_parts = &parts[1..parts.len() - 1];
    if middle_parts.is_empty() {
        return None;
    }
    Some(middle_parts.join("-"))
}

/// Update a zig package hash by substituting the version component.
/// Zig hashes have the format: `<pkg-name>-<version>-<base64sha>`.
/// When the version changes, we substitute just the version part, leaving the
/// base64 sha unchanged (marked as stale until the zig publish step refreshes it
/// via `zig fetch --save`).
///
/// Returns `Some(new_hash)` if the version component changed, `None` otherwise.
pub(super) fn update_zig_package_hash(existing_hash: &str, old_version: &str, new_version: &str) -> Option<String> {
    // Zig hash format: `<name>-<version>-<base64sha>`, e.g.
    // `sample_pkg-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9`
    // We need to find the version component and replace it.
    // The version is sandwiched between the second-to-last and last dash (before the base64).
    //
    // Strategy: split by `-`, find the old version in the parts, and replace it.
    let parts: Vec<&str> = existing_hash.split('-').collect();
    if parts.len() < 3 {
        return None; // Malformed hash
    }

    // The base64 part (last segment) is always non-semver and doesn't contain dashes.
    // Find the position of the old version within the split parts.
    // For a hash like `sample_pkg-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9`,
    // after split: ["sample_pkg", "1.4.0", "rc.50", "Jfgk_..."]
    // We need to identify which parts compose the version.

    // A semver version may contain dots and dashes (e.g., "1.4.0-rc.50").
    // When split by "-", it becomes ["1.4.0", "rc", "50"].
    // The base64 part at the end is a single token with underscores and alphanumerics.

    // Heuristic: the base64 part is the last segment and contains underscores or
    // starts with an uppercase letter (typical base64url). All preceding parts
    // (after the pkg name) form the version.
    let base64_part = parts[parts.len() - 1];
    let is_base64 = base64_part.contains('_') || base64_part.chars().next().is_some_and(|c| c.is_ascii_uppercase());

    if !is_base64 {
        return None; // Couldn't identify base64 part
    }

    // Join the parts that make up the version by searching for old_version.
    // We'll try to find old_version as a substring of the joined middle parts.
    let middle_parts = &parts[1..parts.len() - 1]; // Everything except name and base64
    let joined_middle = middle_parts.join("-");

    if joined_middle.contains(old_version) {
        let new_middle = joined_middle.replace(old_version, new_version);
        let new_hash = format!("{}-{}-{}", parts[0], new_middle, base64_part);
        if new_hash != existing_hash {
            return Some(new_hash);
        }
    }

    None
}

/// Rewrite `version` fields under `[crates.<name>.e2e.registry.packages.<lang>]`
/// in `alef.toml` to track the current workspace version.
///
/// Uses `toml_edit` for format-preserving surgery: comments, blank lines, and
/// key ordering are all preserved.  Only entries that already have a `version`
/// field are touched — this function never inserts a new `version` field.
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

    // Walk `[[crates]]` array (new-style) and `[crates]` table (old-style).
    // Both shapes may carry `.e2e.registry.packages` sub-tables.
    let crate_keys: Vec<String> = doc.iter().map(|(k, _)| k.to_string()).collect();
    for key in &crate_keys {
        if key != "crates" {
            continue;
        }
        let crates_item = match doc.get_mut(key.as_str()) {
            Some(item) => item,
            None => continue,
        };

        // Helper closure: given a mutable reference to a single crate table,
        // walk its `.e2e.registry.packages.*` and update `version` and `hash` fields.
        fn patch_crate_table(crate_table: &mut dyn toml_edit::TableLike, workspace_version: &str) -> bool {
            let e2e = match crate_table.get_mut("e2e").and_then(|i| i.as_table_like_mut()) {
                Some(t) => t,
                None => return false,
            };
            let registry = match e2e.get_mut("registry").and_then(|i| i.as_table_like_mut()) {
                Some(t) => t,
                None => return false,
            };
            let packages = match registry.get_mut("packages").and_then(|i| i.as_table_like_mut()) {
                Some(t) => t,
                None => return false,
            };
            let lang_keys: Vec<String> = packages.iter().map(|(k, _)| k.to_string()).collect();
            let mut any = false;
            for lang in &lang_keys {
                let pkg = match packages.get_mut(lang.as_str()).and_then(|i| i.as_table_like_mut()) {
                    Some(t) => t,
                    None => continue,
                };
                let existing_version_opt = pkg.get("version").and_then(|i| i.as_str()).map(|s| s.to_string());

                // For zig, the embedded version inside `hash` is the source of truth
                // when no explicit `version` field is set — derive it so the hash
                // gets refreshed even when the package entry only carries `hash`.
                let existing_version = match existing_version_opt.clone() {
                    Some(v) => v,
                    None if lang == "zig" => {
                        match pkg
                            .get("hash")
                            .and_then(|i| i.as_str())
                            .and_then(extract_zig_hash_version)
                        {
                            Some(v) => v,
                            None => continue,
                        }
                    }
                    None => continue, // no version field — skip (don't insert)
                };
                if let Some(new_ver) = render_registry_version(lang, workspace_version, &existing_version) {
                    if existing_version_opt.is_some() {
                        if let Some(ver_item) = pkg.get_mut("version") {
                            *ver_item = toml_edit::value(new_ver.clone());
                            any = true;
                        }
                    }

                    // For zig, also update the hash field when the version changes.
                    // Hash format: `<pkg-name>-<version>-<base64sha>`.
                    // We inline-substitute the version part, leaving the base64 sha unchanged.
                    // Note: The sha will be stale until the zig publish workflow
                    // re-runs `zig fetch --save URL` post-release.
                    if lang == "zig" {
                        if let Some(hash_item) = pkg.get_mut("hash") {
                            if let Some(existing_hash) = hash_item.as_str() {
                                if let Some(new_hash) =
                                    update_zig_package_hash(existing_hash, &existing_version, &new_ver)
                                {
                                    *hash_item = toml_edit::value(new_hash);
                                    any = true;
                                }
                            }
                        }
                    }
                }
            }
            any
        }

        // `[[crates]]` is an array of tables.
        if let Some(arr) = crates_item.as_array_of_tables_mut() {
            for crate_table in arr.iter_mut() {
                if patch_crate_table(crate_table, workspace_version) {
                    changed = true;
                }
            }
        }
        // `[crates]` is a plain table (single-crate config style).
        else if let Item::Table(tbl) = crates_item {
            if patch_crate_table(tbl as &mut dyn toml_edit::TableLike, workspace_version) {
                changed = true;
            }
        }
    }

    if changed {
        let new_content = doc.to_string();
        std::fs::write(config_path, &new_content)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }

    Ok(changed)
}
