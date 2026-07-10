//! Feature-gate helpers for generated swift-bridge crates.

use crate::codegen::cfg::collect_cfg_features;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::collections::{BTreeSet, HashSet};

/// Check whether the umbrella source crate exposes the given feature name in its
/// on-disk Cargo.toml.
pub(crate) fn source_crate_has_feature(config: &ResolvedCrateConfig, core_crate_dir: &str, feature: &str) -> bool {
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

pub(crate) fn configured_swift_features(config: &ResolvedCrateConfig, core_crate_dir: &str) -> Vec<String> {
    let base_features = config.features_for_language(Language::Swift);
    let mut features: BTreeSet<String> = base_features.iter().cloned().collect();
    let ocr_active = features.contains("ocr") || features.contains("full");
    let ocr_wasm_present = features.contains("ocr-wasm");
    if ocr_active && !ocr_wasm_present && source_crate_has_feature(config, core_crate_dir, "ocr-wasm") {
        features.insert("ocr-wasm".to_string());
    }
    features.into_iter().collect()
}

pub(crate) fn effective_swift_codegen_features(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    core_crate_dir: &str,
) -> Vec<String> {
    let mut features: BTreeSet<String> = configured_swift_features(config, core_crate_dir).into_iter().collect();
    let excluded: HashSet<&str> = config
        .swift
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    for feature in collect_cfg_features(api) {
        if !excluded.contains(feature.as_str()) {
            features.insert(feature);
        }
    }
    features.into_iter().collect()
}

/// Returns `true` when the `cfg` condition is satisfied by `configured_features`.
///
/// Thin wrapper over [`crate::core::ir::cfg_feature_satisfied`] so the Rust
/// bridge crate and the high-level Swift facade share one cfg-matching
/// implementation (keeping their `visible_*` sets in lockstep).
pub(super) fn cfg_satisfied(cfg: Option<&str>, configured_features: &HashSet<&str>) -> bool {
    crate::core::ir::cfg_feature_satisfied(cfg, configured_features)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_satisfied_feature_matching() {
        let mut features = HashSet::new();
        features.insert("pdf");
        features.insert("html");

        assert!(cfg_satisfied(Some("feature = \"pdf\""), &features));

        assert!(!cfg_satisfied(Some("feature = \"heuristics\""), &features));

        assert!(cfg_satisfied(None, &features));

        let mut full_features = HashSet::new();
        full_features.insert("full");
        assert!(cfg_satisfied(Some("feature = \"heuristics\""), &full_features));
    }

    #[test]
    fn test_cfg_satisfied_any_matching() {
        let mut features = HashSet::new();
        features.insert("ocr");

        assert!(cfg_satisfied(
            Some("any(feature = \"ocr\", feature = \"paddle-ocr\")"),
            &features
        ));

        assert!(!cfg_satisfied(
            Some("any(feature = \"heuristics\", feature = \"embeddings\")"),
            &features
        ));
    }
}
