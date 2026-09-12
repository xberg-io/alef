//! Argument binding and setup rendering for generated Python tests.

use std::collections::{HashMap, HashSet};

use heck::ToSnakeCase;

use crate::e2e::codegen::resolve_field;
use crate::e2e::fixture::Fixture;

use super::super::json::json_to_python_literal;
use super::handle_values::build_handle_kwarg_value;
use super::typed_values::{
    ArgSink, ConstructorSpec, KwargRenderContext, LeafSource, MockUrlInfo, emit_bytes_arg, emit_json_object_arg,
};

/// Read-only inputs to [`build_args_and_setup`], bundled because every field is invariant
/// borrowed/`Copy` state the whole per-arg loop reads -- the loop's own accumulators
/// (`arg_bindings`, `kwarg_exprs`, `teardown`, `placeholder_positions`) are mutated every
/// iteration, so they stay ordinary locals rather than fields here, matching the split
/// `KwargRenderContext`/`ArgSink` draw in `typed_values.rs`.
#[derive(Clone, Copy)]
pub(super) struct ArgSetupContext<'a> {
    pub call_config: &'a crate::e2e::config::CallConfig,
    pub options_type: Option<&'a str>,
    pub options_via: &'a str,
    /// Type names whose public Python spelling is `options.py`'s method-less `@dataclass` mirror,
    /// so `from_json` does not exist on them.
    ///
    /// `options_via` is resolved ONCE for the whole call, against the call's options type -- but it
    /// is then applied to every argument, and a call can mix the two spellings. A measured consumer
    /// call does: its config argument resolves to a native extension type (which has `from_json`)
    /// while its input argument resolves to a public `options.py` dataclass mirror (which does
    /// not), and every generated Python e2e test died on `AttributeError: type object '<Input>' has
    /// no attribute 'from_json'`. Consulted per argument so one argument's eligibility cannot
    /// decide another's. ~keep
    pub from_json_unavailable_types: &'a HashSet<String>,
    pub enum_fields: &'a HashMap<String, String>,
    pub handle_nested_types: &'a HashMap<String, String>,
    pub handle_dict_types: &'a HashSet<String>,
    pub config: &'a crate::core::config::ResolvedCrateConfig,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub enums: &'a [crate::core::ir::EnumDef],
}

