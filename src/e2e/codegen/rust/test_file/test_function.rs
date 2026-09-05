//! Rust e2e test-function rendering.

use std::fmt::Write as FmtWrite;

use heck::ToSnakeCase;

use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;

use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::codegen::rust::args::{
    emit_rust_visitor_method, render_rust_arg, resolve_handle_config_type, resolve_visitor_trait,
};
use crate::e2e::codegen::rust::assertions::render_assertion;
use crate::e2e::codegen::rust::http::render_http_test_function;
use crate::e2e::codegen::rust::mock_server::render_mock_server_setup;

use super::helpers::{resolve_function_name_for_call, resolve_module_for_call};

/// The local a streaming call's drained items are bound to.
///
/// ~keep Exported because a streaming call binds this, and never `result_var`: the raw stream
/// goes to `stream` and is immediately consumed by the collect snippet, so `result` exists in
/// no streaming body. The docs-snippet renderer has to present *this* name, and re-deriving
/// the string there is exactly how the two generators end up disagreeing about which binding
/// the snippet's tail may reference (`E0425`).
pub(crate) const STREAMING_COLLECTED_VAR: &str = "chunks";

/// Argument list appended to a `client_factory` call when the project configures no
/// `[e2e.call.overrides.rust] client_factory_trailing_args`.
///
/// These three `None`s were hardcoded into the emitted line before the override was
/// wired up, so they remain the default: shortening the call for every existing
/// project would break each one whose factory takes more than a key and a base URL.
const RUST_CLIENT_FACTORY_FALLBACK_ARGS: [&str; 3] = ["None", "None", "None"];

