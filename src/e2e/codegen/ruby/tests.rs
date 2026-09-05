#[cfg(test)]
mod not_empty_tests {
    use super::super::assertions::render_assertion;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render_not_empty(field: Option<&str>, result_is_simple: bool) -> String {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "not_empty".to_string(),
            field: field.map(str::to_string),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            result_is_simple,
            &E2eConfig::default(),
            &HashSet::new(),
            &HashMap::new(),
        );
        out
    }

    /// `[].to_s` is `"[]"` — a non-empty String — so measuring the stringified value
    /// made the assertion unfalsifiable on an empty collection.
    #[test]
    fn not_empty_for_ruby_asks_the_value_not_its_string_form() {
        let out = render_not_empty(None, false);
        assert!(!out.contains(".to_s"), "got: {out}");
        assert_eq!(
            out.trim(),
            "expect(result.respond_to?(:empty?) ? !result.empty? : !result.nil?).to be(true)"
        );
    }

    #[test]
    fn not_empty_for_ruby_simple_results_asks_the_value_not_its_string_form() {
        let out = render_not_empty(Some("audio"), true);
        assert!(!out.contains(".to_s"), "got: {out}");
        assert_eq!(
            out.trim(),
            "expect(result.respond_to?(:empty?) ? !result.empty? : !result.nil?).to be(true)"
        );
    }
}

#[cfg(test)]
mod chunk_heading_context_tests {
    use super::super::assertions::render_assertion;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render(field: &str, assertion_type: &str, value: Option<serde_json::Value>) -> String {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value,
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &E2eConfig::default(),
            &HashSet::new(),
            &HashMap::new(),
        );
        out
    }

    /// Magnus generates a real typed accessor for every non-excluded struct field
    /// (`gen_field_accessor` in the magnus backend), the same mechanism that already lets this
    /// file assert `c.content` and `c.embedding` on a Ruby `Chunk`. `heading_context` is such a
    /// field, so it is reachable exactly like it is for Elixir, C#, Java and TypeScript — the old
    /// unconditional "not available on Ruby Chunk binding" skip was never checking anything.
    #[test]
    fn chunks_have_heading_context_is_asserted_not_skipped() {
        let out = render("chunks_have_heading_context", "is_true", None);
        assert!(!out.contains("skipped"), "got: {out}");
        assert_eq!(
            out.trim(),
            "expect((result.chunks || []).all? { |c| c.metadata && !c.metadata.heading_context.nil? }).to be(true)"
        );
    }

    #[test]
    fn chunks_have_heading_context_is_false_is_asserted_not_skipped() {
        let out = render("chunks_have_heading_context", "is_false", None);
        assert!(!out.contains("skipped"), "got: {out}");
        assert_eq!(
            out.trim(),
            "expect((result.chunks || []).all? { |c| c.metadata && !c.metadata.heading_context.nil? }).to be(false)"
        );
    }

    #[test]
    fn first_chunk_starts_with_heading_is_asserted_not_skipped() {
        let out = render("first_chunk_starts_with_heading", "is_true", None);
        assert!(!out.contains("skipped"), "got: {out}");
        assert_eq!(
            out.trim(),
            "expect(!(result.chunks || []).first&.metadata&.heading_context.nil?).to be(true)"
        );
    }

    // The negative-control "enum variant accessor still skips because Ruby serializes it to a
    // Hash" coverage (and its GeneratorGap-classification sibling) moved to
    // `enum_variant_access.rs`, which now derives the skip from a real IR fixture instead of the
    // bare `FieldResolver::new(...)` this module's `render` helper builds — the classification is
    // IR-driven, so it needs an actual enum in scope to exercise, not just a field name. ~keep
}

