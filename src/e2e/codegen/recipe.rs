//! Shared e2e call and argument recipe resolution.
//!
//! This module centralizes binding-agnostic fixture/call decisions so language
//! generators do not infer behavior from downstream-shaped type names.

use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride};
use crate::core::ir::TypeDef;
use crate::e2e::fixture::Fixture;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride};
    use crate::core::ir::TypeDef;
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
}