#[allow(clippy::too_many_arguments)]
pub fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
    dep_name: &str,
    client_factory: Option<&str>,
    docs_client: Option<&crate::e2e::fixture::FixtureDocsClient>,
    snippet_expects_error: bool,
) {
    // Http fixtures get their own integration test code path.
    if fixture.http.is_some() {
        render_http_test_function(out, fixture, dep_name);
        return;
    }

    // Fixtures that have neither `http` nor `mock_response` may be either:
    //  - schema/spec validation fixtures (asyncapi, grpc, graphql_schema, …) with no callable
    //    function → emit an unsupported stub so the suite compiles and preserves test count.
    //  - plain function-call fixtures (e.g. demo_crate::extract_file) with a configured
    //    `[e2e.call]` → fall through to the real function-call code path below.
    if fixture.http.is_none() && fixture.mock_response.is_none() {
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let resolved_fn_name = resolve_function_name_for_call(call_config);
        if resolved_fn_name.is_empty() {
            let fn_name = crate::e2e::escape::sanitize_ident(&fixture.id);
            let description = &fixture.description;
            let _ = writeln!(out, "#[test]");
            let _ = writeln!(out, "fn test_{fn_name}() {{");
            let _ = writeln!(out, "    // {description}");
            let _ = writeln!(
                out,
                "    // Unsupported: no callable API is configured for this fixture type."
            );
            let _ = writeln!(out, "}}");
            return;
        }
        // Non-empty function name: fall through to emit a real function call below.
    }

    let fn_name = crate::e2e::escape::sanitize_ident(&fixture.id);
    let description = &fixture.description;
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // Per-call field resolver: overrides the file-level resolver when this call
    // declares its own result_fields / fields / fields_optional / fields_array /
    // fields_method_calls.
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    // The call's declared Rust result type, resolved from the IR itself (not a hand-configured
    // override) — anchors the IR-derived enum classification below at the exact struct/enum
    // this call returns, so a field name that means different things on different types (e.g.
    // `kind: String` on one struct, `kind: SomeEnum` on another) is never conflated. ~keep
    let call_ir = CallIr { functions, type_defs };
    let call_root_type = resolve_declared_result_type(call_config, "rust", call_ir);
    let call_field_resolver = FieldResolver::new_with_error_aliases(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
        &e2e_config.error_field_aliases,
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_enum_fields(e2e_config.effective_fields_enum(call_config).clone())
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type.clone())
    .with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), call_root_type.clone())
    .with_ir_result_fields(FieldResolver::ir_result_field_facts(type_defs, "rust"), call_root_type)
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields)
    .with_result_is_byte_payload(call_config.effective_result_is_bytes("rust"))
    // `with_ir_fields` only ever proves a bare field name optional (e.g. "chunks"), by
    // unanimity across every declaration of that name in the crate — it has no path context,
    // so a renderer walking a nested path like "results[0].chunks" never matches it once the
    // path crosses more than one segment (`optional_fields` is keyed by the FULL cumulative
    // path the `_with_optionals` renderers build, not by bare name). `presentation.rs` already
    // solved this for doc snippets by anchoring the exact paths about to render, via the IR's
    // own (owner_type, field_name) walk (`with_anchored_optional_paths`); this fixture's own
    // assertion field paths are the equivalent set for a generated test body. Without this, a
    // `results[0].chunks: Option<Vec<Chunk>>` segment field indexed as `results[0].chunks[0]`
    // or measured as `results[0].chunks.len()` renders unwrapped against the `Option`, and the
    // generated Rust fails to compile (`E0608`/`E0624`). ~keep
    .with_anchored_optional_paths(fixture.assertions.iter().filter_map(|a| a.field.as_deref()));
    let field_resolver = &call_field_resolver;
    let function_name = resolve_function_name_for_call(call_config);
    let function_name_snake = function_name.to_snake_case();
    let module = resolve_module_for_call(call_config, dep_name);
    let result_var = call_config.effective_result_var();
    let has_mock = fixture.mock_response.is_some();

    // Resolve Rust-specific overrides early since we need them for returns_result.
    let call_recipe = crate::e2e::codegen::recipe::E2eCallRecipe::resolve("rust", fixture, call_config, type_defs);
    let rust_overrides = call_recipe.override_config;

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
    let _ = writeln!(out, "#[test]");
    let _ = writeln!(out, "fn test_{fn_name}() {{");
    if is_async {
        let _ = writeln!(out, "    common::runtime().block_on(async {{");
    }
    let _ = writeln!(out, "    // {description}");

    // Render the rest of the function body into a side buffer so we can post-process
    // the `mock_server` binding name: if the body never reads `mock_server.url` (typical
    // for error-path fixtures that only need the server held alive via Drop), we rename
    // the binding to `_mock_server` to silence `-D unused_variables`. The original `out`
    // is renamed to `final_out`; the inner code below writes to `out` (the body buffer).
    let final_out = out;
    let mut body_buf = String::new();
    let out = &mut body_buf;

    // Check if any assertion is an error assertion.
    let has_error_assertion = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Extract additional overrides for argument shaping.
    let wrap_options_in_some = rust_overrides.is_some_and(|o| o.wrap_options_in_some);
    let extra_args: Vec<String> = call_recipe.extra_args.to_vec();
    // options_type from the rust override — used to annotate
    // `Default::default()` and `serde_json::from_value(…)` bindings so Rust can infer
    // the concrete type without a trailing positional argument to guide inference.
    let options_type: Option<String> = call_recipe.options_type.map(str::to_string);

    // When the fixture declares a visitor that is passed via an options-field (the
    // visitor-special APIs can accept visitor only through an options field; in
    // that case the options binding must be `mut` so we can
    // assign the visitor field before the call.
    let visitor_via_options = fixture.visitor.is_some() && rust_overrides.is_none_or(|o| o.visitor_function.is_none());

    // Emit input variable bindings from args config.
    let mut arg_exprs: Vec<String> = Vec::new();
    // Track the name of the json_object options arg so we can inject the visitor later.
    let mut options_arg_name: Option<String> = None;
    // When has_error_assertion is true and a handle arg is present, track its name so we
    // can wrap the main call in a match that propagates engine-creation failures as Err.
    let mut error_context_handle_name: Option<String> = None;
    for arg in fixture.resolved_args(call_config) {
        let value = crate::e2e::codegen::resolve_field(&fixture.input, &arg.field);
        let var_name = &arg.name;

        // Handle test_backend args: generate trait stub struct and instance.
        if arg.arg_type == "test_backend"
            && let Some(trait_name) = &arg.trait_name
        {
            // Find the matching trait bridge config.
            if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                // Resolve methods for this trait bridge by looking up the trait in type_defs.
                let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                    .iter()
                    .find(|t| t.name == *trait_name)
                    .map(|t| t.methods.iter().collect())
                    .unwrap_or_default();
                // Emit the test backend stub.
                let emission = crate::e2e::codegen::emit_test_backend("rust", trait_bridge, &methods, fixture, &[], "");
                let expr = emission.arg_expr;
                // Emit `use module::Symbol;` imports inside the function body so that
                // the trait name and any named return/param types are in scope for the
                // `impl TraitName for Stub` block that follows.
                for symbol in &emission.type_imports {
                    let _ = writeln!(out, "    #[allow(unused_imports)]");
                    let _ = writeln!(out, "    use {module}::{symbol};");
                }
                // Emit the stub struct + impl block, indenting every line so that the
                // local struct definition sits inside the test function body.
                for line in emission.setup_block.lines() {
                    if line.is_empty() {
                        let _ = writeln!(out);
                    } else {
                        let _ = writeln!(out, "    {line}");
                    }
                }
                arg_exprs.push(expr);
                continue;
            } else {
                // ~keep An unregistered trait has no vtable/stub to construct: falling
                // through to render the raw (typically null) fixture value as an
                // ordinary arg used to compile only by accident (or not at all), failing
                // at a later `cargo build` far from the fixture that caused it, instead
                // of at `alef e2e generate` time. `c/assertions.rs`'s
                // `build_args_string_c` already fails loudly here for the identical
                // scenario — mirror it (alef task #81).
                panic!(
                    "Rust e2e generator: fixture `{}` requires trait `{trait_name}` for its \
                     `test_backend` arg `{}`, but no `[[crates.trait_bridges]]` entry named \
                     `{trait_name}` is configured",
                    fixture.id, arg.name
                );
            }
        }

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
            &e2e_config.test_documents_dir,
            has_error_assertion,
            resolve_handle_config_type(arg, call_recipe.options_type, type_defs).as_deref(),
            &fixture.docs_files_for_arg(&arg.field),
            arg.vec_inner_is_ref,
            fixture.preserve_input_urls,
        );
        // Add explicit type annotation to json_object bindings so Rust can resolve
        // `Default::default()` and `serde_json::from_value(…)` without a trailing
        // positional argument to guide inference.
        // Skip when the arg has an element_type: render_json_object_arg already emits
        // `serde_json::from_value::<Vec<ElementType>>(…)`, so annotating with options_type
        // would produce a mismatched-types error (e.g. `let actions: CrawlConfig = … Vec<PageAction>`).
        if arg.arg_type == "json_object"
            && arg.element_type.is_none()
            && let Some(ref opts_type) = options_type
        {
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
        // When in error context and the arg is a handle, the binding emitted by
        // render_rust_arg is `{name}_result` (a Result). Track the handle name so
        // the error-context call site can emit a match wrapper.
        if has_error_assertion && arg.arg_type == "handle" {
            error_context_handle_name = Some(var_name.clone());
        }
        for binding in &bindings {
            let _ = writeln!(out, "    {binding}");
        }
        // For functions whose options slot is owned `Option<T>` rather than `&T`,
        // wrap the json_object expression in `Some(...).clone()` so it matches
        // the parameter shape. Other arg types pass through unchanged.
        let final_expr = if has_error_assertion && arg.arg_type == "handle" {
            // In error context, the handle binding is `{name}_result` (a Result).
            // The actual call will be emitted inside a `match {name}_result { Ok({name}) => ...}`
            // wrapper, so the call expression still uses `&{name}` (the unwrapped handle).
            format!("&{var_name}")
        } else if wrap_options_in_some && arg.arg_type == "json_object" {
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
        // The visitor trait name must be configured via
        // `[e2e.call.overrides.rust] visitor_trait`; we propagate the error to the caller.
        let visitor_trait = resolve_visitor_trait(rust_overrides)
            .expect("visitor_trait must be set in [e2e.call.overrides.rust] when a fixture declares a visitor block");
        // The visitor trait requires `std::fmt::Debug`; derive it on the inline struct.
        let _ = writeln!(out, "    #[derive(Debug)]");
        let _ = writeln!(out, "    struct _TestVisitor;");
        let _ = writeln!(out, "    impl {visitor_trait} for _TestVisitor {{");
        for (method_name, action) in &visitor_spec.callbacks {
            emit_rust_visitor_method(out, method_name, action);
        }
        let _ = writeln!(out, "    }}");
        let _ = writeln!(
            out,
            "    let visitor = std::sync::Arc::new(std::sync::Mutex::new(_TestVisitor));"
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
    let call_expr = if let Some(factory) = client_factory {
        // `docs_client` is `None` on the e2e path, so the mock server keeps the
        // base-URL slot there no matter what a fixture's docs metadata says. ~keep
        let base_url_arg = match crate::e2e::codegen::client_factory::docs_base_url(docs_client) {
            Some(url) => format!("Some(\"{}\".to_string())", crate::e2e::escape::escape_rust(url)),
            None if has_mock => "Some(mock_server.url.clone())".to_string(),
            None => "None".to_string(),
        };
        let trailing = crate::e2e::codegen::client_factory::trailing_args(
            docs_client,
            e2e_config,
            call_config,
            "rust",
            &RUST_CLIENT_FACTORY_FALLBACK_ARGS,
        );
        let trailing: String = trailing.iter().map(|arg| format!(", {arg}")).collect();
        let _ = writeln!(
            out,
            "    let client = {module}::{factory}(\"test-key\".to_string(), {base_url_arg}{trailing}).unwrap();"
        );
        format!("client.{function_name}({args_str})")
    } else {
        format!("{function_name}({args_str})")
    };

    let result_is_tree = call_config.effective_result_var() == "tree";
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
        // When a handle (engine) arg is present, the engine creation itself may fail.
        // Wrap the primary call in a match so engine-creation errors propagate as Err
        // instead of panicking via .expect().
        if let Some(ref handle_name) = error_context_handle_name {
            let _ = writeln!(out, "    let {result_var} = match {handle_name}_result {{");
            let _ = writeln!(out, "        Err(e) => Err(e),");
            let _ = writeln!(out, "        Ok({handle_name}) => {{");
            let _ = writeln!(out, "            {call_expr}{await_suffix}");
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "    }};");
        } else {
            let _ = writeln!(out, "    let {result_var} = {call_expr}{await_suffix};");
        }
        // Check if any assertion accesses fields on the Ok value (not error-path fields).
        // Assertions on `error.*` fields access the Err value and do not need `result_ok`.
        let has_non_error_assertions = fixture.assertions.iter().any(|a| {
            !matches!(a.assertion_type.as_str(), "error" | "not_error")
                && !a.field.as_ref().is_some_and(|f| f.starts_with("error."))
        });
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
                None,
            );
        }
        if is_async {
            let _ = writeln!(out, "    }});");
        }
        let _ = writeln!(out, "}}");
        finalize_test_body(final_out, fixture, e2e_config, has_mock, &body_buf);
        return;
    }

    // Non-error path: unwrap the result.
    let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");

    // Streaming detection (call-level `streaming` opt-out is honored).
    // Shared helper auto-detects from unambiguous streaming-only field names
    // (chunks, stream_content, …) — but not from ambiguous fields like `usage`
    // or `finish_reason` that also exist on non-streaming response shapes.
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    let streaming_item_type = is_streaming
        .then(|| {
            crate::e2e::codegen::recipe::streaming_item_type(
                call_config,
                &config.adapters,
                &[function_name.as_str(), function_name_snake.as_str()],
            )
        })
        .flatten();
    // Name of the stream-level variable (the raw stream returned by the call).
    let stream_var = "stream";
    let chunks_var = STREAMING_COLLECTED_VAR;

    // Check if any assertion actually uses the result variable.
    // If all assertions are skipped (field not on result type), use `_` to avoid
    // Rust's "variable never used" warning.
    // For streaming fixtures: streaming virtual fields count as usable — they resolve
    // against the `chunks` collected variable rather than the result type.
    let has_usable_assertion = fixture.assertions.iter().any(|a| {
        if a.assertion_type == "not_error" || a.assertion_type == "error" {
            return false;
        }
        if a.assertion_type == "method_result" {
            // method_result assertions that would generate only an unsupported comment don't use the
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
            Some(f) if !f.is_empty() => {
                if is_streaming && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) {
                    return true;
                }
                // ~keep The renderer's own answer, not a second derivation of it: an assertion
                // `render_assertion` will replace with a skip marker uses no result variable, and
                // a `result_is_simple` call whose result is fieldless is exactly the case the
                // blanket `return true` below gets wrong -- binding `result` for assertions that
                // all skip leaves it unused, which the generated crate denies.
                if crate::e2e::codegen::rust::fieldless_result::unrenderable_field_skip(
                    f,
                    result_var,
                    result_is_simple,
                    field_resolver,
                )
                .is_some()
                {
                    return false;
                }
                // When the call returns a plain type (result_is_simple) or a Tree, the
                // assertion renderer resolves field-bearing assertions against the result
                // variable itself (`render_count_equals_assertion` emits `result.len()`,
                // tree pseudo-fields map to `result.root_node()`, …) rather than treating
                // the field as a struct member. In that case the assertion always uses the
                // result variable regardless of whether `f` is a real field on the type, so
                // the call must bind to `result` and not `_`.
                if result_is_simple || result_is_tree {
                    return true;
                }
                field_resolver.is_valid_for_result(f)
            }
            _ => true,
        }
    });

    // For streaming fixtures the stream itself is stored in `stream` and the
    // collected list in `chunks`.  Non-streaming fixtures use result_var / `_`.
    // A snippet whose fixture asserts `error` renders the `Result` itself with a `match`,
    // so the call must bind to a named variable even though the snippet path cleared the
    // fixture's assertions before getting here. ~keep
    let result_binding = if is_streaming {
        stream_var.to_string()
    } else if snippet_expects_error || has_usable_assertion || fixture.has_docs_presentation() {
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

    // `.expect` would abort the snippet on the very failure the example is documenting,
    // so the error-handling snippet keeps the `Result` intact for its own `match`. ~keep
    let unwrap_suffix = if returns_result && !snippet_expects_error {
        ".expect(\"call failed\")"
    } else {
        ""
    };
    if is_streaming {
        // Streaming: bind the raw stream, then drain it into a Vec.
        let _ = writeln!(out, "    let {stream_var} = {call_expr}{await_suffix}{unwrap_suffix};");
        if let Some(collect) = crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::collect_snippet(
            "rust", stream_var, chunks_var,
        ) {
            let _ = writeln!(out, "    {collect}");
        }
    } else if !returns_result || (only_emptiness_checks && !has_not_error && !fixture.has_docs_presentation()) {
        // Option-returning or non-Result-returning (and not a not_error check): bind raw value, no unwrap.
        // When returns_result=true and has_not_error, fall through to emit .expect() so errors panic.
        let _ = writeln!(out, "    let {result_binding} = {call_expr}{await_suffix};");
    } else if has_not_error || !fixture.assertions.is_empty() || fixture.has_docs_presentation() {
        let _ = writeln!(
            out,
            "    let {result_binding} = {call_expr}{await_suffix}{unwrap_suffix};"
        );
    } else {
        let _ = writeln!(out, "    let {result_binding} = {call_expr}{await_suffix};");
    }

    // Emit local bindings for fields_array fields that are referenced in assertions
    // AND whose names collide with streaming-virtual field names (e.g. `chunks`,
    // `imports`, `structure`).  Without this, the streaming-virtual-field arm in
    // `render_assertion` fires unconditionally on those field names — even for
    // non-streaming fixtures — and emits `assert!(chunks.len() >= 2 as usize, ...)`
    // referencing an undeclared variable.  Binding `let chunks = &result.chunks;`
    // here makes the hardcoded `chunks` identifier in that arm resolve correctly.
    //
    // Only emit for non-streaming fixtures: streaming fixtures already drain the
    // stream into a `chunks: Vec<_>` local via the collect snippet.
    if !is_streaming {
        let mut emitted_array_bindings: std::collections::HashSet<String> = std::collections::HashSet::new();
        for assertion in &fixture.assertions {
            if let Some(f) = &assertion.field
                && !f.is_empty()
                && field_resolver.is_array(f)
                && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
                && !emitted_array_bindings.contains(f.as_str())
            {
                let accessor = field_resolver.accessor(f, "rust", result_var);
                let _ = writeln!(out, "    let {f} = &{accessor};");
                emitted_array_bindings.insert(f.clone());
            }
        }
    }

    // Emit Option field unwrap bindings for any fields accessed in assertions.
    // Use FieldResolver to handle optional fields, including nested/aliased paths.
    // Skipped when the call returns Vec<T>: per-element iteration is emitted by
    // `render_assertion` itself, so the call-site has no single result struct
    // to unwrap fields off of. Also skipped for streaming fixtures: `result_var` (e.g.
    // "result") is never bound in the streaming arm above -- only `stream`/`chunks` are --
    // so unconditionally unwrapping `result.<field>` here emitted a reference to an
    // undeclared variable (E0425) whenever a streaming fixture asserted a string field.
    // Mirrors the `!is_streaming` guard already on the array-binding loop just above. ~keep
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
    // ~keep A fieldless result (a raw byte payload) has no member to bind, so every binding this
    // loop could emit would be `let <local> = result.<field>...` -- the same `E0609` the assertion
    // guard now refuses, reached through a different door. The assertions these bindings exist for
    // are skipped in that case, so suppressing the binding drops nothing.
    if !result_is_vec && !is_streaming && !field_resolver.result_has_no_fields() {
        for assertion in &fixture.assertions {
            if let Some(f) = &assertion.field
                && !f.is_empty()
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
            streaming_item_type,
        );
    }

    if is_async {
        let _ = writeln!(out, "    }});");
    }
    let _ = writeln!(out, "}}");
    finalize_test_body(final_out, fixture, e2e_config, has_mock, &body_buf);
}