#[cfg(test)]
mod trait_bridge_tests {
    use super::super::project::render_spec_helper;
    use super::super::stubs::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};
    use crate::e2e::fixture::Fixture;
    use std::collections::BTreeMap;

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    fn make_param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    fn make_method(name: &str, params: Vec<(&str, TypeRef)>, ret: TypeRef, is_async: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: params.into_iter().map(|(n, ty)| make_param(n, ty)).collect(),
            return_type: ret,
            is_async,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
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
        }
    }

    #[test]
    fn spec_helper_stays_generic_for_library_specific_setup() {
        let content = render_spec_helper(
            true,
            false,
            false,
            "../../fixtures",
            "custom_gem",
            "custom_module",
            "127.0.0.1",
            8000,
            &BTreeMap::new(),
        );

        assert!(
            !content.contains("require 'custom_gem'"),
            "spec helper must not require the generated gem directly:\n{content}"
        );
        assert!(
            !content.contains("CustomModule") && !content.contains("SampleCrate") && !content.contains("sample_crate"),
            "spec helper must avoid library-specific module cleanup:\n{content}"
        );
    }

    /// Genericity test: a synthetic TestTrait with one sync method and Plugin super-trait
    /// must not reference any sample_core-domain names in setup_block or arg_expr.
    #[test]
    fn test_backend_emission_is_generic() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("SomeSuperTrait".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..TraitBridgeConfig::default()
        };

        let do_thing = make_method(
            "do_thing",
            vec![("x", TypeRef::Primitive(crate::core::ir::PrimitiveType::I32))],
            TypeRef::String,
            false,
        );

        let fixture = make_fixture("my_test_fixture");
        let methods = vec![&do_thing];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        // setup_block must not reference any sample_core-domain trait or method names.
        assert!(
            !emission.setup_block.contains("OcrBackend"),
            "setup_block must not hardcode domain trait names, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("process_image"),
            "setup_block must not hardcode domain method names, got:\n{}",
            emission.setup_block
        );
        // Must emit the method name from MethodDef.
        assert!(
            emission.setup_block.contains("do_thing"),
            "setup_block must contain the method name 'do_thing', got:\n{}",
            emission.setup_block
        );
        // Must emit Plugin name method when super_trait is set.
        assert!(
            emission.setup_block.contains("name"),
            "setup_block must emit 'name' for super_trait, got:\n{}",
            emission.setup_block
        );
        // arg_expr must reference the fixture id.
        assert!(
            emission.arg_expr.contains("my_test_fixture"),
            "arg_expr must reference fixture id, got: {}",
            emission.arg_expr
        );
    }

    /// Named return types must emit `'{}'` (JSON-safe string), not `TypeName.new`
    /// which would reference an undefined Ruby constant.
    #[test]
    fn test_backend_named_return_emits_json_string() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes), ("mime_type", TypeRef::String)],
            TypeRef::Named("HiddenRecord".to_string()),
            false,
        );

        let fixture = make_fixture("register_document_extractor_trait_bridge");
        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("'{}'"),
            "Named return type must emit '{{}}' not a constructor call, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("HiddenRecord.new"),
            "setup_block must not reference undefined constant HiddenRecord, got:\n{}",
            emission.setup_block
        );
    }

    /// Backend name must be extracted from fixture.input, not fixture.id.
    #[test]
    fn test_backend_name_from_input() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes)],
            TypeRef::Named("HiddenRecord".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("test-extractor"),
            "setup_block must use input-derived name 'test-extractor', got:\n{}",
            emission.setup_block
        );
        // The fixture id appears in the variable name (stub_register_...) but
        // the name() method must return the input-derived name, not the fixture id.
        assert!(
            !emission
                .setup_block
                .contains("= 'register_document_extractor_trait_bridge'"),
            "name() method must not return fixture id, got:\n{}",
            emission.setup_block
        );
    }

    /// Snapshot: verify exact setup_block shape for a DocumentExtractor-like bridge.
    #[test]
    fn test_backend_snapshot() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![
                ("content", TypeRef::Bytes),
                ("mime_type", TypeRef::String),
                ("config", TypeRef::Named("ExtractionConfig".to_string())),
            ],
            TypeRef::Named("HiddenRecord".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        let expected_setup = concat!(
            "stub_register_document_extractor_trait_bridge = Class.new do\n",
            "  def name = 'test-extractor'\n",
            "  def initialize\n",
            "    nil\n",
            "  end\n",
            "  def shutdown\n",
            "    nil\n",
            "  end\n",
            "  def version = '1.0.0'\n",
            "  def extract_bytes(content, mime_type, config) = '{}'\n",
            "end.new\n",
        );
        assert_eq!(emission.setup_block, expected_setup, "setup_block snapshot mismatch");
        assert_eq!(emission.arg_expr, "stub_register_document_extractor_trait_bridge");
    }
}

// `render_gemfile` tests used to live here too (as `gemfile_tests`), duplicating
// `project::tests` (`src/e2e/codegen/ruby/project.rs`) with different fixture
// literals (`my-gem` vs `my_gem`). The two drifted independently: this module's
// copies still asserted the old `~>` pessimistic-range behavior after
// `project::tests` was updated for exact version pinning, and only one side got
// caught. Consolidated into `project::tests`, the single owner of that
// function's tests — see `render_gemfile_registry_uses_exact_pin` and friends
// there. ~keep
#[cfg(test)]
mod app_harness_tests {
    use super::super::project::render_app_harness;

