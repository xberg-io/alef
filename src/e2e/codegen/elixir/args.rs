//! Elixir e2e argument and setup rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_elixir;
use heck::ToSnakeCase;
use std::collections::HashMap;

use super::stubs::emit_test_backend;
use super::values::json_to_elixir;

/// Grouped read-only inputs to [`build_args_and_setup`], mirroring the `JavaArgsContext` /
/// `KotlinArgsContext` pattern used by the other e2e backends to keep the call-target
/// descriptors out of the parameter list. ~keep
pub(super) struct ElixirArgsContext<'a> {
    pub(super) module_path: &'a str,
    pub(super) options_type: Option<&'a str>,
    pub(super) options_default_fn: Option<&'a str>,
    pub(super) enum_fields: &'a HashMap<String, String>,
    pub(super) fixture: &'a crate::e2e::fixture::Fixture,
    pub(super) _handle_struct_type: Option<&'a str>,
    pub(super) _handle_atom_list_fields: &'a std::collections::HashSet<String>,
    pub(super) test_documents_path: &'a str,
    pub(super) adapter_request_type: Option<&'a str>,
    pub(super) enums: &'a [crate::core::ir::EnumDef],
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) force_keyword_args: bool,
}

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string, teardown_block)`. `teardown_block` is
/// non-empty only for `test_backend` args backed by a trait bridge with an
/// `unregister_fn` — see `emit_test_backend`'s doc comment for why ExUnit
/// needs this teardown.
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    context: ElixirArgsContext<'_>,
) -> (Vec<String>, String, String) {
    if args.is_empty() {
        return build_empty_args_result(input);
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut teardown_block = String::new();

    // NOTE: Elixir requires all positional args before keyword args. To avoid syntax errors,
    // count how many optional args will be rendered as keywords upfront, then decide
    // whether json_object args should be positional or keyword. This aligns with the
    // Rustler backend's keyword-opts threshold: use keyword form for 2+ trailing optional
    // params, stay positional for 1 or 0.
    let use_keyword_form_for_optional_args = compute_trailing_keyword_count(args) >= 2;

    let ctx = ArgEmitContext {
        input,
        module_path: context.module_path,
        options_type: context.options_type,
        options_default_fn: context.options_default_fn,
        enum_fields: context.enum_fields,
        fixture: context.fixture,
        test_documents_path: context.test_documents_path,
        adapter_request_type: context.adapter_request_type,
        enums: context.enums,
        config: context.config,
        type_defs: context.type_defs,
        use_keyword_form_for_optional_args,
    };

    for arg in args {
        if arg.arg_type == "mock_url" {
            push_mock_url_arg(arg, &ctx, &mut setup_lines, &mut parts);
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            push_mock_url_list_arg(arg, &ctx, &mut setup_lines, &mut parts);
            continue;
        }

        if arg.arg_type == "handle" {
            push_handle_arg(arg, &ctx, &mut setup_lines, &mut parts);
            continue;
        }

        if arg.arg_type == "test_backend" {
            push_test_backend_arg(arg, &ctx, &mut setup_lines, &mut parts, &mut teardown_block);
            continue;
        }

        let val = if arg.field == "input" {
            Some(input.get("extract_input").unwrap_or(input))
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };
        push_value_arg(arg, val, &ctx, &mut setup_lines, &mut parts);
    }

    // Elixir requires all positional args before keyword args.
    // Separate positional and keyword args, preserving order within each group.
    // With the keyword-opts threshold applied above (use_keyword_form_for_optional_args),
    // we should never encounter a positional arg after a keyword arg.
    // EXCEPTION: test_backend and string args (plugin names) are always positional
    // because trait-bridge registration functions have fixed positional signatures.
    let args_string = finalize_args_string(parts, args, context.force_keyword_args);
    (setup_lines, args_string, teardown_block)
}

/// The no-args-config shortcut: pass the whole input only when it's non-empty AND not just the
/// harness setup dict. Split out of `build_args_and_setup` purely to keep that function's line
/// count under the file's limit -- the logic is unchanged from its original inline form. ~keep
fn build_empty_args_result(input: &serde_json::Value) -> (Vec<String>, String, String) {
    // No args config: pass the whole input only when it's non-empty AND not just the harness setup dict.
    // Functions with no parameters (e.g. language_count) have empty input
    // and must be called with no arguments - not with `%{}`.
    // Filter out the harness' internal "setup" field - it's not part of the fixture's actual input.
    let cleaned_input = match input {
        serde_json::Value::Object(m) => {
            let mut cleaned = m.clone();
            cleaned.remove("setup");
            if cleaned.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::Object(cleaned)
            }
        }
        other => other.clone(),
    };
    let is_empty_input = matches!(cleaned_input, serde_json::Value::Null);
    if is_empty_input {
        return (Vec::new(), String::new(), String::new());
    }
    (Vec::new(), json_to_elixir(&cleaned_input), String::new())
}

fn compute_trailing_keyword_count(args: &[crate::e2e::config::ArgMapping]) -> usize {
    args.iter()
        .rev()
        .take_while(|a| a.optional)
        .filter(|a| {
            // An arg will be rendered as keyword if it's optional AND has a provided value
            // that's not null. We can't fully evaluate this without checking the input,
            // but we can count optional params at the end - a conservative heuristic.
            a.arg_type != "mock_url" && a.arg_type != "mock_url_list" && a.arg_type != "handle"
        })
        .count()
}

/// Elixir requires all positional args before keyword args; this reconciles `parts` (built up
/// per-arg by `push_*_arg`/`push_value_arg`) into the final call-argument string, either forcing
/// every part into keyword form or separating positional from keyword parts while preserving
/// order within each group. ~keep
fn finalize_args_string(
    parts: Vec<String>,
    args: &[crate::e2e::config::ArgMapping],
    force_keyword_args: bool,
) -> String {
    // EXCEPTION: test_backend and string args (plugin names) are always positional
    // because trait-bridge registration functions have fixed positional signatures.
    let has_test_backend = args
        .iter()
        .any(|a| a.arg_type == "test_backend" || a.arg_type == "string");
    if force_keyword_args && !has_test_backend {
        return parts
            .into_iter()
            .zip(args.iter())
            .map(|(part, arg)| {
                let prefix = format!("{}: ", arg.name);
                if part.starts_with(&prefix) {
                    part
                } else {
                    format!("{}: {part}", arg.name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
    }

    let mut positional_args = Vec::new();
    let mut keyword_args = Vec::new();

    for part in parts {
        let is_keyword = part.contains(": ") && !part.starts_with('"');
        if is_keyword {
            keyword_args.push(part);
        } else {
            positional_args.push(part);
        }
    }

    let mut final_args = positional_args;
    final_args.extend(keyword_args);

    final_args.join(", ")
}

/// Shared read-only context passed to each per-arg-type push helper in the `for arg in args`
/// loop, bundling the fixture/config data every branch needs so no helper's parameter list
/// grows past the repo's six-parameter limit. ~keep
struct ArgEmitContext<'a> {
    input: &'a serde_json::Value,
    module_path: &'a str,
    options_type: Option<&'a str>,
    options_default_fn: Option<&'a str>,
    enum_fields: &'a HashMap<String, String>,
    fixture: &'a crate::e2e::fixture::Fixture,
    test_documents_path: &'a str,
    adapter_request_type: Option<&'a str>,
    enums: &'a [crate::core::ir::EnumDef],
    config: &'a ResolvedCrateConfig,
    type_defs: &'a [crate::core::ir::TypeDef],
    use_keyword_form_for_optional_args: bool,
}

fn push_mock_url_arg(
    arg: &crate::e2e::config::ArgMapping,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    let fixture_id = &ctx.fixture.id;
    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
    let value = ctx.input.get(field).unwrap_or(&serde_json::Value::Null);
    if let Some(url) = crate::e2e::codegen::preserved_url_literal(ctx.fixture.preserve_input_urls, value) {
        setup_lines.push(format!("{} = \"{}\"", arg.name, escape_elixir(url)));
    } else if ctx.fixture.has_host_root_route() {
        let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
        setup_lines.push(format!(
            "{} = System.get_env(\"{env_key}\") || (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"",
            arg.name,
        ));
    } else {
        setup_lines.push(format!(
            "{} = (System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\"",
            arg.name,
        ));
    }
    if let Some(req_type) = ctx.adapter_request_type {
        let req_var = format!("{}_req", arg.name);
        setup_lines.push(format!("{req_var} = %{}.{req_type}{{url: {}}}", ctx.module_path, arg.name));
        parts.push(req_var);
    } else {
        parts.push(arg.name.clone());
    }
}

fn push_mock_url_list_arg(
    arg: &crate::e2e::config::ArgMapping,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    // list of URLs: each element is either a bare path (`/seed1`) - prefixed
    // with the per-fixture mock-server URL at runtime - or an absolute URL
    // kept as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>`
    // first, then `MOCK_SERVER_URL/fixtures/<id>`. Without this branch the
    // codegen falls back to a JSON-array literal of bare relative paths and
    // the Rust HTTP client rejects them.
    let fixture_id = &ctx.fixture.id;
    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
    let val = ctx.input.get(field).unwrap_or(&serde_json::Value::Null);
    let name = &arg.name;
    if let Some(urls) = crate::e2e::codegen::preserved_url_list(ctx.fixture.preserve_input_urls, val) {
        let literals: Vec<String> = urls.iter().map(|url| format!("\"{}\"", escape_elixir(url))).collect();
        setup_lines.push(format!("{name} = [{}]", literals.join(", ")));
        parts.push(name.clone());
        return;
    }
    let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
    let paths: Vec<String> = if let Some(arr) = val.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_elixir(s))))
            .collect()
    } else {
        Vec::new()
    };
    let paths_literal = paths.join(", ");
    setup_lines.push(format!(
        "{name}_base = System.get_env(\"{env_key}\") || ((System.get_env(\"MOCK_SERVER_URL\") || \"\") <> \"/fixtures/{fixture_id}\")"
    ));
    setup_lines.push(format!(
        "{name} = Enum.map([{paths_literal}], fn p -> if String.starts_with?(p, \"http\"), do: p, else: {name}_base <> p end)"
    ));
    parts.push(name.clone());
}

