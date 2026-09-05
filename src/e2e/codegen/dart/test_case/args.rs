//! Per-argument Dart e2e call-arg lowering, extracted from `test_case::render_test_case`.
//!
//! Each function below renders exactly one `arg_type` branch of the original single giant
//! match; moving the switch out keeps the parent's own control flow (call setup, visitor
//! wiring, receiver resolution, body emission) readable on its own. No emitted string,
//! whitespace, ordering, or emission condition changed in the move -- every body below is the
//! original arm's code with its free variables threaded through `DartArgContext` instead of
//! closed over. ~keep

use crate::codegen::naming::underscore_camel_case;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeRef};
use crate::e2e::codegen::call_ir::TargetParams;
use crate::e2e::codegen::recipe::ResolvedE2eCallRecipe;
use crate::e2e::codegen::resolve_field;
use crate::e2e::config::ArgMapping;
use crate::e2e::fixture::Fixture;

use super::super::stubs::emit_test_backend;
use super::super::values::{escape_dart, mime_from_extension, type_name_to_create_from_json_dart};

/// Read-only context threaded through per-argument Dart e2e call-arg rendering. Every field is
/// exactly the free variable the extracted-from `render_test_case` loop closed over; grouping
/// them here is a pure refactor and changes no emitted output. ~keep
pub(super) struct DartArgContext<'a> {
    pub(super) fixture: &'a Fixture,
    pub(super) is_snippet: bool,
    pub(super) bridge_class: &'a str,
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [EnumDef],
    pub(super) call_recipe: &'a ResolvedE2eCallRecipe<'a>,
    pub(super) target_params: TargetParams<'a>,
    pub(super) native_typed_dtos: bool,
    pub(super) options_type: Option<&'a str>,
    pub(super) options_via: &'a str,
    pub(super) is_frb_bridge_call: bool,
    pub(super) adapter_request_type: Option<&'a str>,
    pub(super) file_path_for_mime: Option<&'a str>,
    pub(super) routes_to_source_code: bool,
    pub(super) client_factory_for_args: Option<&'a str>,
}

/// Build the argument list and setup statements for one fixture's call, mutating `setup_lines`
/// and `args` in place -- identical to the loop this replaces in `render_test_case`. ~keep
pub(super) fn build_args_and_setup(setup_lines: &mut Vec<String>, args: &mut Vec<String>, ctx: &DartArgContext<'_>) {
    for (arg_index, arg_def) in ctx.call_recipe.args.iter().enumerate() {
        match arg_def.arg_type.as_str() {
            "mock_url" => {
                render_mock_url_arg(setup_lines, args, arg_def, ctx);
                continue;
            }
            "handle" => {
                render_handle_arg(setup_lines, args, arg_def, ctx);
                continue;
            }
            "mock_url_list" => {
                render_mock_url_list_arg(setup_lines, args, arg_def, ctx);
                continue;
            }
            "test_backend" => {
                render_test_backend_arg(args, arg_def, ctx);
                continue;
            }
            _ => {}
        }

        let arg_value = resolve_field(&ctx.fixture.input, &arg_def.field);
        match arg_def.arg_type.as_str() {
            "bytes" | "file_path" => render_bytes_or_file_path_arg(args, arg_def, arg_value, ctx),
            "int" | "integer" | "i64" => render_int_arg(args, arg_def, arg_value, ctx),
            "float" | "number" => render_float_arg(args, arg_def, arg_value, ctx),
            "bool" | "boolean" => render_bool_arg(args, arg_def, arg_value, ctx),
            "string" => render_string_arg(args, arg_def, arg_index, arg_value, ctx),
            "json_object" => render_json_object_arg(setup_lines, args, arg_def, arg_value, ctx),
            _ => {}
        }
    }
}

