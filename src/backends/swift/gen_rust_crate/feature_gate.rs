//! Feature-gate helpers for generated swift-bridge crates.

use crate::core::config::ResolvedCrateConfig;
use std::collections::HashSet;

/// Check whether the umbrella source crate exposes the given feature name in its
/// on-disk Cargo.toml.
pub(super) fn source_crate_has_feature(config: &ResolvedCrateConfig, core_crate_dir: &str, feature: &str) -> bool {
    let root = match config.workspace_root.as_deref() {
        Some(p) => p.to_path_buf(),
        None => match std::env::current_dir() {
            Ok(p) => p,
            Err(_) => return false,
        },
    };
    let cargo_toml = root.join("crates").join(core_crate_dir).join("Cargo.toml");
    let Ok(content) = std::fs::read_to_string(&cargo_toml) else {
        return false;
    };
    let needle_line_start = format!("{feature} =");
    let mut in_features = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if in_features && trimmed.starts_with(&needle_line_start) {
            return true;
        }
    }
    false
}

/// Returns `true` when the `cfg` condition is satisfied by `configured_features`.
pub(super) fn cfg_satisfied(cfg: Option<&str>, configured_features: &HashSet<&str>) -> bool {
    let Some(cfg_str) = cfg else {
        return true;
    };

    if configured_features.contains("full") {
        return true;
    }

    if let Some(rest) = cfg_str.strip_prefix("feature = \"")
        && let Some(feature_name) = rest.strip_suffix('"')
    {
        return configured_features.contains(feature_name);
    }

    if let Some(inner) = cfg_str
        .strip_prefix("any (")
        .or_else(|| cfg_str.strip_prefix("any("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let feature_names: Vec<&str> = inner
            .split(',')
            .filter_map(|clause| {
                let trimmed = clause.trim();
                trimmed.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"'))
            })
            .collect();

        if !feature_names.is_empty() {
            return feature_names.iter().any(|f| configured_features.contains(f));
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_satisfied_feature_matching() {
        let mut features = HashSet::new();
        features.insert("pdf");
        features.insert("html");

        // Matching feature should be satisfied
        assert!(cfg_satisfied(Some("feature = \"pdf\""), &features));

        // Non-matching feature should not be satisfied
        assert!(!cfg_satisfied(Some("feature = \"heuristics\""), &features));

        // None should be satisfied
        assert!(cfg_satisfied(None, &features));

        // Full feature should satisfy everything
        let mut full_features = HashSet::new();
        full_features.insert("full");
        assert!(cfg_satisfied(Some("feature = \"heuristics\""), &full_features));
    }

    #[test]
    fn test_cfg_satisfied_any_matching() {
        let mut features = HashSet::new();
        features.insert("ocr");

        // any() with matching feature
        assert!(cfg_satisfied(
            Some("any(feature = \"ocr\", feature = \"paddle-ocr\")"),
            &features
        ));

        // any() with no matching features
        assert!(!cfg_satisfied(
            Some("any(feature = \"heuristics\", feature = \"embeddings\")"),
            &features
        ));
    }
}