fn push_handle_arg(
    arg: &crate::e2e::config::ArgMapping,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    // Generate a create_{name} call using {:ok, name} = ... pattern.
    // The NIF now accepts config as an optional JSON string (not a NifStruct/NifMap)
    // so that partial maps work: serde_json::from_str respects #[serde(default)].
    let constructor_name = format!("create_{}", arg.name.to_snake_case());
    let config_value = if arg.field == "input" {
        ctx.input
    } else {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        ctx.input.get(field).unwrap_or(&serde_json::Value::Null)
    };
    let name = &arg.name;
    if config_value.is_null() || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty()) {
        setup_lines.push(format!("{{:ok, {name}}} = {}.{constructor_name}(nil)", ctx.module_path));
    } else {
        // Serialize the config map to a JSON string with Jason so that Rust can
        // deserialize it with serde_json and apply field defaults for missing keys.
        let json_str = serde_json::to_string(config_value).unwrap_or_else(|_| "{}".to_string());
        let escaped = escape_elixir(&json_str);
        setup_lines.push(format!("{name}_config = \"{escaped}\""));
        setup_lines.push(format!("{{:ok, {name}}} = {}.{constructor_name}({name}_config)", ctx.module_path));
    }
    parts.push(arg.name.clone());
}

fn push_test_backend_arg(
    arg: &crate::e2e::config::ArgMapping,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
    teardown_block: &mut String,
) {
    if let Some(trait_name) = &arg.trait_name
        && let Some(trait_bridge) = ctx.config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name)
    {
        // Collect methods from both the main trait and its super-trait (if present).
        // The super-trait methods are needed so stubs implement the full interface.
        let mut methods: Vec<&crate::core::ir::MethodDef> = ctx
            .type_defs
            .iter()
            .find(|t| t.name == *trait_name)
            .map(|t| t.methods.iter().collect())
            .unwrap_or_default();

        // If there's a super-trait, also collect its methods.
        if let Some(super_trait) = &trait_bridge.super_trait
            && let Some(super_type) = ctx.type_defs.iter().find(|t| &t.name == super_trait)
        {
            for method in &super_type.methods {
                // Only add if not already present (avoid duplicates).
                if !methods.iter().any(|m| m.name == method.name) {
                    methods.push(method);
                }
            }
        }

        // Derive the NIF module from the test module path: the NIF module
        // follows the "{AppModule}.Native" convention used by the Elixir scaffold.
        let elixir_nif_module = format!("{}.Native", ctx.module_path);
        let emission = emit_test_backend(trait_bridge, &methods, ctx.fixture, &elixir_nif_module, ctx.module_path);
        teardown_block.push_str(&emission.teardown_block);

        // Extract only the test-level setup part (after the marker).
        // Module-level defs are emitted at file level by render_test_file, not here.
        if let Some(pos) = emission.setup_block.find("__TRAIT_BRIDGE_MODULE_DEFS_END__") {
            let marker_end = emission.setup_block[pos + 32..]
                .find('\n')
                .map(|i| pos + 32 + i + 1)
                .unwrap_or_else(|| emission.setup_block.len());
            let test_setup = emission.setup_block[marker_end..].trim_start().to_string();
            if !test_setup.is_empty() {
                setup_lines.push(test_setup);
            }
        } else {
            // Fallback for non-marker blocks (shouldn't happen for trait bridges)
            setup_lines.push(emission.setup_block);
        }

        parts.push(emission.arg_expr);

        // For register_fn traits (plugin pattern), Rustler requires a second "name" argument.
        // Extract the backend name from fixture input (same logic as emit_test_backend).
        if trait_bridge.register_fn.is_some() {
            let backend_name = extract_backend_name_from_input(&ctx.fixture.input, &ctx.fixture.id);
            parts.push(format!("\"{}\"", escape_elixir(&backend_name)));
        }
        return;
    }
    // A `test_backend` arg fills a required Elixir stub parameter — there is
    // no compilable value to fall back to when the trait isn't configured.
    // Fail generation loudly instead of silently splicing a `nil` argument
    // with a comment where the real stub belongs. ~keep
    panic!(
        "Elixir e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate an Elixir stub without a resolvable trait bridge",
        ctx.fixture.id, arg.name, arg.trait_name
    );
}

