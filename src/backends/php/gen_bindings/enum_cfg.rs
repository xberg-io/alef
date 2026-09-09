use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use anyhow::Context;
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::parse::Parser;

type FeatureGuards = BTreeMap<String, Vec<String>>;

pub(super) fn specialize(
    api: &mut ApiSurface,
    config: &ResolvedCrateConfig,
    declared: &BTreeSet<String>,
) -> anyhow::Result<()> {
    let mut feature_names = BTreeSet::new();
    for definition in &api.enums {
        if crate::codegen::cfg::is_host_owned_rust_path(&api.crate_name, &definition.rust_path) {
            for variant in &definition.variants {
                if let Some(cfg) = &variant.cfg {
                    crate::codegen::cfg::collect_cfg_feature_names(cfg, &mut feature_names);
                }
            }
        }
    }
    let guards = if feature_names.is_empty() {
        FeatureGuards::new()
    } else {
        dependency_feature_guards(api, config)?
    };
    for definition in &mut api.enums {
        if !crate::codegen::cfg::is_host_owned_rust_path(&api.crate_name, &definition.rust_path) {
            continue;
        }
        for variant in &mut definition.variants {
            if let Some(cfg) = &variant.cfg {
                let specialized = specialize_cfg(cfg, declared, &guards)?;
                variant.cfg = (specialized != "all()").then_some(specialized);
            }
        }
    }
    Ok(())
}

fn dependency_feature_guards(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<FeatureGuards> {
    // ~keep Render only: this pure scaffolder is the authority for target edges and core defaults.
    // Re-deriving its dependency features here would diverge when a target override replaces them.
    let files = crate::scaffold::languages::php::scaffold_php_cargo(api, config)?;
    let manifest = files
        .iter()
        .find(|file| file.path.ends_with("Cargo.toml"))
        .context("PHP manifest missing")?;
    let manifest: toml::Value = toml::from_str(&manifest.content).context("parse generated PHP manifest")?;
    let mut guards = FeatureGuards::new();
    add_edge(&mut guards, config, &manifest, "all()")?;
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, table) in targets {
            let predicate = target
                .strip_prefix("cfg(")
                .and_then(|value| value.strip_suffix(')'))
                .context("generated PHP dependency target must be a cfg predicate")?;
            add_edge(&mut guards, config, table, predicate)?;
        }
    }
    add_passthrough_edges(&mut guards, config, &manifest);
    Ok(guards)
}

fn add_passthrough_edges(guards: &mut FeatureGuards, config: &ResolvedCrateConfig, manifest: &toml::Value) {
    let Some(features) = manifest.get("features").and_then(toml::Value::as_table) else {
        return;
    };
    let prefix = format!("{}/", config.name);
    for (binding_feature, members) in features {
        let requested: Vec<String> = members
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .filter_map(|member| member.strip_prefix(&prefix))
            .map(str::to_owned)
            .collect();
        let predicate = format!("feature = {binding_feature:?}");
        for feature in crate::scaffold::core_feature_closure(config, &requested).0 {
            guards.entry(feature).or_default().push(predicate.clone());
        }
    }
}

fn add_edge(
    guards: &mut FeatureGuards,
    config: &ResolvedCrateConfig,
    table: &toml::Value,
    predicate: &str,
) -> anyhow::Result<()> {
    let Some(dependency) = table.get("dependencies").and_then(|deps| deps.get(&config.name)) else {
        return Ok(());
    };
    let dependency = dependency
        .as_table()
        .context("generated PHP core dependency must be a table")?;
    let mut requested: Vec<String> = dependency
        .get("features")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::to_owned)
        .collect();
    if dependency
        .get("default-features")
        .and_then(toml::Value::as_bool)
        .unwrap_or(true)
    {
        requested.extend(crate::scaffold::core_feature_closure(config, &[]).1);
    }
    for feature in crate::scaffold::core_feature_closure(config, &requested).0 {
        guards.entry(feature).or_default().push(predicate.to_string());
    }
    Ok(())
}

