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
    if config.adapters.is_empty() {
        return;
    }

    for typ in &mut api.types {
        for method in &mut typ.methods {
            let handled = config
                .adapters
                .iter()
                .any(|adapter| crate::core::config::extras::adapter_covers_method(adapter, &typ.name, &method.name));
            if handled && !method.binding_excluded {
                method.binding_excluded = true;
                if method.binding_exclusion_reason.is_none() {
                    method.binding_exclusion_reason = Some(format!(
                        "{} entry `{}`",
                        crate::core::ir::ADAPTER_HANDLED_REASON_PREFIX,
                        method.name
                    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{AdapterConfig, AdapterPattern};
    use crate::core::ir::{MethodDef, TypeDef};

    fn streaming_adapter(owner_type: &str, core_path: &str) -> AdapterConfig {
        AdapterConfig {
            name: "chat_stream".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: core_path.to_string(),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: Some(owner_type.to_string()),
            item_type: Some("ChatCompletionChunk".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }
    }

    /// Regression for the bug this issue found: `core_path` is documented on
    /// [`AdapterConfig::core_path`] as a *fully-qualified* Rust path
    /// (`sample_stream_rs::DefaultClient::chat_stream`), but the pre-fix exact-tuple-equality match
    /// compared that whole string against the bare `MethodDef::name` (`chat_stream`) and never
    /// matched. A consumer following the documented contract therefore got a method double-
    /// emitted (both the adapter body and the generic per-method binding), because
    /// `binding_excluded` was never set.
    #[test]
    fn mark_adapter_handled_methods_matches_a_fully_qualified_core_path() {
        let mut api = ApiSurface {
            types: vec![TypeDef {
                name: "DefaultClient".to_string(),
                methods: vec![MethodDef {
                    name: "chat_stream".to_string(),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            adapters: vec![streaming_adapter(
                "DefaultClient",
                "sample_stream_rs::DefaultClient::chat_stream",
            )],
            ..ResolvedCrateConfig::default()
        };

        mark_adapter_handled_methods(&mut api, &config);

        let method = &api.types[0].methods[0];
        assert!(
            method.binding_excluded,
            "a fully-qualified core_path must still be recognised as covering the method"
        );
        assert!(
            method.binding_exclusion_reason.is_some(),
            "an adapter-handled method must record why it was excluded"
        );
    }

    /// A bare `core_path` (no `::`) -- the shape every existing fixture and e2e test uses --
    /// must keep matching exactly as before.
    #[test]
    fn mark_adapter_handled_methods_matches_a_bare_core_path() {
        let mut api = ApiSurface {
            types: vec![TypeDef {
                name: "DefaultClient".to_string(),
                methods: vec![MethodDef {
                    name: "chat_stream".to_string(),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            adapters: vec![streaming_adapter("DefaultClient", "chat_stream")],
            ..ResolvedCrateConfig::default()
        };

        mark_adapter_handled_methods(&mut api, &config);

        assert!(api.types[0].methods[0].binding_excluded);
    }

    /// A qualified `core_path` naming the right method but a different owner type must not
    /// match -- the owner check still gates the method-name match.
    #[test]
    fn mark_adapter_handled_methods_does_not_match_a_qualified_core_path_with_a_different_owner() {
        let mut api = ApiSurface {
            types: vec![TypeDef {
                name: "OtherClient".to_string(),
                methods: vec![MethodDef {
                    name: "chat_stream".to_string(),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            adapters: vec![streaming_adapter(
                "DefaultClient",
                "sample_stream_rs::DefaultClient::chat_stream",
            )],
            ..ResolvedCrateConfig::default()
        };

        mark_adapter_handled_methods(&mut api, &config);

        assert!(!api.types[0].methods[0].binding_excluded);
    }
}
