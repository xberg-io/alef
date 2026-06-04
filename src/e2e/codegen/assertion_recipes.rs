//! Assertion recipe gates for domain-shaped e2e assertions.
//!
//! Most fixture assertions are generic field/text/JSON checks and should work
//! without configuration. The names below are synthetic shortcuts tied to
//! domain-specific result shapes, so they require an explicit recipe opt-in.

use std::collections::HashSet;

use crate::core::config::e2e::CallConfig;
use crate::e2e::fixture::{Assertion, Fixture};

pub(crate) const CHUNKS_RECIPE: &str = "chunks";
pub(crate) const EMBEDDINGS_RECIPE: &str = "embeddings";
pub(crate) const KEYWORDS_RECIPE: &str = "keywords";
pub(crate) const TREE_RECIPE: &str = "tree";

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
    match field {
        "chunks"
        | "chunks_content"
        | "chunks_have_content"
        | "chunks_heading_context"
        | "chunks_have_embeddings"
        | "first_chunk_heading" => Some(CHUNKS_RECIPE),
        "embeddings"
        | "embedding_dimensions"
        | "embeddings_valid"
        | "embeddings_finite"
        | "embeddings_non_zero"
        | "embeddings_normalized" => Some(EMBEDDINGS_RECIPE),
        "keywords" | "keywords_count" => Some(KEYWORDS_RECIPE),
        "root_child_count" | "has_error_nodes" | "error_count" | "tree_error_count" | "tree_to_sexp" => {
            Some(TREE_RECIPE)
        }
        _ => None,
    }
}

pub(crate) fn required_method_recipe(method: &str) -> Option<&'static str> {
    match method {
        "root_child_count" | "run_query" => Some(TREE_RECIPE),
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
    recipes
}

pub(crate) fn missing_recipe_for_language(
    fixture: &Fixture,
    call_config: &CallConfig,
    language: &str,
) -> Option<MissingAssertionRecipe> {
    let enabled = enabled_recipes(fixture, call_config, language);
    fixture.assertions.iter().find_map(|assertion| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::CallOverride;
    use crate::e2e::fixture::Assertion;

    fn fixture(assertions: Vec<Assertion>) -> Fixture {
        Fixture {
            id: "fixture".to_string(),
            category: None,
            description: "fixture".to_string(),
            tags: Vec::new(),
            skip: None,
            env: None,
            call: None,
            input: serde_json::json!({}),
            mock_response: None,
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            assertions,
            source: "fixtures/example.json".to_string(),
            http: None,
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
    fn generic_nested_fields_do_not_require_recipe() {
        let assertion = Assertion {
            assertion_type: "contains".to_string(),
            field: Some("metadata.title".to_string()),
            ..Default::default()
        };

        assert_eq!(required_recipe(&assertion), None);
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

        assert!(missing_recipe_for_language(&fixture, &call_config, "python").is_none());
        assert_eq!(
            missing_recipe_for_language(&fixture, &call_config, "rust"),
            Some(MissingAssertionRecipe {
                field: "chunks_have_embeddings".to_string(),
                recipe: CHUNKS_RECIPE,
            })
        );
    }
}
