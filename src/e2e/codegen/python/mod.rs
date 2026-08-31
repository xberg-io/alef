//! Python e2e test code generator.
//!
//! Generates `e2e/python/conftest.py` and `tests/test_{category}.py` files from
//! JSON fixtures, driven entirely by `E2eConfig` and `CallConfig`.

mod assertions;
pub(crate) mod config;
#[cfg(test)]
mod enum_field_classification_tests;
mod helpers;
mod http;
mod json;
mod snippet;
mod stubs;
mod test_file;
mod test_function;
mod visitor_context;
mod visitors;

use std::path::PathBuf;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::{Fixture, FixtureGroup};
use anyhow::Result;

use self::config::{render_conftest, render_pyproject};
use self::test_file::render_test_file;

pub use stubs::emit_test_backend;

/// Python e2e test code generator.
pub struct PythonE2eCodegen;

impl super::E2eCodegen for PythonE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        errors: &[crate::core::ir::ErrorDef],
    ) -> Result<Vec<GeneratedFile>> {
        let mut files = Vec::new();
        let output_base = PathBuf::from(e2e_config.effective_output()).join("python");

        // NOTE: app_harness.py and the server-pattern conftest.py are emitted
        // by a consumer extension (Extension::emit_e2e "python" arm).
        // alef emits only the non-server-pattern conftest here.

        files.push(GeneratedFile {
            path: output_base.join("conftest.py"),
            content: render_conftest(e2e_config, groups, type_defs, enums),
            generated_header: true,
        });

        // NOTE: do NOT emit a root-level `__init__.py` in the test_app dir.
        // Marking `test_apps/python/` as a Python package interferes with
        // `uv sync` installing the published wheel — the editable install of
        // the e2e project picks up the root `__init__.py` and pytest then
        // resolves the generated package import against an empty local namespace,
        // missing the actual package `__init__.py` shipped in the wheel.
        // The conftest + tests/ subdirectory are sufficient for pytest.
        files.push(GeneratedFile {
            path: output_base.join("tests").join("__init__.py"),
            content: "\n".to_string(),
            generated_header: false,
        });

        let python_pkg = e2e_config.resolve_package("python");
        let default_pkg_name = e2e_config.call.module.replace('_', "-");
        let pkg_name = python_pkg
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .unwrap_or(default_pkg_name.as_str());
        let pkg_path = python_pkg
            .as_ref()
            .and_then(|p| p.path.as_deref())
            .unwrap_or("../../packages/python");
        // Resolve registry pin: explicit per-package override → workspace
        // version → 0.1.0 fallback. `render_pyproject` normalises bare
        // versions to `==<version>` so consumers can pin with bare strings
        // (`"1.4.0-rc.30"`) and still produce a valid PEP 508 requirement;
        // qualified specifiers (`">=1.2"`, `"~=2.0"`) pass through.
        let resolved = config.resolved_version();
        let owned_version: String = python_pkg
            .as_ref()
            .and_then(|p| p.version.clone())
            .or_else(|| resolved.as_ref().map(|v| v.to_string()))
            .unwrap_or_else(|| "0.1.0".to_string());
        files.push(GeneratedFile {
            path: output_base.join("pyproject.toml"),
            content: render_pyproject(pkg_name, pkg_path, &owned_version, e2e_config.dep_mode),
            generated_header: true,
        });

        for group in groups {
            let fixtures: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|fixture| is_python_fixture_runnable(fixture, e2e_config))
                .collect();
            if fixtures.is_empty() {
                continue;
            }

            let filename = format!("test_{}.py", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                &fixtures,
                e2e_config,
                config,
                type_defs,
                enums,
                functions,
                errors,
                false,
            );
            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        // Registry-mode test_apps/ runners (e.g. a consumer's
        // `task smoke:python` step) invoke a fixed `uv run pytest
        // tests/test_smoke.py` smoke target by convention. Emit a minimal
        // smoke test whenever no `smoke` fixture category is present so the
        // runner does not error on a missing path.
        //
        // The emitted file just imports the published package — a true smoke
        // test that catches packaging regressions (missing wheels, broken
        // native extension, import-time errors) without depending on any
        // specific binding API.
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            let smoke_path = output_base.join("tests").join("test_smoke.py");
            let has_smoke_emitted = files.iter().any(|f| f.path == smoke_path);
            if !has_smoke_emitted {
                files.push(GeneratedFile {
                    path: smoke_path,
                    content: render_python_smoke_test(pkg_name),
                    generated_header: true,
                });
            }
        }

        Ok(files)
    }

    fn render_snippet_body(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
    ) -> Result<String> {
        snippet::render_snippet_body(fixture, e2e_config, config, type_defs, enums, &[], &[])
    }

    fn render_snippet_body_with_functions(
        &self,
        fixture: &Fixture,
        e2e_config: &E2eConfig,
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        functions: &[crate::core::ir::FunctionDef],
        errors: &[crate::core::ir::ErrorDef],
    ) -> Result<String> {
        snippet::render_snippet_body(fixture, e2e_config, config, type_defs, enums, errors, functions)
    }

    fn language_name(&self) -> &'static str {
        "python"
    }
}