/// Build arg binding lines and kwarg expressions for a fixture call.
///
/// Returns `(arg_bindings, kwarg_exprs, teardown_block)`. The teardown block
/// contains statements emitted after the fixture call and its assertions —
/// trait-bridge fixtures populate it with `unregister_<trait>("<name>")` so
/// pytest's shared-process registry state is restored between tests.
pub(super) fn build_args_and_setup(
    fixture: &Fixture,
    context: ArgSetupContext<'_>,
) -> (Vec<String>, Vec<String>, String) {
    let ArgSetupContext {
        call_config,
        options_type,
        options_via,
        from_json_unavailable_types,
        enum_fields,
        handle_nested_types,
        handle_dict_types,
        config,
        type_defs,
        enums,
    } = context;

    let mut arg_bindings = Vec::new();
    let mut kwarg_exprs = Vec::new();
    let mut teardown = String::new();
    // Positions in `kwarg_exprs` holding nothing but a `None` placeholder for an absent optional
    // argument. A placeholder is load-bearing only while a real argument follows it; a run of them
    // at the END of the list is pure noise, and the trailing run is the common case -- optional
    // arguments are conventionally declared last, so a fixture supplying none of them rendered
    // `convert(html, None)` where the binding's own signature reads `options=None`. Recorded here
    // rather than inferred afterwards, because a *real* argument whose value is legitimately `None`
    // is indistinguishable from a placeholder once it is in the list. ~keep
    let mut placeholder_positions: HashSet<usize> = HashSet::new();

    for arg in fixture.resolved_args(call_config) {
        let var_name = &arg.name;

        if arg.arg_type == "handle" {
            let handle_context = HandleArgContext {
                fixture,
                arg,
                var_name,
                options_type,
                handle_nested_types,
                handle_dict_types,
            };
            emit_handle_arg(&mut arg_bindings, &mut kwarg_exprs, handle_context);
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name
                && let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name)
            {
                let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                    .iter()
                    .find(|t| t.name == *trait_name)
                    .map(|t| t.methods.iter().collect())
                    .unwrap_or_default();
                let emission = super::super::emit_test_backend(trait_bridge, &methods, fixture);
                arg_bindings.push(emission.setup_block);
                kwarg_exprs.push(emission.arg_expr);
                teardown.push_str(&emission.teardown_block);
                continue;
            }
            // A `test_backend` arg fills a required Python stub parameter — there is
            // no compilable value to fall back to when the trait isn't configured.
            // Fail generation loudly instead of silently splicing a `None` argument
            // with a comment where the real stub belongs. ~keep
            panic!(
                "Python e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate a Python stub without a resolvable trait bridge",
                fixture.id, arg.name, arg.trait_name
            );
        }

        if arg.arg_type == "mock_url" {
            let fixture_id = &fixture.id;
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let value = fixture.input.get(field).unwrap_or(&serde_json::Value::Null);
            let url_expr =
                if let Some(url) = crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value) {
                    format!("\"{}\"", crate::e2e::escape::escape_python(url))
                } else if fixture.has_host_root_route() {
                    format!(
                        "os.environ.get('MOCK_SERVER_{}') or os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'",
                        fixture_id.to_uppercase()
                    )
                } else {
                    format!("os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'")
                };
            arg_bindings.push(format!("    {var_name} = {url_expr}"));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            let fixture_id = &fixture.id;

            // Extract path strings from fixture input array.
            // Try both the declared field and common aliases (batch_urls, urls, etc.)
            //
            // ~keep The preserved-list check must run BEFORE the `{var_name}_base` line
            // below is pushed, not after: every other backend's `mock_url_list` handler
            // (go/setup.rs, java/args.rs, csharp/setup.rs, ...) computes that env-var
            // literal only on the non-preserved path, but this one used to push it
            // unconditionally and just `continue` past it once preserved. A doc snippet
            // never executes the unused `_base` assignment, but it IS published verbatim
            // -- `os.environ['MOCK_SERVER_URL']` in the body is exactly the mock-harness
            // leak `reject_mock_harness_scaffolding` exists to catch, so a fully
            // preserved (docs-safe) fixture still failed the guard on this arg type alone.
            let field_value = crate::e2e::codegen::resolve_urls_field(&fixture.input, &arg.field);
            if let Some(urls) = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, field_value) {
                let urls = urls
                    .iter()
                    .map(|url| format!("\"{}\"", crate::e2e::escape::escape_python(url)))
                    .collect::<Vec<_>>()
                    .join(", ");
                arg_bindings.push(format!("    {var_name} = [{urls}]"));
                kwarg_exprs.push(var_name.to_string());
                continue;
            }

            let base_url_expr = if fixture.has_host_root_route() {
                format!(
                    "os.environ.get('MOCK_SERVER_{}', os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}')",
                    fixture_id.to_uppercase()
                )
            } else {
                format!("os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'")
            };
            arg_bindings.push(format!("    {var_name}_base = {base_url_expr}"));

            let paths: Vec<String> = if let Some(arr) = field_value.as_array() {
                arr.iter()
                    .filter_map(|v| {
                        v.as_str()
                            .map(|s| format!("\"{}\"", crate::e2e::escape::escape_python(s)))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let paths_str = paths.join(", ");

            arg_bindings.push(format!(
                "    {var_name} = [p if p.startswith('http') else f'{{{var_name}_base}}{{p}}' for p in [{paths_str}]]"
            ));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        let value = resolve_field(&fixture.input, &arg.field);

        if value.is_null() && arg.optional {
            // Emit None as a placeholder so subsequent positional args keep their
            // index alignment. With kwarg emission this would just be skipped, but
            // since we emit positional args (commit 40ff92c9), an omitted optional
            // arg in the middle would shift later args into the wrong position.
            placeholder_positions.insert(kwarg_exprs.len());
            kwarg_exprs.push("None".to_string());
            continue;
        }

        if arg.arg_type == "json_object" && !value.is_null() {
            let mut sink = ArgSink {
                bindings: &mut arg_bindings,
                kwarg_exprs: &mut kwarg_exprs,
            };
            let arg_options_type = crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, value);
            let arg_options_via = if options_via == "from_json"
                && arg_options_type.is_some_and(|name| from_json_unavailable_types.contains(name))
            {
                "kwargs"
            } else {
                options_via
            };
            let spec = ConstructorSpec {
                options_type: arg_options_type,
                options_via: arg_options_via,
                element_type: &arg.element_type,
            };
            let mock = MockUrlInfo {
                fixture_id: &fixture.id,
                has_host_root_route: fixture.has_host_root_route(),
            };
            let docs_files = fixture.docs_files_for_arg(&arg.field);
            let context = KwargRenderContext {
                type_defs,
                enums,
                enum_fields,
                docs_files: &docs_files,
                leaf_source: LeafSource::Literal,
            };
            if emit_json_object_arg(&mut sink, value, var_name, &spec, &mock, context) {
                continue;
            }
        }

        if arg.optional && value.is_null() {
            continue;
        }

        if value.is_null() && !arg.optional {
            let default_val = match arg.arg_type.as_str() {
                "string" => "\"\"".to_string(),
                "int" | "integer" => "0".to_string(),
                "float" | "number" => "0.0".to_string(),
                "bool" | "boolean" => "False".to_string(),
                _ => "None".to_string(),
            };
            arg_bindings.push(format!("    {var_name} = {default_val}"));
            kwarg_exprs.push(var_name.to_string());
            continue;
        }

        if arg.arg_type == "bytes" {
            emit_bytes_arg(&mut arg_bindings, &mut kwarg_exprs, value, var_name);
            continue;
        }

        let literal = json_to_python_literal(value);
        let noqa = if literal.contains("/tmp/") {
            "  # noqa: S108"
        } else {
            ""
        };
        arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
        kwarg_exprs.push(var_name.to_string());
    }

    while kwarg_exprs
        .len()
        .checked_sub(1)
        .is_some_and(|last| placeholder_positions.contains(&last))
    {
        kwarg_exprs.pop();
    }

    (arg_bindings, kwarg_exprs, teardown)
}

