//! Shared cfg-expression utilities for language binding backends.
//!
//! Provides recursive parsing of Rust `#[cfg(...)]` condition strings and
//! full-surface feature collection so every backend can forward core-crate
//! features into its own Cargo.toml `[features]` table — preventing
//! `unexpected cfg condition value` errors when items are emitted behind
//! `#[cfg(feature = "X")]` guards.

use crate::core::ir::ApiSurface;
use std::collections::BTreeSet;

/// Extract every `feature = "X"` name referenced by a cfg expression.
///
/// Recursively descends through `any(...)`, `all(...)`, and `not(...)` so that
/// callers can declare a passthrough Cargo feature for every feature the
/// generated source references. Without this, items emitted behind
/// `#[cfg(feature = "X")]` produce
/// `error: unexpected cfg condition value: X` when the binding crate's
/// `Cargo.toml` only declares an unrelated feature list.
///
/// The IR encodes cfgs via `proc_macro2::TokenStream::to_string()`, which
/// inserts whitespace between tokens (e.g. `any (feature = "a" , ...)`); the
/// evaluator normalises that before parsing.
///
/// Unknown cfg patterns (`target_arch`, `target_os`, ...) yield no features
/// — those are recognised by Cargo directly and don't need passthroughs.
pub fn collect_cfg_feature_names(cfg_str: &str, out: &mut BTreeSet<String>) {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        out.insert(feature.to_string());
        return;
    }
    if let Some(inner) = cfg_str
        .strip_prefix("any(")
        .and_then(|s| s.strip_suffix(')'))
        .or_else(|| cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')))
    {
        for cond in parse_cfg_list(inner) {
            collect_cfg_feature_names(&cond, out);
        }
        return;
    }
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        collect_cfg_feature_names(inner.trim(), out);
    }
}

/// Walk the full [`ApiSurface`] and return the set of feature names referenced
/// by any cfg attribute on a type, field, enum variant, or top-level function.
///
/// The set is sorted (via `BTreeSet`) so the resulting Cargo.toml is stable
/// across regenerations.
pub fn collect_cfg_features(api: &ApiSurface) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for typ in &api.types {
        if let Some(cfg) = &typ.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
        for field in &typ.fields {
            if let Some(cfg) = &field.cfg {
                collect_cfg_feature_names(cfg, &mut out);
            }
        }
    }
    for enum_def in &api.enums {
        if let Some(cfg) = &enum_def.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
        for variant in &enum_def.variants {
            if let Some(cfg) = &variant.cfg {
                collect_cfg_feature_names(cfg, &mut out);
            }
        }
    }
    for func in &api.functions {
        if let Some(cfg) = &func.cfg {
            collect_cfg_feature_names(cfg, &mut out);
        }
    }
    out
}

fn parse_cfg_list(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in s.chars() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, TypeDef};

    #[test]
    fn collect_cfg_feature_names_simple_feature() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
        assert_eq!(out, BTreeSet::from(["pdf".to_string()]));
    }

    #[test]
    fn collect_cfg_feature_names_any_compound() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
        let want: BTreeSet<String> = ["html", "xml"].into_iter().map(String::from).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn collect_cfg_feature_names_all_compound() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(
            r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
            &mut out,
        );
        let want: BTreeSet<String> = ["layout-types", "wasm-target"].into_iter().map(String::from).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn collect_cfg_feature_names_ignores_non_feature_cfg() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_cfg_feature_names_whitespace_normalisation() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"any (feature = "a" , feature = "b")"#, &mut out);
        let want: BTreeSet<String> = ["a", "b"].into_iter().map(String::from).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn collect_cfg_features_walks_types_enums_functions() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
        collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
        collect_cfg_feature_names(
            r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
            &mut out,
        );
        collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
        let want: BTreeSet<String> = ["html", "layout-types", "pdf", "wasm-target", "xml"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(out, want);
    }

    #[test]
    fn collect_cfg_features_full_surface_walk() {
        let api = ApiSurface {
            types: vec![TypeDef {
                name: "PdfDoc".to_string(),
                rust_path: "mylib::PdfDoc".to_string(),
                cfg: Some(r#"feature = "pdf""#.to_string()),
                ..Default::default()
            }],
            enums: vec![EnumDef {
                name: "ImageOutputFormat".to_string(),
                variants: vec![
                    EnumVariant {
                        name: "Native".to_string(),
                        cfg: None,
                        ..Default::default()
                    },
                    EnumVariant {
                        name: "Heic".to_string(),
                        cfg: Some(r#"feature = "heic""#.to_string()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let features = collect_cfg_features(&api);
        let want: BTreeSet<String> = ["heic", "pdf"].into_iter().map(String::from).collect();
        assert_eq!(features, want);
    }
}
