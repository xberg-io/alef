use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::items::{EnumDef, ErrorDef, FunctionDef, TypeDef};
use super::service::{HandlerContractDef, ServiceDef};

/// Returns `true` when the `#[cfg(...)]` condition string is satisfied by the
/// given set of enabled feature names.
///
/// Supports the cfg shapes produced by the extractor for feature gates:
/// `feature = "x"`, `any(...)`, `all(...)`, and `not(...)`, composed
/// arbitrarily. A `None` cfg is always satisfied, and the synthetic `full`
/// feature satisfies every gate (mirroring a conventional umbrella feature in
/// source crates).
///
/// Non-feature leaves (e.g. `target_arch = "wasm32"`, `target_os = "..."`)
/// cannot be resolved at generation time — the same binding may compile the
/// item on one target and a stub on another. Such leaves are treated as
/// *indeterminate*, and any expression whose truth value depends on an
/// indeterminate leaf resolves to "satisfied" so the item is **kept** (the
/// conservative choice: never drop an item purely because of a target
/// predicate). Only expressions that are decided entirely by feature gates can
/// cause an item to be dropped.
///
/// The IR encodes cfgs via `proc_macro2::TokenStream::to_string()`, which
/// inserts whitespace between tokens (e.g. `any (feature = "a" , ...)`); the
/// evaluator normalises that before parsing.
#[must_use]
pub fn cfg_feature_satisfied(cfg: Option<&str>, enabled_features: &HashSet<&str>) -> bool {
    let Some(cfg_str) = cfg else {
        return true;
    };

    if enabled_features.contains("full") {
        return true;
    }

    // Indeterminate (target predicate) => keep the item.
    cfg_expr_satisfied(cfg_str, enabled_features).unwrap_or(true)
}

/// Three-valued evaluation of a normalised cfg predicate.
///
/// Returns `Some(true)`/`Some(false)` when the predicate is fully decided by
/// feature gates, and `None` when its value depends on an unresolved
/// non-feature leaf (e.g. `target_arch`). `all`/`any`/`not` propagate the
/// indeterminate result with standard Kleene logic so, for example,
/// `all(feature = "x", target_arch = "wasm32")` is `Some(false)` when `x` is
/// off (short-circuit) but `None` when `x` is on.
fn cfg_expr_satisfied(cfg_str: &str, enabled_features: &HashSet<&str>) -> Option<bool> {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(rest) = cfg_str.strip_prefix("feature = \"")
        && let Some(feature_name) = rest.strip_suffix('"')
    {
        return Some(enabled_features.contains(feature_name));
    }

    if let Some(inner) = cfg_str.strip_prefix("any(").and_then(|s| s.strip_suffix(')')) {
        let mut saw_indeterminate = false;
        for cond in split_cfg_operands(inner) {
            match cfg_expr_satisfied(&cond, enabled_features) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => saw_indeterminate = true,
            }
        }
        return if saw_indeterminate { None } else { Some(false) };
    }

    if let Some(inner) = cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')) {
        let mut saw_indeterminate = false;
        for cond in split_cfg_operands(inner) {
            match cfg_expr_satisfied(&cond, enabled_features) {
                Some(false) => return Some(false),
                Some(true) => {}
                None => saw_indeterminate = true,
            }
        }
        return if saw_indeterminate { None } else { Some(true) };
    }

    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        return cfg_expr_satisfied(inner.trim(), enabled_features).map(|value| !value);
    }

    // Unrecognised leaf (e.g. `target_arch = "..."`): indeterminate.
    None
}

