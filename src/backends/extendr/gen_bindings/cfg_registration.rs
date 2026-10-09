use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

/// Prepend `#[cfg(<pred>)]` to a code item when the source symbol carries a cfg predicate.
pub(super) fn prepend_cfg(cfg: Option<&str>, item: String) -> String {
    match cfg {
        Some(pred) if !pred.is_empty() => format!("#[cfg({pred})]\n{item}"),
        _ => item,
    }
}

/// Normalise a cfg predicate for structural comparison by stripping all whitespace.
fn strip_cfg_whitespace(pred: &str) -> String {
    pred.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Return true if a function's effective cfg means it is always compiled in the binding crate, so
/// it is safe to register in the `extendr_module!` block.
pub(super) fn always_registered(cfg: Option<&str>) -> bool {
    let Some(pred) = cfg else {
        return true;
    };
    let pred = pred.trim();
    if pred.is_empty() {
        return true;
    }
    let normalized = strip_cfg_whitespace(pred);
    let inner = normalized.strip_prefix("any(").and_then(|s| s.strip_suffix(')'));
    if let Some(inner) = inner
        && let Some((left, right)) = inner.split_once(',')
    {
        let complement = |a: &str, b: &str| format!("not({a})") == b;
        return complement(left, right) || complement(right, left);
    }
    false
}

/// Return the cfg features that are effectively enabled for generated R Rust code.
///
/// The R scaffold declares every referenced cfg feature as a passthrough feature. Unless the
/// user explicitly chooses a curated no-default feature set, that scaffold also enables every
/// passthrough feature by default, so codegen must treat those fields as present.
pub(super) fn effective_r_cfg_features(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<String> {
    let configured = config.features_for_language(Language::R);
    let default_features = config.r.as_ref().and_then(|r| r.default_features).unwrap_or(true);
    if !configured.is_empty() && !default_features {
        // The R scaffold writes this list verbatim onto the core dep line, so cargo enables an
        // aggregate's members even though the aggregate name alone matches no `feature = "X"`
        // leaf. Resolving it through the core crate's `[features]` table keeps the resolved-gate
        // surface in step with what the generated Cargo.toml actually compiles. ~keep
        return crate::codegen::cfg::expand_configured_features(config, configured);
    }
    let component_features = crate::codegen::cfg::component_core_features(config);
    crate::codegen::cfg::collect_cfg_features(api)
        .into_iter()
        .filter(|name| !component_features.contains(name))
        .collect()
}

/// Apply the R backend's cfg policy before struct, conversion, and wrapper generation.
///
/// Fields whose cfg is enabled by the R feature set are made unconditional in the generated
/// binding surface. Disabled fields are removed from the binding DTO and marked as stripped so
/// conversion templates keep using core defaults for the missing core slots.
///
/// ~keep Functions get the same treatment, and must: `extendr_module!` rejects a `#[cfg(...)]`
/// on its entries outright ("expected mod, fn or impl"), so R cannot gate a registration the way
/// Magnus gates its `define_module_function` call. The previous workaround was to let
/// [`always_registered`] drop every genuinely cfg-gated function from both the registration block
/// and the R wrapper surface -- unconditionally, whether or not the feature was actually on. That
/// made any crate with a cfg-gated function permanently unable to expose it through R even in the
/// default build. Resolving the predicate here instead means an enabled function reaches R with
/// its gate already discharged (`cfg: None`, so nothing downstream re-tests it), while a disabled
/// one is removed outright so no `extendr_module!` entry or wrapper can name a symbol the crate
/// did not compile.
pub(super) fn apply_r_cfg_policy(api: &ApiSurface, enabled_features: &[String]) -> ApiSurface {
    let mut filtered = api.clone();
    filtered.functions.retain_mut(|func| match func.cfg.as_deref() {
        None => true,
        Some(pred) if always_registered(Some(pred)) || cfg_condition_enabled(pred, enabled_features) => {
            func.cfg = None;
            true
        }
        Some(_) => false,
    });
    for typ in &mut filtered.types {
        let mut stripped = false;
        let mut fields = Vec::with_capacity(typ.fields.len());
        for mut field in typ.fields.drain(..) {
            let Some(cfg) = field.cfg.as_deref() else {
                fields.push(field);
                continue;
            };
            if cfg_condition_enabled(cfg, enabled_features) {
                field.cfg = None;
                fields.push(field);
            } else {
                stripped = true;
            }
        }
        if stripped {
            typ.has_stripped_cfg_fields = true;
        }
        typ.fields = fields;
    }
    filtered
}

fn cfg_condition_enabled(cfg_str: &str, enabled_features: &[String]) -> bool {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();
    let feature_enabled = |feature: &str| {
        enabled_features
            .iter()
            .any(|enabled| enabled == feature || enabled == "full")
    };

    if let Some(feature) = cfg_str.strip_prefix("feature = \"").and_then(|s| s.strip_suffix('"')) {
        return feature_enabled(feature);
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
