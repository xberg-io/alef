//! Assertion recipe gates for domain-shaped e2e assertions.
//!
//! Most fixture assertions are generic field/text/JSON checks and should work
//! without configuration. The names below are synthetic shortcuts tied to
//! domain-specific result shapes or streaming pseudo-fields, so they require
//! an explicit recipe opt-in.

use std::collections::HashSet;

use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::e2e::field_access::{FieldResolver, LeafAnchor};
use crate::e2e::fixture::{Assertion, Fixture};

// Compatibility recipe names for fixture-declared opt-ins. Do not enable
// these implicitly from assertion names outside this gate.
pub(crate) const CHUNKS_RECIPE: &str = "chunks";
pub(crate) const EMBEDDINGS_RECIPE: &str = "embeddings";
pub(crate) const KEYWORDS_RECIPE: &str = "keywords";
pub(crate) const STREAMING_RECIPE: &str = "streaming";
pub(crate) const TREE_RECIPE: &str = "tree";

/// Whether the call's own result type declares `chunks`, directly or through a `result_fields`-
/// declared envelope prefix — the field every `CHUNKS_RECIPE` synthetic handler
/// (`chunks_have_content`, `chunks_have_embeddings`, `chunks_have_heading_context`,
/// `first_chunk_starts_with_heading`) hardcoded as `{result_var}.chunks` before any generic field
/// validation ran.
///
/// ~keep These handlers used to intercept unconditionally, ahead of `is_valid_for_result`, so a
/// crate where `chunks` is declared on some OTHER type in the same IR (e.g. a nested
/// `Document` reached through `Envelope.results`) but not on the call's own root type still got
/// `result.chunks` emitted — code that does not compile. `FieldResolver::anchor_leaf` is the
/// anchored, whole-path oracle that first asks the same question `root_declares_path` always
/// answered (refuse only when positively known absent, same as #291 established for the
/// derived-snippet path), and — new here — tries every `result_fields` prefix the IR confirms
/// reaches `chunks` before agreeing with that refusal. `Some(LeafAnchor::Direct)` (the root
/// declares it, or no anchor exists at all — the pre-existing default) and
/// `Some(LeafAnchor::Prefixed(_))` (an envelope prefix reaches it) both keep rendering; only
/// `None` refuses.
pub(crate) fn chunks_field_declared_by_result(field_resolver: &FieldResolver) -> bool {
    field_resolver.anchor_leaf(CHUNKS_RECIPE).is_some()
}

/// The expression each `CHUNKS_RECIPE` synthetic handler should anchor `.chunks` (or the target
/// language's cased equivalent) onto, in `language`'s accessor syntax.
///
/// Delegates the per-language spelling to [`FieldResolver::accessor`] — the same renderer every
/// generic field access already goes through — so casing, optional-chaining and index syntax for
/// the prefix stay in the one place that already owns them, instead of a second hand-rolled copy
/// per backend. Returns `result_var` unchanged whenever [`chunks_field_declared_by_result`] would
/// refuse (callers must check that first) or when the root declares `chunks` directly.
pub(crate) fn chunks_result_var(field_resolver: &FieldResolver, language: &str, result_var: &str) -> String {
    match field_resolver.anchor_leaf(CHUNKS_RECIPE) {
        Some(LeafAnchor::Prefixed(prefix)) => field_resolver.accessor(&prefix, language, result_var),
        _ => result_var.to_string(),
    }
}

/// The four synthetic handlers that hardcode `result.chunks`.
const CHUNKS_SYNTHETIC_FIELDS: [&str; 4] = [
    "chunks_have_content",
    "chunks_have_embeddings",
    "chunks_have_heading_context",
    "first_chunk_starts_with_heading",
];

/// Why `field`'s synthetic handler must be skipped for this call, or `None` to render it.
///
/// Every backend asks this one question and then spells the answer in its own comment syntax, so
/// only the spelling stays local. Seven copies of the same match arm was how the hardcoded
/// `result.chunks` reached seven languages in the first place; a shared predicate is what keeps a
/// future eighth backend from re-deriving it differently. ~keep
pub(crate) fn chunks_synthetic_skip_reason(field: &str, field_resolver: &FieldResolver) -> Option<String> {
    (CHUNKS_SYNTHETIC_FIELDS.contains(&field) && !chunks_field_declared_by_result(field_resolver))
        .then(|| crate::e2e::codegen::field_skip::FieldSkip::NotAvailableOnResultType.message(field))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MissingAssertionRecipe {
    pub field: String,
    pub recipe: &'static str,
}

pub(crate) fn required_recipe(assertion: &Assertion) -> Option<&'static str> {
    if let Some(field) = assertion.field.as_deref() {
        return required_field_recipe(field);
    }
    assertion.method.as_deref().and_then(required_method_recipe)
}

