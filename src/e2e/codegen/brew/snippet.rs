//! Documentation-snippet body for the Homebrew CLI (shell) target.
//!
//! A published brew snippet is a single, reader-executable `binary subcommand "<url>"
//! --flags` invocation, built from the same call config and argument-binding logic
//! (`super::category::build_cli_command`, `super::category::determine_subcommand`) the
//! executable brew e2e suite already uses for its own test invocations -- see
//! `category::render_test_function` for the sibling that builds the same shape for a real
//! test. The two deliberately share that logic rather than re-deriving it: a shell
//! invocation the executable suite runs and a shell invocation the docs site publishes must
//! never drift apart on how a fixture's arguments become CLI flags.
//!
//! By the time this renders, `crate::e2e::snippets::render_body::render_snippet_body` has
//! already resolved every `mock_url` / `mock_url_list` argument to a public literal (see
//! `crate::e2e::snippets::mock_url_defaults`) and set `fixture.preserve_input_urls`, so
//! `build_cli_command`'s `mock_url` branch binds that literal rather than falling back to
//! `MOCK_SERVER_URL` -- the fallback the mock-harness guard
//! (`crate::e2e::snippets::mock_harness_guard`) exists to catch if it were ever reached here.

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use anyhow::{Result, bail};

use super::category::{build_cli_command, determine_subcommand};

/// The e2e language key this recipe resolves call config and package overrides under.
const LANG: &str = "brew";

pub(super) fn render_snippet_body(fixture: &Fixture, e2e_config: &E2eConfig) -> Result<String> {
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    if let Some(reason) = call_config.unsupported_in.get(LANG) {
        bail!(
            "brew documentation snippet for fixture `{}` is unsupported: {reason}",
            fixture.id
        );
    }
    // Mirrors `BrewCodegen::generate`/`category::render_test_function` exactly: the default
    // subcommand and the CLI flag/static-arg wiring come from the *top-level* `e2e.call`
    // override, the same source `generate()` resolves once for the whole suite -- while the
    // subcommand itself still lets a fixture-resolved call (e.g. a named `e2e.calls.<x>`
    // block the fixture selects) override it outright. Splitting these the other way would
    // make a documentation snippet disagree with the invocation the executable e2e suite
    // actually runs for the same fixture. ~keep
    let top_level_overrides = e2e_config.call.overrides.get(LANG);
    let default_subcommand = top_level_overrides
        .and_then(|value| value.function.as_ref())
        .cloned()
        .unwrap_or_else(|| e2e_config.call.function.clone());
    let fixture_overrides = call_config.overrides.get(LANG);
    let subcommand = match fixture_overrides.and_then(|value| value.function.as_ref()) {
        Some(function) => function.clone(),
        None => determine_subcommand(&fixture.tags, &default_subcommand),
    };
    let binary_name = resolve_binary_name(e2e_config, &call_config.module)?;
    let static_cli_args: Vec<String> = top_level_overrides
        .map(|value| value.cli_args.clone())
        .unwrap_or_default();
    let cli_flags: std::collections::HashMap<String, String> = top_level_overrides
        .map(|value| value.cli_flags.clone())
        .unwrap_or_default();
    let command = build_cli_command(
        fixture,
        &binary_name,
        &subcommand,
        &static_cli_args,
        &cli_flags,
        fixture.resolved_args(call_config),
    )
    .join(" ");
    Ok(crate::e2e::template_env::render(
        "brew/snippet_body.jinja",
        crate::alef_context! { command => command },
    ))
}

