use crate::codegen::cfg as shared_cfg;
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
    true
}

/// Re-export from shared codegen — WASM was the original implementation site;
/// all backends now share the canonical version in [`crate::codegen::cfg`].
pub(super) fn collect_cfg_features(api: &ApiSurface) -> BTreeSet<String> {
    shared_cfg::collect_cfg_features(api)
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
