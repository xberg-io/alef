//! Shared e2e call and argument recipe resolution.
//!
//! This module centralizes binding-agnostic fixture/call decisions so language
//! generators do not infer behavior from project-shaped type names.

use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride};
use crate::core::config::extras::{AdapterConfig, AdapterPattern};
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{FunctionDef, MethodDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::{CallIr, TargetParams};
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

    /// Resolve the config type used to materialize a handle argument.
    ///
    /// Handle fixtures create an opaque owner before invoking the target call.
    /// The config type must come from explicit fixture/call metadata instead of
    /// language generators guessing a project-specific DTO name.
    pub fn handle_config_type(&self, arg: &'a ArgMapping) -> Option<&'a str> {
        if arg.arg_type != "handle" {
            return None;
        }
        arg.element_type.as_deref().or(self.options_type)
    }

    /// Resolve the concrete constructor/deserializer type for one `json_object`
    /// argument value.
    ///
    /// `options_type` remains the default for config-like arguments, while
    /// `ArgMapping::element_type` can name the DTO used by a non-config object
    /// argument. Array values still use `element_type` as their element type and
    /// therefore do not have a single object constructor type.
    pub fn json_object_constructor_type(&self, arg: &'a ArgMapping, value: &serde_json::Value) -> Option<&'a str> {
        json_object_constructor_type(arg, self.options_type, value)
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
    /// `ApiSurface::functions`, supplied only by backends that have opted into type-aware
    /// argument lowering via [`ResolvedE2eCallRecipe::with_functions`].
    ///
    /// `None` is not `&[]`: an unconverted backend must resolve to
    /// [`TargetParams::IrAbsent`] and keep its pre-IR lowering exactly, whereas `&[]` with a
    /// populated `type_defs` would resolve to [`TargetParams::Unresolvable`] for every call
    /// that is not an IR method and change what those backends emit. Opting in is therefore
    /// per-backend and explicit. ~keep
    functions: Option<&'a [FunctionDef]>,
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
            functions: None,
        }
    }

    /// Opt this recipe into type-aware argument lowering by supplying the IR free-function
    /// registry, so [`Self::target_params`] can resolve a real signature.
    ///
    /// Backends that have not been converted simply do not call this and keep answering
    /// [`TargetParams::IrAbsent`], which is what they behaved as before the seam existed. ~keep
    #[must_use]
    pub(crate) fn with_functions(mut self, functions: &'a [FunctionDef]) -> Self {
        self.functions = Some(functions);
        self
    }

    /// What the core IR declares about this call's target parameters for `language`.
    ///
    /// See [`crate::e2e::codegen::call_ir::TargetParams`] for what each state licenses. The
    /// answer is deliberately an *input* to the backend's own lowering, not a verdict: what
    /// expression a declared type wants is a per-language question, and a shared answer would
    /// reject argument spellings that compile today in some languages and not others. ~keep
    pub(crate) fn target_params(&self, language: &str) -> TargetParams<'a> {
        let Some(functions) = self.functions else {
            return TargetParams::IrAbsent;
        };
        TargetParams::resolve(
            self.call_config,
            language,
            CallIr {
                functions,
                type_defs: self.type_defs,
            },
        )
    }

    /// Whether the core IR declares this call's target function/method `async fn`.
    ///
    /// `None` when this recipe was never opted into IR-aware lowering via
    /// [`Self::with_functions`], or when the IR has no signature for `call_config.function`
    /// (an unresolvable or absent call name) -- both cases leave the caller's own
    /// `alef.toml`-configured `async` flag as the only answer, exactly as before this
    /// existed. `Some(is_async)` is the IR's own authoritative answer and should win over a
    /// config flag that only ever says "async", never "definitely not async" (see
    /// `crate::core::config::e2e::call::CallConfig::r#async`, a plain `bool` defaulting to
    /// `false`, so `false` cannot be told apart from "never configured"). ~keep
    pub(crate) fn ir_is_async(&self, language: &str) -> Option<bool> {
        let functions = self.functions?;
        crate::e2e::codegen::call_ir::CallIr {
            functions,
            type_defs: self.type_defs,
        }
        // ~keep `core_lookup_name`, not the raw base `function`: the base is legitimately empty
        // when a call names itself only per language, and an IR lookup on `""` silently answers
        // "no such function" -- which here reads as "not async" and drops the `await`. The guard
        // in `core::config::e2e::raw_function_reads` exists for exactly this mistake.
        .signature(self.call_config.core_lookup_name(language)?.as_ref())
        .map(|signature| signature.is_async)
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

    /// Resolve the config type used to materialize a handle argument.
    ///
    /// Handle fixtures create an opaque owner before invoking the target call.
    /// The config type must come from explicit fixture/call metadata instead of
    /// language generators guessing a project-specific DTO name.
    pub fn handle_config_type(&self, arg: &'a ArgMapping) -> Option<&'a str> {
        if arg.arg_type != "handle" {
            return None;
        }
        arg.element_type.as_deref().or(self.options_type)
    }

    /// Resolve the concrete constructor/deserializer type for one `json_object`
    /// argument value. See [`E2eCallRecipe::json_object_constructor_type`].
    pub fn json_object_constructor_type(&self, arg: &'a ArgMapping, value: &serde_json::Value) -> Option<&'a str> {
        json_object_constructor_type(arg, self.options_type, value)
    }
}

/// Resolve the concrete constructor/deserializer type for one `json_object`
/// argument value.
pub(crate) fn json_object_constructor_type<'a>(
    arg: &'a ArgMapping,
    options_type: Option<&'a str>,
    value: &serde_json::Value,
) -> Option<&'a str> {
    if arg.arg_type != "json_object" || value.is_array() {
        return None;
    }
    if arg.name == "config" {
        options_type.or(arg.element_type.as_deref())
    } else {
        arg.element_type.as_deref().or(options_type)
    }
}

pub(crate) fn trait_bridge_options_type(config: &ResolvedCrateConfig, language: Language) -> Option<&str> {
    config
        .trait_bridges_for(language)
        .find_map(|bridge| bridge.options_type.as_deref())
}

/// ~keep Match only declared bridge operations; fixture ids must never become ABI symbols implicitly.
pub(crate) fn trait_bridge_function_identity<'a>(
    config: &'a ResolvedCrateConfig,
    fixture: &Fixture,
) -> Option<&'a str> {
    let identity = fixture.call.as_deref()?;
    config.trait_bridges_for(Language::Ffi).find_map(|bridge| {
        [
            bridge.register_fn.as_deref(),
            bridge.unregister_fn.as_deref(),
            bridge.clear_fn.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|candidate| *candidate == identity)
    })
}

