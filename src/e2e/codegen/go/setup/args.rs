//! Per-argument-type setup and call-expression rendering for [`super::build_args_and_setup`].
//!
//! Split out of `setup.rs` to keep that file under this crate's file-length limit. Every
//! function here is a direct extraction of one branch of the original per-argument loop -- same
//! conditions, same emitted strings, same ordering -- so moving code here changes nothing about
//! the generated Go test files. ~keep

use crate::e2e::escape::go_string_literal;

use crate::e2e::codegen::go::json_values::{convert_json_for_go, element_type_to_go_slice, json_to_go};

use super::{
    GoArgsContext, GoValueContext, ensure_value_helpers, go_empty_value_expression, go_enum_shape,
    native_go_dto_literal, qualified_go_type, typed_named_argument_expression,
};

/// Per-argument values computed once per loop iteration in [`super::build_args_and_setup`] and
/// threaded into the render helpers below -- kept together because every helper that takes one
/// takes at least two. ~keep
#[derive(Clone, Copy)]
pub(super) struct GoArgRenderContext<'a> {
    pub arg_index: usize,
    pub fixture_id: &'a str,
    pub json_object_type: Option<&'a str>,
    pub native_declared_type: Option<&'a str>,
    pub value_context: GoValueContext<'a>,
}

pub(super) fn render_mock_url_arg(
    arg: &crate::e2e::config::ArgMapping,
    input: &serde_json::Value,
    fixture: &crate::e2e::fixture::Fixture,
    fixture_id: &str,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
    let value = input.get(field).unwrap_or(&serde_json::Value::Null);
    if let Some(url) = crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value) {
        setup_lines.push(format!("{} := {}", arg.name, go_string_literal(url)));
    } else if fixture.has_host_root_route() {
        let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
        setup_lines.push(format!("{} := os.Getenv(\"{env_key}\")", arg.name));
        setup_lines.push(format!(
            "if {} == \"\" {{ {} = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\" }}",
            arg.name, arg.name
        ));
    } else {
        setup_lines.push(format!(
            "{} := os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"",
            arg.name,
        ));
    }
    parts.push(arg.name.clone());
}

pub(super) fn render_mock_url_list_arg(
    arg: &crate::e2e::config::ArgMapping,
    input: &serde_json::Value,
    fixture: &crate::e2e::fixture::Fixture,
    fixture_id: &str,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
    let val = input.get(field).unwrap_or(&serde_json::Value::Null);
    let var_name = &arg.name;

    if let Some(urls) = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, val) {
        let literals: Vec<String> = urls.iter().map(|url| go_string_literal(url)).collect();
        setup_lines.push(format!("{var_name} := []string{{{}}}", literals.join(", ")));
        parts.push(var_name.to_string());
        return;
    }

    let paths: Vec<String> = if let Some(arr) = val.as_array() {
        arr.iter().filter_map(|v| v.as_str().map(go_string_literal)).collect()
    } else {
        Vec::new()
    };

    let paths_literal = paths.join(", ");

    setup_lines.push(format!(
        "{var_name}Base := os.Getenv(\"{env_key}\")\n\tif {var_name}Base == \"\" {{\n\t\t{var_name}Base = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"\n\t}}"
    ));
    setup_lines.push(format!(
        "var {var_name} []string\n\tfor _, p := range []string{{{paths_literal}}} {{\n\t\tif strings.HasPrefix(p, \"http\") {{\n\t\t\t{var_name} = append({var_name}, p)\n\t\t}} else {{\n\t\t\t{var_name} = append({var_name}, {var_name}Base + p)\n\t\t}}\n\t}}"
    ));
    parts.push(var_name.to_string());
}

pub(super) fn render_test_backend_arg(
    arg: &crate::e2e::config::ArgMapping,
    fixture: &crate::e2e::fixture::Fixture,
    context: GoArgsContext<'_>,
    package_decls: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    let GoArgsContext {
        import_alias,
        config,
        type_defs,
        enums,
        ..
    } = context;
    if let Some(trait_name) = &arg.trait_name
        && let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name)
    {
        let emission = crate::e2e::codegen::go::test_backend::resolve_test_backend_emission(
            fixture,
            trait_name,
            trait_bridge,
            config,
            type_defs,
            enums,
            import_alias,
        );
        package_decls.push(emission.setup_block);
        parts.push(emission.arg_expr);
        return;
    }
    // A `test_backend` arg fills a required Go stub parameter — there is no
    // compilable value to fall back to when the trait isn't configured. Fail
    // generation loudly instead of silently splicing a `nil` argument with a
    // comment where the real stub belongs. ~keep
    panic!(
        "Go e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate a Go stub without a resolvable trait bridge",
        fixture.id, arg.name, arg.trait_name
    );
}