pub(crate) fn required_field_recipe(field: &str) -> Option<&'static str> {
    // ~keep `mock.*` (alef issue #433) is gated ahead of the literal match below because its
    // bracket form (`mock.requests["<key>"]`) is open-ended and cannot be listed as a match arm --
    // see `mock_assertions::model` for the grammar this delegates to.
    if super::mock_assertions::is_mock_virtual_field(field) {
        return Some(super::mock_assertions::MOCK_RECIPE);
    }
    match field {
        "stream.items"
        | "stream.items.length"
        | "chunks"
        | "chunks.length"
        | "stream_content"
        | "stream_complete"
        | "no_chunks_after_done"
        | "tool_calls"
        | "finish_reason"
        | "usage"
        | "stream.has_page_event"
        | "stream.has_error_event"
        | "stream.has_complete_event"
        | "stream.event_count_min" => Some(STREAMING_RECIPE),
        "chunks_content"
        | "chunks_have_content"
        | "chunks_have_heading_context"
        | "chunks_heading_context"
        | "chunks_have_embeddings"
        | "first_chunk_heading"
        | "first_chunk_starts_with_heading" => Some(CHUNKS_RECIPE),
        "embeddings"
        | "embedding_dimensions"
        | "embeddings_valid"
        | "embeddings_finite"
        | "embeddings_non_zero"
        | "embeddings_normalized" => Some(EMBEDDINGS_RECIPE),
        "keywords" | "keywords_count" => Some(KEYWORDS_RECIPE),
        "root_child_count"
        | "root_child_count_min"
        | "root_node_type"
        | "named_children_count"
        | "has_error_nodes"
        | "error_count"
        | "tree_error_count"
        | "tree_not_null"
        | "tree_to_sexp" => Some(TREE_RECIPE),
        _ if starts_with_recipe_root(field, &["tool_calls", "finish_reason", "usage"]) => Some(STREAMING_RECIPE),
        _ => None,
    }
}

pub(crate) fn required_method_recipe(method: &str) -> Option<&'static str> {
    match method {
        "root_child_count"
        | "root_node_type"
        | "named_children_count"
        | "has_error_nodes"
        | "contains_node_type"
        | "find_nodes_by_type"
        | "error_count"
        | "tree_error_count"
        | "tree_to_sexp"
        | "run_query" => Some(TREE_RECIPE),
        _ => None,
    }
}

pub(crate) fn enabled_recipes<'a>(
    fixture: &'a Fixture,
    call_config: &'a CallConfig,
    language: &str,
) -> HashSet<&'a str> {
    let mut recipes: HashSet<&str> = fixture.assertion_recipes.iter().map(String::as_str).collect();
    recipes.extend(call_config.assertion_recipes.iter().map(String::as_str));
    if let Some(override_config) = call_config.overrides.get(language) {
        recipes.extend(override_config.assertion_recipes.iter().map(String::as_str));
    }
    // Implicitly enable streaming recipe when the call is marked as streaming.
    if call_config.streaming_enabled().unwrap_or(false) {
        recipes.insert(STREAMING_RECIPE);
    }
    recipes
}

pub(crate) fn missing_recipe_for_language(
    fixture: &Fixture,
    call_config: &CallConfig,
    language: &str,
    e2e_config: &E2eConfig,
) -> Option<MissingAssertionRecipe> {
    let enabled = enabled_recipes(fixture, call_config, language);
    let result_fields = e2e_config.effective_result_fields(call_config);
    fixture.assertions.iter().find_map(|assertion| {
        if assertion
            .field
            .as_deref()
            .is_some_and(|field| field_is_explicit_result_mapping(field, result_fields))
        {
            return None;
        }
        let recipe = required_recipe(assertion)?;
        if enabled.contains(recipe) {
            return None;
        }
        let field = assertion
            .field
            .clone()
            .or_else(|| assertion.method.clone())
            .unwrap_or_else(|| "<unknown>".to_string());
        Some(MissingAssertionRecipe { field, recipe })
    })
}

fn starts_with_recipe_root(field: &str, roots: &[&str]) -> bool {
    roots.iter().any(|root| {
        field.len() > root.len()
            && field.starts_with(root)
            && field[root.len()..]
                .chars()
                .next()
                .is_some_and(|separator| separator == '.' || separator == '[')
    })
}

