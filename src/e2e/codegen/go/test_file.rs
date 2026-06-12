//! Go e2e test file rendering.

use crate::core::hash::{self, CommentStyle};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::test_function::{GoTestFunctionContext, fixture_has_go_callable, render_test_function};
use super::visitors::{emit_go_visitor_struct, resolve_go_visitor_binding, visitor_struct_name};
use crate::e2e::codegen::resolve_field;

pub(super) struct GoTestFileContext<'a> {
    pub(super) go_module_path: &'a str,
    pub(super) import_alias: &'a str,
    pub(super) e2e_config: &'a crate::e2e::config::E2eConfig,
    pub(super) adapters: &'a [crate::core::config::AdapterConfig],
    pub(super) data_enum_names: &'a std::collections::HashSet<&'a str>,
    pub(super) config: &'a crate::core::config::ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [crate::core::ir::EnumDef],
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
    } = context;
    let mut out = String::new();
    let emits_executable_test =
        |fixture: &Fixture| fixture.is_http_test() || fixture_has_go_callable(fixture, e2e_config);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out);

    let needs_pkg = fixtures
        .iter()
        .any(|f| fixture_has_go_callable(f, e2e_config) || f.is_http_test() || f.visitor.is_some());

    let needs_os = fixtures.iter().any(|f| {
        if f.is_http_test() {
            return true;
        }
        if !emits_executable_test(f) {
            return false;
        }
        let call_config =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        let go_override = call_config
            .overrides
            .get("go")
            .or_else(|| e2e_config.call.overrides.get("go"));
        if go_override.and_then(|o| o.client_factory.as_deref()).is_some() {
            return true;
        }
        let call_args = f.resolved_args(call_config);
        if call_args
            .iter()
            .any(|a| a.arg_type == "mock_url" || a.arg_type == "mock_url_list")
        {
            return true;
        }
        call_args.iter().any(|a| {
            if a.arg_type != "bytes" {
                return false;
            }
            let mut current = &f.input;
            let path = a.field.strip_prefix("input.").unwrap_or(&a.field);
            for segment in path.split('.') {
                match current.get(segment) {
                    Some(next) => current = next,
                    None => return false,
                }
            }
            current.is_string()
        })
    });

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
                &f.input
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

    let call_result_is_simple = |cc: &crate::core::config::e2e::CallConfig| -> bool {
        cc.overrides.get("go").is_some_and(|o| o.result_is_simple)
            || cc.result_is_simple
            || cc.overrides.get("rust").map(|o| o.result_is_simple).unwrap_or(false)
    };

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
    });

    let needs_strings = fixtures.iter().any(|f| {
        if !emits_executable_test(f) {
            return false;
        }
        let cc =
            e2e_config.resolve_call_for_fixture(f.call.as_deref(), &f.id, &f.resolved_category(), &f.tags, &f.input);
        if cc.args.iter().any(|arg| arg.arg_type == "mock_url_list") {
            return true;
        }
        let per_call_resolver = FieldResolver::new(
            e2e_config.effective_fields(cc),
            e2e_config.effective_fields_optional(cc),
            e2e_config.effective_result_fields(cc),
            e2e_config.effective_fields_array(cc),
            &std::collections::HashSet::new(),
        );
        f.assertions.iter().any(|a| {
            let type_needs_strings = if a.assertion_type == "equals" {
                a.value.as_ref().is_some_and(|v| v.is_string())
            } else {
                matches!(
                    a.assertion_type.as_str(),
                    "contains" | "contains_all" | "contains_any" | "not_contains" | "starts_with" | "ends_with"
                )
            };
            let simple_result = call_result_is_simple(cc);
            let field_valid = a
                .field
                .as_ref()
                .map(|f| f.is_empty() || simple_result || per_call_resolver.is_valid_for_result(f))
                .unwrap_or(true);
            type_needs_strings && field_valid
        })
    });

    let has_http_fixtures = fixtures.iter().any(|f| f.is_http_test());
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
    for fixture in fixtures.iter() {
        if let Some(visitor_spec) = &fixture.visitor {
            let struct_name = visitor_struct_name(&fixture.id);
            let binding = resolve_go_visitor_binding(config, type_defs, visitor_spec, import_alias);
            emit_go_visitor_struct(&mut body, &struct_name, visitor_spec, import_alias, binding.as_ref());
            let _ = writeln!(body);
        }
    }
    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_function(
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
            },
        );
        if i + 1 < fixtures.len() {
            let _ = writeln!(body);
        }
    }

    let needs_assert = body.contains("assert.");
    let needs_strings = needs_strings || body.contains("strings.");
    let needs_pkg = needs_pkg && body.contains(&format!("{import_alias}."));
    // Even when a fixture *could* need fmt (a CustomTemplate), it might be
    // emitted as a panic stub instead. Require the body to actually reference
    // the package before importing it.
    let needs_fmt = needs_fmt && body.contains("fmt.");

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out, "package e2e_test");
    let _ = writeln!(out);
    let _ = writeln!(out, "import (");
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