fn render_mock_url_arg(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    let name = arg_def.name.clone();
    let value = resolve_field(&fixture.input, &arg_def.field);
    if let Some(url) = crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value)
        .or_else(|| crate::e2e::codegen::snippet_url_literal(ctx.is_snippet, value))
    {
        setup_lines.push(format!("final {name} = '{}';", escape_dart(url)));
    } else {
        let fixture_id = &fixture.id;
        setup_lines.push(format!(r#"final {name} = _fixtureUrl("{fixture_id}");"#));
    }
    // For streaming adapters with a request_type, wrap the URL in the request constructor.
    if let Some(req_type) = ctx.adapter_request_type {
        let req_var = format!("{}Req", name);
        // Extract just the type name (last segment after ::).
        let req_type_name = req_type.rsplit("::").next().unwrap_or(req_type);
        setup_lines.push(format!("final {req_var} = {req_type_name}(url: {name});"));
        args.push(req_var);
    } else {
        args.push(name);
    }
}

fn render_handle_arg(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    let bridge_class = ctx.bridge_class;
    let name = arg_def.name.clone();
    let field = arg_def.field.strip_prefix("input.").unwrap_or(&arg_def.field);
    let config_value = fixture.input.get(field).cloned().unwrap_or(serde_json::Value::Null);
    // Derive the create-function name: "engine" → "createEngine".
    let create_fn = {
        let mut chars = name.chars();
        let pascal = match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        };
        format!("create{pascal}")
    };
    if config_value.is_null() || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty()) {
        setup_lines.push(format!("final {name} = await {bridge_class}.{create_fn}();"));
    } else {
        let json_str = serde_json::to_string(&config_value).unwrap_or_default();
        let config_var = format!("{name}Config");
        // Derive the createFromJson function name from the config TYPE, not the handle name.
        // E.g., for ExtractionConfig → "createExtractionConfigFromJson",
        // for RerankerConfig → "createRerankerConfigFromJson", etc.
        // FRB-generated free function deserializes JSON into the config struct via the
        // Rust `create_<type>_from_json` helper emitted by the dart backend.
        // This avoids relying on a Dart-side `fromJson` constructor (FRB classes don't expose one).
        let config_type_name = ctx.call_recipe.handle_config_type(arg_def).unwrap_or(&arg_def.name);
        let create_from_json_fn = type_name_to_create_from_json_dart(config_type_name);
        setup_lines.push(format!(
            "final {config_var} = await {create_from_json_fn}(json: r'{json_str}');"
        ));
        // Dart wrapper exposes config parameter as a named optional `{ConfigType? config}`
        // (more idiomatic Dart than positional optional). Emit named-argument syntax.
        setup_lines.push(format!(
            "final {name} = await {bridge_class}.{create_fn}(config: {config_var});"
        ));
    }
    args.push(name);
}