/// Mirrors `BrewCodegen::generate`'s own binary-name resolution: the `brew` registry package
/// entry, then the `brew` package override, then falling back to the call's module -- so a
/// documentation snippet always names the same binary the generated e2e test suite invokes.
fn resolve_binary_name(e2e_config: &E2eConfig, call_module: &str) -> Result<String> {
    let name = e2e_config
        .registry
        .packages
        .get(LANG)
        .and_then(|package| package.name.as_ref())
        .cloned()
        .or_else(|| {
            e2e_config
                .packages
                .get(LANG)
                .and_then(|package| package.name.as_ref())
                .cloned()
        })
        .unwrap_or_else(|| call_module.to_string());
    if name.trim().is_empty() {
        bail!(
            "brew documentation snippet has no configured binary name; set \
             `[crates.e2e.registry.packages.brew].name` or `[crates.e2e.call].module`"
        );
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{CallOverride, PackageRef};

    fn fixture() -> Fixture {
        Fixture {
            id: "quick_start".into(),
            description: "Quick start".into(),
            input: serde_json::json!({}),
            preserve_input_urls: true,
            ..Fixture::default()
        }
    }

    fn e2e_with_call(function: &str, module: &str) -> E2eConfig {
        let mut e2e = E2eConfig::default();
        e2e.call.function = function.into();
        e2e.call.module = module.into();
        e2e.call.args = vec![crate::core::config::e2e::ArgMapping {
            name: "url".into(),
            field: "url".into(),
            arg_type: "mock_url".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        e2e
    }

    #[test]
    fn renders_a_single_line_reader_executable_invocation() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"url": "https://example.com/sample.html"});
        let e2e = e2e_with_call("scrape", "mytool");

        let body = render_snippet_body(&fixture, &e2e).expect("brew snippet renders");

        assert_eq!(body, "mytool scrape 'https://example.com/sample.html'\n");
    }

    #[test]
    fn a_brew_specific_function_override_becomes_the_subcommand() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"url": "https://example.com/sample.html"});
        let mut e2e = e2e_with_call("scrape", "mytool");
        e2e.call.overrides.insert(
            "brew".into(),
            CallOverride {
                function: Some("crawl".into()),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(&fixture, &e2e).expect("brew snippet renders");

        assert!(body.starts_with("mytool crawl "), "{body}");
    }

    /// Distinguishes the fixture-resolved call's own `overrides.brew.function` from the
    /// *top-level* `e2e.call`'s -- the previous test above cannot: it inserts its override
    /// into `e2e.call.overrides` directly, and an undecorated fixture resolves to
    /// `&e2e.call` itself, so `call_config.overrides` and `e2e_config.call.overrides` were
    /// literally the same map and that test would keep passing even if the fixture-resolved
    /// override were never consulted at all. Here the fixture selects a NAMED call
    /// (`e2e.calls.special`, the shape a `select_when`-routed or `fixture.call = "special"`
    /// fixture actually takes) whose own brew override differs from -- and must win over --
    /// the top level, which declares no brew override of its own.
    #[test]
    fn a_named_calls_brew_override_wins_over_the_top_level_default_subcommand() {
        use crate::core::config::e2e::CallConfig;

        let mut fixture = fixture();
        fixture.call = Some("special".into());
        fixture.input = serde_json::json!({"url": "https://example.com/sample.html"});
        let mut e2e = e2e_with_call("scrape", "mytool");
        let mut special = CallConfig {
            function: "special_scrape".into(),
            module: "mytool".into(),
            args: e2e.call.args.clone(),
            ..CallConfig::default()
        };
        special.overrides.insert(
            "brew".into(),
            CallOverride {
                function: Some("crawl".into()),
                ..CallOverride::default()
            },
        );
        e2e.calls.insert("special".into(), special);

        let body = render_snippet_body(&fixture, &e2e).expect("brew snippet renders");

        assert!(
            body.starts_with("mytool crawl "),
            "the named call's own brew override must win over the top-level default: {body}"
        );
    }

    #[test]
    fn a_registered_package_name_wins_over_the_call_module() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"url": "https://example.com/sample.html"});
        let mut e2e = e2e_with_call("scrape", "mytool");
        e2e.registry.packages.insert(
            "brew".into(),
            PackageRef {
                name: Some("published-cli".into()),
                ..PackageRef::default()
            },
        );

        let body = render_snippet_body(&fixture, &e2e).expect("brew snippet renders");

        assert!(body.starts_with("published-cli scrape "), "{body}");
    }

    #[test]
    fn a_published_snippet_never_leaks_mock_harness_scaffolding() {
        // Simulates what `mock_url_defaults::with_default_mock_url_literals` actually does
        // for the zero-edit case: it injects a public literal into `input.url` AND sets
        // `preserve_input_urls`, together -- `render_body::render_snippet_body` always
        // performs both before calling this recipe. Setting only the flag with `url` left
        // absent (as an earlier version of this test did) does not exercise that path:
        // `build_cli_command`'s `mock_url` branch reads `input.url` and falls back to
        // `MOCK_SERVER_*` regardless of the flag once the field itself is unset -- and that
        // fallback IS what `reject_mock_harness_scaffolding` exists to catch, which is what
        // that earlier version of this test actually observed.
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"url": "https://example.com"});
        let e2e = e2e_with_call("scrape", "mytool");

        let body = render_snippet_body(&fixture, &e2e).expect("brew snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "{body}");
        crate::e2e::snippets::mock_harness_guard::reject_mock_harness_scaffolding(&body, &fixture, "brew")
            .expect("a snippet built on an already-resolved literal must pass the guard");
    }

    /// The safety net's positive control: when `input.url` is genuinely unresolved (no
    /// upstream literal injection happened), this recipe still falls back to
    /// `build_cli_command`'s raw `MOCK_SERVER_*` binding -- exactly like the executable e2e
    /// suite does -- and the mock-harness guard must catch it. Without this control, the
    /// test above would pass just as happily if this recipe silently swallowed every leak
    /// on its own, which would hide a real defect instead of relying on the guard.
    #[test]
    fn an_unresolved_url_falls_back_to_the_mock_server_and_the_guard_catches_it() {
        let fixture = fixture();
        let e2e = e2e_with_call("scrape", "mytool");

        let body = render_snippet_body(&fixture, &e2e).expect("brew snippet renders");

        assert!(body.contains("MOCK_SERVER"), "{body}");
        let error = crate::e2e::snippets::mock_harness_guard::reject_mock_harness_scaffolding(&body, &fixture, "brew")
            .expect_err("the guard must reject a body naming the mock-server fallback");
        assert!(error.to_string().contains("MOCK_SERVER"), "{error}");
    }

    #[test]
    fn an_unsupported_in_brew_call_is_reported_rather_than_rendered() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"url": "https://example.com/sample.html"});
        let mut e2e = e2e_with_call("interact", "mytool");
        e2e.call
            .unsupported_in
            .insert("brew".into(), "requires serializing Vec<PageAction>".into());

        let error = render_snippet_body(&fixture, &e2e).expect_err("unsupported call must not render");

        assert!(error.to_string().contains("unsupported"), "{error}");
    }

    #[test]
    fn cli_flags_and_static_args_are_appended_in_order() {
        let mut fixture = fixture();
        fixture.input = serde_json::json!({"url": "https://example.com/sample.html", "format": "markdown"});
        let mut e2e = e2e_with_call("scrape", "mytool");
        e2e.call.args.push(crate::core::config::e2e::ArgMapping {
            name: "format".into(),
            field: "format".into(),
            arg_type: "string".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        });
        e2e.call.overrides.insert(
            "brew".into(),
            CallOverride {
                cli_args: vec!["--json".into()],
                cli_flags: std::collections::HashMap::from([("format".to_string(), "--format".to_string())]),
                ..CallOverride::default()
            },
        );

        let body = render_snippet_body(&fixture, &e2e).expect("brew snippet renders");

        assert_eq!(
            body,
            "mytool scrape 'https://example.com/sample.html' --format 'markdown' --json\n"
        );
    }
}
