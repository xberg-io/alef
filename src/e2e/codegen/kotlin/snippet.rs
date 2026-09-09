use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureEnv};

use super::args::{KotlinArgsContext, build_args_and_setup};

/// `build_args_and_setup` is shared with the e2e test emitter, whose generated test
/// class declares a companion-object `MAPPER` constant that its output references.
/// A docs snippet is a standalone `fun main()` with no such companion object, and no
/// binding exposes a public `MAPPER`, so every reference has to be rebound onto the
/// local `mapper` the snippet template declares for itself. This applies to the
/// argument list as much as to the setup lines: a typed-array argument is emitted
/// inline as `listOf(MAPPER.readValue(...), ...)` and never reaches `setup_lines`.
const TEST_CLASS_MAPPER_REFERENCE: &str = "MAPPER.";
const SNIPPET_MAPPER_REFERENCE: &str = "mapper.";

fn rebind_mapper_references(source: &str) -> String {
    source.replace(TEST_CLASS_MAPPER_REFERENCE, SNIPPET_MAPPER_REFERENCE)
}

pub(crate) fn render_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    kotlin_android_style: bool,
) -> anyhow::Result<String> {
    render_snippet_body_with_ir(fixture, e2e_config, config, type_defs, enums, kotlin_android_style, &[])
}

