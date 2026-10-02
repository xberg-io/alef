//! Go e2e test file rendering.

use crate::core::hash::{self, CommentStyle};
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::test_function::{GoTestFunctionContext, fixture_has_go_callable, render_test_function_with_facts};
use super::visitors::{emit_go_visitor_struct, resolve_go_visitor_binding, visitor_struct_name};
use crate::e2e::codegen::resolve_field;

fn deduplicate_shared_package_declaration(body: &mut String, declaration: &str) {
    let mut seen = false;
    *body = body
        .split_inclusive('\n')
        .filter(|line| {
            if line.trim_end() != declaration {
                return true;
            }
            let keep = !seen;
            seen = true;
            keep
        })
        .collect();
}

pub(super) struct GoTestFileContext<'a> {
    pub(super) go_module_path: &'a str,
    pub(super) import_alias: &'a str,
    pub(super) e2e_config: &'a crate::e2e::config::E2eConfig,
    pub(super) adapters: &'a [crate::core::config::AdapterConfig],
    pub(super) data_enum_names: &'a std::collections::HashSet<&'a str>,
    pub(super) config: &'a crate::core::config::ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [crate::core::ir::EnumDef],
    pub(super) errors: &'a [crate::core::ir::ErrorDef],
    pub(super) functions: &'a [crate::core::ir::FunctionDef],
    pub(super) crate_facts: Option<&'a super::test_function::call_resolver::GoCrateResolverFacts>,
}

/// Whether a fixture's `error` assertion declares a value THAT WILL RENDER AS AN ASSERTION,
/// meaning the generated `expects_error` branch will emit `fmt.Sprintf` and therefore requires
/// the `fmt` import — independent of the pre-existing visitor `CustomTemplate` heuristic.
///
/// ~keep Must consult `classify`, not just "a value is declared": a value naming an
/// unsubstantiable variant now renders `declared_error_variant::skip_line`'s bare `//` comment
/// instead of `fmt.Sprintf`, so importing `fmt` for that fixture alone would be unused and Go
/// rejects unused imports at compile time.
fn fixture_needs_fmt_for_declared_error_value(fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) -> bool {
    matches!(
        crate::e2e::codegen::declared_error_variant::classify("go", fixture, errors),
        crate::e2e::codegen::declared_error_variant::DeclaredErrorAssertion::Assert(_)
    )
}

