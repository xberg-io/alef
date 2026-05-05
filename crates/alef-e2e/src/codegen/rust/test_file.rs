//! Per-category test file generation for Rust e2e tests.

use std::fmt::Write as FmtWrite;

use crate::config::E2eConfig;
use crate::escape::sanitize_filename;
use crate::field_access::FieldResolver;
use crate::fixture::{Fixture, FixtureGroup};

use super::args::{emit_rust_visitor_method, render_rust_arg, resolve_visitor_trait};
use super::assertions::render_assertion;
use super::http::render_http_test_function;
use super::mock_server::render_mock_server_setup;

pub(super) fn resolve_function_name_for_call(call_config: &crate::config::CallConfig) -> String {
    call_config
        .overrides
        .get("rust")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| call_config.function.clone())
}

pub(super) fn resolve_module(e2e_config: &E2eConfig, dep_name: &str) -> String {
    resolve_module_for_call(&e2e_config.call, dep_name)
}

pub(super) fn resolve_module_for_call(call_config: &crate::config::CallConfig, dep_name: &str) -> String {
    // For Rust, the module name is the crate identifier (underscores).
    // Priority: override.crate_name > override.module > dep_name
    let overrides = call_config.overrides.get("rust");
    overrides
        .and_then(|o| o.crate_name.clone())
        .or_else(|| overrides.and_then(|o| o.module.clone()))
        .unwrap_or_else(|| dep_name.to_string())
}

pub(super) fn is_skipped(fixture: &Fixture, language: &str) -> bool {
    fixture.skip.as_ref().is_some_and(|s| s.should_skip(language))
}