    #[test]
    fn app_harness_rb_contains_eaddrinuse_retry_block() {
        use crate::core::config::e2e::{E2eConfig, HarnessConfig};
        use crate::e2e::fixture::{Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
        use std::collections::BTreeMap;

        // Build a minimal HTTP fixture so render_app_harness produces server-pattern content.
        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "test_get".to_owned(),
            description: "test fixture".to_owned(),
            category: Some("smoke".to_owned()),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "test".to_owned(),
            http: Some(HttpFixture {
                handler: HttpHandler {
                    route: "/test".to_owned(),
                    method: "GET".to_owned(),
                    body_schema: None,
                    parameters: BTreeMap::new(),
                    middleware: None,
                },
                request: HttpRequest {
                    method: "GET".to_owned(),
                    path: "/test".to_owned(),
                    headers: BTreeMap::new(),
                    query_params: BTreeMap::new(),
                    cookies: BTreeMap::new(),
                    body: None,
                    form_data: None,
                    content_type: None,
                },
                expected_response: HttpExpectedResponse {
                    status_code: 200,
                    body: Some(serde_json::json!({"ok": true})),
                    body_partial: None,
                    headers: BTreeMap::new(),
                    validation_errors: None,
                },
            }),
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };

        let groups = vec![FixtureGroup {
            category: "smoke".to_owned(),
            fixtures: vec![fixture],
        }];
        let e2e_config = E2eConfig {
            harness: HarnessConfig {
                imports: vec!["my_gem".to_owned()],
                ..HarnessConfig::default()
            },
            ..E2eConfig::default()
        };

        let out = render_app_harness(&e2e_config, &groups);

        // The EADDRINUSE retry block must be present in the generated harness
        assert!(
            out.contains("Errno::EADDRINUSE"),
            "expected `Errno::EADDRINUSE` retry block in generated app_harness.rb:\n{out}"
        );
        // The random port selection must be present
        assert!(
            out.contains("rand(40000..60000)") || out.contains("rand("),
            "expected random port selection in generated app_harness.rb:\n{out}"
        );
        // HARNESS_PORT must be printed so spec_helper can read it
        assert!(
            out.contains("HARNESS_PORT="),
            "expected `HARNESS_PORT=` output in generated app_harness.rb:\n{out}"
        );
    }
}

#[cfg(test)]
mod env_setup_tests {
    use super::super::project::render_env_setup;
    use std::collections::BTreeMap;

    #[test]
    fn empty_env_produces_no_setup_block() {
        let env = BTreeMap::new();
        let output = render_env_setup(&env);
        assert_eq!(output, "", "empty env must produce empty string");
    }

    #[test]
    fn non_empty_env_produces_sorted_lines() {
        let mut env = BTreeMap::new();
        env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
        env.insert("FOO".to_string(), "bar".to_string());
        env.insert("BAZ".to_string(), "qux".to_string());

        let output = render_env_setup(&env);

        // Lines must be sorted by key
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines, got: {output}");
        assert!(
            lines[0].contains("BAZ"),
            "first line should be BAZ (alphabetically first), got: {}",
            lines[0]
        );
        assert!(
            lines[1].contains("E2E_ALLOW_PRIVATE_NETWORK"),
            "second line should be E2E_ALLOW_PRIVATE_NETWORK, got: {}",
            lines[1]
        );
        assert!(lines[2].contains("FOO"), "third line should be FOO, got: {}", lines[2]);

        // Each line must use ||= form with proper quoting
        for line in lines {
            assert!(line.contains("||="), "line must use ||= operator: {line}");
        }
    }
}

#[cfg(test)]
mod error_path_marker_tests {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};
    use std::collections::HashMap;

    fn render(extra: Vec<Assertion>) -> String {
        let mut assertions = vec![Assertion {
            assertion_type: "error".to_string(),
            ..Default::default()
        }];
        assertions.extend(extra);
        let fixture = Fixture {
            id: "rate_limited".to_string(),
            description: "Rejects the request".to_string(),
            assertions,
            ..Fixture::default()
        };
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "parse".to_string();
        e2e_config.call.result_var = "result".to_string();
        let enum_fields: HashMap<String, String> = HashMap::new();
        let _ = crate::e2e::codegen::take_skip_records();
        super::super::spec_file::render_spec_file(
            "error",
            &[&fixture],
            "Sample",
            None,
            "sample",
            None,
            &enum_fields,
            false,
            &e2e_config,
            false,
            false,
            &[],
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        )
    }

    /// Ruby's error path renders one `raise_error` matcher and returns, so every other assertion
    /// on the fixture used to leave no trace in the generated spec at all.
    #[test]
    fn ruby_equals_on_an_error_field_is_named_instead_of_dropped() {
        let out = render(vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("error.status_code".to_string()),
            ..Default::default()
        }]);

        // Positive first: the error block really rendered.
        assert!(
            out.contains("raise_error(RuntimeError)"),
            "the error block must render:\n{out}"
        );
        assert!(
            out.contains(
                "# skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
            ),
            "got:\n{out}"
        );

        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "ruby");
        assert_eq!(records[0].field, "equals");
    }

    /// Negative control: a lone `error` assertion must leave the generated spec marker-free.
    #[test]
    fn ruby_a_lone_error_assertion_renders_no_marker() {
        let out = render(Vec::new());

        assert!(
            out.contains("raise_error(RuntimeError)"),
            "the error block must render:\n{out}"
        );
        assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
    }
}