fn push_value_arg(
    arg: &crate::e2e::config::ArgMapping,
    val: Option<&serde_json::Value>,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    match val {
        None | Some(serde_json::Value::Null) if arg.optional => {
            // Optional params map to the keyword-opts `opts \\ []` argument.
            // When the value is absent, omit the keyword entirely - the default `[]` applies.
        }
        None | Some(serde_json::Value::Null) => {
            // Required arg with no fixture value: pass a language-appropriate default.
            let default_val = match arg.arg_type.as_str() {
                "string" => "\"\"".to_string(),
                "int" | "integer" => "0".to_string(),
                "float" | "number" => "0.0".to_string(),
                "bool" | "boolean" => "false".to_string(),
                _ => "nil".to_string(),
            };
            parts.push(default_val);
        }
        Some(v) => push_present_value_arg(arg, v, ctx, setup_lines, parts),
    }
}

fn push_present_value_arg(
    arg: &crate::e2e::config::ArgMapping,
    v: &serde_json::Value,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    // For file_path args, prepend the path to the test_documents directory
    // relative to the e2e/elixir/ directory where `mix test` runs.
    if try_push_file_path_value(arg, v, ctx, parts) {
        return;
    }
    // For bytes args, use File.read! for file paths and Base.decode64! for base64.
    // Inline text (starts with '<', '{', '[' or contains spaces) is used as-is (UTF-8 binary).
    if try_push_bytes_value(arg, v, ctx, setup_lines, parts) {
        return;
    }
    // For json_object args with options_type+options_via, build a proper struct.
    if arg.arg_type == "json_object" && !v.is_null() && try_push_json_object_value(arg, v, ctx, setup_lines, parts) {
        return;
    }
    // Optional args use keyword-opts form: `name: value`.
    let elixir_val = json_to_elixir(v);
    push_arg_value(&arg.name, elixir_val, arg.optional, parts);
}