pub(super) fn render_test_file(category: &str, fixtures: &[&Fixture], context: GoTestFileContext<'_>) -> String {
    let GoTestFileContext {
        go_module_path,
        import_alias,
        e2e_config,
        adapters,
        data_enum_names,
        config,
        type_defs,
        enums,
        errors,
        functions,
        crate_facts,
    } = context;
    let mut out = String::new();
    let emits_executable_test =
        |fixture: &Fixture| fixture.is_http_test() || fixture_has_go_callable(fixture, e2e_config);

    out.push_str(&hash::e2e_header(CommentStyle::DoubleSlash));
    let _ = writeln!(out);

    let needs_pkg = fixtures
        .iter()
        .any(|f| fixture_has_go_callable(f, e2e_config) || f.is_http_test() || f.visitor.is_some());

    let needs_filepath = false;

    let needs_json = fixtures.iter().any(|f| {
        if let Some(http) = &f.http {
            let body_needs_json = http
                .expected_response
                .body
                .as_ref()
                .is_some_and(|b| matches!(b, serde_json::Value::Object(_) | serde_json::Value::Array(_)));
            let partial_needs_json = http.expected_response.body_partial.is_some();
            let ve_needs_json = http
                .expected_response
                .validation_errors
                .as_ref()
                .is_some_and(|v| !v.is_empty());
            if body_needs_json || partial_needs_json || ve_needs_json {
                return true;
            }
        }
        if !emits_executable_test(f) {
            return false;
        }

        let call =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("go", f, call, type_defs);
        let call_args = recipe.args;
        let has_handle = call_args.iter().any(|a| a.arg_type == "handle") && {
            call_args.iter().filter(|a| a.arg_type == "handle").any(|a| {
                let v = resolve_field(&f.input, &a.field);
                !(v.is_null() || v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
                    && super::resolve_handle_config_type(a, recipe.options_type, type_defs).is_some()
            })
        };
        let go_override = call.overrides.get("go");
        let opts_type = go_override.and_then(|o| o.options_type.as_deref()).or_else(|| {
            e2e_config
                .call
                .overrides
                .get("go")
                .and_then(|o| o.options_type.as_deref())
        });
        let has_json_obj = call_args.iter().any(|a| {
            if a.arg_type != "json_object" {
                return false;
            }
            let v = if a.field == "input" {
                f.input.get("extract_input").unwrap_or(&f.input)
            } else {
                let field = a.field.strip_prefix("input.").unwrap_or(&a.field);
                f.input.get(field).unwrap_or(&serde_json::Value::Null)
            };
            if v.is_array() {
                return true;
            }
            opts_type.is_some() && v.is_object() && !v.as_object().is_some_and(|o| o.is_empty())
        });
        has_handle || has_json_obj
    });

    let needs_base64 = false;

    let needs_fmt = fixtures.iter().any(|f| {
        f.visitor.as_ref().is_some_and(|v| {
            v.callbacks.values().any(|action| {
                if let crate::e2e::fixture::CallbackAction::CustomTemplate { template, .. } = action {
                    template.contains('{')
                } else {
                    false
                }
            })
        })
    }) || fixtures.iter().any(|f| fixture_needs_fmt_for_declared_error_value(f, errors))
        // Bracket-wildcard traversal assertions stringify each element with
        // `fmt.Sprintf("%v", …)`; without this the `needs_fmt` conjunction below would
        // veto the import even though the body references the package. ~keep
        || fixtures.iter().any(|f| {
            f.assertions
                .iter()
                .any(|a| a.field.as_deref().is_some_and(|field| field.contains("[].")))
        });

    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());
    // Shadowed below with `|| mock_helper_emitted` once the mock-request-count helper (alef
    // issue #443) has decided whether it needs `net/http`/`io` too -- a fixture with a `mock.*`
    // assertion but no HTTP test still needs both for the helper's own `http.Get`/`io.ReadAll`.
    let needs_http = has_http_fixtures;
    let needs_io = has_http_fixtures;

    let needs_reflect = fixtures.iter().any(|f| {
        if let Some(http) = &f.http {
            let body_needs_reflect = http
                .expected_response
                .body
                .as_ref()
                .is_some_and(|b| matches!(b, serde_json::Value::Object(_) | serde_json::Value::Array(_)));
            let partial_needs_reflect = http.expected_response.body_partial.is_some();
            body_needs_reflect || partial_needs_reflect
        } else {
            false
        }
    });

    let mut body = String::new();
    let computed_facts;
    let crate_facts = if let Some(facts) = crate_facts {
        facts
    } else {
        computed_facts = super::test_function::call_resolver::GoCrateResolverFacts::new(type_defs, enums, config);
        &computed_facts
    };
    for fixture in fixtures.iter() {
        if let Some(visitor_spec) = &fixture.visitor {
            let struct_name = visitor_struct_name(&fixture.id);
            let binding = resolve_go_visitor_binding(config, type_defs, visitor_spec, import_alias);
            emit_go_visitor_struct(&mut body, &struct_name, visitor_spec, import_alias, binding.as_ref());
            let _ = writeln!(body);
        }
    }
    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_function_with_facts(
            &mut body,
            fixture,
            GoTestFunctionContext {
                import_alias,
                e2e_config,
                adapters,
                data_enum_names,
                config,
                type_defs,
                enums,
                errors,
                functions,
            },
            crate_facts,
        );
        if i + 1 < fixtures.len() {
            let _ = writeln!(body);
        }
    }
    deduplicate_shared_package_declaration(&mut body, super::setup::GO_PTR_HELPER);

    // Emit the `mock.*` once-per-suite request-count helper (alef issue #443) iff this file's
    // rendered bodies actually call it -- mirrors the identical `body.contains(...)` gate the
    // rust/python/node reference-backend wave already uses (`rust::test_file::file_rendering`,
    // `python::test_file`, `typescript::test_file::render`): a helper defined but never called
    // is dead code `golangci-lint`'s unused-function check would flag. Prepended BEFORE every
    // `needs_*` import flag below is computed, so the helper's own `strings.`/`fmt.` usage is
    // counted by those flags' existing body scans; `net/http`, `io` and `strconv` are gated on
    // `mock_helper_emitted` explicitly below since they have no other source in this backend
    // that a body scan could safely generalize from (unlike `strings`/`fmt`, `http.`/`io.` are
    // not otherwise scanned from the body at all -- see `needs_http`/`needs_io` below).
    let mut mock_helper_emitted = false;
    if let Ok(capture) = crate::e2e::codegen::mock_assertions::mock_capture("go")
        && body.contains(capture.helper_name)
        && let Some(rendered) = crate::e2e::codegen::mock_assertions::render_helper("go")
    {
        body = format!("{rendered}\n{body}");
        mock_helper_emitted = true;
    }

    let needs_assert = body.contains("assert.");
    // ~keep A fixture-level heuristic here previously predicted `os.` usage from is_http_test,
    // client_factory overrides, and mock_url/bytes-as-path args, then narrowed it with
    // `body.contains("os.")` to dodge Go's unused-import error. It missed a declared
    // `fixture.env.api_key_var` (emits `os.Getenv` outside every predicted shape), which
    // produced the opposite failure: `os.` referenced but not imported. Matches
    // `needs_strings` below — the rendered body is the only sound and complete authority.
    let needs_os = body.contains("os.");
    // #448: every streaming fixture's call now passes `context.Background()` as the streaming
    // start method's required first argument. Read from the rendered body, same as the other
    // `needs_*` flags on this file, rather than re-deriving "is this fixture streaming" here.
    let needs_context = body.contains("context.");
    // ~keep `strings.` is emitted from several independent sites (equality/contains/prefix/
    // suffix assertions, declared-error-value checks, HTTP header/body assertions, mock URL
    // list setup) that drift out of sync with any hand-maintained enumeration — a prior
    // heuristic here missed the declared-error-value path and dropped the import for
    // `error_test.go`. The rendered body is the only sound and complete authority, so this
    // reads it directly instead of re-deriving which assertion kinds need the package.
    let needs_strings = body.contains("strings.");
    let needs_pkg = needs_pkg && body.contains(&format!("{import_alias}."));
    // Even when a fixture *could* need fmt (a CustomTemplate), it might be
    // emitted as a panic stub instead. Require the body to actually reference
    // the package before importing it. The mock-request-count helper (alef issue #443) is a
    // second, independent source of `fmt.` usage the upfront `needs_fmt` prediction never
    // accounts for, so it is OR'd in directly rather than folded into that prediction.
    let needs_fmt = (needs_fmt && body.contains("fmt.")) || mock_helper_emitted;
    // `net/http`/`io` have no prediction to narrow -- `has_http_fixtures` is already exact for
    // the pre-existing HTTP-test caller, so the mock helper's own need is a plain OR.
    let needs_http = needs_http || mock_helper_emitted;
    let needs_io = needs_io || mock_helper_emitted;
    // `strconv` has exactly one caller in this backend: the mock-request-count helper.
    let needs_strconv = mock_helper_emitted;

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out, "package e2e_test");
    let _ = writeln!(out);
    let _ = writeln!(out, "import (");
    if needs_context {
        let _ = writeln!(out, "\t\"context\"");
    }
    if needs_base64 {
        let _ = writeln!(out, "\t\"encoding/base64\"");
    }
    let needs_json = needs_json || body.contains("json.");
    if needs_json || needs_reflect {
        let _ = writeln!(out, "\t\"encoding/json\"");
    }
    if needs_fmt {
        let _ = writeln!(out, "\t\"fmt\"");
    }
    if needs_io {
        let _ = writeln!(out, "\t\"io\"");
    }
    if needs_http {
        let _ = writeln!(out, "\t\"net/http\"");
    }
    if needs_os {
        let _ = writeln!(out, "\t\"os\"");
    }
    let _ = needs_filepath;
    if needs_reflect {
        let _ = writeln!(out, "\t\"reflect\"");
    }
    if needs_strconv {
        let _ = writeln!(out, "\t\"strconv\"");
    }
    if needs_strings {
        let _ = writeln!(out, "\t\"strings\"");
    }
    let _ = writeln!(out, "\t\"testing\"");
    if needs_assert {
        let _ = writeln!(out);
        let _ = writeln!(out, "\t\"github.com/stretchr/testify/assert\"");
    }
    if needs_pkg {
        let _ = writeln!(out);
        let _ = writeln!(out, "\t{import_alias} \"{go_module_path}\"");
    }
    let _ = writeln!(out, ")");
    let _ = writeln!(out);

    out.push_str(&body);
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod fmt_import_tests {
    use super::fixture_needs_fmt_for_declared_error_value;
    use crate::e2e::fixture::{Assertion, Fixture};

    fn error_fixture(value: Option<serde_json::Value>) -> Fixture {
        Fixture {
            assertions: vec![Assertion {
                skip: None,
                assertion_type: "error".to_string(),
                field: None,
                value,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            ..Fixture::default()
        }
    }

    #[test]
    fn declared_error_value_requires_fmt_import() {
        let fixture = error_fixture(Some(serde_json::Value::String("SomeExpectedError".to_string())));

        assert!(fixture_needs_fmt_for_declared_error_value(&fixture, &[]));
    }

    #[test]
    fn undeclared_error_value_does_not_require_fmt_import() {
        let fixture = error_fixture(None);

        assert!(!fixture_needs_fmt_for_declared_error_value(&fixture, &[]));
    }

    #[test]
    fn non_error_assertion_does_not_require_fmt_import() {
        let mut fixture = error_fixture(Some(serde_json::Value::String("SomeExpectedError".to_string())));
        fixture.assertions[0].assertion_type = "contains".to_string();

        assert!(!fixture_needs_fmt_for_declared_error_value(&fixture, &[]));
    }

    /// The defect this fix closes: a fixture whose declared value names a real `ErrorVariant`
    /// with no `#[alef(error_code = N)]` now renders `declared_error_variant::skip_line`'s bare
    /// `//` comment, not `fmt.Sprintf` — so it must NOT pull in the `fmt` import either.
    #[test]
    fn uncoded_known_variant_does_not_require_fmt_import() {
        use crate::core::ir::{ErrorDef, ErrorVariant};

        let fixture = error_fixture(Some(serde_json::Value::String("Authentication".to_string())));
        let errors = vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Authentication".to_string(),
                error_code: None,
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }];

        assert!(!fixture_needs_fmt_for_declared_error_value(&fixture, &errors));
    }

    /// A CODED known variant still renders the `fmt`-using assertion.
    #[test]
    fn coded_known_variant_still_requires_fmt_import() {
        use crate::core::ir::{ErrorDef, ErrorVariant};

        let fixture = error_fixture(Some(serde_json::Value::String("Authentication".to_string())));
        let errors = vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Authentication".to_string(),
                error_code: Some(100),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }];

        assert!(fixture_needs_fmt_for_declared_error_value(&fixture, &errors));
    }
}

