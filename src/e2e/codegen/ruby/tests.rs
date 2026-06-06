#[cfg(test)]
mod trait_bridge_tests {
    use super::super::project::render_spec_helper;
    use super::super::stubs::emit_test_backend;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
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
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
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

#[cfg(test)]
mod gemfile_tests {
    use super::super::project::{render_app_harness, render_gemfile};
    use crate::e2e::config::DependencyMode;

    #[test]
    fn render_gemfile_registry_release_uses_tilde_rocket() {
        let out = render_gemfile("my-gem", "../../packages/ruby", "1.2.3", DependencyMode::Registry);
        assert!(out.contains("gem 'my-gem', '~> 1.2.3'"), "got: {out}");
    }

    #[test]
    fn render_gemfile_registry_prerelease_uses_rubygems_dot_pre_form() {
        let out = render_gemfile("my-gem", "../../packages/ruby", "3.6.0-rc.1", DependencyMode::Registry);
        assert!(
            out.contains("gem 'my-gem', '~> 3.6.0.pre.rc.1'"),
            "pre-release must use .pre. form, got: {out}"
        );
        assert!(
            !out.contains("3.6.0-rc.1"),
            "raw semver dash form must not appear in registry Gemfile, got: {out}"
        );
    }

    #[test]
    fn render_gemfile_registry_already_prefixed_passes_through() {
        // When alef.toml's [crates.e2e.registry.packages.ruby] version field already
        // includes a rubygems operator (`~> 3.6.0.pre.rc.1`), the codegen must use
        // it verbatim — wrapping with another `~> ` produces a double-prefix bug.
        let out = render_gemfile(
            "my-gem",
            "../../packages/ruby",
            "~> 3.6.0.pre.rc.1",
            DependencyMode::Registry,
        );
        assert!(
            out.contains("gem 'my-gem', '~> 3.6.0.pre.rc.1'"),
            "already-prefixed input must pass through verbatim, got: {out}"
        );
        assert!(!out.contains("~> ~>"), "must not double the `~>` prefix, got: {out}");
    }

    #[test]
    fn render_gemfile_local_uses_path() {
        let out = render_gemfile("my-gem", "../../packages/ruby", "3.6.0-rc.1", DependencyMode::Local);
        assert!(out.contains("path: '../../packages/ruby'"), "got: {out}");
        // The target gem line must use path:, not a version constraint.
        assert!(
            out.contains("gem 'my-gem', path:"),
            "local mode must use path: for the target gem, got: {out}"
        );
        assert!(
            !out.contains("gem 'my-gem', '~>"),
            "local mode must not pin a version for the target gem, got: {out}"
        );
    }

    #[test]
    fn app_harness_rb_contains_eaddrinuse_retry_block() {
        use crate::core::config::e2e::{E2eConfig, HarnessConfig};
        use crate::e2e::fixture::{Fixture, FixtureGroup, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};
        use std::collections::BTreeMap;

        // Build a minimal HTTP fixture so render_app_harness produces server-pattern content.
        let fixture = Fixture {
            id: "test_get".to_owned(),
            description: "test fixture".to_owned(),
            category: Some("smoke".to_owned()),
            tags: vec![],
            skip: None,
            env: None,
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