fn try_push_file_path_value(
    arg: &crate::e2e::config::ArgMapping,
    v: &serde_json::Value,
    ctx: &ArgEmitContext<'_>,
    parts: &mut Vec<String>,
) -> bool {
    if arg.arg_type != "file_path" {
        return false;
    }
    let Some(path_str) = v.as_str() else {
        return false;
    };
    let full_path = format!("{}/{path_str}", ctx.test_documents_path);
    let formatted = format!("\"{}\"", escape_elixir(&full_path));
    push_arg_value(&arg.name, formatted, arg.optional, parts);
    true
}

fn try_push_bytes_value(
    arg: &crate::e2e::config::ArgMapping,
    v: &serde_json::Value,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) -> bool {
    if arg.arg_type != "bytes" {
        return false;
    }
    let Some(raw) = v.as_str() else {
        return false;
    };
    let var_name = &arg.name;
    if raw.starts_with('<') || raw.starts_with('{') || raw.starts_with('[') || raw.contains(' ') {
        // Inline text - use as a binary string.
        let formatted = format!("\"{}\"", escape_elixir(raw));
        push_arg_value(&arg.name, formatted, arg.optional, parts);
        return true;
    }
    let first = raw.chars().next().unwrap_or('\0');
    let is_file_path = (first.is_ascii_alphanumeric() || first == '_')
        && raw
            .find('/')
            .is_some_and(|slash_pos| slash_pos > 0 && raw[slash_pos + 1..].contains('.'));
    if is_file_path {
        // Looks like "dir/file.ext" - read from the
        // configured test-documents directory. Convert to a byte-integer
        // list, not a raw binary: the value later crosses a Jason.encode!
        // hop (e.g. the ExtractInput struct's `bytes` field), and a
        // binary containing non-UTF-8 bytes crashes that encode. An
        // integer list matches the shape the already-working inline
        // Vec<u8> array path emits (see the `element_type` array branch
        // below), which the NIF already accepts. ~keep
        let full_path = format!("{}/{raw}", ctx.test_documents_path);
        let escaped = escape_elixir(&full_path);
        setup_lines.push(format!("{var_name} = :binary.bin_to_list(File.read!(\"{escaped}\"))"));
        push_arg_value(&arg.name, var_name.to_string(), arg.optional, parts);
        return true;
    }
    // Treat as base64-encoded binary. Decoding to a raw binary has the
    // same non-UTF-8-crashes-Jason.encode! problem as the file-path
    // case above, so convert to a byte-integer list too. ~keep
    setup_lines.push(format!(
        "{var_name} = :binary.bin_to_list(Base.decode64!(\"{}\", padding: false))",
        escape_elixir(raw)
    ));
    push_arg_value(&arg.name, var_name.to_string(), arg.optional, parts);
    true
}