fn render_mock_url_list_arg(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    // List<String> of URLs: each element is either a bare path (`/seed1`) — prefixed
    // with the SUT URL at runtime — or an absolute URL kept as-is.
    let val = crate::e2e::codegen::resolve_urls_field(&fixture.input, &arg_def.field);
    let preserved_urls = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, val);
    let is_preserved = preserved_urls.is_some();

    let paths: Vec<String> = if let Some(urls) = preserved_urls {
        urls.into_iter().map(|url| format!("'{}'", escape_dart(url))).collect()
    } else if let Some(arr) = val.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str())
            .map(|s| format!("'{}'", escape_dart(s)))
            .collect()
    } else {
        Vec::new()
    };

    let var_name = &arg_def.name;
    let paths_literal = paths.join(", ");

    // ~keep `|| is_snippet`: the `else` arm below binds `_fixtureUrl`, which only the
    // test-file emitter defines, so a standalone snippet taking it does not compile.
    // The fixture's own declared list is what a reader should be shown anyway.
    if is_preserved || ctx.is_snippet {
        setup_lines.push(format!("final {var_name} = <String>[{paths_literal}];"));
    } else {
        let fixture_id = &fixture.id;
        setup_lines.push(format!(r#"final {var_name}Base = _fixtureUrl("{fixture_id}");"#));
        setup_lines.push(format!(
            r#"final {var_name} = <String>[{paths_literal}].map((p) => p.startsWith('http') ? p : {var_name}Base + p).toList();"#
        ));
    }

    // For streaming adapters with a request_type, wrap the URL list in the request constructor.
    if let Some(req_type) = ctx.adapter_request_type {
        let req_var = format!("{}Req", var_name);
        // Extract just the type name (last segment after ::).
        let req_type_name = req_type.rsplit("::").next().unwrap_or(req_type);
        setup_lines.push(format!("final {req_var} = {req_type_name}(urls: {var_name});"));
        args.push(req_var);
    } else {
        args.push(var_name.to_string());
    }
}

fn render_test_backend_arg(args: &mut Vec<String>, arg_def: &ArgMapping, ctx: &DartArgContext<'_>) {
    let fixture = ctx.fixture;
    let config = ctx.config;
    let type_defs = ctx.type_defs;
    let enums = ctx.enums;
    if let Some(trait_name) = &arg_def.trait_name
        && let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name)
    {
        let methods: Vec<&crate::core::ir::MethodDef> = type_defs
            .iter()
            .find(|t| t.name == *trait_name)
            .map(|t| t.methods.iter().collect())
            .unwrap_or_default();
        let emission = emit_test_backend(trait_bridge, &methods, fixture, enums);
        // Dart class definitions are emitted at module-level (before void main)
        // in stubs::collect_test_stub_classes, so we only push the instantiation here.
        args.push(emission.arg_expr);
        return;
    }
    // A `test_backend` arg fills a non-null Dart stub parameter — there is
    // no compilable value to fall back to when the trait isn't configured.
    // Fail generation loudly instead of silently splicing a `null`
    // argument with a comment where the real stub belongs. ~keep
    panic!(
        "Dart e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate a Dart stub without a resolvable trait bridge",
        fixture.id, arg_def.name, arg_def.trait_name
    );
}

fn render_bytes_or_file_path_arg(
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    // `bytes`: value is a file path string; load file contents at test-run time.
    // `file_path`: for dart, normally remapped to bytes via the extract
    // facade convention. The exception is source-code paths — those
    // route through extractFile/extractFileSync directly (see
    // `routes_to_source_code` above), so the path string must be
    // passed verbatim instead of materialised as bytes.
    if let serde_json::Value::String(file_path) = arg_value {
        let arg_expr = if arg_def.arg_type == "file_path" && ctx.routes_to_source_code {
            format!("'{file_path}'")
        } else {
            format!("File('{}').readAsBytesSync()", file_path)
        };
        if ctx.is_frb_bridge_call {
            let dart_param_name = underscore_camel_case(&arg_def.name);
            args.push(format!("{dart_param_name}: {arg_expr}"));
        } else {
            args.push(arg_expr);
        }
    }
}

fn render_int_arg(
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    // Scalar integer argument. Direct FRB calls use named parameters.
    let dart_param_name = underscore_camel_case(&arg_def.name);
    match arg_value {
        serde_json::Value::Number(n) => {
            if ctx.is_frb_bridge_call {
                args.push(format!("{dart_param_name}: {}", n));
            } else {
                args.push(n.to_string());
            }
        }
        serde_json::Value::Null if arg_def.optional => {
            // Optional int absent: omit it.
        }
        _ => {
            // Required int with no fixture value: emit 0 as default.
            if ctx.is_frb_bridge_call {
                args.push(format!("{dart_param_name}: 0"));
            } else {
                args.push("0".to_string());
            }
        }
    }
}

fn render_float_arg(
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    // Scalar float/number argument. Direct FRB calls use named parameters.
    let dart_param_name = underscore_camel_case(&arg_def.name);
    match arg_value {
        serde_json::Value::Number(n) => {
            if ctx.is_frb_bridge_call {
                args.push(format!("{dart_param_name}: {}", n));
            } else {
                args.push(n.to_string());
            }
        }
        serde_json::Value::Null if arg_def.optional => {
            // Optional float absent: omit it.
        }
        _ => {
            // Required float with no fixture value: emit 0.0 as default.
            if ctx.is_frb_bridge_call {
                args.push(format!("{dart_param_name}: 0.0"));
            } else {
                args.push("0.0".to_string());
            }
        }
    }
}

