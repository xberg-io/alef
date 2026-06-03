//! Shared e2e call and argument recipe resolution.
//!
//! This module centralizes binding-agnostic fixture/call decisions so language
//! generators do not infer behavior from downstream-shaped type names.

use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride};
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;
use std::collections::{HashMap, HashSet};

/// Effective call metadata for one fixture in one language.
#[derive(Debug, Clone)]
pub struct E2eCallRecipe<'a> {
    pub args: &'a [ArgMapping],
    pub override_config: Option<&'a CallOverride>,
    pub options_type: Option<&'a str>,
    pub options_via: &'a str,
    pub extra_args: &'a [String],
    type_defs: &'a [TypeDef],
}

impl<'a> E2eCallRecipe<'a> {
    /// Resolve per-language call metadata using existing call config and fixture overrides.
    pub fn resolve(
        language: &str,
        fixture: &'a Fixture,
        call_config: &'a CallConfig,
        type_defs: &'a [TypeDef],
    ) -> Self {
        let override_config = call_config.overrides.get(language);
        let options_type = override_config
            .and_then(|o| o.options_type.as_deref())
            .or(call_config.options_type.as_deref());
        let options_via = override_config
            .and_then(|o| o.options_via.as_deref())
            .unwrap_or("kwargs");
        let args = fixture.resolved_args(call_config);
        let extra_args = override_config.map(|o| o.extra_args.as_slice()).unwrap_or(&[]);

        Self {
            args,
            override_config,
            options_type,
            options_via,
            extra_args,
            type_defs,
        }
    }

    /// True when an absent optional `json_object` arg can be represented as a default value.
    pub fn json_object_arg_has_default(&self, arg: &ArgMapping) -> bool {
        if arg.arg_type != "json_object" {
            return false;
        }
        self.options_type
            .and_then(|name| self.type_defs.iter().find(|ty| ty.name == name))
            .is_some_and(|ty| ty.has_default)
    }

    /// True when a `json_object` config should be materialized through the configured type.
    pub fn should_materialize_json_object(&self, arg: &ArgMapping, value: &serde_json::Value) -> bool {
        if arg.arg_type != "json_object" || self.options_type.is_none() {
            return false;
        }
        if self.options_via == "from_json" {
            return !value.is_null();
        }
        value.is_object() || (value.is_null() && arg.optional && self.json_object_arg_has_default(arg))
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedE2eCallRecipe<'a> {
    pub args: &'a [ArgMapping],
    pub override_config: Option<&'a CallOverride>,
    pub options_type: Option<&'a str>,
    pub options_via: &'a str,
    pub extra_args: &'a [String],
    call_config: &'a CallConfig,
    type_defs: &'a [TypeDef],
}

impl<'a> ResolvedE2eCallRecipe<'a> {
    pub fn resolve(
        language: &str,
        fixture: &'a Fixture,
        call_config: &'a CallConfig,
        type_defs: &'a [TypeDef],
    ) -> Self {
        let base = E2eCallRecipe::resolve(language, fixture, call_config, type_defs);
        Self {
            args: base.args,
            override_config: base.override_config,
            options_type: base.options_type,
            options_via: base.options_via,
            extra_args: base.extra_args,
            call_config,
            type_defs,
        }
    }

    pub fn compatible_options_type(&self, compatible_languages: &[&str]) -> Option<&'a str> {
        self.options_type.or_else(|| {
            compatible_languages.iter().find_map(|language| {
                self.call_config
                    .overrides
                    .get(*language)
                    .and_then(|override_config| override_config.options_type.as_deref())
            })
        })
    }

    pub fn json_object_arg_has_default(&self, arg: &ArgMapping) -> bool {
        if arg.arg_type != "json_object" {
            return false;
        }
        self.options_type
            .and_then(|name| self.type_defs.iter().find(|ty| ty.name == name))
            .is_some_and(|ty| ty.has_default)
    }
}

pub(crate) fn trait_bridge_options_type(config: &ResolvedCrateConfig) -> Option<&str> {
    config
        .trait_bridges
        .iter()
        .find_map(|bridge| bridge.options_type.as_deref())
}

pub(crate) fn trait_bridge_excluded_type_names<'a>(
    config: &'a ResolvedCrateConfig,
    type_defs: &'a [TypeDef],
    methods: &[&'a MethodDef],
) -> HashSet<&'a str> {
    let type_by_name: HashMap<&str, &TypeDef> = type_defs.iter().map(|ty| (ty.name.as_str(), ty)).collect();
    let configured_traits: HashSet<&str> = config
        .trait_bridges
        .iter()
        .flat_map(|bridge| configured_trait_names(bridge).into_iter())
        .collect();
    let mut excluded: HashSet<&str> = type_defs
        .iter()
        .filter(|ty| ty.binding_excluded || ty.is_trait && !configured_traits.contains(ty.name.as_str()))
        .map(|ty| ty.name.as_str())
        .collect();

    for method in methods {
        collect_hidden_named_types(&method.return_type, &type_by_name, &configured_traits, &mut excluded);
        for param in &method.params {
            collect_hidden_named_types(&param.ty, &type_by_name, &configured_traits, &mut excluded);
        }
    }

    excluded
}