/// [`render_snippet_body`], with the free-function registry it cannot see.
///
/// `functions` lets the presentation resolver anchor the snippet's field facts at the call's
/// own declared result type instead of matching field names across the whole crate IR; without
/// it the resolver falls back to the flat, name-keyed answers. Mirrors `java/snippet.rs`'s
/// split for the same reason. ~keep
pub(crate) fn render_snippet_body_with_ir(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    kotlin_android_style: bool,
    functions: &[crate::core::ir::FunctionDef],
) -> anyhow::Result<String> {
    let lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call, type_defs)
        .with_functions(functions);
    let overrides = recipe.override_config;
    let class_name = overrides
        .and_then(|value| value.class.as_deref())
        .unwrap_or(&config.name)
        .rsplit('.')
        .next()
        .unwrap_or(&config.name)
        .to_upper_camel_case();
    let function_name = overrides
        .and_then(|value| value.function.as_deref())
        .unwrap_or(&call.function)
        .to_lower_camel_case();
    let options_type = recipe
        .options_type
        .or_else(|| recipe.compatible_options_type(&["kotlin", "kotlin_android", "java", "csharp"]));

    // Streaming owner_type adapters are facade-exposed as INSTANCE methods on
    // the owner handle (`engine.streamItems(req)`), not as static facade
    // methods — mirrors the test-generation path in test_method.rs and the
    // Java e2e backend. The docs snippet must show the same real call shape
    // callers will actually write, so it is wrong here for the same reason it
    // was wrong in the generated tests.
    let adapter_lookup_name = call.core_lookup_name(lang);
    let adapter = adapter_lookup_name
        .as_deref()
        .and_then(|name| config.adapters.iter().find(|a| a.name == name));
    let is_streaming_owner_adapter = adapter.is_some_and(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming) && a.owner_type.is_some()
    });
    let streaming_owner_handle: Option<String> = if is_streaming_owner_adapter {
        recipe
            .args
            .iter()
            .find(|a| a.arg_type == "handle")
            .map(|a| a.name.clone())
    } else {
        None
    };
    // The adapter-declared request parameter (`[[crates.adapters.params]]`), if any.
    // Mirrors `test_method.rs::streaming_request`: for an owner_type streaming
    // adapter the facade signature takes exactly this declared param, built from
    // the whole fixture input, not from any per-arg ArgMapping the fixture's
    // `call.args` might otherwise declare (those describe the flat-call shape a
    // non-owner_type adapter would use). ~keep
    let streaming_request = adapter.and_then(|a| {
        matches!(a.pattern, crate::core::config::extras::AdapterPattern::Streaming)
            .then(|| a.params.first())
            .flatten()
    });
    // When the request is built from the adapter param below, drop every non-handle
    // ArgMapping before building setup/args — otherwise `build_args_and_setup` would
    // bind and pass those fields a second time, and (per the fallback path) the
    // resulting call would reference the un-rebuilt request as a raw string/JSON
    // fragment instead of the required typed request object. ~keep
    let call_args: std::borrow::Cow<'_, [crate::e2e::config::ArgMapping]> =
        if streaming_owner_handle.is_some() && streaming_request.is_some() {
            std::borrow::Cow::Owned(recipe.args.iter().filter(|a| a.arg_type == "handle").cloned().collect())
        } else {
            std::borrow::Cow::Borrowed(recipe.args)
        };

    let (setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        &call_args,
        KotlinArgsContext {
            fixture,
            class_name: &class_name,
            options_type,
            fixture_id: &fixture.id,
            kotlin_android_style,
            config,
            type_defs,
            enums,
            owner_handle_is_receiver: streaming_owner_handle.is_some(),
            target_params: recipe.target_params(lang),
        },
    )?;
    let mut setup_lines = setup_lines
        .into_iter()
        .map(|line| rebind_mapper_references(&line))
        .collect::<Vec<_>>();
    // Build and bind the declared request object from the fixture input (minus the
    // handle's own fields) — the call target is the owner handle instance, and its
    // method takes this single typed parameter. Without this, `args` from
    // `build_args_and_setup` above is empty (the handle is the receiver, not a
    // positional argument), so the rendered call would be missing its required
    // argument entirely. Mirrors `test_method.rs`'s identical construction for the
    // generated JUnit test. ~keep
    if streaming_owner_handle.is_some()
        && let Some(request) = streaming_request
    {
        let request_name = request.name.to_lower_camel_case();
        let request_type = request.ty.rsplit("::").next().unwrap_or(&request.ty);
        let mut request_input = fixture.input.clone();
        if let Some(object) = request_input.as_object_mut() {
            for handle in recipe.args.iter().filter(|arg| arg.arg_type == "handle") {
                let field = handle.field.strip_prefix("input.").unwrap_or(&handle.field);
                object.remove(field);
            }
        }
        let normalized = crate::e2e::codegen::transform_json_keys_for_language(&request_input, "snake_case");
        let request_json = serde_json::to_string(&normalized).unwrap_or_default();
        // A full literal expression -- quoted, or `+`-chunked when `request_json` is long
        // enough to threaten the JVM's 65535-byte constant cap -- not just escaped content.
        // See `values::kotlin_string_literal`.
        let literal = super::values::kotlin_string_literal(&request_json);
        if crate::e2e::codegen::value_contains_mock_url_placeholder(&normalized) {
            let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
            setup_lines.push(format!(
                "val {request_name}Json = {literal}.replace(\"{}\", System.getProperty(\"mockServer.{}\", System.getenv(\"{env_key}\") ?: \"\"))",
                crate::e2e::escape::escape_kotlin(crate::e2e::codegen::MOCK_URL_PLACEHOLDER),
                fixture.id,
            ));
            setup_lines.push(format!(
                "val {request_name} = mapper.readValue({request_name}Json, {request_type}::class.java)"
            ));
        } else {
            setup_lines.push(format!(
                "val {request_name} = mapper.readValue({literal}, {request_type}::class.java)"
            ));
        }
        args = request_name;
    }
    if let Some(visitor) = &fixture.visitor
        && let Some(visitor_args) =
            super::visitor::attach_visitor(&mut setup_lines, &args, visitor, config, type_defs, enums)
    {
        args = visitor_args;
    }
    if !recipe.extra_args.is_empty() {
        args = if args.is_empty() {
            recipe.extra_args.join(", ")
        } else {
            format!("{args}, {}", recipe.extra_args.join(", "))
        };
    }
    args = rebind_mapper_references(&args);
    let client_factory = overrides
        .and_then(|value| value.client_factory.as_deref())
        .or_else(|| e2e_config.call.overrides.get(lang)?.client_factory.as_deref())
        .or_else(|| {
            if !kotlin_android_style {
                return None;
            }
            // ~keep Android shares Java's JNI factory contract, matching the JUnit emitter.
            call.overrides
                .get("java")
                .and_then(|value| value.client_factory.as_deref())
                .or_else(|| e2e_config.call.overrides.get("java")?.client_factory.as_deref())
        });
    let needs_mapper = args.contains(SNIPPET_MAPPER_REFERENCE)
        || setup_lines.iter().any(|line| line.contains(SNIPPET_MAPPER_REFERENCE));
    let is_async = client_factory.is_some() || call.r#async;
    let package_name = if kotlin_android_style {
        config
            .kotlin_android
            .as_ref()
            .and_then(|value| value.package.clone())
            .unwrap_or_else(|| config.kotlin_package())
    } else {
        config.kotlin_package()
    };
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    let api_key_var = FixtureEnv::api_key_var_or_default(fixture.env.as_ref());
    let base_url = crate::e2e::codegen::client_factory::docs_base_url(fixture.docs_client())
        .map(crate::e2e::escape::escape_kotlin);

    // The template renders the call as `{{ class_name }}.{{ function_name }}(...)`
    // (or, with `client_factory`, constructs a client via
    // `{{ class_name }}.{{ client_factory }}(...)` first and then calls
    // `client.{{ function_name }}(...)`). For a flat-call streaming owner_type
    // adapter, substitute the owner handle's variable name for `class_name` so
    // the call renders as the real instance-method invocation. `client_factory`
    // fixtures already dispatch on `client`, not `class_name`, so this
    // substitution is scoped to the flat-call path where it actually changes
    // the rendered call target.
    let call_target_class_name = if client_factory.is_none() {
        streaming_owner_handle.unwrap_or(class_name)
    } else {
        class_name
    };

    let presentation =
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang, type_defs, enums, functions);
    let result_var = call.effective_result_var();
    Ok(crate::e2e::template_env::render(
        "kotlin/snippet_body.jinja",
        minijinja::context! {
            package_name => package_name,
            needs_mapper => needs_mapper,
            setup_lines => setup_lines,
            client_factory => client_factory.map(ToLowerCamelCase::to_lower_camel_case),
            class_name => call_target_class_name,
            function_name => function_name,
            args => args,
            result_var => result_var,
            returns_void => call.returns_void,
            is_async => is_async,
            fixture_id => fixture.id,
            expects_error => expects_error,
            api_key_var => api_key_var,
            presentation => presentation,
            base_url => base_url,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::{CallConfig, CallOverride};

    mod test_support;

    use test_support::{fixture, line_containing};

    const BYTES_ELEMENT: &str = r#"mapper.readValue("{\"kind\":\"bytes\"}", ExtractInput::class.java)"#;
    const URI_ELEMENT: &str = r#"mapper.readValue("{\"kind\":\"uri\"}", ExtractInput::class.java)"#;

    mod android_tests;
    mod client_factory_tests;
    mod optional_dto_arg_tests;
    mod streaming_tests;
    mod typed_dto_tests;

    /// A batch call whose typed-array argument is materialised inline in the call
    /// expression rather than in a setup line.
    fn batch_call() -> CallConfig {
        CallConfig {
            function: "extract_batch".into(),
            result_var: "result".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "inputs".into(),
                field: "inputs".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: Some("ExtractInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        }
    }

    fn batch_fixture() -> Fixture {
        Fixture {
            id: "extract_batch_bytes_happy".into(),
            description: "Extract several documents in one batch".into(),
            input: serde_json::json!({"inputs": [{"kind": "bytes"}, {"kind": "uri"}]}),
            ..Fixture::default()
        }
    }

    fn batch_snippet(kotlin_android_style: bool) -> String {
        let config = ResolvedCrateConfig {
            name: "xberg".into(),
            ..ResolvedCrateConfig::default()
        };
        render_snippet_body(
            &batch_fixture(),
            &E2eConfig {
                call: batch_call(),
                ..E2eConfig::default()
            },
            &config,
            &[],
            &[],
            kotlin_android_style,
        )
        .expect("snippet renders")
    }

    #[test]
    fn android_batch_snippet_binds_list_elements_to_the_locally_declared_mapper() {
        let body = batch_snippet(true);

        assert!(
            !body.contains("MAPPER"),
            "the snippet must not reference the test class's private MAPPER, got:\n{body}"
        );
        assert!(
            body.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
            "{body}"
        );
        assert!(body.contains("    val mapper = jacksonObjectMapper()"), "{body}");
        assert_eq!(
            line_containing(&body, "extractBatch"),
            format!("val result = Xberg.extractBatch(listOf({BYTES_ELEMENT}, {URI_ELEMENT}))")
        );
    }

    #[test]
    fn jvm_batch_snippet_binds_list_elements_to_the_locally_declared_mapper() {
        let body = batch_snippet(false);

        assert!(
            !body.contains("MAPPER"),
            "the snippet must not reference the test class's private MAPPER, got:\n{body}"
        );
        assert!(
            body.contains("import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper"),
            "{body}"
        );
        assert!(body.contains("    val mapper = jacksonObjectMapper()"), "{body}");
        assert_eq!(
            line_containing(&body, "extractBatch"),
            format!("val result = Xberg.extractBatch(listOf({BYTES_ELEMENT}, {URI_ELEMENT}))")
        );
    }

    #[test]
    fn documented_presentation_binds_the_result_and_reads_the_shown_fields() {
        let documented: Fixture = serde_json::from_value(serde_json::json!({
            "id": "present_items", "description": "Present returned items", "input": null,
            "docs": {"topic": "guides", "presentation": {"operations": [
                {"op": "show", "path": "summary", "display": true},
                {"op": "iterate", "path": "items", "item": "item", "fields": ["label"]}
            ]}}
        }))
        .expect("fixture");
        let e2e = E2eConfig {
            call: CallConfig {
                function: "process".into(),
                result_var: "result".into(),
                ..CallConfig::default()
            },
            result_fields: ["summary".to_string(), "items".to_string()].into_iter().collect(),
            ..E2eConfig::default()
        };
        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let body = render_snippet_body(&documented, &e2e, &config, &[], &[], false).expect("snippet renders");

        assert!(body.contains("val result = Sample.process()"), "{body}");
        assert!(body.contains("println(result.summary())"), "{body}");
        assert!(body.contains("for (item in result.items()) {"), "{body}");
        assert!(body.contains("println(item.label())"), "{body}");
        assert!(
            !body.contains("println(result)"),
            "the whole-result fallback must give way to the documented presentation:\n{body}"
        );
    }

    #[test]
    fn snippet_keeps_the_native_call_without_the_test_harness() {
        let mut call = CallConfig {
            function: "load_document".into(),
            result_var: "document".into(),
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                class: Some("DocumentApi".into()),
                ..CallOverride::default()
            },
        );
        let body = render_snippet_body(
            &fixture(),
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(body.contains("DocumentApi.loadDocument()"));
        assert!(body.contains("println(document)"), "{body}");
        assert!(body.contains("fun main()"));
        assert!(!body.contains("@Test"));
        assert!(!body.contains("assert"));
    }

    #[test]
    fn snippet_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "process".into();
        let body = render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], false)
            .expect("snippet renders");

        assert!(body.contains("catch (error: Exception)"), "{body}");
        assert!(body.contains("error::class.simpleName"), "{body}");
        assert!(!body.contains("AssertionError"), "{body}");
    }

    /// Regression twin of the Java fix (alef task #180): Kotlin compiles to the same JVM
    /// bytecode as Java and inherits the identical 65535-byte `CONSTANT_Utf8` cap on a single
    /// string literal. A fixture whose `json_object` field carries a value long enough to
    /// threaten that cap must never render as a single Kotlin string literal above the limit.
    /// Neutral synthetic payload (`project-agnostic-codegen`): not any real consumer's fixture.
    /// ~keep
    #[test]
    fn a_doc_snippet_never_emits_a_single_kotlin_literal_over_the_jvm_constant_cap() {
        let oversized_payload = "abcdefghij".repeat(10_000); // 100,000 bytes
        let fixture = Fixture {
            id: "large_payload".into(),
            description: "Process a large payload".into(),
            input: serde_json::json!({"content": oversized_payload}),
            ..Fixture::default()
        };
        let mut call = CallConfig {
            function: "process".into(),
            args: vec![crate::e2e::config::ArgMapping {
                name: "options".into(),
                field: "content".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        };
        call.overrides.insert(
            "kotlin".into(),
            CallOverride {
                options_type: Some("PayloadOptions".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(
            &fixture,
            &E2eConfig {
                call,
                ..E2eConfig::default()
            },
            &ResolvedCrateConfig::default(),
            &[],
            &[],
            false,
        )
        .expect("snippet renders");

        assert!(
            body.contains(&oversized_payload[..100]),
            "the snippet must still contain the fixture content, just not as one literal:\n{}",
            &body[..body.len().min(500)]
        );
        for segment in body.split('"') {
            assert!(
                segment.len() <= 65_535,
                "a generated Kotlin doc snippet must never contain a single quoted-literal \
                 segment above the JVM's 65535-byte CONSTANT_Utf8 cap: got {} bytes",
                segment.len()
            );
        }
    }
}