fn render_bool_arg(
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    // Scalar boolean argument. Direct FRB calls use named parameters.
    let dart_param_name = underscore_camel_case(&arg_def.name);
    match arg_value {
        serde_json::Value::Bool(b) => {
            let bool_str = if *b { "true" } else { "false" };
            if ctx.is_frb_bridge_call {
                args.push(format!("{dart_param_name}: {bool_str}"));
            } else {
                args.push(bool_str.to_string());
            }
        }
        serde_json::Value::Null if arg_def.optional => {
            // Optional bool absent: omit it.
        }
        _ => {
            // Required bool with no fixture value: emit false as default.
            if ctx.is_frb_bridge_call {
                args.push(format!("{dart_param_name}: false"));
            } else {
                args.push("false".to_string());
            }
        }
    }
}

fn render_string_arg(
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_index: usize,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    // Dart FRB bridge methods emit all parameters as named-required.
    // Hand-written facades use positional required and named optional.
    // Direct FRB bridge calls (is_frb_bridge_call = true) should emit all as named.
    // Facade methods (extractBytes, extractFile) keep required args positional.
    //
    // The `mime_type` parameter is special: it's positional in facade extract methods
    // but named in direct FRB bridge calls. The `client_factory` path is for stateful
    // clients (e.g., demo-client) which always use named parameters.
    let dart_param_name = underscore_camel_case(&arg_def.name);
    let mime_type_is_positional =
        arg_def.name == "mime_type" && !ctx.is_frb_bridge_call && ctx.client_factory_for_args.is_none();
    match arg_value {
        serde_json::Value::String(s) => {
            // The declared parameter type wins when the IR resolved one: a Dart enum
            // parameter rejects a string literal outright. Falls back to the literal
            // for every other declared type and for an absent IR. ~keep
            let literal = ir_typed_dart_expression(arg_def, arg_index, arg_value, ctx.target_params, ctx.enums)
                .unwrap_or_else(|| format!("'{}'", escape_dart(s)));
            // Direct FRB bridge calls: all parameters are named-required.
            // Client factory methods: all non-config parameters are named-required.
            // Facade methods: required positional, optional named.
            if ctx.is_frb_bridge_call || ctx.client_factory_for_args.is_some() || arg_def.optional {
                if !mime_type_is_positional {
                    args.push(format!("{dart_param_name}: {literal}"));
                } else {
                    args.push(literal);
                }
            } else {
                args.push(literal);
            }
        }
        serde_json::Value::Null
            if arg_def.optional
            // Optional string absent from fixture — try to infer MIME from path
            // when the arg name looks like a MIME-type parameter.
            && arg_def.name == "mime_type" =>
        {
            let inferred = ctx
                .file_path_for_mime
                .and_then(mime_from_extension)
                .unwrap_or("application/octet-stream");
            // Direct FRB bridge calls and client factory use named parameters.
            // Facades use positional for mime_type.
            if mime_type_is_positional {
                args.push(format!("'{inferred}'"));
            } else {
                args.push(format!("{dart_param_name}: '{inferred}'"));
            }
        }
        // Other optional strings with null value are omitted.
        _ => {}
    }
}

fn render_json_object_arg(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    if let Some(elem_type) = &arg_def.element_type {
        render_json_object_with_element_type(setup_lines, args, arg_def, elem_type, arg_value, ctx);
    } else if ctx.options_via == "from_json" {
        render_json_object_from_json_options(setup_lines, args, arg_def, arg_value, ctx);
    } else if ctx.call_recipe.should_materialize_json_object(arg_def, arg_value) && arg_value.is_null() {
        render_json_object_materialize_default(setup_lines, args, arg_def, ctx);
    } else if arg_def.name == "config" {
        render_config_arg(setup_lines, args, arg_def, arg_value, ctx);
    } else if arg_value.is_array() {
        render_json_object_generic_array(setup_lines, args, arg_def, arg_value, ctx);
    } else if let serde_json::Value::Object(_) = &arg_value {
        render_json_object_generic_object(setup_lines, args, arg_def, arg_value, ctx);
    }
}