fn specialize_cfg(cfg: &str, declared: &BTreeSet<String>, guards: &FeatureGuards) -> anyhow::Result<String> {
    let predicate: syn::Meta = syn::parse_str(cfg).context("parse PHP enum cfg predicate")?;
    let specialized = specialize_meta(&predicate, declared, guards)?;
    normalize_meta(&syn::parse_str(&specialized).context("parse specialized PHP enum cfg")?)
}

fn specialize_meta(meta: &syn::Meta, declared: &BTreeSet<String>, guards: &FeatureGuards) -> anyhow::Result<String> {
    if let syn::Meta::NameValue(value) = meta
        && value.path.is_ident("feature")
        && let syn::Expr::Lit(literal) = &value.value
        && let syn::Lit::Str(name) = &literal.lit
    {
        let name = name.value();
        let mut alternatives = guards.get(&name).cloned().unwrap_or_default();
        if alternatives.iter().any(|predicate| predicate == "all()") {
            return Ok("all()".to_string());
        }
        let local = meta.to_token_stream().to_string();
        if declared.contains(&name) && !alternatives.contains(&local) {
            alternatives.push(local);
        }
        return Ok(match alternatives.as_slice() {
            [only] => only.clone(),
            _ => format!("any({})", alternatives.join(", ")),
        });
    }
    if let syn::Meta::List(list) = meta
        && ["all", "any", "not"].iter().any(|name| list.path.is_ident(name))
    {
        let members = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
            .parse2(list.tokens.clone())
            .context("parse PHP compound enum cfg")?;
        let members = members
            .iter()
            .map(|member| specialize_meta(member, declared, guards))
            .collect::<anyhow::Result<Vec<_>>>()?;
        return Ok(format!("{}({})", list.path.to_token_stream(), members.join(", ")));
    }
    Ok(meta.to_token_stream().to_string())
}

fn normalize_meta(meta: &syn::Meta) -> anyhow::Result<String> {
    let syn::Meta::List(list) = meta else {
        return Ok(meta.to_token_stream().to_string());
    };
    if !["all", "any", "not"].iter().any(|name| list.path.is_ident(name)) {
        return Ok(meta.to_token_stream().to_string());
    }
    let members = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .context("parse PHP cfg normalization operands")?;
    let members = members.iter().map(normalize_meta).collect::<anyhow::Result<Vec<_>>>()?;
    let operator = list.path.to_token_stream().to_string();
    Ok(fold_members(&operator, members))
}