/// Split the comma-separated operand list inside an `any(...)`/`all(...)`,
/// respecting nested parentheses so compound operands stay intact.
fn split_cfg_operands(inner: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    result.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
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

    /// Like [`with_cfg_filtered`](Self::with_cfg_filtered), but also drops
    /// cfg-gated *members* — struct fields, enum variants, and enum-variant
    /// fields — whose gate is not satisfied by `enabled_features`.
    ///
    /// The docs generator renders the full public surface of a type (its fields
    /// and variants), so a shallow top-level filter is not enough: a
    /// type that survives can still carry a `#[cfg(feature = "...")]`-gated field
    /// (e.g. `ExtractionConfig.tree_sitter`) that is compiled out of a binding
    /// whose feature set excludes that feature. This produces documentation that
    /// diverges from the binding's real generated surface. Deep filtering keeps
    /// the reference docs consistent with what each binding actually exposes.
    #[must_use]
    pub fn with_cfg_filtered_deep(&self, enabled_features: &HashSet<&str>) -> Self {
        let mut filtered = self.with_cfg_filtered(enabled_features);

        for typ in &mut filtered.types {
            typ.fields
                .retain(|field| cfg_feature_satisfied(field.cfg.as_deref(), enabled_features));
        }

        for enum_def in &mut filtered.enums {
            enum_def
                .variants
                .retain(|variant| cfg_feature_satisfied(variant.cfg.as_deref(), enabled_features));
            for variant in &mut enum_def.variants {
                variant
                    .fields
                    .retain(|field| cfg_feature_satisfied(field.cfg.as_deref(), enabled_features));
            }
        }

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
    fn cfg_feature_satisfied_handles_all_and_not_gates() {
        let enabled = features(&["layout-types"]);
        // all(...) requires every operand
        assert!(cfg_feature_satisfied(
            Some("all(feature = \"layout-types\", not(feature = \"wasm-target\"))"),
            &enabled
        ));
        // not(...) inverts
        assert!(!cfg_feature_satisfied(
            Some("not(feature = \"layout-types\")"),
            &enabled
        ));
        assert!(cfg_feature_satisfied(Some("not(feature = \"wasm-target\")"), &enabled));
        // all(...) fails when one operand is unsatisfied
        assert!(!cfg_feature_satisfied(
            Some("all(feature = \"layout-types\", feature = \"wasm-target\")"),
            &enabled
        ));
    }

    #[test]
    fn cfg_feature_satisfied_handles_whitespace_and_nested_operands() {
        let enabled = features(&["wasm-target"]);
        assert!(cfg_feature_satisfied(
            Some("any (feature = \"wasm-target\" , all(feature = \"a\", feature = \"b\"))"),
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
    fn cfg_feature_satisfied_keeps_items_gated_on_target_predicates() {
        // The real `RegionKind` gate: kept on any non-wasm native binding that
        // enables liter-llm, because the target-arch leaf is indeterminate at
        // generation time and must not cause a drop.
        let enabled = features(&["liter-llm"]);
        assert!(cfg_feature_satisfied(
            Some("all (feature = \"liter-llm\" , not (target_arch = \"wasm32\"))"),
            &enabled
        ));
        // But a decisive feature mismatch still drops, even mixed with a target
        // predicate: liter-llm off short-circuits the `all(...)` to false.
        assert!(!cfg_feature_satisfied(
            Some("all (feature = \"liter-llm\" , not (target_arch = \"wasm32\"))"),
            &features(&["pdf"])
        ));
        // A bare target predicate is always kept.
        assert!(cfg_feature_satisfied(
            Some("not (target_arch = \"wasm32\")"),
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
    fn with_cfg_filtered_deep_drops_gated_fields_and_variants() {
        use crate::core::ir::{EnumVariant, FieldDef};

        let mut config_ty = ty("ExtractionConfig", None);
        config_ty.fields = vec![
            FieldDef {
                name: "use_cache".to_string(),
                cfg: None,
                ..FieldDef::default()
            },
            FieldDef {
                name: "tree_sitter".to_string(),
                cfg: Some("feature = \"tree-sitter\"".to_string()),
                ..FieldDef::default()
            },
        ];

        let mut fmt_enum = en("OutputFormat", None);
        fmt_enum.variants = vec![
            EnumVariant {
                name: "Markdown".to_string(),
                cfg: None,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "CodeAst".to_string(),
                cfg: Some("feature = \"tree-sitter\"".to_string()),
                ..EnumVariant::default()
            },
        ];

        let mut surface = ApiSurface::default();
        surface.types.push(config_ty);
        surface
            .types
            .push(ty("TreeSitterConfig", Some("feature = \"tree-sitter\"")));
        surface.enums.push(fmt_enum);

        let filtered = surface.with_cfg_filtered_deep(&features(&["pdf"]));

        // Gated top-level type dropped.
        let type_names: Vec<&str> = filtered.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(type_names, vec!["ExtractionConfig"]);
        // Gated field dropped, ungated field kept.
        let field_names: Vec<&str> = filtered.types[0].fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(field_names, vec!["use_cache"]);
        // Gated variant dropped, ungated variant kept.
        let variant_names: Vec<&str> = filtered.enums[0].variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(variant_names, vec!["Markdown"]);
    }

    #[test]
    fn with_cfg_filtered_deep_keeps_gated_members_under_full() {
        use crate::core::ir::FieldDef;

        let mut config_ty = ty("ExtractionConfig", None);
        config_ty.fields = vec![FieldDef {
            name: "tree_sitter".to_string(),
            cfg: Some("feature = \"tree-sitter\"".to_string()),
            ..FieldDef::default()
        }];

        let mut surface = ApiSurface::default();
        surface.types.push(config_ty);

        let filtered = surface.with_cfg_filtered_deep(&features(&["full"]));
        assert_eq!(filtered.types[0].fields.len(), 1);
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