fn render_json_object_with_element_type(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    elem_type: &str,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    if arg_value.is_object() {
        render_json_object_element_object(setup_lines, args, arg_def, elem_type, arg_value, ctx);
    } else if elem_type == "String" && arg_value.is_array() {
        render_json_object_string_array(setup_lines, args, arg_def, arg_value, ctx);
    } else if arg_value.is_array() {
        render_json_object_typed_array(setup_lines, args, arg_def, elem_type, arg_value, ctx);
    }
}

fn render_json_object_element_object(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    elem_type: &str,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
    let escaped_json = escape_dart(&json_str);
    let var_name = underscore_camel_case(&arg_def.name);
    let dart_fn = type_name_to_create_from_json_dart(elem_type);
    let json_source = if crate::e2e::codegen::value_contains_mock_url_placeholder(arg_value) {
        setup_lines.push(format!("final {var_name}MockBaseUrl = _fixtureUrl(\"{}\");", fixture.id));
        setup_lines.push(format!(
            "final {var_name}Json = '{escaped_json}'.replaceAll(r'{}', {var_name}MockBaseUrl);",
            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
        ));
        format!("{var_name}Json")
    } else {
        format!("'{escaped_json}'")
    };
    setup_lines.push(format!("final {var_name} = await {dart_fn}(json: {json_source});"));
    if ctx.is_frb_bridge_call {
        let dart_param_name = underscore_camel_case(&arg_def.name);
        args.push(format!("{dart_param_name}: {var_name}"));
    } else {
        args.push(var_name);
    }
}