/// Read-only inputs to [`emit_handle_arg`], bundled because every field is invariant borrowed
/// state describing the one "handle" arg being emitted -- `arg_bindings`/`kwarg_exprs` stay
/// their own `&mut` parameters since `emit_handle_arg` mutates both.
#[derive(Clone, Copy)]
struct HandleArgContext<'a> {
    fixture: &'a Fixture,
    arg: &'a crate::e2e::config::ArgMapping,
    var_name: &'a str,
    options_type: Option<&'a str>,
    handle_nested_types: &'a HashMap<String, String>,
    handle_dict_types: &'a HashSet<String>,
}

fn emit_handle_arg(arg_bindings: &mut Vec<String>, kwarg_exprs: &mut Vec<String>, context: HandleArgContext<'_>) {
    let HandleArgContext {
        fixture,
        arg,
        var_name,
        options_type,
        handle_nested_types,
        handle_dict_types,
    } = context;

    let constructor_name = format!("create_{}", arg.name.to_snake_case());
    let config_value = resolve_field(&fixture.input, &arg.field);
    if config_value.is_null() || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty()) {
        arg_bindings.push(format!("    {var_name} = {constructor_name}(None)"));
    } else if let Some(obj) = config_value.as_object() {
        let kwargs: Vec<String> = obj
            .iter()
            .map(|(k, v)| {
                let snake_key = k.to_snake_case();
                let py_val = build_handle_kwarg_value(k, v, handle_nested_types, handle_dict_types);
                format!("{snake_key}={py_val}")
            })
            .collect();
        let config_class = options_type.unwrap_or_else(|| {
            panic!(
                "python e2e: handle arg `{}` requires `options_type` on the call config (set `[e2e.call] options_type = \"...\"` to the Python class name of the handle's config struct)",
                arg.name
            )
        });
        let single_line = format!("    {var_name}_config = {config_class}({})", kwargs.join(", "));
        if single_line.len() <= 120 {
            arg_bindings.push(single_line);
        } else {
            let mut lines = format!("    {var_name}_config = {config_class}(\n");
            for kw in &kwargs {
                lines.push_str(&format!("        {kw},\n"));
            }
            lines.push_str("    )");
            arg_bindings.push(lines);
        }
        arg_bindings.push(format!("    {var_name} = {constructor_name}({var_name}_config)"));
    } else {
        let literal = json_to_python_literal(config_value);
        arg_bindings.push(format!("    {var_name} = {constructor_name}({literal})"));
    }
    kwarg_exprs.push(var_name.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_and_setup_empty_args_returns_empty_vecs() {
        use crate::e2e::fixture::Fixture;
        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "t".to_string(),
            description: "d".to_string(),
            input: serde_json::Value::Null,
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            setup: Vec::new(),
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        };
        let call_config = crate::e2e::config::CallConfig::default();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let context = ArgSetupContext {
            call_config: &call_config,
            options_type: None,
            options_via: "kwargs",
            from_json_unavailable_types: &HashSet::new(),
            enum_fields: &HashMap::new(),
            handle_nested_types: &HashMap::new(),
            handle_dict_types: &HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
        };
        let (bindings, exprs, _teardown) = build_args_and_setup(&fixture, context);
        assert!(bindings.is_empty());
        assert!(exprs.is_empty());
    }

    fn json_object_fixture(field: &str) -> crate::e2e::fixture::Fixture {
        use crate::e2e::fixture::Fixture;
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "t".to_string(),
            description: "d".to_string(),
            input: serde_json::json!({ field: { "kind": "bytes" } }),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: Vec::new(),
            call: None,
            skip: None,
            env: None,
            setup: Vec::new(),
            visitor: None,
            args: vec![crate::e2e::config::ArgMapping {
                name: "input".to_string(),
                field: field.to_string(),
                arg_type: "json_object".to_string(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            assertion_recipes: vec![],
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        }
    }

    fn render_one_json_object_arg(unavailable: &HashSet<String>) -> Vec<String> {
        let fixture = json_object_fixture("input");
        let call_config = crate::e2e::config::CallConfig::default();
        let config = crate::core::config::ResolvedCrateConfig::default();
        let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
        let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
        let context = ArgSetupContext {
            call_config: &call_config,
            options_type: Some("ExtractInput"),
            options_via: "from_json",
            from_json_unavailable_types: unavailable,
            enum_fields: &HashMap::new(),
            handle_nested_types: &HashMap::new(),
            handle_dict_types: &HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
        };
        build_args_and_setup(&fixture, context).0
    }

    /// `options_via` is resolved once per call, against the CALL's options type, then applied to
    /// every argument. A downstream call mixes the two spellings -- a native options type
    /// that has `from_json` and a public `ExtractInput` dataclass that does not -- so every
    /// generated Python e2e test raised `AttributeError: type object 'ExtractInput' has no
    /// attribute 'from_json'`, and the whole suite was red.
    #[test]
    fn a_dataclass_mirrored_argument_does_not_get_a_from_json_constructor() {
        let unavailable: HashSet<String> = ["ExtractInput".to_string()].into_iter().collect();
        let bindings = render_one_json_object_arg(&unavailable);
        assert!(
            !bindings.iter().any(|line| line.contains("ExtractInput.from_json(")),
            "got: {bindings:?}"
        );
    }

    /// The other side of the same switch: a type the native module really does expose `from_json`
    /// on must keep using it, or the downgrade is just a blanket removal.
    #[test]
    fn an_unwrapped_argument_keeps_its_from_json_constructor() {
        let bindings = render_one_json_object_arg(&HashSet::new());
        assert!(
            bindings.iter().any(|line| line.contains("ExtractInput.from_json(")),
            "got: {bindings:?}"
        );
    }
}