fn try_push_json_object_value(
    arg: &crate::e2e::config::ArgMapping,
    v: &serde_json::Value,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) -> bool {
    let object_type = crate::e2e::codegen::recipe::json_object_constructor_type(arg, ctx.options_type, v);
    if let (Some(_opts_type), Some(options_fn), Some(obj)) = (object_type, ctx.options_default_fn, v.as_object()) {
        push_options_via_default_fn(arg, obj, options_fn, ctx, setup_lines, parts);
        return true;
    }
    // When options_type is set but options_via is NOT, emit a struct-literal form.
    // The auto-generated Rustler facade signature (`def f(html, options \\ nil)
    // when is_map(options)`) requires a map, not a JSON string - and Elixir
    // structs ARE maps, so a struct literal matches the guard. Falling through
    // to the JSON-string emission below would yield `f(html, "{json}")`, which
    // crashes the facade with FunctionClauseError. Emit positional/keyword
    // form per `use_keyword_form_for_optional_args` to mirror the threshold
    // applied to JSON-string emission.
    if let (Some(opts_type), None, Some(_obj)) = (object_type, ctx.options_default_fn, v.as_object()) {
        push_options_struct_literal(arg, opts_type, v, ctx, setup_lines, parts);
        return true;
    }
    if let Some(elem_type) = &arg.element_type
        && try_push_element_type_array(arg, v, elem_type, ctx, setup_lines, parts)
    {
        return true;
    }
    // When there's no options_type+options_via, the Elixir NIF expects a JSON
    // string (Option<String> decoded by serde_json) rather than an Elixir map.
    // Serialize the JSON value to a string literal here.
    // Emit as positional or keyword based on trailing optional arg count.
    // If 2+ trailing optional args exist, use keyword form to avoid mixing
    // positional args after keyword args. Otherwise, stay positional for
    // compatibility with positional-default style facades.
    if !v.is_null() {
        let json_str = serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string());
        let escaped = escape_elixir(&json_str);
        let formatted = format!("\"{escaped}\"");
        push_arg_value(&arg.name, formatted, ctx.use_keyword_form_for_optional_args && arg.optional, parts);
        return true;
    }
    false
}