/// Which trait-bridge registry operation a fixture identifies, per
/// [`trait_bridge_derived_c_identity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TraitBridgeRegistryOperation {
    Register,
    Unregister,
    Clear,
}

/// Resolve the C ABI symbol name the FFI backend actually generates for a fixture that
/// identifies a trait-bridge registry operation -- as opposed to
/// [`trait_bridge_function_identity`], which returns the raw `register_fn`/
/// `unregister_fn`/`clear_fn` config text verbatim.
///
/// `src/backends/ffi/trait_bridge/registration.rs:91-92,140-141` computes the exported
/// symbol two different ways depending on the operation:
///
/// - `register`: `format!("{prefix}_{register_fn_name}")` -- built directly from the
///   `register_fn` config text, which is therefore already the ABI truth.
/// - `unregister`/`clear`: `format!("{prefix}_unregister_{trait_snake}")` /
///   `format!("{prefix}_clear_{trait_snake}")`, where `trait_snake` is the bridge's
///   `trait_name` in `snake_case` (via `heck::ToSnakeCase`, matching
///   `TraitBridgeSpec::trait_snake` in `src/codegen/generators/trait_bridge/spec.rs:55`).
///   The `unregister_fn`/`clear_fn` config text is consulted only to decide *whether*
///   the function exists (i.e. to match the fixture to a bridge); its literal spelling
///   is discarded. A crate author who names `clear_fn = "clear_ocr_backends"` (plural)
///   on a trait named `OcrBackend` gets an exported symbol `clear_ocr_backend`
///   (singular) -- not `clear_ocr_backends`.
///
/// Returns the unprefixed, un-language-prefixed function name (e.g.
/// `"clear_ocr_backend"`) alongside the operation kind, so the caller can apply the
/// standard `{prefix}_` treatment and, for `Unregister`/`Clear`, append the trailing
/// `out_error` out-param those two operations always take (see `unregister_fn.jinja`
/// and `clear_fn.jinja`) but `register` does not need here because register-shaped
/// fixtures never reach this generic void-call fallback (they require the vtable/
/// user_data machinery in `trait_bridge_snippet.rs`).
///
/// Returns `None` when `fixture.call` does not identify any bridge operation (mirrors
/// `trait_bridge_function_identity`).
pub(crate) fn trait_bridge_derived_c_identity(
    config: &ResolvedCrateConfig,
    fixture: &Fixture,
) -> Option<(TraitBridgeRegistryOperation, String)> {
    use heck::ToSnakeCase;

    let identity = fixture.call.as_deref()?;
    config.trait_bridges_for(Language::Ffi).find_map(|bridge| {
        if bridge.register_fn.as_deref() == Some(identity) {
            return Some((TraitBridgeRegistryOperation::Register, identity.to_string()));
        }
        if bridge.unregister_fn.as_deref() == Some(identity) {
            let trait_snake = bridge.trait_name.to_snake_case();
            return Some((
                TraitBridgeRegistryOperation::Unregister,
                format!("unregister_{trait_snake}"),
            ));
        }
        if bridge.clear_fn.as_deref() == Some(identity) {
            let trait_snake = bridge.trait_name.to_snake_case();
            return Some((TraitBridgeRegistryOperation::Clear, format!("clear_{trait_snake}")));
        }
        None
    })
}