/// The snippet generator (`render_snippet_body`, used for docs-site examples) and the spec
/// generator (`render_spec_file`, used for `e2e/ruby/spec/*.rb` and `test_apps/ruby/spec/*.rb`)
/// both build a `json_object` argument's constructor through the single shared
/// `args::build_args_and_setup` -> `values::qualify_ruby_type` choke point. Before that helper
/// existed, both call sites unconditionally prepended the call's module onto `options_type`
/// (`format!("{mod_qualifier}::{opts_type}")`), so an `options_type` that already named a module
/// -- the gem's own, or a deliberately different one -- was re-qualified and doubled
/// (`Sample::Sample::DocumentRequest`) identically in both generators. These tests pin the
/// contract (`options_type` is a bare class name; a value that already contains `::` is used
/// verbatim) and assert the two generators render the exact same constructor expression for
/// every shape, so a future change to one call site cannot silently drift from the other. ~keep
#[cfg(test)]
mod options_type_qualification_tests {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::e2e::{ArgMapping, CallOverride};
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::Fixture;
    use std::collections::HashMap;

    fn build_e2e(options_type: &str) -> E2eConfig {
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.call.module = "sample".into();
        e2e.call.args = vec![ArgMapping {
            name: "request".into(),
            field: "request".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e.call.overrides.insert(
            "ruby".into(),
            CallOverride {
                options_type: Some(options_type.into()),
                ..CallOverride::default()
            },
        );
        e2e
    }

    fn fixture() -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "document_input", "description": "Read a document",
            "input": {"request": {"content": "hello"}},
            "assertions": [{"type": "not_error"}]
        }))
        .expect("fixture")
    }

    /// Render both generators for `options_type_value` and return
    /// `(snippet constructor line, spec constructor line)`, each trimmed of surrounding
    /// whitespace so only indentation differences are ignored.
    fn render_both(options_type_value: &str) -> (String, String) {
        let e2e = build_e2e(options_type_value);

        let snippet_body =
            super::super::snippet::render_snippet_body(&fixture(), &e2e, &ResolvedCrateConfig::default(), &[], &[])
                .expect("snippet");
        let snippet_line = snippet_body
            .lines()
            .find(|line| line.contains(".new("))
            .unwrap_or_else(|| panic!("snippet has no constructor line:\n{snippet_body}"))
            .trim()
            .to_string();

        let empty_enum: HashMap<String, String> = HashMap::new();
        let spec_body = super::super::spec_file::render_spec_file(
            "docs",
            &[&fixture()],
            "sample",
            None,
            "sample",
            Some(options_type_value),
            &empty_enum,
            false,
            &e2e,
            false,
            false,
            &[],
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            &[],
            &[],
        );
        let spec_line = spec_body
            .lines()
            .find(|line| line.contains(".new("))
            .unwrap_or_else(|| panic!("spec has no constructor line:\n{spec_body}"))
            .trim()
            .to_string();

        (snippet_line, spec_line)
    }

    #[test]
    fn bare_options_type_is_qualified_under_the_call_module_in_both_generators() {
        let (snippet_line, spec_line) = render_both("DocumentRequest");

        assert_eq!(
            snippet_line, "result = Sample.process(Sample::DocumentRequest.new(content: 'hello'))",
            "snippet: {snippet_line}"
        );
        assert_eq!(
            snippet_line, spec_line,
            "snippet and spec must render the same constructor"
        );
    }

    #[test]
    fn already_qualified_options_type_is_not_double_prefixed_in_either_generator() {
        let (snippet_line, spec_line) = render_both("Sample::DocumentRequest");

        assert_eq!(
            snippet_line, "result = Sample.process(Sample::DocumentRequest.new(content: 'hello'))",
            "snippet: {snippet_line}"
        );
        assert_eq!(
            snippet_line, spec_line,
            "snippet and spec must render the same constructor"
        );
    }

    #[test]
    fn foreign_module_options_type_is_preserved_verbatim_in_both_generators() {
        let (snippet_line, spec_line) = render_both("Zzz::DocumentRequest");

        assert_eq!(
            snippet_line, "result = Sample.process(Zzz::DocumentRequest.new(content: 'hello'))",
            "snippet: {snippet_line}"
        );
        assert_eq!(
            snippet_line, spec_line,
            "snippet and spec must render the same constructor"
        );
    }
}
