//! R e2e test generator using testthat.

mod args;
mod assertion_mock_capture;
mod assertions;
mod not_error_assertion;
mod project;
mod stubs;
mod test_case;
mod test_file;
mod values;
mod visitor;

#[cfg(test)]
mod mock_capture_gate_tests;

pub use stubs::emit_test_backend;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::{Context as _, Result};
use std::path::PathBuf;

use super::E2eCodegen;

/// R e2e code generator.
pub struct RCodegen;

impl E2eCodegen for RCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
        _functions: &[crate::core::ir::FunctionDef],
        errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let result_is_simple = call.result_is_simple || overrides.is_some_and(|o| o.result_is_simple);
        let result_is_r_list = overrides.is_some_and(|o| o.result_is_r_list);

        let r_pkg = e2e_config.resolve_package("r");
        let pkg_name = r_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| module_path.clone());
        let pkg_path = r_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/r".to_string());
        let pkg_version = r_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .or_else(|| config.resolved_version())
            .unwrap_or_else(|| "0.1.0".to_string());

        files.push(GeneratedFile {
            path: output_base.join("DESCRIPTION"),
            content: project::render_description(&pkg_name, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        files.push(GeneratedFile {
            path: output_base.join("run_tests.R"),
            content: project::render_test_runner(&pkg_name, &pkg_path, e2e_config.dep_mode),
            generated_header: true,
        });

        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            files.push(GeneratedFile {
                path: output_base.join("install.R"),
                content: project::render_install_r(
                    &pkg_name,
                    &pkg_version,
                    e2e_config
                        .registry
                        .github_repo
                        .as_deref()
                        .context("R registry mode requires `[crates.e2e.registry] github_repo`")?,
                ),
                generated_header: false,
            });
        }

        files.push(GeneratedFile {
            path: output_base.join("tests").join("setup-fixtures.R"),
            content: project::render_setup_fixtures(&e2e_config.test_documents_relative_from(1), &e2e_config.env),
            generated_header: true,
        });

        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("test_{}.R", sanitize_filename(&group.category));
            let content = test_file::render_test_file(
                &group.category,
                &active,
                result_is_simple,
                result_is_r_list,
                &test_file::TestFileContext {
                    e2e_config,
                    config,
                    type_defs,
                    errors,
                },
            );
            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "r"
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        self.render_snippet_body_with_functions(fixture, e2e_config, config, type_defs, enums, &[], &[])
    }

    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        _errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        let call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let r_override = call.overrides.get("r");
        let result_is_simple = call.result_is_simple || r_override.is_some_and(|value| value.result_is_simple);
        let result_is_r_list = r_override.is_some_and(|value| value.result_is_r_list);
        let mut snippet_fixture = fixture.clone();
        if !fixture
            .assertions
            .iter()
            .any(|assertion| assertion.assertion_type == "error")
        {
            snippet_fixture.assertions.clear();
        }
        let mut test_case = String::new();
        test_case::render_test_case(
            &mut test_case,
            &snippet_fixture,
            e2e_config,
            result_is_simple,
            result_is_r_list,
            config,
            type_defs,
            &[],
        );
        let body = test_case
            .lines()
            .skip(1)
            .take(test_case.lines().count().saturating_sub(2))
            .map(|line| line.strip_prefix("  ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        let expects_error = fixture
            .assertions
            .iter()
            .any(|assertion| assertion.assertion_type == "error");
        // The presentation accessors are rooted on `result_var`, which neither the error
        // path nor a void call ever binds. ~keep
        let presentation = if call.returns_void || expects_error {
            Vec::new()
        } else {
            crate::e2e::codegen::presentation::resolve(fixture, e2e_config, "r", type_defs, enums, functions)
        };
        let body = if call.returns_void || expects_error || !presentation.is_empty() {
            body
        } else {
            format!("{body}\nprint({})", call.effective_result_var())
        };
        let package = e2e_config
            .resolve_package("r")
            .and_then(|package| package.name)
            .unwrap_or_else(|| call.module.clone());
        Ok(crate::e2e::template_env::render(
            "r/snippet_body.jinja",
            minijinja::context! { package => package, body => body, presentation => presentation },
        ))
    }
}

#[cfg(test)]
mod snippet_tests {
    use super::*;
    use crate::e2e::codegen::E2eCodegen;
    use crate::e2e::fixture::{CallbackAction, VisitorSpec};

    #[test]
    fn visitor_snippet_reuses_r_call_preparation_without_testthat() {
        let mut fixture = Fixture {
            id: "custom_text".into(),
            description: "Custom text".into(),
            input: serde_json::json!({ "html": "<p>Hello</p>" }),
            ..Fixture::default()
        };
        fixture.visitor = Some(VisitorSpec {
            callbacks: [("visit_text".into(), CallbackAction::Continue)].into(),
        });
        let mut e2e = E2eConfig::default();
        e2e.call.function = "render_document".into();
        e2e.call.module = "sampleR".into();

        let rendered = RCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("R visitor snippet renders");

        assert!(rendered.contains("library(\"sampleR\", character.only = TRUE)"));
        assert!(rendered.contains("visitor <- list("));
        assert!(rendered.contains("visit_text = function(ctx, text)"));
        assert!(rendered.contains("render_document("));
        assert!(!rendered.contains("test_that("));
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
        e2e.call.module = "sampleR".into();
        e2e.call.result_var = "result".into();
        e2e.result_fields = ["summary".to_string(), "items".to_string()].into_iter().collect();

        let rendered = RCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("R snippet renders");

        assert!(rendered.contains("result <- "), "{rendered}");
        assert!(rendered.contains("cat(result$summary, \"\\n\")"), "{rendered}");
        assert!(rendered.contains("for (item in result$items) {"), "{rendered}");
        assert!(rendered.contains("print(item$label)"), "{rendered}");
        assert!(
            !rendered.contains("print(result)"),
            "the whole-result fallback must give way to the documented presentation:\n{rendered}"
        );
    }

    #[test]
    fn successful_snippet_displays_the_call_result() {
        let fixture = Fixture {
            id: "list_widgets".into(),
            description: "List widgets".into(),
            ..Fixture::default()
        };
        let mut e2e = E2eConfig::default();
        e2e.call.function = "list_widgets".into();
        e2e.call.result_var = "widgets".into();

        let rendered = RCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("R snippet renders");

        assert!(
            rendered.contains("widgets <- jsonlite::fromJSON(list_widgets()"),
            "{rendered}"
        );
        assert!(rendered.contains("print(widgets)"), "{rendered}");
    }

    /// R's snippet path has no client-factory concept (`render_snippet_body` never
    /// constructs a stateful client), so the Java/C#/Zig/Dart-style client-construction
    /// test does not apply here. What R's args renderer CAN leak into a documentation
    /// snippet is a `$mock_url` placeholder embedded inside a `json_object` arg's value
    /// (`r/args.rs` lines ~150-160): that branch emits a
    /// `Sys.getenv("MOCK_SERVER_URL")`/`/fixtures/<id>` setup line and a `gsub` over it.
    ///
    /// Nothing inside `render_snippet_body` disarms it — the substitution happens one
    /// level up, in `snippets::render_snippet_body`, which renders
    /// `Fixture::docs_call_fixture()` rather than the raw fixture. This test therefore
    /// renders through the same docs fixture the funnel builds, and pins that the
    /// illustrative URL replaces the placeholder before the args renderer sees it.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "rate_limit_429", "description": "Rate limited",
            "input": { "config": { "webhook_url": "$mock_url/callback" } }
        }))
        .expect("fixture");
        let fixture = fixture.docs_call_fixture();
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.options_type = Some("ChatOptions".into());
        e2e.call.args.push(crate::e2e::config::ArgMapping {
            name: "config".into(),
            field: "input.config".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        });

        let body = RCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("R snippet renders");

        assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
        assert!(
            !body.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{body}"
        );
        assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
        assert!(
            body.contains("https://example.com/callback"),
            "the placeholder must be replaced by an illustrative URL, not dropped:\n{body}"
        );
    }
}
