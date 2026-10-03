//! PHP (ext-php-rs) binding generator backend for alef.

mod gen_bindings;
pub mod layout;
pub mod naming;
mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::PhpBackend;
pub(crate) use gen_bindings::types::{flat_field_name, is_tagged_data_enum, is_untagged_data_enum};
pub use gen_bindings::types::{is_php_prop_scalar, php_field_can_be_constructor_param};

const VISITOR_EXCLUSION_TOKEN: &str = "visitor";

/// Whether PHP's binding configuration opts out of the synthetic visitor surface.
///
/// `visitor` is an established pseudo-function name used by e2e/snippet generation: it owns a
/// trait bridge rather than one Rust `FunctionDef`. PHP codegen, docs, and README generation must
/// therefore apply it to the bridge-owned trait and handle too, or those generators disagree about
/// whether the binding exposes visitors. ~keep
pub(crate) fn visitor_surface_is_excluded(config: &crate::core::config::ResolvedCrateConfig) -> bool {
    config
        .exclude
        .functions
        .iter()
        .any(|name| name == VISITOR_EXCLUSION_TOKEN)
        || config
            .php
            .as_ref()
            .is_some_and(|php| php.exclude_functions.iter().any(|name| name == VISITOR_EXCLUSION_TOKEN))
}

/// Trait/type-alias names owned by visitor bridges and hidden by PHP's `visitor` opt-out. ~keep
pub(crate) fn excluded_visitor_type_names(
    config: &crate::core::config::ResolvedCrateConfig,
) -> std::collections::HashSet<&str> {
    if !visitor_surface_is_excluded(config) {
        return std::collections::HashSet::new();
    }

    config
        .trait_bridges
        .iter()
        .filter(|bridge| bridge.param_name.as_deref() == Some(VISITOR_EXCLUSION_TOKEN))
        .flat_map(|bridge| [Some(bridge.trait_name.as_str()), bridge.type_alias.as_deref()])
        .flatten()
        .collect()
}

pub(crate) fn without_excluded_visitor_types(
    api: &crate::core::ir::ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
) -> Option<crate::core::ir::ApiSurface> {
    let excluded = excluded_visitor_type_names(config);
    if excluded.is_empty() {
        return None;
    }

    let mut filtered = api.clone();
    filtered.types.retain(|item| !excluded.contains(item.name.as_str()));
    Some(filtered)
}