pub fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    dep_name: &str,
    needs_mock_server: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&alef_core::hash::header(alef_core::hash::CommentStyle::DoubleSlash));
    let _ = writeln!(out, "//! E2e tests for category: {category}");
    let _ = writeln!(out);

    let module = resolve_module(e2e_config, dep_name);
    let field_resolver = FieldResolver::new(
        &e2e_config.fields,
        &e2e_config.fields_optional,
        &e2e_config.result_fields,
        &e2e_config.fields_array,
    );

    // Check if this file has http-fixture tests (separate from call-based tests).
    let file_has_http = fixtures.iter().any(|f| f.http.is_some());
    // Call-based: has mock_response OR is a plain function-call fixture (no http, no mock) with a
    // configured function name. Pure schema/spec stubs (function name empty) use the stub path.
    let file_has_call_based = fixtures.iter().any(|f| {
        if f.mock_response.is_some() {
            return true;
        }
        if f.http.is_none() && f.mock_response.is_none() {
            let call_config = e2e_config.resolve_call(f.call.as_deref());
            let fn_name = resolve_function_name_for_call(call_config);
            return !fn_name.is_empty();
        }
        false
    });

    // Collect all unique (module, function) pairs needed across call-based fixtures only.
    // Resolve client_factory from the default call's rust override. When set, the generated tests
    // create a client via `module::factory(...)` and call methods on it rather than importing and
    // calling free functions. In that case we skip the function `use` imports entirely.
    let rust_call_override = e2e_config.call.overrides.get("rust");
    let client_factory = rust_call_override.and_then(|o| o.client_factory.as_deref());

    // Http fixtures and pure stub fixtures use different code paths and don't import the call function.
    if file_has_call_based && client_factory.is_none() {
        let mut imported: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
        for fixture in fixtures.iter().filter(|f| {
            if f.mock_response.is_some() {
                return true;
            }
            if f.http.is_none() && f.mock_response.is_none() {
                let call_config = e2e_config.resolve_call(f.call.as_deref());
                let fn_name = resolve_function_name_for_call(call_config);
                return !fn_name.is_empty();
            }
            false
        }) {
            let call_config = e2e_config.resolve_call(fixture.call.as_deref());
            let fn_name = resolve_function_name_for_call(call_config);
            let mod_name = resolve_module_for_call(call_config, dep_name);
            imported.insert((mod_name, fn_name));
        }
        // Emit use statements, grouping by module when possible.
        let mut by_module: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for (mod_name, fn_name) in &imported {
            by_module.entry(mod_name.clone()).or_default().push(fn_name.clone());
        }
        for (mod_name, fns) in &by_module {
            if fns.len() == 1 {
                let _ = writeln!(out, "use {mod_name}::{};", fns[0]);
            } else {
                let joined = fns.join(", ");
                let _ = writeln!(out, "use {mod_name}::{{{joined}}};");
            }
        }
    }

    // Http fixtures use App + RequestContext for integration tests.
    if file_has_http {
        let _ = writeln!(out, "use {module}::{{App, RequestContext}};");
    }

    // Import handle constructor functions and the config type they use.
    let has_handle_args = e2e_config.call.args.iter().any(|a| a.arg_type == "handle");
    if has_handle_args {
        let _ = writeln!(out, "use {module}::CrawlConfig;");
    }
    for arg in &e2e_config.call.args {
        if arg.arg_type == "handle" {
            use heck::ToSnakeCase;
            let constructor_name = format!("create_{}", arg.name.to_snake_case());
            let _ = writeln!(out, "use {module}::{constructor_name};");
        }
    }

    // When client_factory is set, emit trait imports required to call methods on the client object.
    // Traits like LlmClient, FileClient, etc. must be in scope for method dispatch to work.
    if client_factory.is_some() && file_has_call_based {
        let trait_imports: Vec<String> = e2e_config
            .call
            .overrides
            .get("rust")
            .map(|o| o.trait_imports.clone())
            .unwrap_or_default();
        for trait_name in &trait_imports {
            let _ = writeln!(out, "use {module}::{trait_name};");
        }
    }

    // Import mock_server module when any fixture in this file uses mock_response.
    let file_needs_mock = needs_mock_server && fixtures.iter().any(|f| f.mock_response.is_some());
    if file_needs_mock {
        let _ = writeln!(out, "mod mock_server;");
        let _ = writeln!(out, "use mock_server::{{MockRoute, MockServer}};");
    }

    // Import the visitor trait, result enum, and node context when any fixture
    // in this file declares a `visitor` block. Without these, the inline
    // `impl HtmlVisitor for _TestVisitor` block fails to resolve.
    // Visitor types live in the `visitor` sub-module of the crate, not the crate root.
    let file_needs_visitor = fixtures.iter().any(|f| f.visitor.is_some());
    if file_needs_visitor {
        let visitor_trait = resolve_visitor_trait(&module);
        let _ = writeln!(
            out,
            "use {module}::visitor::{{{visitor_trait}, NodeContext, VisitResult}};"
        );
    }

    // When the rust override specifies an `options_type` (e.g. `ConversionOptions`),
    // type annotations are emitted on json_object bindings so that `Default::default()`
    // and `serde_json::from_value(…)` can be resolved without a trailing positional arg.
    // Import the named type so it is in scope in every test function in this file.
    if file_has_call_based {
        let rust_options_type = e2e_config
            .call
            .overrides
            .get("rust")
            .and_then(|o| o.options_type.as_deref());
        if let Some(opts_type) = rust_options_type {
            // Only emit if the call has a json_object arg (the type annotation is only
            // added to json_object bindings).
            let has_json_object_arg = e2e_config.call.args.iter().any(|a| a.arg_type == "json_object");
            if has_json_object_arg {
                let _ = writeln!(out, "use {module}::{opts_type};");
            }
        }
    }

    let _ = writeln!(out);

    for fixture in fixtures {
        render_test_function(&mut out, fixture, e2e_config, dep_name, &field_resolver, client_factory);
        let _ = writeln!(out);
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    dep_name: &str,
    field_resolver: &FieldResolver,
    client_factory: Option<&str>,
) {
    // Http fixtures get their own integration test code path.
    if fixture.http.is_some() {
        render_http_test_function(out, fixture, dep_name);
        return;
    }

    // Fixtures that have neither `http` nor `mock_response` may be either:
    //  - schema/spec validation fixtures (asyncapi, grpc, graphql_schema, …) with no callable
    //    function → emit a TODO stub so the suite compiles and preserves test count.
    //  - plain function-call fixtures (e.g. kreuzberg::extract_file) with a configured
    //    `[e2e.call]` → fall through to the real function-call code path below.
    if fixture.http.is_none() && fixture.mock_response.is_none() {
        let call_config = e2e_config.resolve_call(fixture.call.as_deref());
        let resolved_fn_name = resolve_function_name_for_call(call_config);
        if resolved_fn_name.is_empty() {
            let fn_name = crate::escape::sanitize_ident(&fixture.id);
            let description = &fixture.description;
            let _ = writeln!(out, "#[tokio::test]");
            let _ = writeln!(out, "async fn test_{fn_name}() {{");
            let _ = writeln!(out, "    // {description}");
            let _ = writeln!(
                out,
                "    // TODO: implement when a callable API is available for this fixture type."
            );
            let _ = writeln!(out, "}}");
            return;
        }
        // Non-empty function name: fall through to emit a real function call below.
    }

    let fn_name = crate::escape::sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = resolve_function_name_for_call(call_config);
    let module = resolve_module_for_call(call_config, dep_name);
    let result_var = &call_config.result_var;
    let has_mock = fixture.mock_response.is_some();

    // Resolve Rust-specific overrides early since we need them for returns_result.
    let rust_overrides = call_config.overrides.get("rust");

    // Determine if this call returns Result<T, E>. Per-rust override takes precedence.
    // When client_factory is set, methods always return Result<T>.
    let returns_result = rust_overrides
        .and_then(|o| o.returns_result)
        .unwrap_or(if client_factory.is_some() {
            true
        } else {
            call_config.returns_result
        });

    // Tests with a mock server are always async (Axum requires a Tokio runtime).
    let is_async = call_config.r#async || has_mock;
    if is_async {
        let _ = writeln!(out, "#[tokio::test]");
        let _ = writeln!(out, "async fn test_{fn_name}() {{");
    } else {
        let _ = writeln!(out, "#[test]");
        let _ = writeln!(out, "fn test_{fn_name}() {{");
    }
    let _ = writeln!(out, "    // {description}");

    // Emit mock server setup before building arguments so arg expressions can
    // reference `mock_server.url` when needed.
    if has_mock {
        render_mock_server_setup(out, fixture, e2e_config);
    }

    // Check if any assertion is an error assertion.
    let has_error_assertion = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Extract additional overrides for argument shaping.
    let wrap_options_in_some = rust_overrides.is_some_and(|o| o.wrap_options_in_some);
    let extra_args: Vec<String> = rust_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    // options_type from the rust override (e.g. "ConversionOptions") — used to annotate
    // `Default::default()` and `serde_json::from_value(…)` bindings so Rust can infer
    // the concrete type without a trailing positional argument to guide inference.
    let options_type: Option<String> = rust_overrides.and_then(|o| o.options_type.clone());

    // When the fixture declares a visitor that is passed via an options-field (the
    // html-to-markdown core `convert` API accepts visitor only through
    // `ConversionOptions.visitor`), the options binding must be `mut` so we can
    // assign the visitor field before the call.
    let visitor_via_options = fixture.visitor.is_some() && rust_overrides.is_none_or(|o| o.visitor_function.is_none());

    // Emit input variable bindings from args config.
    let mut arg_exprs: Vec<String> = Vec::new();
    // Track the name of the json_object options arg so we can inject the visitor later.
    let mut options_arg_name: Option<String> = None;
    for arg in &call_config.args {
        let value = crate::codegen::resolve_field(&fixture.input, &arg.field);
        let var_name = &arg.name;
        let (mut bindings, expr) = render_rust_arg(
            var_name,
            value,
            &arg.arg_type,
            arg.optional,
            &module,
            &fixture.id,
            if has_mock {
                Some("mock_server.url.as_str()")
            } else {
                None
            },
            arg.owned,
            arg.element_type.as_deref(),
        );
        // Add explicit type annotation to json_object bindings so Rust can resolve
        // `Default::default()` and `serde_json::from_value(…)` without a trailing
        // positional argument to guide inference.
        if arg.arg_type == "json_object" {
            if let Some(ref opts_type) = options_type {
                bindings = bindings
                    .into_iter()
                    .map(|b| {
                        // `let {name} = …` → `let {name}: {opts_type} = …`
                        let prefix = format!("let {var_name} = ");
                        if b.starts_with(&prefix) {
                            format!("let {var_name}: {opts_type} = {}", &b[prefix.len()..])
                        } else {
                            b
                        }
                    })
                    .collect();
            }
        }
        // When the visitor will be injected via the options field, the options binding
        // must be declared `mut` so we can assign `options.visitor = Some(visitor)`.
        if visitor_via_options && arg.arg_type == "json_object" {
            options_arg_name = Some(var_name.clone());
            bindings = bindings
                .into_iter()
                .map(|b| {
                    // `let {name}` → `let mut {name}`
                    let prefix = format!("let {var_name}");
                    if b.starts_with(&prefix) {
                        format!("let mut {}", &b[4..])
                    } else {
                        b
                    }
                })
                .collect();
        }
        for binding in &bindings {
            let _ = writeln!(out, "    {binding}");
        }
        // For functions whose options slot is owned `Option<T>` rather than `&T`,
        // wrap the json_object expression in `Some(...).clone()` so it matches
        // the parameter shape. Other arg types pass through unchanged.
        let final_expr = if wrap_options_in_some && arg.arg_type == "json_object" {
            if visitor_via_options {
                // Visitor will be injected into options before the call; pass by move
                // (no .clone() needed).
                let name = if let Some(rest) = expr.strip_prefix('&') {
                    rest.to_string()
                } else {
                    expr.clone()
                };
                format!("Some({name})")
            } else if let Some(rest) = expr.strip_prefix('&') {
                format!("Some({rest}.clone())")
            } else {
                format!("Some({expr})")
            }
        } else {
            expr
        };
        arg_exprs.push(final_expr);
    }

    // Emit visitor if present in fixture.
    if let Some(visitor_spec) = &fixture.visitor {
        // HtmlVisitor requires `std::fmt::Debug`; derive it on the inline struct.
        let _ = writeln!(out, "    #[derive(Debug)]");
        let _ = writeln!(out, "    struct _TestVisitor;");
        let _ = writeln!(out, "    impl {} for _TestVisitor {{", resolve_visitor_trait(&module));
        for (method_name, action) in &visitor_spec.callbacks {
            emit_rust_visitor_method(out, method_name, action);
        }
        let _ = writeln!(out, "    }}");
        let _ = writeln!(
            out,
            "    let visitor = std::rc::Rc::new(std::cell::RefCell::new(_TestVisitor));"
        );
        if visitor_via_options {
            // Inject the visitor via the options field rather than as a positional arg.
            let opts_name = options_arg_name.as_deref().unwrap_or("options");
            let _ = writeln!(out, "    {opts_name}.visitor = Some(visitor);");
        } else {
            // Binding uses a visitor_function override that takes visitor as positional arg.
            arg_exprs.push("Some(visitor)".to_string());
        }
    } else {
        // No fixture-supplied visitor: append any extra positional args declared in
        // the rust override (e.g. trailing `None` for an Option<VisitorParam> slot).
        arg_exprs.extend(extra_args);
    }

    let args_str = arg_exprs.join(", ");

    let await_suffix = if is_async { ".await" } else { "" };

    // When client_factory is configured, emit a `create_client` call and dispatch
    // methods on the returned client object instead of calling free functions.
    // The mock server URL (when present) is passed as `base_url`; otherwise `None`.
    let call_expr = if let Some(factory) = client_factory {
        let base_url_arg = if has_mock {
            "Some(mock_server.url.clone())"
        } else {
            "None"
        };
        let _ = writeln!(
            out,
            "    let client = {module}::{factory}(\"test-key\".to_string(), {base_url_arg}, None, None, None).unwrap();"
        );
        format!("client.{function_name}({args_str})")
    } else {
        format!("{function_name}({args_str})")
    };

    let result_is_tree = call_config.result_var == "tree";
    // When the call config or rust override sets result_is_simple, the function
    // returns a plain type (String, Vec<T>, etc.) — field-access assertions use
    // the result var directly.
    let result_is_simple = call_config.result_is_simple || rust_overrides.is_some_and(|o| o.result_is_simple);
    // When result_is_vec is set, the function returns Vec<T>. Field-path assertions
    // are wrapped in `.iter().all(|r| ...)` so every element is checked.
    let result_is_vec = rust_overrides.is_some_and(|o| o.result_is_vec);
    // When result_is_option is set, the function returns Option<T>. Field-path
    // assertions unwrap first via `.as_ref().expect("Option should be Some")`.
    let result_is_option = call_config.result_is_option || rust_overrides.is_some_and(|o| o.result_is_option);

    if has_error_assertion {
        let _ = writeln!(out, "    let {result_var} = {call_expr}{await_suffix};");
        // Check if any assertion is NOT an error assertion (i.e., accesses fields on the Ok value).
        let has_non_error_assertions = fixture
            .assertions
            .iter()
            .any(|a| !matches!(a.assertion_type.as_str(), "error" | "not_error"));
        // When returns_result=true and there are field assertions (non-error), we need to
        // handle the Result wrapper: unwrap Ok for field assertions, extract Err for error assertions.
        if returns_result && has_non_error_assertions {
            // Emit a temporary binding for the unwrapped Ok value.
            let _ = writeln!(out, "    let {result_var}_ok = {result_var}.as_ref().ok();");
        }
        // Render error assertions.
        for assertion in &fixture.assertions {
            render_assertion(
                out,
                assertion,
                result_var,
                &module,
                dep_name,
                true,
                &[],
                field_resolver,
                result_is_tree,
                result_is_simple,
                false,
                false,
                returns_result,
            );
        }
        let _ = writeln!(out, "}}");
        return;
    }

    // Non-error path: unwrap the result.
    let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");

    // Check if any assertion actually uses the result variable.
    // If all assertions are skipped (field not on result type), use `_` to avoid
    // Rust's "variable never used" warning.
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        if a.assertion_type == "method_result" {
            // method_result assertions that would generate only a TODO comment don't use the
            // result variable. These are: missing `method` field, or unsupported `check` type.
            let supported_checks = [
                "equals",
                "is_true",
                "is_false",
                "greater_than_or_equal",
                "count_min",
                "is_error",
                "contains",
                "not_empty",
                "is_empty",
            ];
            let check = a.check.as_deref().unwrap_or("is_true");
            if a.method.is_none() || !supported_checks.contains(&check) {
                return false;
            }
        }
        match &a.field {
            Some(f) if !f.is_empty() => field_resolver.is_valid_for_result(f),
            _ => true,
        }
    });

    let result_binding = if has_usable_assertion {
        result_var.to_string()
    } else {
        "_".to_string()
    };

    // Detect Option-returning functions: only skip unwrap when ALL assertions are
    // pure emptiness/bool checks with NO field access (is_none/is_some on the result itself).
    // If any assertion accesses a field (e.g. `html`), we need the inner value, so unwrap.
    let has_field_access = fixture
        .assertions
        .iter()
        .any(|a| a.field.as_ref().is_some_and(|f| !f.is_empty()));
    let only_emptiness_checks = !has_field_access
        && fixture.assertions.iter().all(|a| {
            matches!(
                a.assertion_type.as_str(),
                "is_empty" | "is_false" | "not_empty" | "is_true" | "not_error"
            )
        });

    let unwrap_suffix = if returns_result {
        ".expect(\"should succeed\")"
    } else {
        ""
    };
    if !returns_result || (only_emptiness_checks && !has_not_error) {
        // Option-returning or non-Result-returning (and not a not_error check): bind raw value, no unwrap.
        // When returns_result=true and has_not_error, fall through to emit .expect() so errors panic.
        let _ = writeln!(out, "    let {result_binding} = {call_expr}{await_suffix};");
    } else if has_not_error || !fixture.assertions.is_empty() {
        let _ = writeln!(
            out,
            "    let {result_binding} = {call_expr}{await_suffix}{unwrap_suffix};"
        );
    } else {
        let _ = writeln!(out, "    let {result_binding} = {call_expr}{await_suffix};");
    }

    // Emit Option field unwrap bindings for any fields accessed in assertions.
    // Use FieldResolver to handle optional fields, including nested/aliased paths.
    // Skipped when the call returns Vec<T>: per-element iteration is emitted by
    // `render_assertion` itself, so the call-site has no single result struct
    // to unwrap fields off of.
    let string_assertion_types = [
        "equals",
        "contains",
        "contains_all",
        "contains_any",
        "not_contains",
        "starts_with",
        "ends_with",
        "min_length",
        "max_length",
        "matches_regex",
    ];
    let mut unwrapped_fields: Vec<(String, String)> = Vec::new(); // (fixture_field, local_var)
    if !result_is_vec {
        for assertion in &fixture.assertions {
            if let Some(f) = &assertion.field {
                if !f.is_empty()
                    && string_assertion_types.contains(&assertion.assertion_type.as_str())
                    && !unwrapped_fields.iter().any(|(ff, _)| ff == f)
                {
                    // Only unwrap optional string fields — numeric optionals (u64, usize)
                    // don't support .as_deref() and should be compared directly.
                    let is_string_assertion = assertion.value.as_ref().is_none_or(|v| v.is_string());
                    if !is_string_assertion {
                        continue;
                    }
                    if let Some((binding, local_var)) = field_resolver.rust_unwrap_binding(f, result_var) {
                        let _ = writeln!(out, "    {binding}");
                        unwrapped_fields.push((f.clone(), local_var));
                    }
                }
            }
        }
    }

    // Render assertions.
    for assertion in &fixture.assertions {
        if assertion.assertion_type == "not_error" {
            // Already handled by .expect() above.
            continue;
        }
        render_assertion(
            out,
            assertion,
            result_var,
            &module,
            dep_name,
            false,
            &unwrapped_fields,
            field_resolver,
            result_is_tree,
            result_is_simple,
            result_is_vec,
            result_is_option,
            returns_result,
        );
    }

    let _ = writeln!(out, "}}");
}

/// Collect test file names for use in build.zig and similar build scripts.
pub fn collect_test_filenames(groups: &[FixtureGroup]) -> Vec<String> {
    groups
        .iter()
        .filter(|g| !g.fixtures.is_empty())
        .map(|g| format!("{}_test.rs", sanitize_filename(&g.category)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_module_for_call_prefers_crate_name_override() {
        use crate::config::CallConfig;
        use std::collections::HashMap;
        let mut overrides = HashMap::new();
        overrides.insert(
            "rust".to_string(),
            crate::config::CallOverride {
                crate_name: Some("custom_crate".to_string()),
                module: Some("ignored_module".to_string()),
                ..Default::default()
            },
        );
        let call = CallConfig {
            overrides,
            ..Default::default()
        };
        let result = resolve_module_for_call(&call, "dep_name");
        assert_eq!(result, "custom_crate");
    }
}