/// Resolve the concrete stream item type for an e2e call.
///
/// Explicit call recipe metadata wins. Otherwise infer from matching streaming
/// adapters. Returning `None` is intentional: event assertions must be skipped
/// or diagnosed instead of guessing a project-specific union type.
pub(crate) fn streaming_item_type<'a>(
    call_config: &'a CallConfig,
    adapters: &'a [AdapterConfig],
    function_names: &[&str],
) -> Option<&'a str> {
    call_config.streaming_item_type().or_else(|| {
        adapters
            .iter()
            .filter(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming))
            .find(|adapter| function_names.iter().any(|name| adapter.name == *name))
            .and_then(|adapter| adapter.item_type.as_deref())
            .and_then(|item_type| item_type.rsplit("::").next())
            .filter(|value| !value.is_empty())
    })
}

pub(crate) fn trait_bridge_excluded_type_names<'a>(
    config: &'a ResolvedCrateConfig,
    language: Language,
    type_defs: &'a [TypeDef],
    methods: &[&'a MethodDef],
) -> HashSet<&'a str> {
    trait_bridge_excluded_type_names_with_enums(config, language, type_defs, methods, &HashSet::new())
}

pub(crate) fn trait_bridge_excluded_type_names_with_enums<'a>(
    config: &'a ResolvedCrateConfig,
    language: Language,
    type_defs: &'a [TypeDef],
    methods: &[&'a MethodDef],
    known_enum_names: &HashSet<&str>,
) -> HashSet<&'a str> {
    let type_by_name: HashMap<&str, &TypeDef> = type_defs.iter().map(|ty| (ty.name.as_str(), ty)).collect();
    let configured_traits: HashSet<&str> = config
        .trait_bridges_for(language)
        .flat_map(|bridge| configured_trait_names(bridge).into_iter())
        .collect();
    let mut excluded: HashSet<&str> = type_defs
        .iter()
        .filter(|ty| ty.binding_excluded || ty.is_trait && !configured_traits.contains(ty.name.as_str()))
        .map(|ty| ty.name.as_str())
        .collect();

    for method in methods {
        collect_hidden_named_types(
            &method.return_type,
            &type_by_name,
            &configured_traits,
            known_enum_names,
            &mut excluded,
        );
        for param in &method.params {
            collect_hidden_named_types(
                &param.ty,
                &type_by_name,
                &configured_traits,
                known_enum_names,
                &mut excluded,
            );
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
    known_enum_names: &HashSet<&str>,
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
                // Only exclude unknown named types that are not known enums.
                // Enum types are not in type_by_name (which contains only TypeDefs)
                // but are valid types that should pass through with their correct name.
                if !known_enum_names.contains(name.as_str()) {
                    excluded.insert(name.as_str());
                }
            }
        },
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            collect_hidden_named_types(inner, type_by_name, configured_traits, known_enum_names, excluded);
        }
        TypeRef::Map(key, value) => {
            collect_hidden_named_types(key, type_by_name, configured_traits, known_enum_names, excluded);
            collect_hidden_named_types(value, type_by_name, configured_traits, known_enum_names, excluded);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride, StreamingConfig, StreamingRecipe};
    use crate::core::config::extras::{AdapterConfig, AdapterPattern};
    use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
    use crate::core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn fixture() -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "neutral_fixture".to_string(),
            category: Some("smoke".to_string()),
            description: "neutral fixture".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::json!({}),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions: Vec::new(),
            source: "fixture.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
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
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn json_arg(name: &str, element_type: Option<&str>) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: name.to_string(),
            arg_type: "json_object".to_string(),
            optional: false,
            owned: false,
            element_type: element_type.map(str::to_string),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    #[test]
    fn config_json_constructor_prefers_options_type() {
        let arg = json_arg("config", Some("RequestConfig"));
        let value = serde_json::json!({ "enabled": true });

        assert_eq!(
            json_object_constructor_type(&arg, Some("ExtractionConfig"), &value),
            Some("ExtractionConfig")
        );
    }

    #[test]
    fn non_config_json_constructor_prefers_element_type() {
        let arg = json_arg("input", Some("ExtractInput"));
        let value = serde_json::json!({ "kind": "bytes" });

        assert_eq!(
            json_object_constructor_type(&arg, Some("ExtractionConfig"), &value),
            Some("ExtractInput")
        );
    }

    fn handle_arg() -> ArgMapping {
        ArgMapping {
            name: "engine".to_string(),
            field: "input.config".to_string(),
            arg_type: "handle".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
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
    fn handle_config_type_uses_explicit_call_metadata() {
        let call = CallConfig {
            options_type: Some("SampleEngineConfig".to_string()),
            args: vec![handle_arg()],
            ..CallConfig::default()
        };

        let fixture = fixture();
        let recipe = E2eCallRecipe::resolve("go", &fixture, &call, &[]);

        assert_eq!(recipe.handle_config_type(&call.args[0]), Some("SampleEngineConfig"));
    }

    #[test]
    fn handle_config_type_prefers_arg_element_type_override() {
        let mut arg = handle_arg();
        arg.element_type = Some("ExplicitHandleConfig".to_string());
        let call = CallConfig {
            options_type: Some("FallbackConfig".to_string()),
            args: vec![arg],
            ..CallConfig::default()
        };

        let fixture = fixture();
        let recipe = E2eCallRecipe::resolve("java", &fixture, &call, &[]);

        assert_eq!(recipe.handle_config_type(&call.args[0]), Some("ExplicitHandleConfig"));
    }

    fn streaming_adapter(name: &str, item_type: &str) -> AdapterConfig {
        AdapterConfig {
            name: name.to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: "sample::stream".to_string(),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: None,
            item_type: Some(item_type.to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }
    }

    #[test]
    fn streaming_item_type_prefers_explicit_call_recipe() {
        let call = CallConfig {
            streaming: Some(StreamingConfig::Recipe(StreamingRecipe {
                item_type: Some("ConfiguredEvent".to_string()),
                ..StreamingRecipe::default()
            })),
            ..CallConfig::default()
        };
        let adapters = vec![streaming_adapter("stream_events", "sample::AdapterEvent")];

        assert_eq!(
            streaming_item_type(&call, &adapters, &["stream_events"]),
            Some("ConfiguredEvent")
        );
    }

    #[test]
    fn streaming_item_type_infers_from_matching_adapter() {
        let call = CallConfig::default();
        let adapters = vec![streaming_adapter("stream_events", "sample::AdapterEvent")];

        assert_eq!(
            streaming_item_type(&call, &adapters, &["stream_events"]),
            Some("AdapterEvent")
        );
        assert_eq!(streaming_item_type(&call, &adapters, &["other"]), None);
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
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "PrimaryTrait".to_string(),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let type_defs = vec![hidden_record, unbridged_trait, public_options];

        let excluded = trait_bridge_excluded_type_names(&config, Language::Go, &type_defs, &[&method]);

        assert!(excluded.contains("HiddenRecord"));
        assert!(excluded.contains("SecondaryTrait"));
        assert!(!excluded.contains("PublicOptions"));
    }

    #[test]
    fn trait_bridge_function_identity_uses_declared_registry_operations() {
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "SampleBackend".to_string(),
                register_fn: Some("register_sample_backend".to_string()),
                unregister_fn: Some("unregister_sample_backend".to_string()),
                clear_fn: Some("clear_sample_backends".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let fixture = Fixture {
            call: Some("clear_sample_backends".to_string()),
            ..Fixture::default()
        };

        assert_eq!(
            trait_bridge_function_identity(&config, &fixture),
            Some("clear_sample_backends")
        );
    }

    #[test]
    fn trait_bridge_function_identity_does_not_infer_from_fixture_names() {
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "SampleBackend".to_string(),
                clear_fn: Some("clear_sample_backends".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let fixture = Fixture {
            id: "clear_sample_backends".to_string(),
            call: Some("different_operation".to_string()),
            ..Fixture::default()
        };

        assert_eq!(trait_bridge_function_identity(&config, &fixture), None);
    }

    /// `clear_fn = "clear_ocr_backends"` (plural) is the config text, but
    /// `registration.rs` derives the exported symbol from the trait name's snake_case
    /// form (`OcrBackend` -> `ocr_backend`, singular) and discards the config text's
    /// spelling entirely. This fails against the pre-fix code because
    /// `trait_bridge_function_identity` (and the naive fallback in `c.rs` built on it)
    /// returns the raw config text `"clear_ocr_backends"` verbatim -- the plural,
    /// nonexistent symbol -- instead of the derived singular one.
    #[test]
    fn trait_bridge_derived_c_identity_derives_clear_symbol_from_trait_snake_case_not_config_text() {
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "OcrBackend".to_string(),
                clear_fn: Some("clear_ocr_backends".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let fixture = Fixture {
            call: Some("clear_ocr_backends".to_string()),
            ..Fixture::default()
        };

        assert_eq!(
            trait_bridge_derived_c_identity(&config, &fixture),
            Some((TraitBridgeRegistryOperation::Clear, "clear_ocr_backend".to_string()))
        );
    }

    /// `unregister_fn = "unregister_ocr_backend"` happens to already match the
    /// trait-snake derivation (trait names are already singular), so the *name*
    /// coincides either way. This test pins the operation classification
    /// (`Unregister`, needed by the caller to know an `out_error` out-param must be
    /// appended) which the pre-fix `trait_bridge_function_identity` cannot report at
    /// all -- it returns a bare `&str` with no operation kind. Against code that
    /// (incorrectly) reused `trait_bridge_function_identity`'s return shape here, this
    /// would fail to compile/typecheck as a stand-in for "the operation kind is
    /// unavailable"; expressed as a behavioral assertion, it fails if `Register` or
    /// `Clear` is misclassified as `Unregister`'s counterpart derivation.
    #[test]
    fn trait_bridge_derived_c_identity_classifies_unregister_and_keeps_its_already_matching_name() {
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "OcrBackend".to_string(),
                unregister_fn: Some("unregister_ocr_backend".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let fixture = Fixture {
            call: Some("unregister_ocr_backend".to_string()),
            ..Fixture::default()
        };

        assert_eq!(
            trait_bridge_derived_c_identity(&config, &fixture),
            Some((
                TraitBridgeRegistryOperation::Unregister,
                "unregister_ocr_backend".to_string()
            ))
        );
    }

    /// `register_fn`'s exported symbol IS `{prefix}_{register_fn_name}` -- the literal
    /// config text -- per `registration.rs:91`, so the derived path must return the
    /// raw text unchanged for `register`, not re-derive from `trait_snake`. This
    /// fails against a naive "always derive from trait_snake" implementation, which
    /// would wrongly produce `"register_sample_backend"` here too by coincidence on
    /// this fixture, but would diverge the moment `register_fn`'s text differs from
    /// `trait_snake` (e.g. a bridge author choosing `register_fn = "register_backend"`
    /// on a multi-word trait) -- so this test also documents why `register` cannot
    /// share the `unregister`/`clear` derivation branch.
    #[test]
    fn trait_bridge_derived_c_identity_trusts_register_fn_config_text_verbatim() {
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "SampleBackend".to_string(),
                register_fn: Some("register_sample_backend".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let fixture = Fixture {
            call: Some("register_sample_backend".to_string()),
            ..Fixture::default()
        };

        assert_eq!(
            trait_bridge_derived_c_identity(&config, &fixture),
            Some((
                TraitBridgeRegistryOperation::Register,
                "register_sample_backend".to_string()
            ))
        );
    }

    /// An unconverted backend never calls `with_functions`, so the seam must answer
    /// `IrAbsent` -- the state that licenses no type claim and preserves its existing
    /// lowering exactly. Without this the whole opt-in design is unverified. ~keep
    #[test]
    fn target_params_defaults_to_ir_absent_without_opting_in() {
        let fixture = fixture();
        let call_config = CallConfig {
            function: "complete".to_string(),
            args: vec![json_arg("request", None)],
            ..CallConfig::default()
        };
        let recipe = ResolvedE2eCallRecipe::resolve("java", &fixture, &call_config, &[]);
        assert!(matches!(
            recipe.target_params("java"),
            crate::e2e::codegen::call_ir::TargetParams::IrAbsent
        ));
        assert_eq!(recipe.target_params("java").declared_type_name("request", 0), None);
    }

    /// A populated `type_defs` with no `functions` opt-in must still be `IrAbsent`, not
    /// `Unresolvable` -- `&[]` and "not supplied" are different answers, and conflating them
    /// would change what every unconverted backend emits. ~keep
    #[test]
    fn type_defs_alone_do_not_opt_a_backend_in() {
        let fixture = fixture();
        let call_config = CallConfig {
            function: "complete".to_string(),
            ..CallConfig::default()
        };
        let type_defs = [TypeDef {
            name: "Client".to_string(),
            ..TypeDef::default()
        }];
        let recipe = ResolvedE2eCallRecipe::resolve("java", &fixture, &call_config, &type_defs);
        assert!(matches!(
            recipe.target_params("java"),
            crate::e2e::codegen::call_ir::TargetParams::IrAbsent
        ));
    }

    #[test]
    fn with_functions_resolves_the_declared_parameter_type() {
        let fixture = fixture();
        let call_config = CallConfig {
            function: "complete".to_string(),
            ..CallConfig::default()
        };
        let functions = [crate::core::ir::FunctionDef {
            name: "complete".to_string(),
            params: vec![ParamDef {
                name: "request".to_string(),
                ty: TypeRef::Named("CompletionRequest".to_string()),
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("CompletionResponse".to_string()),
            ..crate::core::ir::FunctionDef::default()
        }];
        let recipe = ResolvedE2eCallRecipe::resolve("java", &fixture, &call_config, &[]).with_functions(&functions);
        assert_eq!(
            recipe.target_params("java").declared_type_name("request", 0),
            Some("CompletionRequest")
        );
    }
}
