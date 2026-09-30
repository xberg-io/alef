use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, ParamDef, PrimitiveType, TypeRef};
use ahash::AHashSet;

pub(super) fn format_bulleted_errors(messages: &[String]) -> String {
    messages
        .iter()
        .map(|message| format!("- {message}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn validate_extracted_api(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    validate_json_parameter_limits(api, config)?;
    let bridged_trait_names: AHashSet<&str> = config.configured_trait_bridge_names().collect();
    let validation_report =
        crate::core::validation::validate_api_surface_with_bridged_traits(api, &bridged_trait_names);
    // `options_field` trait bridges read a specific field off a specific options struct rather
    // than a whole-trait carrier, so this check needs the full bridge config (options_type,
    // resolved field name), not just the trait names threaded through `validation_report`.
    // Kept as a separate call rather than folded into `validate_api_surface_with_bridged_traits`
    // itself, whose `bridged_trait_names: &AHashSet<&str>` signature several other call sites
    // (and their tests) depend on. ~keep
    let carrier_diagnostics = config.trait_bridge_carrier_diagnostics(api);
    let (suppressed, fatal): (Vec<_>, Vec<_>) =
        validation_report
            .errors()
            .chain(carrier_diagnostics.iter())
            .partition(|d| {
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

fn validate_json_parameter_limits(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for limit in &config.json_parameter_limits {
        if limit.default_max == 0 {
            anyhow::bail!(
                "json_parameter_limits entry for `{}.{}` must set default_max greater than zero",
                limit.operation,
                limit.parameter
            );
        }
        if !seen.insert((&limit.operation, &limit.parameter)) {
            anyhow::bail!(
                "duplicate json_parameter_limits entry for `{}.{}`",
                limit.operation,
                limit.parameter
            );
        }

        let candidates: Vec<&[ParamDef]> = api
            .functions
            .iter()
            .filter(|function| function.name == limit.operation)
            .map(|function| function.params.as_slice())
            .collect();
        let params = match candidates.as_slice() {
            [] => anyhow::bail!(
                "json_parameter_limits references unknown free function `{}`",
                limit.operation
            ),
            [params] => *params,
            _ => anyhow::bail!(
                "json_parameter_limits free function name `{}` is ambiguous",
                limit.operation
            ),
        };
        let parameter = params
            .iter()
            .find(|parameter| parameter.name == limit.parameter)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "json_parameter_limits references unknown parameter `{}.{}`",
                    limit.operation,
                    limit.parameter
                )
            })?;
        if parameter.optional
            || !matches!(&parameter.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)))
        {
            anyhow::bail!(
                "json_parameter_limits parameter `{}.{}` must be a required Vec of named DTOs serialized through raw JSON",
                limit.operation,
                limit.parameter
            );
        }
        let max_parameter = params
            .iter()
            .find(|parameter| parameter.name == limit.max_parameter)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "json_parameter_limits references unknown max_parameter `{}.{}`",
                    limit.operation,
                    limit.max_parameter
                )
            })?;
        if !max_parameter.optional || !matches!(&max_parameter.ty, TypeRef::Primitive(PrimitiveType::U32)) {
            anyhow::bail!(
                "json_parameter_limits max_parameter `{}.{}` must be an optional u32",
                limit.operation,
                limit.max_parameter
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod json_parameter_limit_tests {
    use super::*;
    use crate::core::config::JsonParameterLimitConfig;
    use crate::core::ir::{FunctionDef, ParamDef};

    fn surface() -> ApiSurface {
        ApiSurface {
            functions: vec![FunctionDef {
                name: "redact".to_string(),
                params: vec![
                    ParamDef {
                        name: "findings".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("Finding".to_string()))),
                        ..Default::default()
                    },
                    ParamDef {
                        name: "max_findings".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
                        optional: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            json_parameter_limits: vec![JsonParameterLimitConfig {
                operation: "redact".to_string(),
                parameter: "findings".to_string(),
                max_parameter: "max_findings".to_string(),
                default_max: 10_000,
            }],
            ..Default::default()
        }
    }

    fn error_for(mutator: impl FnOnce(&mut ApiSurface, &mut ResolvedCrateConfig)) -> String {
        let mut api = surface();
        let mut config = config();
        mutator(&mut api, &mut config);
        validate_json_parameter_limits(&api, &config)
            .expect_err("invalid mapping must fail closed")
            .to_string()
    }

    #[test]
    fn accepts_valid_mapping() {
        validate_json_parameter_limits(&surface(), &config()).expect("valid mapping");
    }

    #[test]
    fn rejects_unknown_function_parameter_and_max_parameter() {
        assert!(
            error_for(|_, config| config.json_parameter_limits[0].operation = "missing".into())
                .contains("unknown free function")
        );
        assert!(
            error_for(|_, config| config.json_parameter_limits[0].parameter = "missing".into())
                .contains("unknown parameter")
        );
        assert!(
            error_for(|_, config| config.json_parameter_limits[0].max_parameter = "missing".into())
                .contains("unknown max_parameter")
        );
    }

    #[test]
    fn rejects_wrong_parameter_and_limit_types() {
        assert!(error_for(|api, _| api.functions[0].params[0].ty = TypeRef::String).contains("required Vec"));
        assert!(
            error_for(|api, _| {
                api.functions[0].params[0].ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
            })
            .contains("named DTOs")
        );
        assert!(error_for(|api, _| api.functions[0].params[0].optional = true).contains("required Vec"));
        assert!(error_for(|api, _| api.functions[0].params[1].optional = false).contains("optional u32"));
        assert!(
            error_for(|api, _| api.functions[0].params[1].ty = TypeRef::Primitive(PrimitiveType::I32))
                .contains("optional u32")
        );
    }

    #[test]
    fn rejects_duplicate_mapping_and_zero_default() {
        assert!(
            error_for(|_, config| config
                .json_parameter_limits
                .push(config.json_parameter_limits[0].clone()))
            .contains("duplicate")
        );
        assert!(error_for(|_, config| config.json_parameter_limits[0].default_max = 0).contains("greater than zero"));
    }
}