/// Render a minimal smoke test importing the published Python package.
///
/// The test asserts the module imports cleanly — a regression here points
/// at a packaging fault (missing wheel for platform, broken native
/// extension, import-time exception) rather than a binding-API issue.
fn render_python_smoke_test(pip_name: &str) -> String {
    use crate::core::hash::{self, CommentStyle};

    let header = hash::header(CommentStyle::Hash);

    let ctx = minijinja::context! {
        header => header,
        pip_name => pip_name,
    };

    crate::e2e::template_env::render("python/test_smoke.py.jinja", ctx)
}

fn is_python_fixture_runnable(fixture: &Fixture, e2e_config: &E2eConfig) -> bool {
    if !super::fixture_inclusion(fixture, "python", e2e_config).is_included() {
        return false;
    }

    if let Some(http) = &fixture.http {
        return http.expected_response.status_code != 101;
    }

    !fixture.assertions.is_empty()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Build a minimal `MethodDef` for tests.
#[cfg(test)]
fn test_method(
    name: &str,
    return_type: crate::core::ir::TypeRef,
    is_async: bool,
    has_default_impl: bool,
) -> crate::core::ir::MethodDef {
    crate::core::ir::MethodDef {
        name: name.to_string(),
        params: Vec::new(),
        return_type,
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
        has_default_impl,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::codegen::E2eCodegen;

    fn make_fixture(id: &str, input: serde_json::Value) -> crate::e2e::fixture::Fixture {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "description": "test fixture",
            "input": input,
            "assertions": []
        }))
        .expect("minimal fixture JSON must parse")
    }

    #[test]
    fn language_name_is_python() {
        let codegen = PythonE2eCodegen;
        assert_eq!(codegen.language_name(), "python");
    }

    #[test]
    fn snippet_body_matches_python_test_argument_and_call_rendering() {
        use crate::core::config::NewAlefConfig;

        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "convert"
module = "example_api"
args = [
  { name = "html", field = "html", type = "string" },
  { name = "options", field = "options", type = "json_object", optional = true },
  { name = "label", field = "label", type = "string", optional = true },
]
[crates.e2e.call.overrides.python]
options_type = "ConversionOptions"
enum_module = "example_api.options"
[crates.e2e.call.overrides.python.enum_fields]
headingStyle = "HeadingStyle"
"#,
        )
        .expect("snippet config must parse");
        let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
        let resolved = cfg.resolve().expect("config resolves").remove(0);
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "enum_options",
            "description": "convert markup",
            "input": {"html": "<h1>Hello</h1>", "options": {"headingStyle": "atx"}},
            "assertions": [{"type": "not_error"}]
        }))
        .expect("fixture must parse");

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &resolved, &[], &[])
            .expect("snippet renders");

        assert!(rendered.contains("HeadingStyle(\"atx\")"), "{rendered}");
        assert!(rendered.contains("ConversionOptions("), "{rendered}");
        // `label` is a TRAILING absent optional, so its `None` placeholder is dropped: the binding
        // declares `label=None` itself, and spelling it at the call site is noise in a docs
        // example. The middle case is the control below. ~keep
        assert!(rendered.contains("convert(html, options)"), "{rendered}");
        assert!(!rendered.contains("convert(html, options, None)"), "{rendered}");
        assert!(rendered.contains("from example_api import"), "{rendered}");
        assert!(rendered.contains("convert"), "{rendered}");
        assert!(rendered.contains("def main() -> None:"), "{rendered}");
        assert!(!rendered.contains("pytest"), "{rendered}");
        assert!(!rendered.contains("def test_"), "{rendered}");
    }

    /// The control for the trailing-placeholder trim above: an absent optional with a real argument
    /// AFTER it is load-bearing. These calls are rendered positionally, so dropping the middle
    /// `None` would silently slide `label`'s value into `options`' slot. ~keep
    #[test]
    fn an_absent_optional_followed_by_a_real_argument_keeps_its_none_placeholder() {
        use crate::core::config::NewAlefConfig;

        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "convert"
module = "example_api"
args = [
  { name = "html", field = "html", type = "string" },
  { name = "options", field = "options", type = "json_object", optional = true },
  { name = "label", field = "label", type = "string", optional = true },
]
[crates.e2e.call.overrides.python]
options_type = "ConversionOptions"
"#,
        )
        .expect("snippet config must parse");
        let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
        let resolved = cfg.resolve().expect("config resolves").remove(0);
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "labelled",
            "description": "convert markup with a label",
            "input": {"html": "<h1>Hello</h1>", "label": "docs"},
            "assertions": [{"type": "not_error"}]
        }))
        .expect("fixture must parse");

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &resolved, &[], &[])
            .expect("snippet renders");

        assert!(rendered.contains("convert(html, None, label)"), "{rendered}");
    }

    #[test]
    fn snippet_presentation_binds_and_consumes_a_non_void_call_result() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "list_widgets",
            "description": "List widgets",
            "docs": {
                "topic": "widgets",
                "shows": ["items"]
            },
            "input": null,
            "assertions": [{"type": "not_error"}]
        }))
        .expect("fixture must parse");
        let e2e = E2eConfig {
            call: crate::e2e::config::CallConfig {
                function: "list_widgets".to_string(),
                result_var: "result".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("snippet renders");

        assert!(rendered.contains("result = list_widgets()"), "{rendered}");
        assert!(rendered.contains("print(result.items)"), "{rendered}");
        assert!(!rendered.contains("_ = list_widgets()"), "{rendered}");
    }

    #[test]
    fn snippet_body_renders_expected_error_as_an_executable_example() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let rendered = PythonE2eCodegen
            .render_snippet_body(
                &fixture,
                &E2eConfig::default(),
                &ResolvedCrateConfig::default(),
                &[],
                &[],
            )
            .expect("snippet");

        assert!(rendered.contains("except Error as error:"), "{rendered}");
        assert!(!rendered.contains("AssertionError"), "{rendered}");
    }

    // Regression for a bug where a crate with no `[crate] error_type` set had its snippet
    // error name fabricated from the crate name (`TreeSitterLanguagePackError`) instead of
    // falling back to the bare `"Error"` the generated bindings actually export. Unlike
    // `snippet_body_renders_expected_error_as_an_executable_example` above, this test sets a
    // non-empty crate name so a pascal-cased fallback would actually produce a wrong,
    // non-"Error" name and the assertion would catch it. ~keep
    #[test]
    fn snippet_body_falls_back_to_bare_error_for_crate_with_no_error_type_configured() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "invalid_input", "description": "Reject invalid input", "input": null,
            "assertions": [{"type": "error"}]
        }))
        .expect("fixture");
        let config = ResolvedCrateConfig {
            name: "tree-sitter-language-pack".to_string(),
            ..Default::default()
        };
        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &E2eConfig::default(), &config, &[], &[])
            .expect("snippet");

        assert!(!rendered.contains("TreeSitterLanguagePackError"), "{rendered}");
        assert!(rendered.contains("except Error as error:"), "{rendered}");
    }

    /// Builds a fictional `my-lib` config whose `create_widget` call passes a
    /// `WidgetRequest` json_object arg via `options_via = "from_json"` — the same shape
    /// as a real consumer config that predates the pyo3 backend actually emitting
    /// `from_json()` for that type.
    fn widget_snippet_config() -> (crate::e2e::config::E2eConfig, crate::core::config::ResolvedCrateConfig) {
        use crate::core::config::NewAlefConfig;

        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "create_widget"
module = "my_lib"
async = true
[[crates.e2e.call.args]]
name = "request"
field = "input"
type = "json_object"
owned = true
[crates.e2e.call.overrides.python]
client_factory = "create_client"
options_type = "WidgetRequest"
options_via = "from_json"
from_json_module = "my_lib._internal_bindings"
"#,
        )
        .expect("config must parse");
        let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
        let mut resolved = cfg.resolve().expect("config resolves").remove(0);
        // `crate_has_serde` walks up from this path looking for a Cargo.toml with serde +
        // serde_json dependencies; point it at this (alef) crate's own `src/` — whose Cargo.toml
        // declares both — instead of the fictional `my-lib` consumer path, which resolves to a
        // directory that doesn't exist on disk and would make the crate-level condition always
        // false regardless of the scenario each test means to exercise. ~keep
        resolved.output_paths.insert(
            "python".to_string(),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        );
        (e2e, resolved)
    }

    fn widget_fixture() -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "create_widget_smoke",
            "description": "Create a widget",
            "docs": {"topic": "widgets", "stem": "create-widget", "title": "Create a widget"},
            "input": {"name": "gadget", "count": 3},
            "mock_response": {"status": 200, "body": {"id": "w1"}},
            "assertions": [{"type": "not_error"}]
        }))
        .expect("fixture must parse")
    }

    #[test]
    fn should_fall_back_to_kwargs_and_bind_result_when_options_type_lacks_serde() {
        let (e2e, resolved) = widget_snippet_config();
        let fixture = widget_fixture();

        // No TypeDef for WidgetRequest is supplied — matching a type the pyo3 backend
        // never annotated `has_serde`, so it never emitted a `from_json()` staticmethod.
        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &resolved, &[], &[])
            .expect("snippet renders");

        assert!(
            !rendered.contains(".from_json("),
            "must not call from_json() on a type the backend never marked has_serde: {rendered}"
        );
        assert!(
            rendered.contains("WidgetRequest(name=") || rendered.contains("WidgetRequest(count="),
            "expected a kwargs constructor call, got: {rendered}"
        );
        assert!(
            rendered.contains("result = await client.create_widget(request)"),
            "the call result must be bound so it can be printed, got: {rendered}"
        );
        assert!(rendered.contains("print(result)"), "{rendered}");
    }

    /// As of `093c42f31`, the pyo3 `.pyi` stub generator declares `from_json` under the exact
    /// same gate pyo3's Rust-codegen uses (`has_serde` and convertible — see
    /// `helpers::pyo3_would_inject_from_json`), so a type that passes it keeps
    /// `options_via = "from_json"` end to end and the emitted call type-checks against the
    /// shipped stub. See `test_file::tests` for direct coverage of the import-deduplication fix
    /// on this now-reachable from_json import branch. ~keep
    #[test]
    fn should_use_from_json_when_pyo3_would_inject_it() {
        let (e2e, resolved) = widget_snippet_config();
        let fixture = widget_fixture();
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            ..Default::default()
        }];

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &resolved, &type_defs, &[])
            .expect("snippet renders");

        assert!(
            rendered.contains("WidgetRequest.from_json("),
            "expected a from_json() construction now that the stub declares it: {rendered}"
        );
        let import_lines_with_type: Vec<&str> = rendered
            .lines()
            .filter(|line| line.starts_with("from ") && line.contains("WidgetRequest"))
            .collect();
        assert_eq!(
            import_lines_with_type,
            vec!["from my_lib._internal_bindings import WidgetRequest"],
            "WidgetRequest must be imported exactly once, from the native bindings module: {rendered}"
        );
        assert!(
            rendered.contains("from my_lib import create_client"),
            "the client factory must still be imported from the public module: {rendered}"
        );
    }

    /// A docs snippet is emitted for every fixture, including ones a language skips in the e2e
    /// suite. The snippet's import block is lifted out of the generated test file, so any rule
    /// that suppresses imports for skipped fixtures silently produces a snippet whose body calls
    /// symbols it never imports. Every symbol an emitter writes must resolve within the emitted
    /// unit, whatever the fixture's skip status. ~keep
    #[test]
    fn should_import_referenced_symbols_when_the_fixture_is_skipped_for_python() {
        let (e2e, resolved) = widget_snippet_config();
        let mut fixture = widget_fixture();
        fixture.skip = Some(crate::e2e::fixture::SkipDirective {
            languages: vec!["python".to_string()],
            reason: Some("no python coverage for this fixture".to_string()),
        });
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            ..Default::default()
        }];

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &resolved, &type_defs, &[])
            .expect("snippet renders");

        assert!(
            rendered.contains("create_client("),
            "precondition: the snippet body must call the client factory: {rendered}"
        );
        assert!(
            rendered.contains("WidgetRequest.from_json("),
            "precondition: the snippet body must construct the request type: {rendered}"
        );
        assert!(
            rendered.contains("from my_lib import create_client"),
            "the client factory the body calls must be imported: {rendered}"
        );
        assert!(
            rendered.contains("from my_lib._internal_bindings import WidgetRequest"),
            "the request type the body constructs must be imported: {rendered}"
        );
    }

    /// Mirrors the measured liter-llm defect exactly: `has_serde` alone is not the pyo3 gate
    /// for emitting `from_json()`. pyo3 also requires the type to pass
    /// `core_to_binding_convertible_types` — a serde-derived type whose field references a
    /// type that never resolves fails that second half even though `has_serde` is true, so
    /// pyo3 never emits `from_json()` for it. This must downgrade through the full
    /// `render_snippet_body` pipeline, not just the isolated helper. ~keep
    #[test]
    fn should_fall_back_to_kwargs_when_type_has_serde_but_fails_core_to_binding_convertibility() {
        use crate::core::ir::{FieldDef, TypeDef, TypeRef};

        let (e2e, resolved) = widget_snippet_config();
        let fixture = widget_fixture();
        let type_defs = vec![TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "extra".to_string(),
                ty: TypeRef::Named("UnresolvedExternalType".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }];

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &resolved, &type_defs, &[])
            .expect("snippet renders");

        assert!(
            !rendered.contains(".from_json("),
            "must not call from_json() on a type that fails core-to-binding convertibility \
             even though has_serde is true: {rendered}"
        );
        assert!(
            rendered.contains("WidgetRequest(name=") || rendered.contains("WidgetRequest(count="),
            "expected a kwargs constructor call, got: {rendered}"
        );
    }

    /// Builds a fictional `my-lib` config where the `from_json` request type lives on a
    /// *named* call (`create_widget`) while the default `[crates.e2e.call]` carries no
    /// `json_object` args at all. Mirrors the real liter-llm shape: a default `chat` call
    /// plus named calls like `create_batch`, each with its own python override. This is the
    /// scenario `build_thirdparty_imports`'s `needs_options_type` computed wrong before it was
    /// fixed to resolve the call per fixture — it read the *default* call's args instead of the
    /// fixture's actual resolved call, so a default call with zero `json_object` args made
    /// `needs_options_type` false even though the fixture's own call needed the from_json
    /// dedup. `per_call_native_types_are_excluded_from_public_imports` and
    /// `build_thirdparty_imports_does_not_duplicate_the_from_json_type_across_modules` in
    /// `test_file.rs` cover the same fix at the unit level; this drives it end to end through
    /// `render_snippet_body` with a real named-call config, which is what actually failed. ~keep
    fn named_call_widget_snippet_config() -> (crate::e2e::config::E2eConfig, crate::core::config::ResolvedCrateConfig) {
        use crate::core::config::NewAlefConfig;

        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "list_widgets"
module = "my_lib"
async = true
[crates.e2e.call.overrides.python]
client_factory = "create_client"
[crates.e2e.calls.create_widget]
function = "create_widget"
module = "my_lib"
async = true
[[crates.e2e.calls.create_widget.args]]
name = "request"
field = "input"
type = "json_object"
owned = true
[crates.e2e.calls.create_widget.overrides.python]
options_type = "WidgetRequest"
options_via = "from_json"
from_json_module = "my_lib._internal_bindings"
"#,
        )
        .expect("config must parse");
        let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
        let mut resolved = cfg.resolve().expect("config resolves").remove(0);
        resolved.output_paths.insert(
            "python".to_string(),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        );
        (e2e, resolved)
    }

    fn named_call_widget_fixture() -> Fixture {
        serde_json::from_value(serde_json::json!({
            "id": "create_widget_smoke",
            "description": "Create a widget",
            "docs": {"topic": "widgets", "stem": "create-widget", "title": "Create a widget"},
            "call": "create_widget",
            "input": {"name": "gadget", "count": 3},
            "mock_response": {"status": 200, "body": {"id": "w1"}},
            "assertions": [{"type": "not_error"}]
        }))
        .expect("fixture must parse")
    }

    /// Regression for the exact liter-llm defect: a docs snippet whose `from_json` request
    /// type is configured on a named call (not the default one) must still import that type
    /// exactly once, from the native bindings module — not once from the public package root
    /// and again from the native module. Asserts the precise emitted import lines. ~keep
    #[test]
    fn named_call_from_json_type_is_imported_exactly_once() {
        let (e2e, resolved) = named_call_widget_snippet_config();
        let fixture = named_call_widget_fixture();
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            ..Default::default()
        }];

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &resolved, &type_defs, &[])
            .expect("snippet renders");

        let import_lines: Vec<&str> = rendered.lines().filter(|line| line.starts_with("from ")).collect();
        assert_eq!(
            import_lines,
            vec![
                "from my_lib import create_client",
                "from my_lib._internal_bindings import WidgetRequest",
            ],
            "WidgetRequest must be imported exactly once, from the native bindings module, and \
             must not also appear on the public import line: {rendered}"
        );
    }

    /// Pins that a `client_factory` fixture's Python documentation snippet reads its
    /// credential via `os.environ[...]` — the substitution `render_snippet_body` applies over
    /// the harness's hardcoded `api_key="test-key"` literal (mod.rs ~line 183-191) — and never
    /// carries the e2e mock-server env vars, fixture route, or literal credential.
    #[test]
    fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "rate_limit_429",
            "description": "Rate limited",
            "input": null,
            "mock_response": {"status": 429}
        }))
        .expect("fixture must parse");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "python".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("snippet renders");

        assert!(
            !rendered.contains("MOCK_SERVER"),
            "mock-server env var leaked:\n{rendered}"
        );
        assert!(
            !rendered.contains("/fixtures/rate_limit_429"),
            "mock-server fixture route leaked:\n{rendered}"
        );
        assert!(
            !rendered.contains("\"test-key\""),
            "literal credential leaked:\n{rendered}"
        );
        assert!(
            rendered.contains("os.environ[\"API_KEY\"]"),
            "credential is not read from the environment:\n{rendered}"
        );
        assert!(
            rendered.contains("client = create_client(api_key=os.environ[\"API_KEY\"])"),
            "client is not constructed the way a reader would:\n{rendered}"
        );
    }

    /// A `client_factory` fixture that declares `env.api_key_var` — the common shape for a
    /// real fixture, since `rate_limit_429` above deliberately omits it — must still read the
    /// credential as a single direct `os.environ[...]` expression. `render_test_file`'s
    /// `client_factory` path guards that read with `pytest.skip(...)` for the executable e2e
    /// suite's real-API-vs-mock dispatch, which is invalid, undefined-name Python outside a
    /// pytest test function; a docs snippet must never carry it.
    #[test]
    fn client_factory_snippet_with_declared_env_var_never_leaks_pytest_skip() {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "chat_basic",
            "description": "Basic chat",
            "input": null,
            "env": {"api_key_var": "OPENAI_API_KEY"}
        }))
        .expect("fixture must parse");
        let mut e2e = E2eConfig::default();
        e2e.call.function = "chat".into();
        e2e.call.result_var = "result".into();
        e2e.call.overrides.insert(
            "python".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("snippet renders");

        assert!(
            !rendered.contains("pytest"),
            "docs snippet must not reference pytest:\n{rendered}"
        );
        assert!(
            rendered.contains("client = create_client(api_key=os.environ[\"OPENAI_API_KEY\"])"),
            "client must be constructed directly from the environment:\n{rendered}"
        );
    }

    /// A fixture whose docs declare a custom `client.base_url` — the mechanism a
    /// `configuration/custom-base-url` topic uses — must show that base URL in its Python
    /// snippet, mirroring the Java/Rust/Elixir generators' `docs_client` handling
    /// (`java/snippet.rs::a_snippet_renders_the_base_url_the_fixture_documents`). Paired with
    /// `client_factory_snippet_never_points_the_reader_at_the_mock_server` above (whose fixture
    /// declares no `docs.client` and must keep rendering the bare, no-`base_url` call) as the
    /// negative control: an indiscriminate "always add base_url" change would fail that test.
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
            "python".into(),
            crate::e2e::config::CallOverride {
                client_factory: Some("create_client".into()),
                ..Default::default()
            },
        );

        let rendered = PythonE2eCodegen
            .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
            .expect("snippet renders");

        assert!(
            rendered.contains(
                "client = create_client(api_key=os.environ[\"API_KEY\"], base_url=\"https://llm.internal.example.com/v1\")"
            ),
            "the snippet for a custom-base-url topic must show the custom base URL:\n{rendered}"
        );
    }

    #[test]
    fn generate_empty_groups_produces_config_files_only() {
        use crate::core::config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my-lib"
result_var = "result"
"#,
        )
        .unwrap();
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let codegen = PythonE2eCodegen;
        let files = codegen.generate(&[], &e2e, &resolved, &[], &[], &[], &[]).unwrap();
        // conftest.py, tests/__init__.py, pyproject.toml
        // (NO root __init__.py — would shadow the published `demo_client` package
        // during `uv sync`'s editable install of the e2e project.)
        assert_eq!(files.len(), 3, "expected 3 config files, got: {}", files.len());
        let paths: Vec<_> = files
            .iter()
            .map(|f| f.path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(paths.iter().any(|p| p.ends_with("conftest.py")));
        assert!(paths.iter().any(|p| p.ends_with("pyproject.toml")));
        // tests/__init__.py is kept so pytest can resolve `from tests.X import …` if used
        assert!(
            paths.iter().any(|p| p.ends_with("tests/__init__.py")),
            "tests/__init__.py must still be emitted; got: {paths:?}"
        );
        assert!(
            !paths
                .iter()
                .any(|p| p.ends_with("python/__init__.py") && !p.ends_with("tests/__init__.py")),
            "no root-level __init__.py in test_apps/python/ — would shadow the published wheel \
             during uv sync's editable install. got: {paths:?}"
        );
    }

    #[test]
    fn emit_test_backend_python_generates_class_and_instance_expr() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let m1 = test_method("do_work", TypeRef::String, false, false);
        let m2 = test_method("async_op", TypeRef::Named("WorkResult".to_string()), true, false);
        let methods = [&m1, &m2];

        let fixture = make_fixture("py_test_fixture", serde_json::json!({ "name": "my-python-backend" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // setup_block must define a Python class.
        assert!(
            emission.setup_block.contains("class _TestStub_py_test_fixture"),
            "setup_block should define the stub class, got: {}",
            emission.setup_block
        );
        // Must NOT hardcode sample_core-domain trait names.
        assert!(
            !emission.setup_block.contains("OcrBackend"),
            "setup_block must not hardcode OcrBackend"
        );
        assert!(
            !emission.setup_block.contains("DocumentExtractor"),
            "setup_block must not hardcode DocumentExtractor"
        );

        // name() emitted because super_trait is set.
        assert!(
            emission.setup_block.contains("def name("),
            "setup_block should emit name() when super_trait is set"
        );
        assert!(
            emission.setup_block.contains("my-python-backend"),
            "name() should return the backend name from input"
        );

        // Required methods emitted.
        assert!(
            emission.setup_block.contains("def do_work("),
            "required method do_work should be emitted"
        );
        assert!(
            emission.setup_block.contains("async def async_op("),
            "required async method should be emitted"
        );

        // arg_expr is a plain instantiation.
        assert_eq!(
            emission.arg_expr, "_TestStub_py_test_fixture()",
            "arg_expr should be a plain constructor call"
        );

        // setup_block must be indented 4 spaces for function-local scope.
        assert!(
            emission.setup_block.starts_with("    class "),
            "setup_block should be 4-space indented, got: {}",
            emission.setup_block
        );

        // Named return type must use {} not WorkResult().
        assert!(
            emission.setup_block.contains("return {}"),
            "Named return type should emit {{}} not a constructor call, got: {}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("WorkResult()"),
            "Named return type must not emit a constructor call, got: {}",
            emission.setup_block
        );
    }

    #[test]
    fn emit_test_backend_python_skips_default_impl_methods() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            ..Default::default()
        };

        let required = test_method("must_implement", TypeRef::String, false, false);
        let optional = test_method("may_implement", TypeRef::String, false, true);
        let methods = [&required, &optional];

        let fixture = make_fixture("py_skip_defaults", serde_json::json!({}));
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("def must_implement("),
            "required method should be emitted"
        );
        assert!(
            !emission.setup_block.contains("def may_implement("),
            "optional method should be skipped"
        );
    }

    #[test]
    fn emit_test_backend_python_includes_shutdown_with_super_trait() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{PrimitiveType, TypeRef};

        let bridge = TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let dimension_method = test_method("dimensions", TypeRef::Primitive(PrimitiveType::I32), false, false);
        let methods = [&dimension_method];

        let fixture = make_fixture(
            "py_embedding_backend",
            serde_json::json!({ "name": "test-embedding-backend" }),
        );
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // Verify name() is emitted
        assert!(
            emission.setup_block.contains("def name(self):"),
            "name() should be emitted with super_trait"
        );

        // Verify initialize() is emitted
        assert!(
            emission.setup_block.contains("def initialize(self):"),
            "initialize() should be emitted with super_trait"
        );

        // Verify shutdown() is emitted (the fix)
        assert!(
            emission.setup_block.contains("def shutdown(self):"),
            "shutdown() should be emitted with super_trait, got: {}",
            emission.setup_block
        );

        // All three should be present with correct bodies
        assert!(
            emission.setup_block.contains("def name(self):")
                && emission.setup_block.contains("return \"test-embedding-backend\""),
            "name() should return backend name"
        );
        assert!(
            emission.setup_block.contains("def initialize(self):") && emission.setup_block.contains("pass"),
            "initialize() should have pass body"
        );
        assert!(
            emission.setup_block.contains("def shutdown(self):") && emission.setup_block.contains("pass"),
            "shutdown() should have pass body"
        );
    }

    #[test]
    fn emit_test_backend_python_numeric_return_types_return_nonzero() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{PrimitiveType, TypeRef};

        let bridge = TraitBridgeConfig {
            trait_name: "EmbeddingBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        // Test integer return types
        let dimensions_method = test_method("dimensions", TypeRef::Primitive(PrimitiveType::I32), false, false);
        let size_method = test_method("embedding_size", TypeRef::Primitive(PrimitiveType::U64), false, false);
        let float_method = test_method("similarity_score", TypeRef::Primitive(PrimitiveType::F64), false, false);
        let bool_method = test_method("is_valid", TypeRef::Primitive(PrimitiveType::Bool), false, false);
        let methods = [&dimensions_method, &size_method, &float_method, &bool_method];

        let fixture = make_fixture(
            "py_numeric_backend",
            serde_json::json!({ "name": "test-numeric-backend" }),
        );
        let emission = emit_test_backend(&bridge, &methods, &fixture);

        // Integer types should return 1 instead of 0 (for validation constraints)
        assert!(
            emission.setup_block.contains("def dimensions(") && emission.setup_block.contains("return 1"),
            "I32 should return 1, got: {}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("def embedding_size(") && emission.setup_block.contains("return 1"),
            "U64 should return 1, got: {}",
            emission.setup_block
        );

        // Float types should return 0.0
        assert!(
            emission.setup_block.contains("def similarity_score(") && emission.setup_block.contains("return 0.0"),
            "F64 should return 0.0, got: {}",
            emission.setup_block
        );

        // Bool should return False
        assert!(
            emission.setup_block.contains("def is_valid(") && emission.setup_block.contains("return False"),
            "Bool should return False, got: {}",
            emission.setup_block
        );
    }

    #[test]
    fn emit_test_backend_python_populates_teardown_when_unregister_fn_is_set() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_backend".to_string()),
            unregister_fn: Some("unregister_test_backend".to_string()),
            ..Default::default()
        };

        let m = test_method("do_work", TypeRef::String, false, false);
        let methods = [&m];
        let fixture = make_fixture("teardown_fixture", serde_json::json!({ "name": "my-backend" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission
                .teardown_block
                .contains("unregister_test_backend(\"my-backend\")"),
            "teardown_block should call the unregister fn with the backend name, got: {:?}",
            emission.teardown_block
        );
    }

    #[test]
    fn emit_test_backend_python_omits_teardown_when_unregister_fn_is_unset() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::TypeRef;

        // No `unregister_fn` configured (e.g. per-call bridge with no global registry).
        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            ..Default::default()
        };

        let m = test_method("do_work", TypeRef::String, false, false);
        let methods = [&m];
        let fixture = make_fixture("no_teardown_fixture", serde_json::json!({ "name": "x" }));

        let emission = emit_test_backend(&bridge, &methods, &fixture);

        assert!(
            emission.teardown_block.is_empty(),
            "teardown_block should be empty when no unregister_fn is configured, got: {:?}",
            emission.teardown_block
        );
    }
}