pub(super) fn render_handle_arg(
    arg: &crate::e2e::config::ArgMapping,
    input: &serde_json::Value,
    context: GoArgsContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    use heck::ToUpperCamelCase;

    let GoArgsContext {
        import_alias,
        options_type,
        expects_error,
        type_defs,
        ..
    } = context;

    let constructor_name = format!("Create{}", arg.name.to_upper_camel_case());
    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
    let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
    let create_err_handler = if expects_error {
        "assert.Error(t, createErr)\n\t\treturn".to_string()
    } else {
        "t.Fatalf(\"create handle failed: %v\", createErr)".to_string()
    };
    if config_value.is_null() || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty()) {
        setup_lines.push(format!(
            "{name}, createErr := {import_alias}.{constructor_name}(nil)\n\tif createErr != nil {{\n\t\t{create_err_handler}\n\t}}",
            name = arg.name,
        ));
    } else {
        let json_str = serde_json::to_string(config_value).unwrap_or_default();
        let go_literal = go_string_literal(&json_str);
        let name = &arg.name;
        if let Some(config_type) = resolve_handle_config_type(arg, options_type, type_defs) {
            setup_lines.push(format!(
                "var {name}Config {import_alias}.{config_type}\n\tif err := json.Unmarshal([]byte({go_literal}), &{name}Config); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
            ));
            setup_lines.push(format!(
                "{name}, createErr := {import_alias}.{constructor_name}(&{name}Config)\n\tif createErr != nil {{\n\t\t{create_err_handler}\n\t}}"
            ));
        } else {
            setup_lines.push(format!(
                "{name}, createErr := {import_alias}.{constructor_name}(nil)\n\tif createErr != nil {{\n\t\t{create_err_handler}\n\t}}"
            ));
        }
    }
    parts.push(arg.name.clone());
}

/// `Ok(true)` means an empty-native-DTO literal was pushed and the caller should move on to the
/// next argument; `Ok(false)` leaves `setup_lines`/`parts` untouched so the caller falls through
/// to its other rendering arms exactly as if this check had never run. ~keep
pub(super) fn try_render_empty_native_json_object(
    arg: &crate::e2e::config::ArgMapping,
    val: Option<&serde_json::Value>,
    render_ctx: GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) -> anyhow::Result<bool> {
    let GoArgsContext {
        options_type,
        options_ptr,
        native_dtos,
        ..
    } = context;
    let GoArgRenderContext {
        json_object_type,
        value_context,
        ..
    } = render_ctx;
    if native_dtos
        && arg.arg_type == "json_object"
        && val.is_none_or(serde_json::Value::is_null)
        && let Some(type_name) = json_object_type
        && let Some(literal) = native_go_dto_literal(
            &serde_json::Value::Object(serde_json::Map::new()),
            type_name,
            value_context,
        )?
    {
        setup_lines.push(format!("{} := {literal}", arg.name));
        // Every other `json_object` branch below consults `options_ptr` before deciding how to
        // pass the value; this one did not, so a fixture that supplies no options at all bound a
        // typed empty DTO and then handed the binding a value where its signature declares `*T`.
        // That is the whole of the "cannot use options (variable of struct type X) as *X" wall
        // -- it hit every fixture without an options object, which is most of them. ~keep
        parts.push(if Some(type_name) == options_type && options_ptr {
            format!("&{}", arg.name)
        } else {
            arg.name.clone()
        });
        return Ok(true);
    }
    Ok(false)
}

pub(super) fn render_bytes_arg(
    arg: &crate::e2e::config::ArgMapping,
    val: Option<&serde_json::Value>,
    setup_lines: &mut Vec<String>,
    parts: &mut Vec<String>,
) {
    let var_name = format!("{}Bytes", arg.name);
    match val {
        None | Some(serde_json::Value::Null) => {
            if arg.optional {
                parts.push("nil".to_string());
            } else {
                parts.push("[]byte{}".to_string());
            }
        }
        Some(serde_json::Value::String(s)) => {
            let go_path = go_string_literal(s);
            setup_lines.push(format!(
                "{var_name}, {var_name}Err := os.ReadFile({go_path})\n\tif {var_name}Err != nil {{\n\t\tt.Fatalf(\"read fixture {s}: %v\", {var_name}Err)\n\t}}"
            ));
            parts.push(var_name);
        }
        Some(other) => {
            parts.push(format!("[]byte({})", json_to_go(other)));
        }
    }
}