/// Emit mock-server setup (if needed) and the rendered body to the test file output.
///
/// The body buffer is scanned for references to `mock_server.` to decide the binding name:
/// if any non-setup reference exists, the binding is `mock_server`; otherwise it is
/// `_mock_server` to silence `-D unused_variables` while still keeping the server alive
/// via Drop. Error-path fixtures typically fall into the latter case — they need the
/// server bound to a name but never read `mock_server.url`.
fn finalize_test_body(out: &mut String, fixture: &Fixture, e2e_config: &E2eConfig, has_mock: bool, body: &str) {
    if has_mock {
        let var_name = if body.contains("mock_server.") {
            "mock_server"
        } else {
            "_mock_server"
        };
        render_mock_server_setup(out, fixture, e2e_config, var_name);
    }
    crate::e2e::codegen::fail_on_unavailable_field_markers(body, "rust", &fixture.id, &fixture.assertions);
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(body, "rust", &fixture.id);
    out.push_str(body);
}

#[cfg(test)]
#[path = "test_function/optional_segment_len_tests.rs"]
mod optional_segment_len_tests;

#[cfg(test)]
#[path = "test_function/ir_only_array_classification_tests.rs"]
mod ir_only_array_classification_tests;

#[cfg(test)]
#[path = "test_function/shared_runtime_tests.rs"]
mod shared_runtime_tests;
