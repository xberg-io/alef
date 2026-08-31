use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use ahash::AHashSet;

pub(super) fn format_bulleted_errors(messages: &[String]) -> String {
    messages
        .iter()
        .map(|message| format!("- {message}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn validate_extracted_api(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    let bridged_trait_names: AHashSet<&str> = config
        .trait_bridges
        .iter()
        .map(|bridge| bridge.trait_name.as_str())
        .collect();
    let validation_report =
        crate::core::validation::validate_api_surface_with_bridged_traits(api, &bridged_trait_names);
    // `options_field` trait bridges read a specific field off a specific options struct rather
    // than a whole-trait carrier, so this check needs the full bridge config (options_type,
    // resolved field name), not just the trait names threaded through `validation_report`.
    // Kept as a separate call rather than folded into `validate_api_surface_with_bridged_traits`
    // itself, whose `bridged_trait_names: &AHashSet<&str>` signature several other call sites
    // (and their tests) depend on. ~keep
    let carrier_diagnostics =
        crate::core::validation::trait_bridge_carrier_diagnostics(api, &config.trait_bridges);
    let (suppressed, fatal): (Vec<_>, Vec<_>) =
        validation_report.errors().chain(carrier_diagnostics.iter()).partition(|d| {
            !crate::core::validation::is_critical_unsuppressible(d.code)
                && config
                    .suppress_validation_codes
                    .iter()
                    .any(|code| code == &d.code.to_string())
        });
    for diagnostic in suppressed {
        // The consumer explicitly opted into suppress_validation_codes for this diagnostic;
        // re-printing it at warn level defeats their own declared setting. ~keep
        tracing::debug!("[suppressed] {diagnostic}");
    }
    if !fatal.is_empty() {
        let formatted = fatal
            .iter()
            .map(|d| {
                let path = d
                    .item_path
                    .as_deref()
                    .map(|p| format!(" item `{p}`"))
                    .unwrap_or_default();
                format!("- [{}]{path} {}", d.code, d.reason)
            })
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!("{}", formatted);
    }
    Ok(())
}