pub(super) fn optional_null_argument_expression(
    arg: &crate::e2e::config::ArgMapping,
    render_ctx: GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
) -> String {
    let GoArgsContext {
        import_alias,
        options_ptr,
        enums,
        ..
    } = context;
    let GoArgRenderContext { json_object_type, .. } = render_ctx;
    match arg.arg_type.as_str() {
        "string" => "nil".to_string(),
        "json_object" => {
            if options_ptr {
                "nil".to_string()
            } else if let Some(opts_type) = json_object_type {
                go_empty_value_expression(import_alias, opts_type, enums)
            } else {
                "nil".to_string()
            }
        }
        _ => "nil".to_string(),
    }
}

pub(super) fn required_null_argument_expression(
    arg: &crate::e2e::config::ArgMapping,
    render_ctx: GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
) -> String {
    let GoArgsContext {
        import_alias,
        options_ptr,
        enums,
        ..
    } = context;
    let GoArgRenderContext { json_object_type, .. } = render_ctx;
    match arg.arg_type.as_str() {
        "string" => "\"\"".to_string(),
        "int" | "integer" | "i64" => "0".to_string(),
        "float" | "number" => "0.0".to_string(),
        "bool" | "boolean" => "false".to_string(),
        "json_object" => {
            if options_ptr {
                "nil".to_string()
            } else if let Some(opts_type) = json_object_type {
                go_empty_value_expression(import_alias, opts_type, enums)
            } else {
                "nil".to_string()
            }
        }
        _ => "nil".to_string(),
    }
}

pub(super) fn render_json_object_argument(
    v: &serde_json::Value,
    arg: &crate::e2e::config::ArgMapping,
    render_ctx: GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
    package_decls: &mut Vec<String>,
    setup_lines: &mut Vec<String>,
) -> anyhow::Result<String> {
    let GoArgsContext {
        import_alias,
        options_type,
        options_ptr,
        enums,
        native_dtos,
        ..
    } = context;
    let GoArgRenderContext {
        json_object_type,
        value_context,
        ..
    } = render_ctx;

    let is_array = v.is_array();
    let is_empty_obj = !is_array && v.is_object() && v.as_object().is_some_and(|o| o.is_empty());
    if native_dtos
        && !is_array
        && let Some(opts_type) = json_object_type
        && let Some(literal) = native_go_dto_literal(v, opts_type, value_context)?
    {
        ensure_value_helpers(package_decls, &literal);
        setup_lines.push(format!("{} := {}", arg.name, literal.replace('\n', "\n\t")));
        let arg_expr = if Some(opts_type) == options_type && options_ptr {
            format!("&{}", arg.name)
        } else {
            arg.name.clone()
        };
        return Ok(arg_expr);
    }
    if is_empty_obj {
        return Ok(if options_ptr {
            "nil".to_string()
        } else if let Some(opts_type) = json_object_type {
            go_empty_value_expression(import_alias, opts_type, enums)
        } else {
            "nil".to_string()
        });
    }
    if is_array {
        return Ok(render_json_object_array_argument(v, arg, render_ctx, context, setup_lines));
    }
    let Some(opts_type) = json_object_type else {
        return Ok(json_to_go(v));
    };
    Ok(render_json_object_typed_argument(v, arg, opts_type, render_ctx, context, setup_lines))
}

