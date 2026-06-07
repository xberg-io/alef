use crate::core::ir::{ApiSurface, TypeRef};
use std::collections::BTreeSet;

/// Check if a TypeRef references a Named type that is in the exclude set.
/// Used to skip fields whose types were excluded from WASM generation,
/// preventing references to non-existent Js* wrapper types.
pub(super) fn field_references_excluded_type(ty: &TypeRef, exclude_types: &[String]) -> bool {
    match ty {
        TypeRef::Named(name) => exclude_types.iter().any(|e| e == name),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => field_references_excluded_type(inner, exclude_types),
        TypeRef::Map(k, v) => {
            field_references_excluded_type(k, exclude_types) || field_references_excluded_type(v, exclude_types)
        }
        _ => false,
    }
}

/// Check if an item is gated behind a disabled feature.
///
/// Evaluates cfg condition strings against the enabled feature list.
/// Returns `true` when the cfg condition is *not* satisfied (i.e. the item
/// must be excluded from generation).  Handles:
/// - `feature = "name"`
/// - `any(feature = "a", feature = "b", ...)`
/// - `all(feature = "a", feature = "b", ...)`
/// - `not(<inner>)`
///
/// The IR encodes cfgs via `proc_macro2::TokenStream::to_string()`, which
/// inserts whitespace between tokens (e.g. `any (feature = "a" , ...)`); the
/// evaluator normalises that before parsing.
pub(super) fn is_gated_behind_disabled_feature(cfg: &Option<String>, enabled_features: &[String]) -> bool {
    let Some(cfg_str) = cfg else {
        return false;
    };
    !cfg_condition_enabled(cfg_str, enabled_features)
}

pub(super) fn cfg_condition_enabled(cfg_str: &str, enabled_features: &[String]) -> bool {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        return enabled_features.iter().any(|ef| ef == feature);
    }
    if let Some(inner) = cfg_str.strip_prefix("any(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .any(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    if let Some(inner) = cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')) {
        return parse_cfg_list(inner)
            .iter()
            .all(|cond| cfg_condition_enabled(cond, enabled_features));
    }
    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        return !cfg_condition_enabled(inner.trim(), enabled_features);
    }
    // Unknown pattern → treat as enabled (no exclusion). Preserves prior behaviour
    // for cfgs the WASM backend has never inspected (target_arch, target_os, ...).
    true
}

/// Extract every `feature = "X"` referenced by a cfg expression.
///
/// Recursively descends through `any(...)`, `all(...)`, and `not(...)` so that
/// the wasm Cargo.toml emitter can declare a passthrough Cargo feature for
/// every feature the generated source references. Without this, items emitted
/// behind `#[cfg(feature = "X")]` produce
/// `error: unexpected cfg condition value: X` when the binding crate's
/// `Cargo.toml` only declares an unrelated feature list (e.g. `wasm-target`).
///
/// Unknown cfg patterns (`target_arch`, `target_os`, ...) yield no features
/// — those are recognised by Cargo directly and don't need passthroughs.
pub(super) fn collect_cfg_feature_names(cfg_str: &str, out: &mut BTreeSet<String>) {
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
/// by any cfg attribute on a type, field, enum, or top-level function.
///
/// The set is sorted (via `BTreeSet`) so the resulting Cargo.toml is stable
/// across regenerations.
pub(super) fn collect_cfg_features(api: &ApiSurface) -> BTreeSet<String> {
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
    use super::collect_cfg_feature_names;
    use std::collections::BTreeSet;

    #[test]
    fn collect_cfg_feature_names_extracts_every_feature_reference() {
        let mut out = BTreeSet::new();
        collect_cfg_feature_names(r#"feature = "pdf""#, &mut out);
        collect_cfg_feature_names(r#"any(feature = "html", feature = "xml")"#, &mut out);
        collect_cfg_feature_names(
            r#"all(feature = "layout-types", not(feature = "wasm-target"))"#,
            &mut out,
        );
        // Unknown / non-feature cfg expressions yield nothing.
        collect_cfg_feature_names(r#"target_arch = "wasm32""#, &mut out);
        let want: BTreeSet<String> = ["html", "layout-types", "pdf", "wasm-target", "xml"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(out, want);
    }
}
