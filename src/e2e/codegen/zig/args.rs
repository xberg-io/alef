use super::assertions::json_to_zig;
use super::stubs::emit_test_backend_with_excluded;
use super::*;

pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    fixture_id: &str,
    _module_name: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
) -> (Vec<String>, String, bool) {
    if args.is_empty() {
        return (Vec::new(), String::new(), false);
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut setup_needs_gpa = false;

    for arg in args {
        if arg.arg_type == "mock_url" {
            let name = arg.name.clone();
            let id_upper = fixture_id.to_uppercase();
            setup_lines.push(format!(
                "const {name} = if (std.c.getenv(\"MOCK_SERVER_{id_upper}\")) |_pf| try std.fmt.allocPrint(allocator, \"{{s}}\", .{{std.mem.span(_pf)}}) else try std.fmt.allocPrint(allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}});"
            ));
            setup_lines.push(format!("defer allocator.free({name});"));
            parts.push(name);
            setup_needs_gpa = true;
            continue;
        }

        // Handle args (engine handle): serialize config to JSON string literal, or null.
        // The Zig binding accepts ?[]const u8 for engine params (creates handle internally).
        if arg.arg_type == "handle" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_str = match input.get(field) {
                Some(serde_json::Value::Null) | None => "null".to_string(),
                Some(v) => format!("\"{}\"", escape_zig(&serde_json::to_string(v).unwrap_or_default())),
            };
            parts.push(json_str);
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    let excluded_named =
                        crate::e2e::codegen::recipe::trait_bridge_excluded_type_names(config, type_defs, &methods);
                    let emission = emit_test_backend_with_excluded(trait_bridge, &methods, fixture, &excluded_named);
                    // emit_test_backend uses "lib." as a placeholder; substitute the real module.
                    let setup_block = emission.setup_block.replace("lib.", &format!("{_module_name}."));
                    let arg_expr = emission.arg_expr.replace("lib.", &format!("{_module_name}."));
                    // setup_block lines already carry no indentation (the caller adds 4 spaces).
                    // Push each logical line individually so the render loop adds uniform indent.
                    for line in setup_block.lines() {
                        setup_lines.push(line.to_string());
                    }
                    parts.push(arg_expr);
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("zig");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
            continue;
        }

        // The Zig wrapper accepts struct parameters
        // as JSON `[]const u8`, converting them to opaque FFI handles via the
        // `<prefix>_<snake>_from_json` helper at the binding layer. Emit the
        // fixture's configuration value as a JSON string literal, falling back
        // to `"{}"` when the fixture omits a config so callers exercise the
        // default path.
        if arg.name == "config" && arg.arg_type == "json_object" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_str = match input.get(field) {
                Some(serde_json::Value::Null) | None => "{}".to_string(),
                Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
            };
            parts.push(format!("\"{}\"", escape_zig(&json_str)));
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        // When `field` is empty or refers to `input` itself (no dotted subfield),
        // the entire fixture `input` value is the payload — most commonly for
        // `json_object` request bodies (chat/embed/etc.). Without this guard
        // `input.get("input")` returns `None` and we fall through to `"{}"`,
        // which the FFI rejects as a deserialization error.
        let val = if field.is_empty() || field == "input" {
            Some(input.get("extract_input").unwrap_or(input))
        } else {
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Zig functions don't have default arguments, so we must
                // pass `null` explicitly for every optional parameter.
                parts.push("null".to_string());
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    "json_object" => "\"{}\"".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // For `json_object` arguments other than `config` (handled
                // above) the Zig binding accepts a JSON `[]const u8`, so we
                // serialize the entire fixture value as a single JSON string
                // literal rather than rendering it as a Zig array/struct.
                if arg.arg_type == "json_object" {
                    let json_str = serde_json::to_string(v).unwrap_or_default();
                    if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                        let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                        let base_var = format!("{}_mock_base_url", arg.name);
                        let json_var = format!("{}_json", arg.name);
                        setup_lines.push(format!(
                            "const {base_var} = if (std.c.getenv(\"{env_key}\")) |_pf| try std.fmt.allocPrint(allocator, \"{{s}}\", .{{std.mem.span(_pf)}}) else try std.fmt.allocPrint(allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |url| std.mem.span(url) else \"http://localhost:8080\"}});"
                        ));
                        setup_lines.push(format!("defer allocator.free({base_var});"));
                        setup_lines.push(format!(
                            "const {json_var} = try std.mem.replaceOwned(u8, allocator, \"{}\", \"{}\", {base_var});",
                            escape_zig(&json_str),
                            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                        ));
                        setup_lines.push(format!("defer allocator.free({json_var});"));
                        parts.push(json_var);
                        setup_needs_gpa = true;
                    } else {
                        parts.push(format!("\"{}\"", escape_zig(&json_str)));
                    }
                } else if arg.arg_type == "bytes" {
                    // `bytes` args are file paths in fixtures — read the file into a
                    // local buffer. The cwd is set to test_documents/ at runtime.
                    // Zig 0.16 uses std.Io.Dir.cwd() (not std.fs.cwd()) and requires
                    // an `io` instance from std.testing.io in test context.
                    if let serde_json::Value::String(path) = v {
                        let var_name = format!("{}_bytes", arg.name);
                        let epath = escape_zig(path);
                        setup_lines.push(format!(
                            "const {var_name} = try std.Io.Dir.cwd().readFileAlloc(std.testing.io, \"{epath}\", std.heap.c_allocator, .unlimited);"
                        ));
                        setup_lines.push(format!("defer std.heap.c_allocator.free({var_name});"));
                        parts.push(var_name);
                    } else {
                        parts.push(json_to_zig(v));
                    }
                } else {
                    parts.push(json_to_zig(v));
                }
            }
        }
    }

    (setup_lines, parts.join(", "), setup_needs_gpa)
}
