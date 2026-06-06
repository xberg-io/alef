use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::http::render_http_test_case;
use super::test_case::render_test_case;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_file(
    _category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    module_path: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    element_constructors: &[crate::core::config::GleamElementConstructor],
    json_object_wrapper: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "import gleeunit");
    let _ = writeln!(out, "import gleeunit/should");

    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());
    if has_http_fixtures {
        let _ = writeln!(out, "import gleam/httpc");
        let _ = writeln!(out, "import gleam/http");
        let _ = writeln!(out, "import gleam/http/request");
        let _ = writeln!(out, "import gleam/list");
        let _ = writeln!(out, "import gleam/result");
        let _ = writeln!(out, "import gleam/string");
        let _ = writeln!(out, "import envoy");
    }

    let has_non_http_with_override = fixtures.iter().any(|f| !f.is_http_test());
    if has_non_http_with_override {
        let _ = writeln!(out, "import {module_path}");
        let _ = writeln!(out, "import e2e_gleam");
        let needs_envoy_for_binding = !has_http_fixtures
            && fixtures.iter().filter(|f| !f.is_http_test()).any(|f| {
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                cc.args.iter().any(|a| a.arg_type == "mock_url")
            });
        if needs_envoy_for_binding {
            let _ = writeln!(out, "import envoy");
        }
    }
    let _ = writeln!(out);

    let mut needed_modules: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();

    for fixture in fixtures {
        if fixture.is_http_test() {
            continue;
        }

        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let call_field_resolver = FieldResolver::new(
            e2e_config.effective_fields(call_config),
            e2e_config.effective_fields_optional(call_config),
            e2e_config.effective_result_fields(call_config),
            e2e_config.effective_fields_array(call_config),
            e2e_config.effective_fields_method_calls(call_config),
        );
        let field_resolver = &call_field_resolver;
        let has_bytes_arg = fixture.resolved_args(call_config).iter().any(|a| a.arg_type == "bytes");
        let has_optional_string_arg = fixture
            .resolved_args(call_config)
            .iter()
            .any(|a| a.arg_type == "string" && a.optional);
        let has_json_object_arg = fixture
            .resolved_args(call_config)
            .iter()
            .any(|a| a.arg_type == "json_object");
        let has_handle_arg = fixture
            .resolved_args(call_config)
            .iter()
            .any(|a| a.arg_type == "handle");
        let has_client_factory = call_config
            .overrides
            .get("gleam")
            .and_then(|o| o.client_factory.as_deref())
            .is_some()
            || e2e_config
                .call
                .overrides
                .get("gleam")
                .and_then(|o| o.client_factory.as_deref())
                .is_some();
        if has_bytes_arg || has_optional_string_arg || has_json_object_arg || has_handle_arg || has_client_factory {
            needed_modules.insert("option");
        }
        for assertion in &fixture.assertions {
            let needs_case_expr = assertion
                .field
                .as_deref()
                .is_some_and(|f| field_resolver.tagged_union_split(f).is_some());
            if needs_case_expr {
                needed_modules.insert("option");
            }
            if let Some(f) = &assertion.field {
                if field_resolver.is_optional(field_resolver.resolve(f)) {
                    needed_modules.insert("option");
                }
            }
            match assertion.assertion_type.as_str() {
                "contains_any" => {
                    needed_modules.insert("string");
                    needed_modules.insert("list");
                }
                "contains" | "contains_all" | "not_contains" | "starts_with" | "ends_with" => {
                    needed_modules.insert("string");
                    if let Some(f) = &assertion.field {
                        let resolved = field_resolver.resolve(f);
                        if field_resolver.is_array(f) || field_resolver.is_array(resolved) {
                            needed_modules.insert("list");
                        }
                    } else if call_config.result_is_array
                        || call_config.result_is_vec
                        || field_resolver.is_array("")
                        || field_resolver.is_array(field_resolver.resolve(""))
                    {
                        needed_modules.insert("list");
                    }
                }
                "not_empty" | "is_empty" => {
                    if let Some(f) = &assertion.field {
                        let resolved = field_resolver.resolve(f);
                        let is_opt = field_resolver.is_optional(resolved);
                        let is_arr = field_resolver.is_array(f) || field_resolver.is_array(resolved);
                        if is_arr {
                            needed_modules.insert("list");
                        } else if is_opt {
                            needed_modules.insert("option");
                        } else {
                            needed_modules.insert("string");
                        }
                    } else {
                        needed_modules.insert("list");
                    }
                }
                "count_min" | "count_equals" => {
                    needed_modules.insert("list");
                }
                "min_length" | "max_length" => {
                    needed_modules.insert("string");
                }
                "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {}
                _ => {}
            }
            if needs_case_expr {
                if let Some(f) = &assertion.field {
                    let resolved = field_resolver.resolve(f);
                    if field_resolver.is_array(resolved) {
                        needed_modules.insert("list");
                    }
                }
            }
            if let Some(f) = &assertion.field {
                if f.split('.').any(|seg| seg == "length") {
                    needed_modules.insert("list");
                }
            }
            if let Some(f) = &assertion.field {
                if !f.is_empty() {
                    let parts: Vec<&str> = f.split('.').collect();
                    let has_opt_prefix = (1..parts.len()).any(|i| {
                        let prefix_path = parts[..i].join(".");
                        field_resolver.is_optional(&prefix_path)
                    });
                    if has_opt_prefix {
                        needed_modules.insert("option");
                        if matches!(assertion.assertion_type.as_str(), "not_empty" | "is_empty") {
                            let resolved = field_resolver.resolve(f);
                            if field_resolver.is_array(f) || field_resolver.is_array(resolved) {
                                needed_modules.insert("list");
                            } else {
                                needed_modules.insert("string");
                            }
                        }
                    }
                }
            }
        }
    }

    for module in &needed_modules {
        let _ = writeln!(out, "import gleam/{module}");
    }
    if !needed_modules.is_empty() {
        let _ = writeln!(out);
    }

    for fixture in fixtures {
        if fixture.is_http_test() {
            render_http_test_case(&mut out, fixture);
        } else {
            render_test_case(
                &mut out,
                fixture,
                e2e_config,
                module_path,
                function_name,
                result_var,
                args,
                element_constructors,
                json_object_wrapper,
            );
        }
        let _ = writeln!(out);
    }

    out
}
