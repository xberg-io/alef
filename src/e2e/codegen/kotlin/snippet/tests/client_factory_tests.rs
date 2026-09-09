//! Docs-snippet rendering for fixtures that declare a `client_factory`.

use super::*;
use crate::e2e::config::{CallConfig, CallOverride};

/// Pins that a `client_factory` docs snippet reads the credential from the
/// environment and never points the reader at the e2e mock server: no
/// `MOCK_SERVER` env var, no `mockServer` system property (both used by the
/// generated JUnit test's mock-mode branch in `test_method.rs`), no
/// `/fixtures/<id>` route, and no inlined `"test-key"` credential.
#[test]
fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
    let fixture = Fixture {
        id: "rate_limit_429".into(),
        description: "Rate limited".into(),
        input: serde_json::Value::Null,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "chat".into(),
        result_var: "result".into(),
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin".into(),
        CallOverride {
            client_factory: Some("create_client".into()),
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

    assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
    assert!(!body.contains("mockServer"), "mock-server property leaked:\n{body}");
    assert!(
        !body.contains("/fixtures/rate_limit_429"),
        "mock-server fixture route leaked:\n{body}"
    );
    assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
    assert!(
        body.contains("System.getenv(\"API_KEY\")"),
        "credential is not read from the environment:\n{body}"
    );
    assert!(
        body.contains("createClient(apiKey = apiKey)"),
        "an unconfigured project must construct the client without a mock base URL:\n{body}"
    );
}

fn client_release_snippet(expects_error: bool) -> String {
    let mut fixture = Fixture {
        id: "rate_limit_429".into(),
        description: "Rated limited".into(),
        input: serde_json::Value::Null,
        ..Fixture::default()
    };
    if expects_error {
        fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
    }
    let mut call = CallConfig {
        function: "chat".into(),
        result_var: "result".into(),
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin".into(),
        CallOverride {
            client_factory: Some("create_client".into()),
            ..CallOverride::default()
        },
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
        false,
    )
    .expect("snippet renders")
}

/// The generated Kotlin client wrapper implements `AutoCloseable` and delegates `close()` to
/// the underlying Java facade (`client_class_header.jinja` / `client_close_method.jinja` in
/// the Kotlin backend), so `kotlin.use` is the correct release for a client a docs snippet
/// constructs — it is the stdlib idiom for exactly this type, and it closes on both normal
/// return and a thrown exception, unlike a bare trailing `client.close()`. ~keep
#[test]
fn client_factory_snippet_releases_the_client_in_a_use_block() {
    let body = client_release_snippet(false);

    assert!(
        body.contains("Sample.createClient(apiKey = apiKey).use { client -> client.chat() }"),
        "the client must be released via a `use` block around the call:\n{body}"
    );
    assert!(
        !body.contains("client.close()"),
        "no bare close() call must remain:\n{body}"
    );
}

/// The error-path half of `client_factory_snippet_releases_the_client_in_a_use_block`.
/// `kotlin.use` is implemented as try/finally internally, so it releases the client
/// whether `client.chat(...)` returns or throws — unlike the pre-existing template, whose
/// `client.close()` sat on its own line after the call and was skipped whenever the call
/// itself threw, leaking the client on every failed request. ~keep
#[test]
fn client_factory_snippet_releases_the_client_on_the_error_path() {
    let body = client_release_snippet(true);

    let try_open = body
        .find("    try {")
        .expect("expects-error snippet still opens a try block");
    let use_block = body
        .find("Sample.createClient(apiKey = apiKey).use { client -> client.chat() }")
        .expect("client construction moves inside the try, unchanged in shape");
    let catch_clause = body.find("catch (error: Exception)").expect("catch clause present");
    assert!(
        try_open < use_block && use_block < catch_clause,
        "the use block must sit inside the try that the catch closes:\n{body}"
    );
    assert!(
        !body.contains("client.close()"),
        "no bare close() call must remain:\n{body}"
    );
}

/// Negative control for the two tests above, and the pin that keeps this change scoped: a
/// fixture with no `client_factory` constructs no client, so its snippet must be byte-for-byte
/// what it was — no `.use {`, no `close()`, just the plain call. A change that wraps every
/// snippet's call in a `use` block unconditionally would fail here.
#[test]
fn snippet_without_a_client_factory_is_unchanged() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "invalid_input", "description": "Reject invalid input", "input": null,
        "assertions": [{"type": "error"}]
    }))
    .expect("fixture");
    let mut e2e = E2eConfig::default();
    e2e.call.function = "process".into();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(&fixture, &e2e, &config, &[], &[], false).expect("snippet renders");

    assert!(
        !body.contains(".use {"),
        "a snippet that constructs no client must emit no use block:\n{body}"
    );
    assert!(
        !body.contains("close()"),
        "a snippet that constructs no client must emit no close call:\n{body}"
    );
    assert!(
        body.contains("    val result = Sample.process()"),
        "the plain call must be unchanged:\n{body}"
    );
}