fn render_json_object_string_array(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    // Scalar string array. Direct FRB bridge calls require named parameters.
    // Facades can declare these as required positional.
    let mock_base_var = if crate::e2e::codegen::value_contains_mock_url_placeholder(arg_value) {
        let var_name = format!("{}MockBaseUrl", arg_def.name);
        setup_lines.push(format!("final {var_name} = _fixtureUrl(\"{}\");", fixture.id));
        Some(var_name)
    } else {
        None
    };
    let items: Vec<String> = arg_value
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .map(|s| {
            if let Some(base_var) = mock_base_var.as_deref()
                && s.contains(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
            {
                format!(
                    "'{}'.replaceAll(r'{}', {base_var})",
                    escape_dart(s),
                    crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                )
            } else {
                format!("'{}'", escape_dart(s))
            }
        })
        .collect();
    let list_literal = format!("<String>[{}]", items.join(", "));
    if ctx.is_frb_bridge_call {
        let dart_param_name = underscore_camel_case(&arg_def.name);
        args.push(format!("{dart_param_name}: {list_literal}"));
    } else {
        args.push(list_literal);
    }
}

fn render_json_object_typed_array(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    elem_type: &str,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    // Generic typed array (for example `items: [BatchBytesItem]`). Decode via jsonDecode at
    // test-run time and convert to typed instances.
    let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
    let var_name = arg_def.name.clone();
    let json_source = if crate::e2e::codegen::value_contains_mock_url_placeholder(arg_value) {
        setup_lines.push(format!("final {var_name}MockBaseUrl = _fixtureUrl(\"{}\");", fixture.id));
        setup_lines.push(format!(
            "final {var_name}Json = r'{json_str}'.replaceAll(r'{}', {var_name}MockBaseUrl);",
            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
        ));
        format!("{var_name}Json")
    } else {
        format!("r'{json_str}'")
    };
    // FRB-generated `create<ElementType>FromJson(json:)` factory
    // takes a JSON string per item. Map each map to its typed
    // instance and await the futures together so the typed list
    // matches the binding's parameter type.
    let dart_fn = type_name_to_create_from_json_dart(elem_type);
    setup_lines.push(format!(
        "final {var_name} = await Future.wait((jsonDecode({json_source}) as List<dynamic>).map((element) => {dart_fn}(json: jsonEncode(element))));"
    ));
    // For generic arrays, emit named parameter if it's a direct FRB call
    if ctx.is_frb_bridge_call {
        let dart_param_name = underscore_camel_case(&arg_def.name);
        args.push(format!("{dart_param_name}: {var_name}"));
    } else {
        args.push(var_name);
    }
}

fn render_json_object_from_json_options(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    let type_defs = ctx.type_defs;
    let native_typed_dtos = ctx.native_typed_dtos;
    // `from_json` path: construct a typed mirror-struct via the generated
    // `create<TypeName>FromJson(json: '...')` bridge helper, then pass it
    // as the named FRB parameter `req: _var`.
    //
    // The helper is generated by `emit_from_json_fn` in the dart bridge-crate
    // generator and made available as a top-level function via the exported
    // the generated bridge package. The parameter name used in the
    // bridge method call is always `req:` for single-request-object methods
    // (derived from the Rust IR param name).
    if let Some(opts_type) = ctx
        .call_recipe
        .json_object_constructor_type(arg_def, arg_value)
        .or(ctx.options_type)
        && !arg_value.is_null()
    {
        if native_typed_dtos
            && let Some(expression) = super::super::values::render_native_dart_dto(
                opts_type,
                arg_value,
                type_defs,
                &fixture.docs_files_for_arg(&arg_def.field),
            )
        {
            let var_name = underscore_camel_case(&arg_def.name);
            setup_lines.push(format!("final {var_name} = {expression};"));
            args.push(format!("req: {var_name}"));
            return;
        }
        let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
        // Escape for Dart single-quoted string literal (handles embedded quotes,
        // backslashes, and interpolation markers).
        let escaped_json = escape_dart(&json_str);
        let var_name = underscore_camel_case(&arg_def.name);
        let dart_fn = type_name_to_create_from_json_dart(opts_type);
        setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{escaped_json}');"));
        // FRB bridge method param name is `req` for all single-request methods.
        // Use `req:` as the named argument label.
        args.push(format!("req: {var_name}"));
    }
}

fn render_json_object_materialize_default(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    ctx: &DartArgContext<'_>,
) {
    let type_defs = ctx.type_defs;
    let enums = ctx.enums;
    if let Some(opts_type) = ctx.options_type {
        let var_name = underscore_camel_case(&arg_def.name);
        let dart_fn = type_name_to_create_from_json_dart(opts_type);
        setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{{}}');"));
        // The declaration this call has to match is emitted by
        // `backends::dart::gen_bindings::functions::emit_function`, which makes only
        // a parameter named `config` named-optional, and only when it can synthesize
        // a default expression for it. Both sides read that one predicate rather than
        // re-deriving the shape, because each half is well-formed alone and only the
        // composed output can show a disagreement. Args not named `config` keep their
        // existing named-label behaviour here. ~keep
        let is_config_positional = arg_def.name == "config" && is_dart_config_positional(opts_type, type_defs, enums);
        if is_config_positional {
            args.push(var_name);
        } else {
            let dart_param_name = underscore_camel_case(&arg_def.name);
            args.push(format!("{dart_param_name}: {var_name}"));
        }
    }
}

fn render_config_arg(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    if let serde_json::Value::Object(map) = &arg_value {
        if !map.is_empty() {
            render_config_arg_object_nonempty(setup_lines, args, arg_def, arg_value, ctx);
        } else {
            render_config_arg_object_empty(setup_lines, args, arg_def, arg_value, ctx);
        }
    } else if arg_def.optional {
        render_config_arg_optional_absent(setup_lines, args, arg_def, arg_value, ctx);
    }
}

fn render_config_arg_object_nonempty(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let type_defs = ctx.type_defs;
    let enums = ctx.enums;
    // Round-trip object config JSON through a generated helper.
    // Resolve config type from explicit element_type first, then fall back
    // to options_type from the call recipe, then to the arg name as a last resort.
    let opts_type = ctx
        .call_recipe
        .json_object_constructor_type(arg_def, arg_value)
        .or(arg_def.element_type.as_deref())
        .or(ctx.options_type)
        .unwrap_or(&arg_def.name);
    let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
    let escaped_json = escape_dart(&json_str);
    let var_name = underscore_camel_case(&arg_def.name);
    let dart_fn = type_name_to_create_from_json_dart(opts_type);
    setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{escaped_json}');"));
    if is_dart_config_positional(opts_type, type_defs, enums) {
        args.push(var_name);
    } else {
        let dart_param_name = underscore_camel_case(&arg_def.name);
        args.push(format!("{dart_param_name}: {var_name}"));
    }
}

fn render_config_arg_object_empty(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let type_defs = ctx.type_defs;
    let enums = ctx.enums;
    // Empty config object: construct a default instance via FRB's
    // `create<Type>FromJson(json: '{}')` helper (supports all
    // configured config types). This ensures the
    // call signature matches the binding, which expects a required
    // config parameter even when all fields use their defaults.
    // Resolve config type from element_type, options_type, or arg name.
    let opts_type = ctx
        .call_recipe
        .json_object_constructor_type(arg_def, arg_value)
        .or(arg_def.element_type.as_deref())
        .or(ctx.options_type);
    if let Some(opts_type) = opts_type {
        let var_name = underscore_camel_case(&arg_def.name);
        let dart_fn = type_name_to_create_from_json_dart(opts_type);
        setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{{}}');"));
        if is_dart_config_positional(opts_type, type_defs, enums) {
            args.push(var_name);
        } else {
            let dart_param_name = underscore_camel_case(&arg_def.name);
            args.push(format!("{dart_param_name}: {var_name}"));
        }
    }
}

fn render_config_arg_optional_absent(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let type_defs = ctx.type_defs;
    let enums = ctx.enums;
    // Fixture has no config block (null/absent) but the Dart facade
    // declares the arg as a required-positional non-nullable type
    // (e.g. `embed_texts_async(texts, settings)` with `SampleSettings`).
    // Construct a default instance via FRB's
    // `create<Type>FromJson(json: '{}')` helper when IR metadata says
    // the configured type has a default.
    let opts_type = ctx
        .call_recipe
        .json_object_constructor_type(arg_def, arg_value)
        .or(arg_def.element_type.as_deref())
        .or(ctx.options_type);
    if let Some(opts_type) = opts_type.filter(|_| {
        ctx.call_recipe.json_object_arg_has_default(arg_def)
            || ctx.call_recipe.should_materialize_json_object(arg_def, arg_value)
    }) {
        let var_name = underscore_camel_case(&arg_def.name);
        let dart_fn = type_name_to_create_from_json_dart(opts_type);
        setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{{}}');"));
        if is_dart_config_positional(opts_type, type_defs, enums) {
            args.push(var_name);
        } else {
            let dart_param_name = underscore_camel_case(&arg_def.name);
            args.push(format!("{dart_param_name}: {var_name}"));
        }
    }
}

fn render_json_object_generic_array(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    // Generic JSON array (e.g. batch_urls: ["/page1", "/page2"]).
    // Decode via jsonDecode and cast to List<String> at test-run time.
    let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
    let var_name = arg_def.name.clone();
    setup_lines.push(format!(
        "final {var_name} = (jsonDecode(r'{json_str}') as List<dynamic>).cast<String>();"
    ));
    // Direct FRB bridge calls use named parameters
    if ctx.is_frb_bridge_call {
        let dart_param_name = underscore_camel_case(&arg_def.name);
        args.push(format!("{dart_param_name}: {var_name}"));
    } else {
        args.push(var_name);
    }
}

fn render_json_object_generic_object(
    setup_lines: &mut Vec<String>,
    args: &mut Vec<String>,
    arg_def: &ArgMapping,
    arg_value: &serde_json::Value,
    ctx: &DartArgContext<'_>,
) {
    let fixture = ctx.fixture;
    // Generic options-style json_object arg (for APIs whose
    // a typed options arg). When the
    // fixture provides input.options and the call config declares an
    // `options_type`, build the mirror struct via the FRB-generated
    // `create<OptionsType>FromJson(json: '...')` helper. Use the arg's
    // original name (e.g. `options`) as the named parameter label.
    //
    // When the fixture also carries a visitor spec, swap to the
    // `create<OptionsType>FromJsonWithVisitor(json, visitor)` helper
    // (emitted by `alef-backend-dart` for trait bridges with `type_alias`
    // + `options_field` binding). The `visitor` variable is materialised
    // in the visitor block below — its setup line is inserted ahead of
    // this options call by `build_dart_visitor`.
    if let serde_json::Value::Object(map) = &arg_value
        && !map.is_empty()
        && let Some(opts_type) = ctx
            .call_recipe
            .json_object_constructor_type(arg_def, arg_value)
            .or(ctx.options_type)
    {
        let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
        let escaped_json = escape_dart(&json_str);
        let dart_param_name = underscore_camel_case(&arg_def.name);
        let var_name = underscore_camel_case(&arg_def.name);
        let dart_fn = type_name_to_create_from_json_dart(opts_type);
        if fixture.visitor.is_some() {
            setup_lines.push(format!(
                "final {var_name} = await {dart_fn}WithVisitor(json: '{escaped_json}', visitor: visitor);"
            ));
        } else {
            setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{escaped_json}');"));
        }
        // Dart bridge method declares options as keyword-only parameter.
        // Always emit as named argument regardless of optionality.
        args.push(format!("{dart_param_name}: {var_name}"));
    }
}

fn is_dart_config_positional(opts_type: &str, type_defs: &[crate::core::ir::TypeDef], enums: &[EnumDef]) -> bool {
    !crate::backends::dart::config_param_is_named_optional("config", opts_type, type_defs, enums)
}

/// The Dart expression for an argument whose *declared* parameter type the core IR resolved, or
/// `None` to keep the existing `arg_type`-only lowering.
///
/// This is the Dart answer to the shared question [`TargetParams`] poses, not a shared verdict.
/// `ArgMapping::arg_type` defaults to `"string"`, so before the seam a fixture string bound for an
/// enum-typed parameter came out as a *quoted string literal*, which the Dart analyzer rejects
/// against a generated `enum` parameter -- flutter_rust_bridge mirrors a flat Rust enum as a real
/// Dart `enum`, and Dart has no implicit `String` -> enum conversion.
///
/// The replacement names the variant exactly as the Dart binding emitter does. `backends::dart::
/// gen_bindings::functions::render_enum_variant_default` spells a flat enum's variant
/// `{Enum}.{dart_safe_ident(variant.to_lower_camel_case())}` -- the `dart_safe_ident` pass is what
/// keeps a variant named after a Dart reserved word (`default` -> `default_`) resolvable. The
/// fixture value is a *serde wire* value, so it is matched against `wire_variant_value` rather than
/// against the Rust variant name. ~keep
///
/// Deliberately narrow, matching the Java/C#/Swift conversions. Only a bare `TypeRef::Named`
/// qualifies; only a **flat** (all-unit) enum qualifies, because a data-carrying enum becomes a
/// `sealed class` hierarchy whose variants are constructor calls, not bare references; and a wire
/// value naming no variant keeps its literal, because a fixture may be feeding a deliberately
/// invalid value to exercise the binding's own validation. ~keep
fn ir_typed_dart_expression(
    arg: &ArgMapping,
    index: usize,
    value: &serde_json::Value,
    target_params: TargetParams<'_>,
    enums: &[EnumDef],
) -> Option<String> {
    let TypeRef::Named(declared) = &target_params.param_for(&arg.name, index)?.ty else {
        return None;
    };
    let text = value.as_str()?;
    let enum_def = enums
        .iter()
        .find(|enum_def| &enum_def.name == declared && enum_def.variants.iter().all(|v| v.fields.is_empty()))?;
    let variant = enum_def.variants.iter().find(|variant| {
        crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        ) == text
    })?;
    let variant_name = crate::codegen::naming::dart_value_identifier(&heck::ToLowerCamelCase::to_lower_camel_case(
        variant.name.as_str(),
    ));
    Some(format!("{declared}.{variant_name}"))
}