fn configured_trait_names(bridge: &TraitBridgeConfig) -> Vec<&str> {
    let mut names = vec![bridge.trait_name.as_str()];
    if let Some(super_trait) = bridge.super_trait.as_deref() {
        names.push(super_trait.rsplit("::").next().unwrap_or(super_trait));
    }
    names
}

fn collect_hidden_named_types<'a>(
    ty: &'a TypeRef,
    type_by_name: &HashMap<&'a str, &'a TypeDef>,
    configured_traits: &HashSet<&'a str>,
    excluded: &mut HashSet<&'a str>,
) {
    match ty {
        TypeRef::Named(name) => match type_by_name.get(name.as_str()) {
            Some(type_def) if type_def.binding_excluded => {
                excluded.insert(type_def.name.as_str());
            }
            Some(type_def) if type_def.is_trait && !configured_traits.contains(type_def.name.as_str()) => {
                excluded.insert(type_def.name.as_str());
            }
            Some(_) => {}
            None => {
                excluded.insert(name.as_str());
            }
        },
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            collect_hidden_named_types(inner, type_by_name, configured_traits, excluded);
        }
        TypeRef::Map(key, value) => {
            collect_hidden_named_types(key, type_by_name, configured_traits, excluded);
            collect_hidden_named_types(value, type_by_name, configured_traits, excluded);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride};
    use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
    use crate::core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn fixture() -> Fixture {
        Fixture {
            id: "neutral_fixture".to_string(),
            category: Some("smoke".to_string()),
            description: "neutral fixture".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({}),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertions: Vec::new(),
            source: "fixture.json".to_string(),
            http: None,
        }
    }

    fn config_arg() -> ArgMapping {
        ArgMapping {
            name: "settings".to_string(),
            field: "input.settings".to_string(),
            arg_type: "json_object".to_string(),
            optional: true,
            owned: false,
            element_type: None,
            go_type: None,
            trait_name: None,
        }
    }

    #[test]
    fn call_level_options_type_and_type_default_materialize_absent_config() {
        let call = CallConfig {
            options_type: Some("SampleSettings".to_string()),
            args: vec![config_arg()],
            ..CallConfig::default()
        };
        let type_defs = vec![TypeDef {
            name: "SampleSettings".to_string(),
            has_default: true,
            ..TypeDef::default()
        }];

        let fixture = fixture();
        let recipe = E2eCallRecipe::resolve("dart", &fixture, &call, &type_defs);
        assert_eq!(recipe.options_type, Some("SampleSettings"));
        assert!(recipe.json_object_arg_has_default(&call.args[0]));
        assert!(recipe.should_materialize_json_object(&call.args[0], &serde_json::Value::Null));
    }

    #[test]
    fn language_override_options_type_wins_over_call_level() {
        let mut call = CallConfig {
            options_type: Some("SampleSettings".to_string()),
            args: vec![config_arg()],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "rust".to_string(),
            CallOverride {
                options_type: Some("RustSettings".to_string()),
                extra_args: vec!["None".to_string()],
                ..CallOverride::default()
            },
        );

        let fixture = fixture();
        let recipe = E2eCallRecipe::resolve("rust", &fixture, &call, &[]);
        assert_eq!(recipe.options_type, Some("RustSettings"));
        assert_eq!(recipe.extra_args, &["None".to_string()]);
    }

    #[test]
    fn trait_bridge_exclusions_use_ir_visibility_and_bridge_config() {
        let hidden_record = TypeDef {
            name: "HiddenRecord".to_string(),
            binding_excluded: true,
            ..TypeDef::default()
        };
        let unbridged_trait = TypeDef {
            name: "SecondaryTrait".to_string(),
            is_trait: true,
            ..TypeDef::default()
        };
        let public_options = TypeDef {
            name: "PublicOptions".to_string(),
            ..TypeDef::default()
        };
        let method = MethodDef {
            name: "run".to_string(),
            params: vec![
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("PublicOptions".to_string()),
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "hidden".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("HiddenRecord".to_string()))),
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Named("SecondaryTrait".to_string()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "PrimaryTrait".to_string(),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let type_defs = vec![hidden_record, unbridged_trait, public_options];

        let excluded = trait_bridge_excluded_type_names(&config, &type_defs, &[&method]);

        assert!(excluded.contains("HiddenRecord"));
        assert!(excluded.contains("SecondaryTrait"));
        assert!(!excluded.contains("PublicOptions"));
    }
}