fn push_options_via_default_fn(
    arg: &crate::e2e::config::ArgMapping,
    obj: &serde_json::Map<String, serde_json::Value>,
    options_fn: &str,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    // Add setup line to initialize options from default function.
    let options_var = format!("{}_value", arg.name);
    setup_lines.push(format!("{options_var} = {}.{options_fn}()", ctx.module_path));

    // For each field in the options object, add a struct update line.
    for (k, vv) in obj.iter() {
        let snake_key = k.to_snake_case();
        let elixir_val = if let Some(_enum_type) = ctx.enum_fields.get(k) {
            if let Some(s) = vv.as_str() {
                let snake_val = s.to_snake_case();
                // Use atom for enum values, not string
                format!(":{snake_val}")
            } else {
                json_to_elixir(vv)
            }
        } else {
            json_to_elixir(vv)
        };
        setup_lines.push(format!("{options_var} = %{{{options_var} | {snake_key}: {elixir_val}}}"));
    }

    // Push the variable name as the argument.
    // Optional args (with `\\ []` or `\\ nil`) always use keyword form
    // so that the facade can handle them via Keyword.get() or defaults.
    parts.push(format!("{}: {options_var}", arg.name));
}

fn push_options_struct_literal(
    arg: &crate::e2e::config::ArgMapping,
    opts_type: &str,
    v: &serde_json::Value,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    let obj = v.as_object().expect("caller verified v is a JSON object");
    let fixture_id = &ctx.fixture.id;
    let options_var = format!("{}_value", arg.name);
    let as_keyword = ctx.use_keyword_form_for_optional_args && arg.optional;
    if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
        let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
        let base_var = format!("{}_mock_base_url", arg.name);
        setup_lines.push(format!(
            "{base_var} = System.get_env(\"{env_key}\") || \"#{{System.get_env(\"MOCK_SERVER_URL\")}}/fixtures/{fixture_id}\""
        ));
        let fields = render_struct_fields(
            obj,
            ctx.enum_fields,
            Some(&base_var),
            &ctx.fixture.docs_files_for_arg(&arg.field),
        );
        setup_lines.push(format!("{options_var} = %{}.{opts_type}{{{fields}}}", ctx.module_path));
        push_arg_value(&arg.name, options_var, as_keyword, parts);
        return;
    }
    let fields = render_struct_fields(obj, ctx.enum_fields, None, &ctx.fixture.docs_files_for_arg(&arg.field));
    setup_lines.push(format!("{options_var} = %{}.{opts_type}{{{fields}}}", ctx.module_path));
    push_arg_value(&arg.name, options_var, as_keyword, parts);
}