fn render_json_object_array_argument(
    v: &serde_json::Value,
    arg: &crate::e2e::config::ArgMapping,
    render_ctx: GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
    setup_lines: &mut Vec<String>,
) -> String {
    let GoArgsContext {
        import_alias,
        data_enum_names,
        ..
    } = context;
    let GoArgRenderContext { fixture_id, .. } = render_ctx;

    let go_slice_type = if let Some(go_t) = arg.go_type.as_deref() {
        if go_t.starts_with('[') {
            go_t.to_string()
        } else {
            let qualified = if go_t.contains('.') {
                go_t.to_string()
            } else {
                format!("{import_alias}.{go_t}")
            };
            format!("[]{qualified}")
        }
    } else {
        element_type_to_go_slice(arg.element_type.as_deref(), import_alias)
    };

    let element_type_name = if let Some(go_t) = arg.go_type.as_deref() {
        if go_t.starts_with('[') {
            None
        } else if let Some(idx) = go_t.rfind('.') {
            Some(&go_t[idx + 1..])
        } else {
            Some(go_t)
        }
    } else {
        arg.element_type.as_deref()
    };

    let is_sum_type = element_type_name.is_some_and(|et| data_enum_names.contains(et));
    let converted_v = convert_json_for_go(v.clone());
    let var_name = &arg.name;
    let json_str = serde_json::to_string(&converted_v).unwrap_or_default();
    let go_literal = go_string_literal(&json_str);
    if crate::e2e::codegen::value_contains_mock_url_placeholder(&converted_v) {
        let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
        setup_lines.push(format!(
            "{var_name}MockBaseURL := os.Getenv(\"{env_key}\")\n\tif {var_name}MockBaseURL == \"\" {{\n\t\t{var_name}MockBaseURL = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"\n\t}}"
        ));
        setup_lines.push(format!(
            "{var_name}JSON := strings.ReplaceAll({go_literal}, \"{}\", {var_name}MockBaseURL)",
            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
        ));
    }
    let json_expr = if crate::e2e::codegen::value_contains_mock_url_placeholder(&converted_v) {
        format!("{var_name}JSON")
    } else {
        go_literal
    };

    if is_sum_type {
        let element_type = element_type_name.unwrap();
        setup_lines.push(format!(
            "var {var_name}Raw []json.RawMessage\n\tif err := json.Unmarshal([]byte({json_expr}), &{var_name}Raw); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
        ));
        setup_lines.push(format!(
            "var {var_name} {go_slice_type}\n\tfor _, raw := range {var_name}Raw {{\n\t\telem, err := {import_alias}.Unmarshal{element_type}(raw)\n\t\tif err != nil {{\n\t\t\tt.Fatalf(\"unmarshal {element_type} failed: %v\", err)\n\t\t}}\n\t\t{var_name} = append({var_name}, elem)\n\t}}"
        ));
    } else {
        setup_lines.push(format!(
            "var {var_name} {go_slice_type}\n\tif err := json.Unmarshal([]byte({json_expr}), &{var_name}); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
        ));
    }
    var_name.to_string()
}

fn render_json_object_typed_argument(
    v: &serde_json::Value,
    arg: &crate::e2e::config::ArgMapping,
    opts_type: &str,
    render_ctx: GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
    setup_lines: &mut Vec<String>,
) -> String {
    let GoArgsContext {
        import_alias,
        options_type,
        options_ptr,
        enums,
        ..
    } = context;
    let GoArgRenderContext { fixture_id, .. } = render_ctx;

    let remapped_v = if Some(opts_type) == options_type && options_ptr {
        convert_json_for_go(v.clone())
    } else {
        v.clone()
    };
    let json_str = serde_json::to_string(&remapped_v).unwrap_or_default();
    let go_literal = go_string_literal(&json_str);
    let var_name = &arg.name;
    if crate::e2e::codegen::value_contains_mock_url_placeholder(&remapped_v) {
        let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
        setup_lines.push(format!(
            "{var_name}MockBaseURL := os.Getenv(\"{env_key}\")\n\tif {var_name}MockBaseURL == \"\" {{\n\t\t{var_name}MockBaseURL = os.Getenv(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\"\n\t}}"
        ));
        setup_lines.push(format!(
            "{var_name}JSON := strings.ReplaceAll({go_literal}, \"{}\", {var_name}MockBaseURL)",
            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
        ));
    }
    let json_expr = if crate::e2e::codegen::value_contains_mock_url_placeholder(&remapped_v) {
        format!("{var_name}JSON")
    } else {
        go_literal
    };
    // `encoding/json` cannot unmarshal into an interface value: `var x
    // pkg.T; json.Unmarshal(data, &x)` compiles for a sealed-interface `T`
    // and then fails at run time with `cannot unmarshal object into Go
    // value of type pkg.T`. The Go binding backend emits a
    // `Unmarshal<T>(data []byte) (T, error)` dispatcher for exactly this,
    // which the sibling array arm above already uses for its elements. ~keep
    if matches!(
        go_enum_shape(enums, opts_type),
        Some(crate::backends::go::GoEnumRepresentation::DataInterface)
    ) {
        let go_enum_name = crate::codegen::naming::go_type_name(opts_type);
        setup_lines.push(format!(
            "{var_name}, {var_name}Err := {import_alias}.Unmarshal{go_enum_name}([]byte({json_expr}))\n\tif {var_name}Err != nil {{\n\t\tt.Fatalf(\"unmarshal {go_enum_name} failed: %v\", {var_name}Err)\n\t}}"
        ));
        return var_name.to_string();
    }
    let type_name = qualified_go_type(import_alias, opts_type);
    setup_lines.push(format!(
        "var {var_name} {type_name}\n\tif err := json.Unmarshal([]byte({json_expr}), &{var_name}); err != nil {{\n\t\tt.Fatalf(\"config parse failed: %v\", err)\n\t}}"
    ));
    if Some(opts_type) == options_type && options_ptr {
        format!("&{var_name}")
    } else {
        var_name.to_string()
    }
}