#[cfg(test)]
mod shared_helper_tests {
    use super::{GoTestFileContext, render_test_file};
    use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn optional_u32_fixture(id: &str, value: u32) -> Fixture {
        Fixture {
            id: id.to_string(),
            description: "optional primitive argument".to_string(),
            input: serde_json::json!({"limit": value}),
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn ptr_helper_is_emitted_once_for_multiple_fixtures_in_one_file() {
        let e2e_config = E2eConfig {
            call: CallConfig {
                function: "inspect".to_string(),
                module: "example.com/sample".to_string(),
                returns_result: true,
                args: vec![ArgMapping {
                    name: "limit".to_string(),
                    field: "input.limit".to_string(),
                    arg_type: "u32".to_string(),
                    optional: true,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let fixtures = [
            optional_u32_fixture("first_limit", 1),
            optional_u32_fixture("second_limit", 2),
        ];
        let fixture_refs = fixtures.iter().collect::<Vec<_>>();
        let config = crate::core::config::ResolvedCrateConfig::default();

        let rendered = render_test_file(
            "contract",
            &fixture_refs,
            GoTestFileContext {
                go_module_path: "example.com/sample",
                import_alias: "sample",
                e2e_config: &e2e_config,
                adapters: &[],
                data_enum_names: &std::collections::HashSet::new(),
                config: &config,
                type_defs: &[],
                enums: &[],
                errors: &[],
                functions: &[],
                crate_facts: None,
            },
        );

        assert_eq!(rendered.matches("func ptr[T any](value T) *T").count(), 1, "{rendered}");
        assert!(rendered.contains("ptr(uint32(1))"), "{rendered}");
        assert!(rendered.contains("ptr(uint32(2))"), "{rendered}");
    }
}