fn try_push_element_type_array(
    arg: &crate::e2e::config::ArgMapping,
    v: &serde_json::Value,
    elem_type: &String,
    ctx: &ArgEmitContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) -> bool {
    // Internally-tagged enums (#[serde(tag = "type")]) - emit a list of
    // Rustler NifTaggedEnum tuples. `:variant_atom` for unit variants,
    // `{:variant_atom, %{field: value}}` for struct variants. Variant
    // and field atoms are derived from Rust names via snake_case;
    // Rustler's NifTaggedEnum decoder ignores serde renames.
    if v.is_array()
        && let Some(enum_def) = ctx.enums.iter().find(|e| &e.name == elem_type && e.serde_tag.is_some())
    {
        let formatted = emit_tagged_enum_array(v, enum_def, ctx.enums);
        push_arg_value(&arg.name, formatted, arg.optional, parts);
        return true;
    }
    // When element_type is set to a simple type (e.g. Vec<String>).
    // The NIF accepts an Elixir list directly - emit one.
    if v.is_array() {
        let fixture_id = &ctx.fixture.id;
        if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
            let base_var = format!("{}_mock_base_url", arg.name);
            let json_var = format!("{}_json", arg.name);
            let value_var = format!("{}_value", arg.name);
            let formatted = json_to_elixir(v);
            setup_lines.push(format!(
                "{base_var} = System.get_env(\"{env_key}\") || \"#{{System.get_env(\"MOCK_SERVER_URL\")}}/fixtures/{fixture_id}\""
            ));
            setup_lines.push(format!(
                "{json_var} = Jason.encode!({formatted}) |> String.replace(\"{}\", {base_var})",
                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
            ));
            setup_lines.push(format!("{value_var} = Jason.decode!({json_var})"));
            push_arg_value(&arg.name, value_var, arg.optional, parts);
            return true;
        }
        let formatted = json_to_elixir(v);
        push_arg_value(&arg.name, formatted, arg.optional, parts);
        return true;
    }
    false
}

/// Push `value` onto `parts`, as a `name: value` keyword arg when `as_keyword` is true, or
/// positionally otherwise. Bundles the repeated optional-vs-positional dispatch used throughout
/// the file_path/bytes/json_object value branches above -- extracted verbatim from their
/// identical `if arg.optional { "name: value" } else { value }` bodies. ~keep
fn push_arg_value(name: &str, value: String, as_keyword: bool, parts: &mut Vec<String>) {
    if as_keyword {
        parts.push(format!("{name}: {value}"));
    } else {
        parts.push(value);
    }
}

