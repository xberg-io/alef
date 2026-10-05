use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::config::e2e::ArgMapping;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateReturnForm {
    #[default]
    Dict,
    BareString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEnv {
    #[serde(default)]
    pub api_key_var: Option<String>,
}

impl FixtureEnv {
    pub(crate) fn api_key_var_or_default(env: Option<&Self>) -> &str {
        env.and_then(|value| value.api_key_var.as_deref()).unwrap_or("API_KEY")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupCall {
    pub call: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocs {
    pub topic: String,
    #[serde(default)]
    pub stem: Option<String>,
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub shows: Vec<String>,
    #[serde(default)]
    pub error: Option<bool>,
    #[serde(default)]
    pub presentation: Option<FixtureDocsPresentation>,
    #[serde(default)]
    pub client: Option<FixtureDocsClient>,
    #[serde(default)]
    pub side_effects: SideEffectClass,
    #[serde(default)]
    pub coverage_exceptions: BTreeMap<String, SnippetCoverageException>,
    /// Facts this fixture supplies for `[crates.e2e.snippets].sample_url_template` placeholders
    /// other than `{path}` (see `crate::core::config::e2e::SampleUrlTemplate`), e.g.
    /// `{"digest": "9f86d081..."}` for a template shaped
    /// `"https://cdn.example.org/objects/{digest}"`.
    ///
    /// Kept on the fixture rather than a separate corpus-manifest file the run would have to
    /// read and keep in sync: the fact that decides this fixture's own address belongs beside
    /// the fixture it describes. Unused, and harmless to leave empty, when the project
    /// configures no `sample_url_template` at all -- resolution then falls back to
    /// `sample_base_url` exactly as it always has.
    #[serde(default)]
    pub sample_url_vars: BTreeMap<String, String>,
    /// The corpus-relative path of this fixture's underlying content file, used to look up
    /// `[crates.e2e.snippets].sample_url_manifest` entries for it (see
    /// `crate::core::config::e2e::SampleUrlManifest`).
    ///
    /// Distinct from `sample_url_vars` above: this names WHICH manifest entry belongs to this
    /// fixture, while `sample_url_vars` declares facts directly. A fixture with no manifest
    /// configured, or with no `body_file` declared, is unaffected either way -- resolution falls
    /// back through `sample_url_vars` and then `sample_base_url` exactly as it always has.
    #[serde(default)]
    pub body_file: Option<String>,
    /// The public address THIS fixture's sample input really is served at, overriding
    /// `[crates.e2e.snippets].sample_base_url` for this fixture alone.
    ///
    /// Exists for the asymmetric corpus: a project declares `[crates.e2e.snippets].mock_only`
    /// because almost none of its sample inputs are hosted anywhere, and the handful that
    /// genuinely are say so here. The override wins over the corpus default in BOTH directions
    /// -- it supplies an address where the corpus declares none, and it replaces the corpus
    /// base where one is configured -- so a mock-only corpus is a default, never a ceiling.
    ///
    /// Resolved through exactly the same validator and the same `join` as the corpus-level
    /// base, so a fixture whose URL argument is undeclared publishes this value verbatim, and
    /// one whose input declares a mock-relative path (`"/pdf/report.pdf"`) publishes this value
    /// with that path appended -- identical to what a corpus-wide `sample_base_url` of this
    /// value would have produced for this fixture.
    ///
    /// Declaring it also re-enables the reserved-domain warning for this fixture: `mock_only`
    /// suppresses "no public address exists for this fixture", never "this fixture claimed one
    /// and it did not resolve". See `crate::e2e::snippets::sample_url_policy`.
    #[serde(default)]
    pub sample_url: Option<String>,
}

/// How a documentation snippet constructs its client, for fixtures whose subject
/// *is* the client configuration.
///
/// A snippet is rendered with the mock harness stripped, so the generator's client
/// construction degenerates to `factory(<credential>, <no base URL>, <defaults…>)`.
/// A `configuration/*` fixture documenting a client setting can therefore not show
/// the setting it is named for unless it says so here.
///
/// This is docs-only. Generators reach it through [`super::Fixture::docs_client`] and
/// must pass it in from a documentation-only call site, so it cannot retarget the
/// executable e2e suite, whose client has to keep pointing at the mock server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocsClient {
    /// Base URL the snippet's client is constructed with.
    ///
    /// Carried as plain data rather than a rendered expression: every binding wraps
    /// an optional string differently (`Some("…".to_string())` in Rust, a bare literal
    /// in Java), and that wrapping is the generator's knowledge, not the fixture's.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Verbatim argument expressions following the base URL, keyed by binding language
    /// (`rust`, `java`, …).
    ///
    /// Overrides `[e2e.call.overrides.<lang>] client_factory_trailing_args` for this
    /// fixture alone; a language absent from the map keeps the configured list. Values
    /// are emitted as written, so they must be valid source in the target language —
    /// use this for settings with no language-agnostic representation (timeouts,
    /// retry policies, builder expressions).
    #[serde(default)]
    pub args: BTreeMap<String, Vec<String>>,
    /// Client configuration, as a JSON object, for a fixture whose subject is how the client
    /// is configured (a budget, a cache) rather than which call it makes.
    ///
    /// ~keep The snippet constructs the client through the language's
    /// `client_factory_from_json` (see `CallOverride`) with this value serialized verbatim as a
    /// string literal. It is the complete configuration: nothing is merged into it, so include
    /// the credential and any base URL the factory needs, and `base_url`/`args` above do not
    /// apply. Unlike those, it is part of what the fixture tests, so a fixture that declares it
    /// is excluded from the executable e2e suite instead of running against a plain client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl FixtureDocsClient {
    /// Verbatim trailing argument expressions this fixture declares for `language`,
    /// or `None` when it defers to the configured default.
    pub fn args_for(&self, language: &str) -> Option<&[String]> {
        self.args.get(language).map(Vec::as_slice)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocsPresentation {
    #[serde(default)]
    pub call: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub args: Option<Vec<ArgMapping>>,
    #[serde(default)]
    pub files: Vec<FixtureDocsFileInput>,
    #[serde(default)]
    pub operations: Vec<FixtureDocsOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocsFileInput {
    pub field: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FixtureDocsOperation {
    Show {
        path: String,
        /// Print the value with the language's human-readable formatter rather than its
        /// debug/inspection one.
        ///
        /// Defaults to `false`, which keeps the debug formatting every existing fixture
        /// renders with today. Set it for a `path` that resolves to a plain string or
        /// number, where the Rust snippet's `{:?}` publishes `Some(Text("Hello!"))` in
        /// place of the `Hello!` a reader is looking for. Mirrors `Iterate::display`.
        #[serde(default)]
        display: bool,
    },
    Iterate {
        path: String,
        item: String,
        #[serde(default)]
        fields: Vec<String>,
        #[serde(default)]
        display: bool,
        #[serde(default)]
        optional: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnippetCoverageException {
    pub reason: String,
    pub documentation: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideEffectClass {
    #[default]
    #[serde(alias = "none", alias = "local")]
    Safe,
    Network,
    Process,
    Install,
    Server,
}

#[cfg(test)]
mod tests {
    use super::{FixtureDocs, FixtureDocsOperation, FixtureEnv, SideEffectClass};

    #[test]
    fn snippet_credential_name_prefers_fixture_metadata_with_a_generic_fallback() {
        let configured = FixtureEnv {
            api_key_var: Some("SAMPLE_SERVICE_TOKEN".into()),
        };
        assert_eq!(
            FixtureEnv::api_key_var_or_default(Some(&configured)),
            "SAMPLE_SERVICE_TOKEN"
        );
        assert_eq!(FixtureEnv::api_key_var_or_default(None), "API_KEY");
    }

    #[test]
    fn side_effects_round_trip_without_collapsing_classes() {
        for class in [
            SideEffectClass::Safe,
            SideEffectClass::Network,
            SideEffectClass::Process,
            SideEffectClass::Install,
            SideEffectClass::Server,
        ] {
            let encoded = serde_json::to_string(&class).unwrap();
            assert_eq!(serde_json::from_str::<SideEffectClass>(&encoded).unwrap(), class);
        }
    }

    #[test]
    fn docs_paths_deserialize_as_target_specific_relative_paths() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "fallback",
            "paths": {
                "node": "config/example.md",
                "wasm": "browser/example.md"
            }
        }))
        .expect("fixture docs deserialize");

        assert_eq!(docs.paths["node"], "config/example.md");
        assert_eq!(docs.paths["wasm"], "browser/example.md");
    }

    #[test]
    fn legacy_safe_aliases_remain_accepted_but_external_mutation_is_rejected() {
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""none""#).unwrap(),
            SideEffectClass::Safe
        );
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""local""#).unwrap(),
            SideEffectClass::Safe
        );
        assert!(serde_json::from_str::<SideEffectClass>(r#""external_mutation""#).is_err());
    }

    #[test]
    fn structured_presentation_deserializes_without_language_code() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "configuration",
            "presentation": {
                "call": "process_file",
                "input": {"source": "guide.txt"},
                "args": [{"name": "source", "field": "source", "type": "string"}],
                "files": [{"field": "/source", "path": "examples/guide.txt"}],
                "operations": [
                    {"op": "show", "path": "summary"},
                    {"op": "iterate", "path": "items", "item": "item", "fields": ["text"], "optional": true}
                ]
            }
        }))
        .expect("fixture docs presentation deserialize");

        let presentation = docs.presentation.expect("presentation");
        assert_eq!(presentation.input, Some(serde_json::json!({"source": "guide.txt"})));
        assert_eq!(presentation.call.as_deref(), Some("process_file"));
        assert_eq!(presentation.files[0].field, "/source");
        assert!(matches!(
            presentation.operations[0],
            FixtureDocsOperation::Show { display: false, .. }
        ));
        assert!(matches!(
            presentation.operations[1],
            FixtureDocsOperation::Iterate { optional: true, .. }
        ));
    }

    #[test]
    fn show_operation_display_flag_is_opt_in() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "configuration",
            "presentation": {
                "operations": [
                    {"op": "show", "path": "summary"},
                    {"op": "show", "path": "text", "display": true}
                ]
            }
        }))
        .expect("fixture docs presentation deserialize");

        let presentation = docs.presentation.expect("presentation");
        assert!(
            matches!(
                presentation.operations[0],
                FixtureDocsOperation::Show { display: false, .. }
            ),
            "an existing `show` without the flag must keep debug formatting"
        );
        assert!(
            matches!(
                presentation.operations[1],
                FixtureDocsOperation::Show { display: true, .. }
            ),
            "`display: true` must survive deserialization"
        );
    }

    #[test]
    fn docs_client_deserializes_a_base_url_and_per_language_argument_lists() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "configuration",
            "client": {
                "base_url": "https://llm.internal.example.com/v1",
                "args": {"rust": ["Some(30)", "None", "None"], "java": ["30", "null", "null"]}
            }
        }))
        .expect("fixture docs client deserialize");

        let client = docs.client.expect("client");
        assert_eq!(client.base_url.as_deref(), Some("https://llm.internal.example.com/v1"));
        let expected: Vec<String> = ["Some(30)", "None", "None"].map(String::from).to_vec();
        assert_eq!(client.args_for("rust"), Some(expected.as_slice()));
        assert_eq!(
            client.args_for("gleam"),
            None,
            "an unlisted language must not be invented"
        );
    }

    #[test]
    fn docs_client_config_round_trips_as_the_object_the_fixture_wrote() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "budget",
            "client": {"config": {"api_key": "k", "budget": {"global_limit": 0.0}}}
        }))
        .expect("fixture docs client config deserializes");
        let client = docs.client.expect("client");
        assert_eq!(
            client.config,
            Some(serde_json::json!({"api_key": "k", "budget": {"global_limit": 0.0}}))
        );
        assert_eq!(
            serde_json::to_value(super::FixtureDocsClient::default()).expect("serializes"),
            serde_json::json!({"base_url": null, "args": {}}),
            "a client without a config must serialize exactly as it did before the key existed"
        );
    }

    #[test]
    fn docs_without_a_client_key_carry_no_client_override() {
        let docs: FixtureDocs =
            serde_json::from_value(serde_json::json!({"topic": "configuration"})).expect("fixture docs deserialize");
        assert_eq!(docs.client, None);
    }

    #[test]
    fn sample_url_vars_default_to_empty_and_deserialize_when_declared() {
        let bare: FixtureDocs =
            serde_json::from_value(serde_json::json!({"topic": "configuration"})).expect("fixture docs deserialize");
        assert!(bare.sample_url_vars.is_empty());

        let with_vars: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "configuration",
            "sample_url_vars": {"digest": "9f86d081884c7d659a2feaa0c55ad015"}
        }))
        .expect("fixture docs with sample_url_vars deserialize");
        assert_eq!(
            with_vars.sample_url_vars.get("digest").map(String::as_str),
            Some("9f86d081884c7d659a2feaa0c55ad015")
        );
    }

    #[test]
    fn body_file_defaults_to_none_and_deserializes_when_declared() {
        let bare: FixtureDocs =
            serde_json::from_value(serde_json::json!({"topic": "configuration"})).expect("fixture docs deserialize");
        assert_eq!(bare.body_file, None);

        let with_body_file: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "configuration",
            "body_file": "pdf/memo.pdf"
        }))
        .expect("fixture docs with body_file deserialize");
        assert_eq!(with_body_file.body_file.as_deref(), Some("pdf/memo.pdf"));
    }

    #[test]
    fn concise_docs_contract_deserializes_structured_input_and_result_intent() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "configuration",
            "description": "Process a structured source.",
            "input": {"source": {"kind": "text", "value": "Hello"}},
            "shows": ["summary", "items[0].label"],
            "error": false
        }))
        .expect("fixture docs deserialize");

        assert_eq!(
            docs.input,
            Some(serde_json::json!({"source": {"kind": "text", "value": "Hello"}}))
        );
        assert_eq!(docs.shows, vec!["summary", "items[0].label"]);
        assert_eq!(docs.error, Some(false));
    }
}
