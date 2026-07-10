use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::items::{EnumDef, ErrorDef, FunctionDef, TypeDef};
use super::service::{HandlerContractDef, ServiceDef};

/// Returns `true` when the `#[cfg(...)]` condition string is satisfied by the
/// given set of enabled feature names.
///
/// Supports the two cfg shapes produced by the extractor for feature gates:
/// `feature = "x"` and `any(feature = "x", feature = "y")`. A `None` cfg is
/// always satisfied, and the synthetic `full` feature satisfies every gate
/// (mirroring a conventional umbrella feature in source crates). Any
/// cfg shape that is not a recognised feature gate (e.g. `target_os = "..."`)
/// is treated as satisfied so non-feature gating is left to the compiler.
#[must_use]
pub fn cfg_feature_satisfied(cfg: Option<&str>, enabled_features: &HashSet<&str>) -> bool {
    let Some(cfg_str) = cfg else {
        return true;
    };

    if enabled_features.contains("full") {
        return true;
    }

    if let Some(rest) = cfg_str.strip_prefix("feature = \"")
        && let Some(feature_name) = rest.strip_suffix('"')
    {
        return enabled_features.contains(feature_name);
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
            return feature_names.iter().any(|f| enabled_features.contains(f));
        }
    }

    true
}

/// Complete API surface extracted from a Rust crate's public interface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSurface {
    pub crate_name: String,
    pub version: String,
    pub types: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
    pub enums: Vec<EnumDef>,
    pub errors: Vec<ErrorDef>,
    #[serde(default)]
    pub excluded_type_paths: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub excluded_trait_names: std::collections::HashSet<String>,
    #[serde(default)]
    pub services: Vec<ServiceDef>,
    #[serde(default)]
    pub handler_contracts: Vec<HandlerContractDef>,
    #[serde(default)]
    pub unsupported_public_items: Vec<UnsupportedPublicItem>,
}

impl ApiSurface {
    /// Returns a clone of this surface with same-named cfg-variant functions collapsed to one.
    ///
    /// Single-surface backends (Java, C#, Go, Kotlin, Swift, Dart, PHP, Ruby, Elixir, R/extendr)
    /// emit one host method per function. When the extractor preserves a real impl and a
    /// stub fallback under disjoint `cfg` gates, those two entries would otherwise become two
    /// host methods with identical signatures — a duplicate-method compile error. This collapses
    /// each such group into a single entry whose `cfg` is the OR of all members'. Rust-cfg-gated
    /// backends (FFI, napi, pyo3, wasm) must NOT call this: they emit both `#[cfg]`-guarded items
    /// and rely on `rustc` selecting one per feature set.
    #[must_use]
    pub fn with_deduped_functions(&self) -> Self {
        let mut deduped = self.clone();
        deduped.functions = crate::codegen::fn_dedup::dedup_same_name_functions(&self.functions);
        deduped
    }

    /// Returns a clone of this surface with every type, enum, and function whose
    /// `#[cfg(feature = "...")]` gate is not satisfied by `enabled_features` removed.
    ///
    /// Single-surface host facades (Swift, and any other backend that emits one
    /// host symbol per item with no Rust-cfg gating of its own) must only emit
    /// references to items that the Rust bridge crate actually compiles for the
    /// configured feature set. The Rust bridge crate filters its `visible_*` sets
    /// with the same cfg check, so applying this to the host-facing `ApiSurface`
    /// keeps the two layers consistent: the high-level facade never references a
    /// type or function that the bridge layer dropped under the active features.
    ///
    /// Only feature gates are evaluated (see [`cfg_feature_satisfied`]); other cfg
    /// shapes such as `target_os` are left in place for the compiler to resolve.
    #[must_use]
    pub fn with_cfg_filtered(&self, enabled_features: &HashSet<&str>) -> Self {
        let mut filtered = self.clone();
        filtered
            .types
            .retain(|t| cfg_feature_satisfied(t.cfg.as_deref(), enabled_features));
        filtered
            .enums
            .retain(|e| cfg_feature_satisfied(e.cfg.as_deref(), enabled_features));
        filtered
            .functions
            .retain(|f| cfg_feature_satisfied(f.cfg.as_deref(), enabled_features));
        filtered
    }
}

/// A public item that was discovered but not extracted into binding IR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnsupportedPublicItem {
    pub item_kind: String,
    pub item_path: String,
    pub reason: String,
    pub suggested_fix: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, FunctionDef, TypeDef};

    fn features(names: &[&'static str]) -> HashSet<&'static str> {
        names.iter().copied().collect()
    }

    fn ty(name: &str, cfg: Option<&str>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..TypeDef::default()
        }
    }

    fn en(name: &str, cfg: Option<&str>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..EnumDef::default()
        }
    }

    fn func(name: &str, cfg: Option<&str>) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..FunctionDef::default()
        }
    }

    #[test]
    fn cfg_feature_satisfied_handles_none_and_full() {
        let enabled = features(&["pdf"]);
        assert!(cfg_feature_satisfied(None, &enabled));
        assert!(cfg_feature_satisfied(Some("feature = \"pdf\""), &enabled));
        assert!(!cfg_feature_satisfied(Some("feature = \"presets\""), &enabled));
        assert!(cfg_feature_satisfied(
            Some("feature = \"presets\""),
            &features(&["full"])
        ));
    }

    #[test]
    fn cfg_feature_satisfied_handles_any_gate() {
        let enabled = features(&["ocr"]);
        assert!(cfg_feature_satisfied(
            Some("any(feature = \"ocr\", feature = \"paddle-ocr\")"),
            &enabled
        ));
        assert!(!cfg_feature_satisfied(
            Some("any(feature = \"presets\", feature = \"heuristics\")"),
            &enabled
        ));
    }

    #[test]
    fn cfg_feature_satisfied_leaves_non_feature_gates_satisfied() {
        assert!(cfg_feature_satisfied(
            Some("target_os = \"windows\""),
            &features(&["pdf"])
        ));
    }

    #[test]
    fn with_cfg_filtered_drops_unsatisfied_items_only() {
        let mut surface = ApiSurface::default();
        surface.types.push(ty("PdfMetadata", Some("feature = \"pdf\"")));
        surface.types.push(ty("Preset", Some("feature = \"presets\"")));
        surface.types.push(ty("AlwaysOn", None));
        surface.enums.push(en("PresetCategory", Some("feature = \"presets\"")));
        surface.enums.push(en("MimeKind", None));
        surface
            .functions
            .push(func("analyze_document", Some("feature = \"heuristics\"")));
        surface.functions.push(func("extract", None));

        let filtered = surface.with_cfg_filtered(&features(&["pdf"]));

        let type_names: Vec<&str> = filtered.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(type_names, vec!["PdfMetadata", "AlwaysOn"]);
        let enum_names: Vec<&str> = filtered.enums.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(enum_names, vec!["MimeKind"]);
        let fn_names: Vec<&str> = filtered.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(fn_names, vec!["extract"]);
    }

    #[test]
    fn with_cfg_filtered_keeps_everything_under_full() {
        let mut surface = ApiSurface::default();
        surface.types.push(ty("Preset", Some("feature = \"presets\"")));
        surface
            .functions
            .push(func("analyze_document", Some("feature = \"heuristics\"")));

        let filtered = surface.with_cfg_filtered(&features(&["full"]));
        assert_eq!(filtered.types.len(), 1);
        assert_eq!(filtered.functions.len(), 1);
    }
}
