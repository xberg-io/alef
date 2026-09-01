use anyhow::{Result, bail};

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, FunctionDef, TypeDef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

pub(super) fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> Result<String> {
    let docs_fixture = fixture.docs_call_fixture();
    let call = e2e_config.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, &docs_fixture);
    let default_factory = e2e_config
        .call
        .overrides
        .get("wasm")
        .and_then(|value| value.client_factory.as_deref());
    let effective_factory = call
        .overrides
        .get("wasm")
        .and_then(|value| value.client_factory.as_deref())
        .or(default_factory);
    let Some(function) = call.effective_function("wasm") else {
        bail!(
            "call routed for fixture `{}` has no function identity for WASM: neither the call's \
             base `function` nor `overrides.wasm.function` supplies one",
            docs_fixture.id
        );
    };
    if effective_factory.is_none() && !functions.is_empty() {
        match crate::backends::wasm::wasm_callability(function, functions, config) {
            crate::backends::wasm::WasmCallability::Callable => {}
            crate::backends::wasm::WasmCallability::NotExported => {
                bail!("WASM target does not export the configured `{function}` fixture function");
            }
            // Distinct from `NotExported`: the name resolves to nothing at all under either
            // spelling, so this is a config error, not a gap in the WASM target -- it must never
            // be retired with a `docs.coverage_exceptions` entry. ~keep
            crate::backends::wasm::WasmCallability::UnknownSymbol => {
                bail!(
                    "fixture `{}` routes WASM to `{function}`, but no API-surface function or trait-bridge \
                     registry entry answers to that name under either its Rust or its JavaScript spelling",
                    docs_fixture.id
                );
            }
        }
    }
    let module = e2e_config
        .resolve_package("wasm")
        .and_then(|package| package.name)
        .unwrap_or_else(|| config.wasm_package_name());
    let wasm_type_prefix = config.wasm_type_prefix();
    let body =
        super::super::typescript::test_file::render_snippet_body(super::super::typescript::test_file::SnippetContext {
            lang: "wasm",
            fixture,
            module: &module,
            client_factory: effective_factory,
            e2e_config,
            type_defs,
            enums,
            functions,
            wasm_type_prefix: &wasm_type_prefix,
            config,
        });
    // A `configuration/custom-base-url`-style topic documents `docs.client.base_url` so the
    // reader sees the setting the topic is about, mirroring the Java/Rust/Elixir/Python
    // generators' `docs_client` handling. `render_snippet_body` above is shared with
    // `typescript/mod.rs`'s own docs path and always emits the single-argument
    // `factory("your-api-key")` shape with no base-URL slot, so widening its signature would
    // reach TypeScript's docs snippets too. Substituting the exact rendered call here instead
    // (Python's model) keeps the change scoped to WASM's docs path alone. ~keep
    let body = match (
        effective_factory,
        fixture.docs_client().and_then(|client| client.base_url.as_deref()),
    ) {
        (Some(factory), Some(base_url)) => {
            let bare_call = format!("{factory}(\"your-api-key\")");
            let with_base_url = format!(
                "{factory}(\"your-api-key\", \"{}\")",
                crate::e2e::escape::escape_js(base_url)
            );
            body.replace(&bare_call, &with_base_url)
        }
        _ => body,
    };
    if let Some(unexported) = first_unexported_type_reference(&body, config, &wasm_type_prefix) {
        bail!(
            "fixture `{}` renders a WASM snippet naming `{unexported}`, which `[crates.wasm] exclude_types` \
             keeps out of the WASM binding. Drop the exclusion, or give the fixture a \
             `docs.coverage_exceptions` entry for wasm",
            docs_fixture.id
        );
    }
    Ok(body)
}

/// The first excluded type name the rendered snippet spells, if any.
///
/// The snippet's imports are derived from the crate IR (`wasm_prefixed_wrapped_type` prefixes any
/// name the IR declares), and the IR is not the WASM binding's export list: `exclude_types` keeps
/// a type out of the generated crate entirely, so a snippet that constructs it imports a symbol
/// the package does not have. This is the type-side twin of the [`WasmCallability::NotExported`]
/// refusal above, and it takes the same answer — a fixture whose example cannot be spelled against
/// this binding is a recorded coverage gap, never a published snippet that fails to compile.
///
/// Both spellings are checked because the prefix is applied only to names the IR declares: an
/// excluded type reached through a config override can appear unprefixed. ~keep
///
/// [`WasmCallability::NotExported`]: crate::backends::wasm::WasmCallability::NotExported
fn first_unexported_type_reference(body: &str, config: &ResolvedCrateConfig, type_prefix: &str) -> Option<String> {
    let excluded = config.wasm.as_ref().map(|wasm| &wasm.exclude_types)?;
    excluded
        .iter()
        .flat_map(|name| [format!("{type_prefix}{name}"), name.clone()])
        .find(|spelling| names_identifier(body, spelling))
}