fn fold_members(operator: &str, mut members: Vec<String>) -> String {
    if operator == "not" {
        return match members.as_slice() {
            [only] if only == "all()" => "any()".into(),
            [only] if only == "any()" => "all()".into(),
            [only] if only.starts_with("not(") && only.ends_with(')') => only[4..only.len() - 1].into(),
            _ => format!("not({})", members.join(", ")),
        };
    }
    let (neutral, absorbing) = if operator == "all" {
        ("all()", "any()")
    } else {
        ("any()", "all()")
    };
    if members.iter().any(|member| member == absorbing) {
        return absorbing.into();
    }
    members.retain(|member| member != neutral);
    match members.as_slice() {
        [] => neutral.into(),
        [only] => only.clone(),
        _ => format!("{operator}({})", members.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unguarded_host_and_external_enums_generate_without_scaffold_configuration() {
        use crate::core::backend::Backend;
        use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
        for rust_path in ["sample::Outcome", "external::Outcome"] {
            let api = ApiSurface {
                crate_name: "sample".into(),
                enums: vec![EnumDef {
                    name: "Outcome".into(),
                    rust_path: rust_path.into(),
                    serde_tag: Some("wire_tag".into()),
                    serde_content: Some("wire_content".into()),
                    has_serde: true,
                    variants: vec![EnumVariant {
                        name: "Ready".into(),
                        is_tuple: true,
                        fields: vec![FieldDef {
                            name: "0".into(),
                            ty: TypeRef::String,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let files = crate::backends::php::PhpBackend
                .generate_bindings(&api, &ResolvedCrateConfig::default())
                .unwrap_or_else(|error| panic!("{rust_path}: {error:#}"));
            assert!(
                files.iter().any(|file| file.content.contains("wire_tag")),
                "{rust_path}"
            );
        }
    }

    #[test]
    fn target_only_host_and_feature_guarded_external_enums_need_no_core_manifest() {
        use crate::core::ir::{EnumDef, EnumVariant};
        for (rust_path, cfg) in [
            ("sample::Outcome", "unix"),
            ("external::Outcome", r#"feature = "remote""#),
        ] {
            let mut api = ApiSurface {
                crate_name: "sample".into(),
                enums: vec![EnumDef {
                    rust_path: rust_path.into(),
                    variants: vec![EnumVariant {
                        cfg: Some(cfg.into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            specialize(&mut api, &ResolvedCrateConfig::default(), &BTreeSet::new())
                .unwrap_or_else(|error| panic!("{rust_path}: {error:#}"));
            assert_eq!(api.enums[0].variants[0].cfg.as_deref(), Some(cfg));
        }
    }

    #[test]
    fn compound_guards_preserve_targets_and_toggleable_features() {
        let declared = BTreeSet::from(["optional".to_string()]);
        let guards = BTreeMap::from([("forced".to_string(), vec!["all()".to_string()])]);
        for (input, expected) in [
            (
                r#"all(feature = "forced", target_os = "linux")"#,
                r#"target_os = "linux""#,
            ),
            (r#"any(feature = "forced", feature = "optional")"#, "all()"),
            (r#"not(feature = "forced")"#, "any()"),
            (r#"feature = "missing""#, "any()"),
        ] {
            assert_eq!(specialize_cfg(input, &declared, &guards).expect("specialize"), expected);
        }
    }

    #[test]
    fn target_specific_dependency_feature_remains_target_specific() {
        let guards = BTreeMap::from([("remote".to_string(), vec![r#"target_os = "linux""#.to_string()])]);
        assert_eq!(
            specialize_cfg(r#"feature = "remote""#, &BTreeSet::new(), &guards).expect("cfg"),
            r#"target_os = "linux""#
        );
    }

    #[test]
    fn specialized_variants_pass_real_clippy_without_redundant_cfg() {
        let root = tempfile::tempdir().expect("fixture");
        super::super::host_enum_feature_tests::write_fixture(root.path(), true, None, false, false);
        let mut command = std::process::Command::new("cargo");
        command
            .args(["clippy", "--offline", "--quiet", "--manifest-path"])
            .arg(root.path().join("crates/core-lib-php/Cargo.toml"))
            .args(["--", "-Dclippy::non_minimal_cfg", "-Dunexpected_cfgs"])
            .env("CARGO_TARGET_DIR", root.path().join("target"))
            .env("CARGO_BUILD_JOBS", "1");
        let (success, output) =
            crate::snippets::validators::run_command(&mut command, 60).expect("bounded generated conversion Clippy");
        assert!(success, "{output}");
    }

    #[test]
    fn boolean_folding_preserves_target_exclusion_and_optional_guards() {
        let guards = BTreeMap::from([("forced".into(), vec!["all()".into()])]);
        let declared = BTreeSet::from(["optional".into()]);
        for (input, expected) in [
            (r#"all(feature = "missing", unix)"#, "any()"),
            (r#"any(feature = "missing", unix)"#, "unix"),
            (r#"not(feature = "missing")"#, "all()"),
            (
                r#"all(feature = "forced", any(feature = "missing", not(windows)))"#,
                "not(windows)",
            ),
            (r#"not(not(feature = "optional"))"#, r#"feature = "optional""#),
            (
                r#"all(unix, any(windows, feature = "optional"))"#,
                r#"all(unix, any(windows, feature = "optional"))"#,
            ),
        ] {
            assert_eq!(specialize_cfg(input, &declared, &guards).expect("cfg"), expected);
        }
    }
}
