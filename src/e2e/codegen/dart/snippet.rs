use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Context, Result};

/// Render a Dart documentation snippet without any core IR to consult.
///
/// Kept as the five-argument entry point every existing caller and test already uses: with no
/// `functions` the seam resolves to `TargetParams::IrAbsent`, which is exactly the state this path
/// was always in, so its output is unchanged by the seam. ~keep
pub(super) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<String> {
    render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, enums, &[])
}

pub(super) fn render_snippet_body_with_ir(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Result<String> {
    if fixture.is_http_test() {
        return render_http_snippet(fixture);
    }
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let mut fixture_without_assertions = fixture.clone();
    fixture_without_assertions.assertions.clear();
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Trait-bridge fixtures (e.g. `register_validator: trait bridge`) reference a
    // `_create<Stub>Wrapper()` factory in the call expression below. Dart forbids class
    // definitions inside a function, so — mirroring the full e2e test-file emitter — the
    // stub class and its factory function must be hoisted to module scope, above
    // `main()`. Without this the snippet calls a factory function that is never defined.
    let mut stub_classes = String::new();
    super::stubs::collect_test_stub_classes(
        &mut stub_classes,
        &fixture_without_assertions,
        e2e_config,
        config,
        type_defs,
        enums,
    );
    let stub_classes = stub_classes.trim_end().to_string();
    let bridge_class = config.dart_bridge_class_name();
    let first_class_map = super::values::build_dart_first_class_map(type_defs, enums, e2e_config);
    let mut test_case = String::new();
    super::test_case::render_test_case(
        &mut test_case,
        &fixture_without_assertions,
        super::test_case::DartTestCaseContext {
            e2e_config,
            lang: "dart",
            bridge_class: &bridge_class,
            dart_first_class_map: &first_class_map,
            adapters: &config.adapters,
            config,
            type_defs,
            enums,
            functions,
            errors: &[],
            native_typed_dtos: true,
            is_snippet: true,
        },
    );
    let statements = extract_test_statements(&test_case)
        .with_context(|| format!("extracting Dart snippet body for fixture `{}`", fixture.id))?;
    let package = e2e_config
        .resolve_package("dart")
        .and_then(|value| value.name)
        .unwrap_or_else(|| config.dart_pubspec_name());
    let module = config.dart_library_name();
    let bridge_module = format!("{}_bridge_generated", config.name.replace('-', "_"));
    let needs_json = statements
        .iter()
        .any(|statement| statement.contains("jsonDecode(") || statement.contains("jsonEncode("));
    let needs_io = expects_error
        || !call.returns_void
        || statements
            .iter()
            .any(|statement| statement.contains("File(") || statement.contains("Platform.environment"));
    // A trait-bridge stub's signatures are spelled by `DartMapper`, so the question of which
    // of its class names come from `dart:typed_data` is answered where that mapping lives.
    // Spot-checking `Uint8List` here missed `Float64List`/`Int64List` — the stub named the
    // class and the snippet never imported its library. ~keep
    let needs_typed_data = crate::backends::dart::type_map::needs_dart_typed_data(&stub_classes);
    // Own resolver rather than `presentation::resolve`'s, for the same reason php/snippet.rs
    // does it: `build_resolver` furnishes no per-language representation facts, so a path
    // stepping into a tagged-union variant rendered as `format?.html?` -- a getter a freezed
    // sealed class does not have. Mirrors `build_resolver`'s `new` + `with_ir_fields`, which
    // `resolve_with` does not apply, so this is a superset of the previous behaviour. ~keep
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) =
        crate::e2e::field_access::FieldResolver::ir_field_sets(type_defs);
    let field_resolver = crate::e2e::field_access::FieldResolver::new(
        e2e_config.effective_fields(call),
        e2e_config.effective_fields_optional(call),
        e2e_config.effective_result_fields(call),
        e2e_config.effective_fields_array(call),
        e2e_config.effective_fields_method_calls(call),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_variant_accessors(super::variant_accessors::build_variant_accessor_map(enums));
    let presentation = crate::e2e::codegen::presentation::resolve_with(
        fixture,
        e2e_config,
        "dart",
        &field_resolver,
        type_defs,
        enums,
        functions,
    );
    Ok(crate::e2e::template_env::render(
        "dart/snippet_body.jinja",
        minijinja::context! {
            package => package, module => module, bridge_module => bridge_module,
            statements => statements, needs_json => needs_json,
            needs_io => needs_io,
            needs_typed_data => needs_typed_data,
            expects_error => expects_error,
            error_type => config.error_type_name(),
            result_var => call.effective_result_var(),
            returns_void => call.returns_void,
            stub_classes => stub_classes,
            presentation => presentation,
        },
    ))
}

fn render_http_snippet(fixture: &Fixture) -> Result<String> {
    let http = fixture.http.as_ref().expect("HTTP fixture checked by caller");
    let plan = crate::e2e::codegen::client::http_call::plan_request(http);
    let mut headers = plan.headers;
    if let Some(content_type) = &plan.content_type
        && !headers.keys().any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("content-type".into(), content_type.clone());
    }
    if !http.request.cookies.is_empty() {
        headers.insert(
            "cookie".into(),
            http.request
                .cookies
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    let raw_body = plan.body.as_ref().is_some_and(|body| {
        matches!(body, serde_json::Value::String(_))
            && plan
                .content_type
                .as_deref()
                .is_some_and(crate::e2e::codegen::client::is_raw_text_content_type)
    });
    Ok(crate::e2e::template_env::render(
        "dart/http_snippet.jinja",
        minijinja::context! {
            method => http.request.method.to_uppercase(),
            path => format!("/fixtures/{}{}", fixture.id, http.request.path),
            headers => headers.iter().map(|(key, value)| minijinja::context! {
                key => super::values::escape_dart(key), value => super::values::escape_dart(value),
            }).collect::<Vec<_>>(),
            body_json => plan.body.as_ref().map(serde_json::to_string).transpose()?,
            raw_body => raw_body,
        },
    ))
}

fn extract_test_statements(rendered: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = rendered.lines().collect();
    let start = lines.iter().position(|line| line.trim_start().starts_with("test("))? + 1;
    let end = lines.iter().rposition(|line| line.trim() == "});")?;
    Some(
        lines[start..end]
            .iter()
            .map(|line| line.strip_prefix("    ").unwrap_or(line).to_string())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_the_native_test_body() {
        let rendered = "  test('sample', () async {\n    final value = await Api.load();\n  });\n";
        assert_eq!(
            extract_test_statements(rendered),
            Some(vec!["final value = await Api.load();".to_string()])
        );
    }

    #[test]
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        e2e.call.result_var = "result".into();
        e2e.result_fields = ["summary".to_string(), "items".to_string()].into_iter().collect();

        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(body.contains("final result = await "), "{body}");
        assert!(body.contains("stdout.writeln(result.summary);"), "{body}");
        assert!(body.contains("for (final item in result.items) {"), "{body}");
        assert!(body.contains("stdout.writeln(item.label);"), "{body}");
        assert!(
            !body.contains("stdout.writeln(result);"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
    }

    #[test]
    fn renders_native_call_without_test_harness() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[]).unwrap();
        assert!(body.contains("loadDocument()"));
        assert!(body.contains("Future<void> main() async"));
        assert!(body.contains("show RustLib"));
        assert!(body.contains("await RustLib.init()"));
        assert!(body.contains("RustLib.dispose()"));
        assert!(!body.contains("test("));
        assert!(!body.contains("expect("));
    }

    // Regression test: 188 of 190 Dart doc snippets failed `dart analyze` with
    // "The function '_fixtureUrl' isn't defined." A `client_factory` call (e.g.
    // `createClient`) makes `render_test_case` emit `final mockUrl = _fixtureUrl(...)`
    // plus a `baseUrl: mockUrl` argument — `_fixtureUrl` is only ever defined by the
    // full e2e test-file emitter, never by the standalone snippet emitter. The snippet
    // must strip the mock-URL harness entirely, matching the PHP/Ruby/Go/TypeScript
    // emitters, which all construct their client without a baseUrl override.
    #[test]
    fn snippet_omits_undefined_fixture_url_helper_from_client_factory_call() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "edge_batch_empty_list", "description": "Empty batch list", "input": null
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "create_client".into();
        e2e_config.call.overrides.insert(
            "dart".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("createClient".into()),
                ..Default::default()
            },
        );

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(
            !body.contains("_fixtureUrl"),
            "snippet must not reference the undefined _fixtureUrl helper:\n{body}"
        );
        assert!(
            !body.contains("baseUrl:"),
            "snippet must not pass a mock baseUrl override:\n{body}"
        );
        assert!(
            body.contains("Platform.environment['API_KEY']"),
            "snippet must read the generic credential environment variable:\n{body}"
        );
    }

    /// Pins the `is_snippet` branch in `dart/test_case.rs` (~line 934): a `client_factory`
    /// call must construct the client the way a reader would — no mock-server env var, no
    /// `/fixtures/<id>` route, no literal test credential — reading the API key from the
    /// environment instead.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "rate_limit_429", "description": "Rate limited", "input": null
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "chat".into();
        e2e_config.call.result_var = "result".into();
        e2e_config.call.overrides.insert(
            "dart".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("'test-key'"), "literal credential leaked:\n{body}");
        assert!(
            body.contains("Platform.environment['API_KEY']"),
            "credential is not read from the environment:\n{body}"
        );
        assert!(
            body.contains("await Bridge.createClient(apiKey)"),
            "client is not constructed the way a reader would:\n{body}"
        );
        assert!(
            !body.contains("baseUrl:"),
            "no docs.client is declared, so no baseUrl argument should be emitted:\n{body}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its Dart
    /// snippet, mirroring the Java/Elixir/Rust/Python generators' `docs_client` handling
    /// (`java/snippet.rs::a_snippet_renders_the_base_url_the_fixture_documents`). Paired with
    /// `client_factory_snippet_never_points_the_reader_at_the_mock_server` above (whose fixture
    /// declares no `docs.client` and must keep rendering the bare, no-`baseUrl` call) as the
    /// negative control: an indiscriminate "always add baseUrl" change would fail that test.
    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "custom_base_url", "description": "Custom base URL", "input": null,
            "docs": {
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "chat".into();
        e2e_config.call.result_var = "result".into();
        e2e_config.call.overrides.insert(
            "dart".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(
            body.contains("await Bridge.createClient(apiKey, baseUrl: 'https://llm.internal.example.com/v1')"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
        );
    }

    #[test]
    fn snippet_uses_package_reference_and_configured_library_entrypoint() {
        let fixture = Fixture {
            id: "sample".into(),
            description: "Sample".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "load_document".into();
        e2e.packages.insert(
            "dart".into(),
            crate::core::config::e2e::PackageRef {
                name: Some("sample_harness_dep".into()),
                ..Default::default()
            },
        );
        let mut config = ResolvedCrateConfig::default();
        config.name = "sample-core".into();
        config.dart = Some(crate::core::config::languages::DartConfig {
            lib_name: Some("sample_entrypoint".into()),
            ..Default::default()
        });

        let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet");

        assert!(
            body.contains("import 'package:sample_harness_dep/sample_entrypoint.dart';"),
            "{body}"
        );
        assert!(
            body.contains("package:sample_harness_dep/src/sample_core_bridge_generated/frb_generated.dart"),
            "{body}"
        );
        assert!(!body.contains("sample_core.dart"), "{body}");
    }

    #[test]
    fn renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let body = render_snippet_body(
            &fixture,
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet");

        assert!(body.contains("on Error catch (error)"), "{body}");
        assert!(!body.contains("expected call to fail"), "{body}");
    }

    /// Regression: a non-void call inside an `expects_error` try block must still print its
    /// result, exactly as the non-error branch already does. Before this fix the presentation/
    /// print block was nested entirely under `{% else %}` of `expects_error`, so `result` was
    /// bound (`final result = ...`) but never referenced anywhere in the error-path body — a
    /// `UNUSED_LOCAL_VARIABLE` warning `dart analyze` treats as a hard failure at the
    /// `typecheck` validation level, tslp/liter-llm's `edge_batch_already_cancelled`-shaped
    /// snippets among them. ~keep
    #[test]
    fn error_path_snippet_still_prints_a_bound_non_void_result() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "cancel_already_cancelled", "description": "Cancel an already-cancelled batch", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let body = render_snippet_body(
            &fixture,
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .expect("snippet");

        assert!(
            body.contains("stdout.writeln(result)"),
            "a bound non-void result inside the error-path try block must be printed, got:\n{body}"
        );
    }

    #[test]
    fn renders_http_request_without_test_harness_assertions() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "create_item", "description": "Create item", "input": null,
            "http": {
                "handler": {"route": "/items", "method": "POST"},
                "request": {"method": "POST", "path": "/items", "body": {"name": "sample"}},
                "expected_response": {"status_code": 201}
            }
        }))
        .unwrap();
        let body = render_snippet_body(
            &fixture,
            &E2eConfig::default(),
            &ResolvedCrateConfig::default(),
            &[],
            &[],
        )
        .unwrap();
        assert!(body.contains("HttpClient"));
        assert!(body.contains("/fixtures/create_item/items"));
        assert!(body.contains("request.write(jsonEncode(jsonDecode"));
        assert!(!body.contains("expect("));
    }

    fn make_trait_bridge(trait_name: &str) -> crate::core::config::TraitBridgeConfig {
        crate::core::config::TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
            ..Default::default()
        }
    }

    fn make_method(name: &str) -> crate::core::ir::MethodDef {
        crate::core::ir::MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            is_async: true,
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

    // Regression test for defect #543/1: a trait-bridge snippet (register_validator,
    // register_ocr_backend, ...) references a `_createTestStub<Fixture>Wrapper()` factory
    // function in its call expression. The full e2e test-file emitter hoists the stub
    // class + factory to module scope via a separate pass (`collect_test_stub_classes`);
    // the doc-snippet emitter must run the same pass, or the emitted snippet calls a
    // function that is never defined. This fixture's call is void-returning to isolate
    // the stub-emission defect from the separate void-binding defect (see the sibling
    // test below) — proving neither test can pass while only the other defect is fixed.
    #[test]
    fn snippet_hoists_test_backend_stub_class_and_factory_above_main() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "register_backend", "description": "register: trait bridge", "input": null
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "registerBackend".into();
        e2e_config.call.returns_void = true;
        e2e_config.call.args.push(crate::e2e::config::ArgMapping {
            name: "backend".into(),
            field: "input.backend".into(),
            arg_type: "test_backend".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("TestTrait".into()),
        });
        let mut config = ResolvedCrateConfig::default();
        config.trait_bridges.push(make_trait_bridge("TestTrait"));
        let type_defs = [crate::core::ir::TypeDef {
            name: "TestTrait".into(),
            methods: vec![make_method("doWork")],
            ..Default::default()
        }];

        let body = render_snippet_body(&fixture, &e2e_config, &config, &type_defs, &[]).expect("snippet");

        assert!(
            body.contains("class TestStubRegisterBackend extends TestTrait"),
            "stub class must be emitted at module scope:\n{body}"
        );
        assert!(
            body.contains("Future<TestTraitDartImpl> _createTestStubRegisterBackendWrapper()"),
            "factory function must be defined, not just referenced:\n{body}"
        );
        assert!(
            body.contains("await _createTestStubRegisterBackendWrapper()"),
            "call site must invoke the now-defined factory:\n{body}"
        );
        // The stub class must appear before `main()`; Dart forbids local class declarations.
        let class_pos = body.find("class TestStubRegisterBackend").expect("class present");
        let main_pos = body.find("Future<void> main()").expect("main present");
        assert!(class_pos < main_pos, "stub class must precede main():\n{body}");
    }

    #[test]
    fn enum_array_uses_the_generated_package_decoder() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "run_steps", "description": "Run workflow steps",
            "input": {"steps": [{"type": "approve", "identifier": "42"}]}
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "run_workflow".into();
        e2e_config.call.args.push(crate::e2e::config::ArgMapping {
            name: "steps".into(),
            field: "steps".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: Some("WorkflowStep".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        });

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(
            body.contains("createWorkflowStepFromJson(json: jsonEncode(element))"),
            "enum elements must use the generated package decoder:\n{body}"
        );
        assert!(
            !body.contains("_parse"),
            "no hand-authored enum decoder may be emitted:\n{body}"
        );
    }

    #[test]
    fn scalar_enum_array_does_not_cast_elements_to_maps() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "run_steps", "description": "Run workflow steps",
            "input": {"steps": ["approve", "reject"]}
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "run_workflow".into();
        e2e_config.call.args.push(crate::e2e::config::ArgMapping {
            name: "steps".into(),
            field: "steps".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: Some("WorkflowStep".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        });

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(
            body.contains("createWorkflowStepFromJson(json: jsonEncode(element))"),
            "{body}"
        );
        assert!(!body.contains("cast<Map<String, dynamic>>()"), "{body}");
    }

    // Regression test for defect #543/2: binding the result of a `Future<void>`-returning
    // call (`final result = await voidCall();`) is a Dart compile error even when `result`
    // is never read — the initializer's void value is what's illegal, not an unused
    // variable. This fixture uses a plain (non-trait-bridge) void call with no
    // `test_backend` argument, isolating the void-binding defect from the stub-emission
    // defect covered above.
    #[test]
    fn snippet_does_not_bind_a_void_returning_call_to_a_variable() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "clear_validators", "description": "clear validators", "input": null
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "clear_validators".into();
        e2e_config.call.returns_void = true;

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(!body.contains("final result ="), "void call must not be bound:\n{body}");
        assert!(
            body.contains(".clearValidators();"),
            "void call must still be awaited directly without a binding:\n{body}"
        );
    }

    // ~keep Regression test: `dart analyze --fatal-infos` enforces Dart's
    // `no_leading_underscores_for_local_identifiers` lint. A Dart local variable cannot be
    // library-private (privacy is a library-scope concept in Dart), so a `_`-prefixed local
    // only trips this lint for no benefit -- it was a naming convention carried over from
    // languages (e.g. Python) where a leading underscore does mean something on a local.
    // `dart/test_case.rs` used to build these two scratch-variable families with a literal
    // `_` prefix (`format!("_{}", arg_def.name)` and the hardcoded `"_client"` receiver);
    // together they accounted for most of the 188 originally failing published snippets.
    // Exercises both fixed families in one fixture: the `client_factory` receiver and a
    // generic `json_object` arg. Asserts the snippet is non-trivially emitted (the two
    // locals and the call that uses them are all present) before asserting the negative
    // (no local declaration starts with `_`), so this cannot pass vacuously against empty
    // output. Paired with `snippet_hoists_test_backend_stub_class_and_factory_above_main`
    // above, which is the control: `_createTestStubRegisterBackendWrapper` is a *module-scope*
    // private function (real Dart privacy applies there) and must keep its underscore.
    #[test]
    fn snippet_client_and_json_object_locals_have_no_leading_underscore() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "chat_basic", "description": "Send a chat request",
            "input": {"request": {"model": "gpt-4o", "messages": []}}
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "chat".into();
        e2e_config.call.result_var = "result".into();
        e2e_config.call.options_type = Some("ChatRequest".into());
        e2e_config.call.overrides.insert(
            "dart".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );
        e2e_config.call.args.push(crate::e2e::config::ArgMapping {
            name: "request".into(),
            field: "input.request".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        });

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        // Non-vacuous first: both scratch locals, and the call that consumes them, are
        // really emitted -- proving the assertions below aren't vacuously true against
        // empty or dropped output.
        assert!(
            body.contains("final client = await Bridge.createClient(apiKey)"),
            "client local must be emitted:\n{body}"
        );
        assert!(
            body.contains("final request = await createChatRequestFromJson("),
            "request local must be emitted:\n{body}"
        );
        assert!(
            body.contains("client.chat(request: request)"),
            "the call must use both locals by their new (underscore-free) names:\n{body}"
        );

        assert!(
            !body.lines().any(|line| line.trim_start().starts_with("final _")),
            "no local declaration may start with `_` -- Dart locals cannot be private, so a \
             leading underscore only trips `no_leading_underscores_for_local_identifiers`:\n{body}"
        );
    }

    fn mock_url_arg(name: &str, field: &str, arg_type: &str) -> crate::e2e::config::ArgMapping {
        crate::e2e::config::ArgMapping {
            name: name.into(),
            field: field.into(),
            arg_type: arg_type.into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    /// The half of the `_fixtureUrl` defect `snippet_omits_undefined_fixture_url_helper_from_
    /// client_factory_call` never reached: that test's fixture declares no `mock_url` argument,
    /// so it only exercised the ONE emission site the `is_snippet` flag already guarded. A
    /// fixture that declares a `mock_url` arg whose URL is already meaningful (so
    /// `mock_url_defaults` injects nothing and `preserve_input_urls` stays false) took the
    /// unguarded arm and published `_fixtureUrl("...")` into a scope that never defines it.
    #[test]
    fn a_mock_url_arg_binds_the_fixtures_own_url_instead_of_the_undefined_helper() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "rejects_internal_host", "description": "Rejects an internal host",
            "input": {"url": "https://docs.example.com/report.pdf"}
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "fetch_document".into();
        e2e_config.call.args.push(mock_url_arg("url", "input.url", "mock_url"));

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        // Positive first: the argument really was bound, so the absence below means something.
        assert!(
            body.contains("final url = "),
            "the mock_url arg must still be bound: {body}"
        );
        assert!(
            body.contains("final url = 'https://docs.example.com/report.pdf';"),
            "the snippet must show the URL the fixture declares: {body}"
        );
        assert!(
            !body.contains("_fixtureUrl"),
            "a standalone snippet must not call a helper only the test file defines: {body}"
        );
    }

    /// The list counterpart, which had the same unguarded arm and additionally emitted a
    /// `<var>Base` local built from `_fixtureUrl`.
    #[test]
    fn a_mock_url_list_arg_binds_the_fixtures_own_urls_instead_of_the_undefined_helper() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "batch_fetch", "description": "Fetch a batch",
            "input": {"urls": ["https://docs.example.com/a.pdf", "https://docs.example.com/b.pdf"]}
        }))
        .expect("fixture");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "fetch_batch".into();
        e2e_config
            .call
            .args
            .push(mock_url_arg("urls", "input.urls", "mock_url_list"));

        let body =
            render_snippet_body(&fixture, &e2e_config, &ResolvedCrateConfig::default(), &[], &[]).expect("snippet");

        assert!(
            body.contains("final urls = <String>["),
            "the list arg must still be bound: {body}"
        );
        assert!(
            body.contains("'https://docs.example.com/a.pdf'"),
            "the snippet must show the URLs the fixture declares: {body}"
        );
        assert!(
            !body.contains("_fixtureUrl"),
            "a standalone snippet must not call a helper only the test file defines: {body}"
        );
        assert!(
            !body.contains("urlsBase"),
            "the mock-server base local must not survive into a standalone snippet: {body}"
        );
    }
}