fn field_is_explicit_result_mapping(field: &str, result_fields: &HashSet<String>) -> bool {
    let root = field.split_once(['.', '[']).map(|(root, _)| root).unwrap_or(field);
    result_fields.contains(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::CallOverride;
    use crate::e2e::fixture::Assertion;

    fn fixture(assertions: Vec<Assertion>) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "fixture".to_string(),
            category: None,
            description: "fixture".to_string(),
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
            assertions,
            source: "fixtures/example.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }
    }

    #[test]
    fn embeddings_fields_require_embeddings_recipe() {
        let assertion = Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("embeddings_valid".to_string()),
            ..Default::default()
        };

        assert_eq!(required_recipe(&assertion), Some(EMBEDDINGS_RECIPE));
    }

    #[test]
    fn chunk_synthetic_fields_require_chunks_recipe() {
        for field in [
            "chunks_have_content",
            "chunks_have_embeddings",
            "chunks_have_heading_context",
            "first_chunk_starts_with_heading",
        ] {
            let assertion = Assertion {
                assertion_type: "is_true".to_string(),
                field: Some(field.to_string()),
                ..Default::default()
            };

            assert_eq!(required_recipe(&assertion), Some(CHUNKS_RECIPE), "field: {field}");
        }
    }

    #[test]
    fn streaming_virtual_fields_require_streaming_recipe() {
        for field in [
            "stream.items",
            "stream.items.length",
            "stream_content",
            "stream_complete",
            "tool_calls[0].function.name",
            "usage.total_tokens",
        ] {
            let assertion = Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some(field.to_string()),
                ..Default::default()
            };

            assert_eq!(required_recipe(&assertion), Some(STREAMING_RECIPE), "field: {field}");
        }
    }

    #[test]
    fn mock_request_count_fields_require_mock_requests_recipe() {
        for field in [
            "mock.requests.total",
            "mock.requests[\"POST /v1/chat\"]",
            "mock.requests[\"/v1/chat\"]",
        ] {
            let assertion = Assertion {
                assertion_type: "equals".to_string(),
                field: Some(field.to_string()),
                value: Some(serde_json::json!(1)),
                ..Default::default()
            };

            assert_eq!(
                required_recipe(&assertion),
                Some(super::super::mock_assertions::MOCK_RECIPE),
                "field: {field}"
            );
        }
    }

    #[test]
    fn tree_fields_and_methods_require_tree_recipe() {
        for field in [
            "root_child_count",
            "root_child_count_min",
            "root_node_type",
            "named_children_count",
            "tree_not_null",
            "tree_to_sexp",
        ] {
            let assertion = Assertion {
                assertion_type: "not_empty".to_string(),
                field: Some(field.to_string()),
                ..Default::default()
            };

            assert_eq!(required_recipe(&assertion), Some(TREE_RECIPE), "field: {field}");
        }

        for method in ["run_query", "contains_node_type", "find_nodes_by_type"] {
            let assertion = Assertion {
                assertion_type: "method_result".to_string(),
                method: Some(method.to_string()),
                ..Default::default()
            };

            assert_eq!(required_recipe(&assertion), Some(TREE_RECIPE), "method: {method}");
        }
    }

    #[test]
    fn generic_nested_fields_do_not_require_recipe() {
        let assertion = Assertion {
            assertion_type: "contains".to_string(),
            field: Some("metadata.title".to_string()),
            ..Default::default()
        };

        assert_eq!(required_recipe(&assertion), None);
    }

    #[test]
    fn explicit_result_mapping_allows_domain_shaped_field_without_recipe() {
        let fixture = fixture(vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("usage.total_tokens".to_string()),
            value: Some(serde_json::json!(42)),
            ..Default::default()
        }]);
        let mut call_config = CallConfig::default();
        call_config.result_fields.insert("usage".to_string());
        let e2e_config = E2eConfig::default();

        assert!(missing_recipe_for_language(&fixture, &call_config, "rust", &e2e_config).is_none());
    }

    #[test]
    fn global_result_fields_allow_domain_shaped_field_without_per_call_override() {
        let fixture = fixture(vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("usage.total_tokens".to_string()),
            value: Some(serde_json::json!(42)),
            ..Default::default()
        }]);
        let call_config = CallConfig::default();
        let mut e2e_config = E2eConfig::default();
        e2e_config.result_fields.insert("usage".to_string());

        assert!(missing_recipe_for_language(&fixture, &call_config, "rust", &e2e_config).is_none());
    }

    #[test]
    fn per_language_override_enables_recipe() {
        let fixture = fixture(vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("chunks_have_embeddings".to_string()),
            ..Default::default()
        }]);
        let mut call_config = CallConfig::default();
        let mut override_config = CallOverride::default();
        override_config.assertion_recipes.insert(CHUNKS_RECIPE.to_string());
        call_config.overrides.insert("python".to_string(), override_config);
        let e2e_config = E2eConfig::default();

        assert!(missing_recipe_for_language(&fixture, &call_config, "python", &e2e_config).is_none());
        assert_eq!(
            missing_recipe_for_language(&fixture, &call_config, "rust", &e2e_config),
            Some(MissingAssertionRecipe {
                field: "chunks_have_embeddings".to_string(),
                recipe: CHUNKS_RECIPE,
            })
        );
    }

    #[test]
    fn streaming_enabled_implicitly_enables_streaming_recipe() {
        let fixture = fixture(vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("stream.has_page_event".to_string()),
            ..Default::default()
        }]);
        let call_config = CallConfig {
            streaming: Some(crate::core::config::e2e::StreamingConfig::Enabled(true)),
            ..Default::default()
        };
        let e2e_config = E2eConfig::default();

        // When streaming is enabled on the call, the streaming recipe should be
        // implicitly available without being listed in assertion_recipes.
        assert!(missing_recipe_for_language(&fixture, &call_config, "rust", &e2e_config).is_none());
    }

    /// The regression shape: `Envelope { results: Vec<Document> }`, and `Document` (reached only
    /// through `results`, never through `Envelope` directly) declares `chunks`. Flat, any-type
    /// name reachability cannot tell `Envelope` and `Document` apart, so it would wave a bare
    /// `chunks` through for a call whose OWN root type is `Envelope` — the shape every
    /// `CHUNKS_RECIPE` synthetic handler hardcodes as `{result_var}.chunks`. Anchoring at the
    /// call's declared root, per-path, is what tells them apart. ~keep
    mod chunks_synthetic_root_anchoring {
        use super::*;
        use crate::core::ir::{FieldDef, TypeDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        fn envelope_and_document_type_defs() -> Vec<TypeDef> {
            vec![
                TypeDef {
                    name: "Envelope".to_string(),
                    fields: vec![FieldDef {
                        name: "results".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                        ..FieldDef::default()
                    }],
                    ..TypeDef::default()
                },
                TypeDef {
                    name: "Document".to_string(),
                    fields: vec![FieldDef {
                        name: "chunks".to_string(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                        ..FieldDef::default()
                    }],
                    ..TypeDef::default()
                },
            ]
        }

        fn resolver_anchored_at(root_type: &str) -> FieldResolver {
            let type_defs = envelope_and_document_type_defs();
            let map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
            let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
            FieldResolver::new(
                &HashMap::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
            )
            .with_ir_result_fields(map, Some(root_type.to_string()))
            .with_ir_fields(reachable, excluded, optional)
        }

        /// The defect: a call whose own root type (`Envelope`) never declares `chunks` — it is
        /// only reachable through the nested `Document` — must not have its `CHUNKS_RECIPE`
        /// synthetic handlers hardcode `result.chunks` against it.
        #[test]
        fn chunks_only_reachable_via_another_ir_type_is_refused() {
            let resolver = resolver_anchored_at("Envelope");
            assert!(
                !chunks_field_declared_by_result(&resolver),
                "`chunks` belongs to `Document`, not the call's own root `Envelope`"
            );
        }

        /// The control: a call whose root type genuinely declares `chunks` must keep resolving —
        /// the fix must not turn into "refuse every chunks fixture."
        #[test]
        fn chunks_the_call_root_genuinely_declares_still_resolves() {
            let resolver = resolver_anchored_at("Document");
            assert!(
                chunks_field_declared_by_result(&resolver),
                "`Document` declares `chunks` directly"
            );
        }

        /// No anchor at all (no resolved root type — the pre-existing state for every call site
        /// before this fix) must keep the permissive default: `None` from the oracle is "no
        /// answer", not "refuse."
        #[test]
        fn no_anchored_root_type_keeps_the_permissive_default() {
            let type_defs = envelope_and_document_type_defs();
            let map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
            let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
            let resolver = FieldResolver::new(
                &HashMap::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
            )
            .with_ir_result_fields(map, None)
            .with_ir_fields(reachable, excluded, optional);
            assert!(chunks_field_declared_by_result(&resolver));
        }
    }
}
