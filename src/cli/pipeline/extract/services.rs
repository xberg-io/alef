use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;

use super::validation::format_bulleted_errors;

pub(super) fn run_service_extraction(api: &mut ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    let service_errors = crate::extract::extractor::service::extract_services(api, config);
    if !service_errors.is_empty() {
        let formatted = format_bulleted_errors(&service_errors);
        anyhow::bail!("service extraction failed:\n{formatted}");
    }
    Ok(())
}

pub(super) fn mark_adapter_handled_methods(api: &mut ApiSurface, config: &ResolvedCrateConfig) {
    use ahash::AHashSet;

    let adapter_handled: AHashSet<(String, String)> = config
        .adapters
        .iter()
        .filter_map(|adapter| {
            adapter
                .owner_type
                .as_deref()
                .map(|owner| (owner.to_string(), adapter.core_path.clone()))
        })
        .collect();

    if adapter_handled.is_empty() {
        return;
    }

    for typ in &mut api.types {
        for method in &mut typ.methods {
            if adapter_handled.contains(&(typ.name.clone(), method.name.clone())) && !method.binding_excluded {
                method.binding_excluded = true;
                if method.binding_exclusion_reason.is_none() {
                    method.binding_exclusion_reason =
                        Some(format!("handled by [[crates.adapters]] entry `{}`", method.name));
                }
            }
        }
    }
}

pub(super) fn strip_excluded_methods_from_types(api: &mut ApiSurface, config: &ResolvedCrateConfig) {
    if config.exclude.methods.is_empty() {
        return;
    }
    for typ in &mut api.types {
        typ.methods.retain(|m| {
            let key = format!("{}.{}", typ.name, m.name);
            !config.exclude.methods.contains(&key)
        });
    }
}