/// Whether `body` uses `identifier` as a whole identifier rather than inside a longer one, so an
/// excluded `Config` does not match every `ConfigBuilder` the snippet mentions. ~keep
fn names_identifier(body: &str, identifier: &str) -> bool {
    let is_identifier_char = |character: char| character.is_alphanumeric() || character == '_' || character == '$';
    body.match_indices(identifier).any(|(start, matched)| {
        let before_is_identifier = body[..start].chars().next_back().is_some_and(is_identifier_char);
        let after_is_identifier = body[start + matched.len()..]
            .chars()
            .next()
            .is_some_and(is_identifier_char);
        !before_is_identifier && !after_is_identifier
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_follow_the_wasm_function_surface() {
        let mut fixture = Fixture {
            id: "download_assets".into(),
            description: "Download assets".into(),
            ..Fixture::default()
        };
        let functions = vec![
            FunctionDef {
                name: "download".into(),
                rust_path: "sample::download".into(),
                cfg: Some(r#"feature = "download""#.into()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "prefetch".into(),
                rust_path: "sample::prefetch".into(),
                cfg: Some(r#"not(feature = "download")"#.into()),
                ..FunctionDef::default()
            },
        ];
        let mut e2e = E2eConfig::default();
        e2e.call.function = "download".into();

        let unavailable = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("disabled function must not produce a WASM snippet");
        assert!(unavailable.to_string().contains("does not export"));

        e2e.calls.insert(
            "wrapped_download".into(),
            crate::e2e::config::CallConfig {
                function: "download".into(),
                overrides: std::iter::once((
                    "wasm".into(),
                    crate::core::config::e2e::CallOverride {
                        client_factory: Some("createClient".into()),
                        ..Default::default()
                    },
                ))
                .collect(),
                ..Default::default()
            },
        );
        fixture.call = Some("wrapped_download".into());
        let client_recipe = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect("client method need not be a direct module export");
        assert!(client_recipe.contains("import { createClient }"), "{client_recipe}");
        assert!(client_recipe.contains("client.download("), "{client_recipe}");
        assert!(!client_recipe.contains("import { download }"), "{client_recipe}");

        fixture.call = None;
        e2e.call.function = "prefetch".into();
        let available = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect("enabled function renders");
        assert!(available.contains("import { prefetch }"), "{available}");
    }

    /// Pins the `client_setup` construction in `typescript/test_file/snippet.rs` (~line
    /// 117-119), which WASM shares via `render_snippet_body`: a `client_factory` call must
    /// never reference the mock server or the fixture's `/fixtures/<id>` route. Unlike the
    /// Java/C#/Zig/Dart snippets, this path does not read the credential from the
    /// environment — it substitutes the established `"your-api-key"` documentation
    /// placeholder instead (see `go/snippet.rs`, `ruby/snippet_body.jinja`, and
    /// `php/snippet_body.jinja` for the same convention), which is not the literal mock
    /// credential either.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "wasm".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("createClient".into()),
                ..Default::default()
            },
        );

        let body = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[]).expect("snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("createClient(\"your-api-key\")"),
            "client is not constructed the way a reader would:\n{body}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its WASM
    /// snippet, mirroring the Java/Rust/Elixir/Python generators' `docs_client` handling
    /// (`python/mod.rs::client_factory_snippet_renders_the_base_url_the_fixture_documents`).
    #[test]
    fn client_factory_snippet_renders_the_base_url_the_fixture_documents() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "custom_base_url",
            "description": "Custom base URL",
            "input": null,
            "docs": {
                "topic": "configuration",
                "client": {"base_url": "https://llm.internal.example.com/v1"}
            }
        }))
        .expect("fixture must parse");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "wasm".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("createClient".into()),
                ..Default::default()
            },
        );

        let rendered = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[]).expect("snippet renders");

        assert!(
            rendered.contains("createClient(\"your-api-key\", \"https://llm.internal.example.com/v1\")"),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{rendered}"
        );
    }

    /// Negative control for `client_factory_snippet_renders_the_base_url_the_fixture_documents`:
    /// a fixture with no `docs.client` at all must keep rendering the bare, no-base-URL call.
    /// An indiscriminate "always add base_url" change would fail this test.
    #[test]
    fn client_factory_snippet_without_docs_client_keeps_the_bare_call() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "wasm".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("createClient".into()),
                ..Default::default()
            },
        );

        let rendered = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[]).expect("snippet renders");

        assert!(
            rendered.contains("createClient(\"your-api-key\")"),
            "no docs.client must keep the bare client construction call:\n{rendered}"
        );
        assert!(
            !rendered.contains("your-api-key\", \""),
            "no docs.client must not invent a base URL argument:\n{rendered}"
        );
    }

    #[test]
    fn trait_bridge_registry_calls_are_callable_from_wasm_snippets() {
        // A bridge's register/unregister/clear functions are absent from the plain function
        // surface by construction -- the trait-bridge generator emits them into
        // `__alef_wasm_bridge_*` instead. Gating snippets on the codegen predicate therefore
        // rejects every registry operation even though WASM exports all of them.
        let fixture = Fixture {
            id: "clear_validators".into(),
            description: "Clear all validators".into(),
            ..Fixture::default()
        };
        let unrelated = vec![FunctionDef {
            name: "extract".into(),
            rust_path: "sample::extract".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.call.function = "clear_validators".into();
        let config = ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "Validator".into(),
                clear_fn: Some("clear_validators".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        let rendered = render(&fixture, &e2e, &config, &[], &[], &unrelated)
            .expect("a bridge-managed registry call is exported by the WASM bridge module");
        assert!(rendered.contains("clearValidators"), "{rendered}");
    }

    #[test]
    fn a_call_without_any_function_identity_is_reported_as_such() {
        // An empty name must never reach the export check: it renders as an empty identifier and
        // reads as a capability gap when the real fault is a call with no identity configured.
        let fixture = Fixture {
            id: "clear_validators".into(),
            description: "Clear all validators".into(),
            ..Fixture::default()
        };
        let functions = vec![FunctionDef {
            name: "extract".into(),
            rust_path: "sample::extract".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.call.function = String::new();

        let error = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("a call with no function identity cannot render");
        let error = error.to_string();
        assert!(error.contains("has no function identity"), "{error}");
        assert!(!error.contains("does not export"), "{error}");
    }

    /// Build the call shape a crate uses when its bindings disagree on the spelling and there is
    /// no language-neutral name to put at the base: `function = ""` plus one override per
    /// language. `clear_reranker_backends` in xberg's `alef.toml` is exactly this.
    fn call_named_only_by_overrides(overrides: &[(&str, &str)]) -> crate::e2e::config::CallConfig {
        crate::e2e::config::CallConfig {
            function: String::new(),
            overrides: overrides
                .iter()
                .map(|(lang, function)| {
                    (
                        (*lang).to_string(),
                        crate::core::config::e2e::CallOverride {
                            function: Some((*function).to_string()),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    fn reranker_bridge_config() -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            trait_bridges: vec![crate::core::config::TraitBridgeConfig {
                trait_name: "RerankerBackend".into(),
                clear_fn: Some("clear_reranker_backends".into()),
                ..Default::default()
            }],
            ..ResolvedCrateConfig::default()
        }
    }

    #[test]
    fn a_wasm_override_supplies_the_function_identity_the_empty_base_lacks() {
        let mut fixture = Fixture {
            id: "reranker_backends_clear".into(),
            description: "Clear all reranker backends".into(),
            ..Fixture::default()
        };
        fixture.call = Some("clear_reranker_backends".into());
        let unrelated = vec![FunctionDef {
            name: "rerank".into(),
            rust_path: "sample::rerank".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_reranker_backends".into(),
            call_named_only_by_overrides(&[("wasm", "clearRerankerBackends")]),
        );

        let rendered = render(&fixture, &e2e, &reranker_bridge_config(), &[], &[], &unrelated)
            .expect("the wasm override names a function the wasm bridge module exports");
        assert!(rendered.contains("clearRerankerBackends()"), "{rendered}");
    }

    #[test]
    fn an_override_for_another_language_does_not_give_wasm_an_identity() {
        let mut fixture = Fixture {
            id: "reranker_backends_clear".into(),
            description: "Clear all reranker backends".into(),
            ..Fixture::default()
        };
        fixture.call = Some("clear_reranker_backends".into());
        let unrelated = vec![FunctionDef {
            name: "rerank".into(),
            rust_path: "sample::rerank".into(),
            ..FunctionDef::default()
        }];
        let mut e2e = E2eConfig::default();
        e2e.calls.insert(
            "clear_reranker_backends".into(),
            call_named_only_by_overrides(&[("python", "clear_reranker_backends")]),
        );

        let error = render(&fixture, &e2e, &reranker_bridge_config(), &[], &[], &unrelated)
            .expect_err("an override for python says nothing about what wasm exports")
            .to_string();
        assert!(error.contains("has no function identity"), "{error}");
        assert!(!error.contains("does not export"), "{error}");
    }

    #[test]
    fn a_wasm_override_naming_an_unexported_function_still_fails() {
        let mut fixture = Fixture {
            id: "download_assets".into(),
            description: "Download assets".into(),
            ..Fixture::default()
        };
        fixture.call = Some("download".into());
        let functions = vec![
            FunctionDef {
                name: "download".into(),
                rust_path: "sample::download".into(),
                cfg: Some(r#"feature = "download""#.into()),
                ..FunctionDef::default()
            },
            FunctionDef {
                name: "prefetch".into(),
                rust_path: "sample::prefetch".into(),
                cfg: Some(r#"not(feature = "download")"#.into()),
                ..FunctionDef::default()
            },
        ];
        let mut e2e = E2eConfig::default();
        e2e.calls
            .insert("download".into(), call_named_only_by_overrides(&[("wasm", "download")]));

        let gated = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("resolving the override must not excuse a function the target drops")
            .to_string();
        assert!(gated.contains("does not export"), "{gated}");
        assert!(gated.contains("`download`"), "{gated}");

        e2e.calls.insert(
            "download".into(),
            call_named_only_by_overrides(&[("wasm", "fetchAssets")]),
        );
        let absent = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &functions)
            .expect_err("a name nothing answers to under either spelling is not callable")
            .to_string();
        assert!(absent.contains("`fetchAssets`"), "{absent}");
        assert!(
            absent.contains("answers to that name"),
            "a name that resolves to nothing is a config error, not a WASM capability gap: {absent}"
        );
        assert!(!absent.contains("does not export"), "{absent}");
    }

    fn client_release_snippet(expects_error: bool) -> String {
        let mut fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        if expects_error {
            fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
        }
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.r#async = true;
        e2e.call.overrides.insert(
            "wasm".into(),
            crate::core::config::e2e::CallOverride {
                client_factory: Some("createClient".into()),
                ..Default::default()
            },
        );
        render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[]).expect("snippet renders")
    }

    /// wasm-bindgen gives every exported class a `free()` and JS never calls it on its own
    /// schedule, so a snippet that constructs a client and returns hands the reader an example
    /// that grows the wasm linear heap. `try`/`finally` rather than a trailing `client.free();`
    /// is the load-bearing part: an awaited call that rejects skips every statement after it. ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_in_a_finally_block() {
        let body = client_release_snippet(false);

        assert!(body.contains("  try {\n"), "the body must be scoped to a try:\n{body}");
        assert!(
            body.contains("  } finally {\n    client.free();\n  }"),
            "the client must be released from a finally block:\n{body}"
        );
        assert!(
            body.contains("    const result = await client.chat("),
            "the call moves one level in under the try:\n{body}"
        );
    }

    /// The error-path half of `client_factory_snippet_releases_the_client_in_a_finally_block`.
    /// The `expects_error` arm already carries a `try`/`catch`, and Kotlin's existing release is
    /// broken precisely because it sits inside the `try` while the `catch` releases nothing — so
    /// pin that the release is attached as a `finally` on that same statement, after the `catch`.
    /// ~keep
    #[test]
    fn client_factory_snippet_releases_the_client_on_the_error_path() {
        let body = client_release_snippet(true);

        let catch_block = body.find("} catch (error) {").expect("expects-error snippet catches");
        let release = body.find("} finally {").expect("finally clause");
        assert!(
            catch_block < release,
            "the release must be a finally on the try the failing call sits in:\n{body}"
        );
        assert!(
            body.contains("  } finally {\n    client.free();\n  }"),
            "the error-path snippet must release the client too:\n{body}"
        );
    }

    /// Negative control for the two tests above, and the pin that keeps this change scoped: a
    /// fixture with no `client_factory` constructs no client, so its snippet must be byte-for-byte
    /// what it was — no release, no `finally`, and the two-space body indentation the shared
    /// TypeScript template has always emitted. Re-indenting unconditionally would fail here, and
    /// so would emitting a `free()` on an identifier the snippet never declares. ~keep
    #[test]
    fn snippet_without_a_client_factory_is_unchanged() {
        let fixture = Fixture {
            id: "rate_limit_429".into(),
            description: "Rate limited".into(),
            input: serde_json::Value::Null,
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.r#async = true;

        let body = render(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[], &[]).expect("snippet renders");

        assert!(
            !body.contains(".free()"),
            "a snippet that constructs no client must emit no release:\n{body}"
        );
        assert!(
            !body.contains("finally"),
            "a snippet that constructs no client must not gain a release scope:\n{body}"
        );
        assert!(
            body.contains("  const result = await chat("),
            "the body must keep its two-space indentation:\n{body}"
        );
    }
}