pub(super) fn render_optional_string_argument(
    v: &serde_json::Value,
    arg: &crate::e2e::config::ArgMapping,
    render_ctx: GoArgRenderContext<'_>,
    package_decls: &mut Vec<String>,
    setup_lines: &mut Vec<String>,
) -> anyhow::Result<String> {
    let GoArgRenderContext {
        native_declared_type,
        value_context,
        ..
    } = render_ctx;
    // An optional parameter is emitted as `*T`, and `&{name}Val` where
    // `{name}Val` is bound to a bare string literal is a `*string` — correct
    // only when `T` really is `string`. A declared enum needs the typed
    // expression bound instead, so the address taken is of a value of the
    // parameter's own type. ~keep
    let var_name = format!("{}Val", arg.name);
    let typed = if let Some(type_name) = native_declared_type {
        typed_named_argument_expression(v, type_name, value_context, &arg.name, false)?
    } else {
        None
    };
    let go_val = typed.unwrap_or_else(|| json_to_go(v));
    ensure_value_helpers(package_decls, &go_val);
    setup_lines.push(format!("{var_name} := {}", go_val.replace('\n', "\n\t")));
    Ok(format!("&{var_name}"))
}

/// The catch-all every non-`json_object`, non-`bytes` argument falls into.
///
/// Without a declared type it can only stringify the fixture value, which lands a bare literal
/// against whatever the parameter really is; with one it renders the expression that type
/// actually takes. ~keep
pub(super) fn render_typed_default_argument(
    v: &serde_json::Value,
    arg: &crate::e2e::config::ArgMapping,
    render_ctx: GoArgRenderContext<'_>,
    context: GoArgsContext<'_>,
    package_decls: &mut Vec<String>,
    setup_lines: &mut Vec<String>,
) -> anyhow::Result<String> {
    let GoArgsContext { target, .. } = context;
    let GoArgRenderContext {
        arg_index,
        native_declared_type,
        value_context,
        ..
    } = render_ctx;
    let typed = if let Some(type_name) = native_declared_type {
        let uses_pointer = target.param_for(&arg.name, arg_index).is_some_and(|param| param.optional);
        typed_named_argument_expression(v, type_name, value_context, &arg.name, uses_pointer)?
    } else {
        None
    };
    if let Some(expression) = typed {
        ensure_value_helpers(package_decls, &expression);
        // A DTO literal spans lines; an argument list cannot, so it is bound to
        // the argument's own variable and passed by name. ~keep
        if expression.contains('\n') {
            setup_lines.push(format!("{} := {}", arg.name, expression.replace('\n', "\n\t")));
            Ok(arg.name.clone())
        } else {
            Ok(expression)
        }
    } else {
        Ok(json_to_go(v))
    }
}

fn resolve_handle_config_type(
    arg: &crate::e2e::config::ArgMapping,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    if arg.arg_type != "handle" {
        return None;
    }
    options_type.map(str::to_string).or_else(|| {
        let candidate = format!("{}Config", arg.name.to_uppercase_first());
        type_defs.iter().any(|ty| ty.name == candidate).then_some(candidate)
    })
}

trait UppercaseFirst {
    fn to_uppercase_first(&self) -> String;
}

impl UppercaseFirst for str {
    fn to_uppercase_first(&self) -> String {
        let mut chars = self.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }
}