fn render_struct_fields(
    obj: &serde_json::Map<String, serde_json::Value>,
    enum_fields: &HashMap<String, String>,
    mock_base_var: Option<&str>,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
) -> String {
    obj.iter()
        .map(|(k, vv)| {
            let snake_key = k.to_snake_case();
            let pointer = format!("/{k}");
            let elixir_val = if let Some(file) = docs_files.iter().find(|file| file.field == pointer) {
                crate::e2e::template_env::render(
                    "elixir/docs_file_read.jinja",
                    minijinja::context! { path => escape_elixir(&file.path) },
                )
                .trim_end()
                .to_string()
            } else if enum_fields.contains_key(k) {
                if let Some(s) = vv.as_str() {
                    let snake_val = s.to_snake_case();
                    format!(":{snake_val}")
                } else {
                    render_elixir_value(vv, mock_base_var)
                }
            } else {
                render_elixir_value(vv, mock_base_var)
            };
            format!("{snake_key}: {elixir_val}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_elixir_value(value: &serde_json::Value, mock_base_var: Option<&str>) -> String {
    if let Some(base_var) = mock_base_var
        && crate::e2e::codegen::value_contains_mock_url_placeholder(value)
    {
        match value {
            serde_json::Value::String(s) => format!(
                "String.replace(\"{}\", \"{}\", {base_var})",
                escape_elixir(s),
                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
            ),
            _ => {
                let value_literal = json_to_elixir(value);
                format!(
                    "Jason.decode!(Jason.encode!({value_literal}) |> String.replace(\"{}\", {base_var}), keys: :atoms)",
                    crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                )
            }
        }
    } else {
        json_to_elixir(value)
    }
}

/// Match an input JSON value (string) against a unit-only enum and return the
/// corresponding Rustler atom literal (e.g. `:down`). Returns None if the enum
/// is not unit-only or the value does not match any variant.
pub(super) fn match_unit_enum_atom(value: &serde_json::Value, enum_def: &crate::core::ir::EnumDef) -> Option<String> {
    let s = value.as_str()?;
    if enum_def.variants.iter().any(|v| !v.fields.is_empty()) {
        return None;
    }
    for variant in &enum_def.variants {
        let wire_tag = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if wire_tag == s {
            return Some(format!(":{}", variant.name.to_snake_case()));
        }
    }
    None
}

/// Emit an Elixir list literal of Rustler NifTaggedEnum tuples for an internally-tagged
/// enum array. Each element renders as `:variant_atom` (unit) or
/// `{:variant_atom, %{field: value}}` (struct), with variant/field atoms derived
/// from the Rust names via snake_case (NifTaggedEnum ignores serde rename for atoms).
fn emit_tagged_enum_array(
    value: &serde_json::Value,
    enum_def: &crate::core::ir::EnumDef,
    all_enums: &[crate::core::ir::EnumDef],
) -> String {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return json_to_elixir(value),
    };
    let tag_key = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let mut elements: Vec<String> = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = match item.as_object() {
            Some(o) => o,
            None => {
                elements.push(json_to_elixir(item));
                continue;
            }
        };
        let tag_value = obj.get(tag_key).and_then(|v| v.as_str()).unwrap_or("");
        let matched = enum_def.variants.iter().find(|variant| {
            let wire_tag = crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            wire_tag == tag_value
        });
        let Some(variant) = matched else {
            elements.push(json_to_elixir(item));
            continue;
        };
        let variant_atom = format!(":{}", variant.name.to_snake_case());
        if variant.fields.is_empty() {
            elements.push(variant_atom);
            continue;
        }
        let mut field_strs: Vec<String> = Vec::with_capacity(variant.fields.len());
        for field in &variant.fields {
            let wire_field = field.serde_rename.as_deref().unwrap_or(&field.name);
            let rust_field_atom = field.name.clone();
            let emitted_val = if let Some(field_val) = obj.get(wire_field) {
                // If the field's type is a Named reference to a unit-only enum, convert
                // the input string value to an atom via that enum's rename_all.
                if let crate::core::ir::TypeRef::Named(type_name) = &field.ty {
                    all_enums
                        .iter()
                        .find(|e| &e.name == type_name && e.serde_tag.is_none())
                        .and_then(|nested| match_unit_enum_atom(field_val, nested))
                        .unwrap_or_else(|| json_to_elixir(field_val))
                } else {
                    json_to_elixir(field_val)
                }
            } else if field.optional {
                // Optional fields missing from the JSON should use `nil` as default
                "nil".to_string()
            } else {
                // Non-optional fields missing from the JSON should not be included
                // (could indicate an error in the fixture, but we skip for safety)
                continue;
            };
            field_strs.push(format!("{rust_field_atom}: {emitted_val}"));
        }
        let map_body = field_strs.join(", ");
        elements.push(format!("{{{variant_atom}, %{{{map_body}}}}}"));
    }
    format!("[{}]", elements.join(", "))
}

/// Extract the backend name from fixture input for register_fn traits.
///
/// Looks for a "name" field at the root or nested one level deep,
/// then falls back to the first string value encountered, then to the fallback.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    if let Some(obj) = input.as_object() {
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object()
                && let Some(s) = inner.get("name").and_then(|v| v.as_str())
            {
                return s.to_string();
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}