/// Companion of `client_factory_snippet_never_points_the_reader_at_the_mock_server`:
/// a fixture whose `docs.client.base_url` names an endpoint must show that endpoint
/// as a named `baseUrl` argument on the client-construction call.
#[test]
fn a_snippet_renders_the_base_url_the_fixture_documents() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "custom_base_url",
        "description": "Custom base URL",
        "input": null,
        "docs": {
            "topic": "configuration",
            "client": {"base_url": "https://llm.internal.example.com/v1"}
        }
    }))
    .expect("fixture");
    let mut call = CallConfig {
        function: "chat".into(),
        result_var: "result".into(),
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin".into(),
        CallOverride {
            client_factory: Some("create_client".into()),
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
        body.contains("createClient(apiKey = apiKey, baseUrl = \"https://llm.internal.example.com/v1\")"),
        "the snippet for a custom-base-url topic must show the custom base URL:\n{body}"
    );
}

/// Negative control for the base-URL wiring above: a fixture with no `docs.client`
/// must keep rendering the bare call, unchanged by the new optional argument.
#[test]
fn a_fixture_without_a_docs_client_keeps_the_bare_client_construction_call() {
    let fixture = Fixture {
        id: "rate_limit_429".into(),
        description: "Rate limited".into(),
        input: serde_json::Value::Null,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "chat".into(),
        result_var: "result".into(),
        ..CallConfig::default()
    };
    call.overrides.insert(
        "kotlin".into(),
        CallOverride {
            client_factory: Some("create_client".into()),
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
        body.contains("createClient(apiKey = apiKey)"),
        "an unconfigured project must construct the client without a base URL:\n{body}"
    );
    assert!(
        !body.contains("baseUrl"),
        "no docs client must mean no baseUrl argument:\n{body}"
    );
}

fn android_factory_body(global: &[(&str, &str)], named: Option<&[(&str, &str)]>, android: bool) -> String {
    let make_call = |factories: &[(&str, &str)]| {
        let mut call = CallConfig {
            function: "chat".into(),
            ..CallConfig::default()
        };
        for (language, factory) in factories {
            call.overrides.insert(
                (*language).into(),
                CallOverride {
                    client_factory: Some((*factory).into()),
                    ..CallOverride::default()
                },
            );
        }
        call
    };
    let mut e2e = E2eConfig {
        call: make_call(global),
        ..E2eConfig::default()
    };
    let mut fixture = Fixture {
        id: "factory_precedence".into(),
        ..Fixture::default()
    };
    if let Some(factories) = named {
        e2e.calls.insert("named".into(), make_call(factories));
        fixture.call = Some("named".into());
    }
    render_snippet_body(
        &fixture,
        &e2e,
        &ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        },
        &[],
        &[],
        android,
    )
    .expect("Android snippet renders")
}

#[test]
fn android_snippet_inherits_java_factory_for_default_and_named_calls() {
    for named in [None, Some(&[][..]), Some(&[("java", "named_factory")][..])] {
        let body = android_factory_body(&[("java", "global_factory")], named, true);
        let factory = if named.is_some_and(|entries| !entries.is_empty()) {
            "namedFactory"
        } else {
            "globalFactory"
        };
        assert!(
            body.contains(&format!(
                "Sample.{factory}(apiKey = apiKey).use {{ client -> client.chat() }}"
            )),
            "{body}"
        );
        assert!(
            !body.contains("Sample.chat("),
            "instance method emitted as static: {body}"
        );
    }
}

#[test]
fn android_snippet_preserves_android_factory_precedence_over_java() {
    let globals = [("kotlin_android", "global_android"), ("java", "global_java")];
    for (named, expected) in [
        (
            &[("kotlin_android", "named_android"), ("java", "named_java")][..],
            "namedAndroid",
        ),
        (&[("java", "named_java")][..], "globalAndroid"),
        (&[][..], "globalAndroid"),
    ] {
        let body = android_factory_body(&globals, Some(named), true);
        assert!(
            body.contains(&format!(
                "Sample.{expected}(apiKey = apiKey).use {{ client -> client.chat() }}"
            )),
            "{body}"
        );
    }
}

#[test]
fn android_factory_fallback_keeps_plain_and_non_android_calls_unchanged() {
    for body in [
        android_factory_body(&[], Some(&[]), true),
        android_factory_body(&[("java", "java_factory")], None, false),
    ] {
        assert!(body.contains("Sample.chat()"), "{body}");
        assert!(!body.contains(".use {"), "{body}");
        assert!(!body.contains("System.getenv"), "{body}");
    }
}
