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
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if let Some(url) = crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value) {
                parts.push(format!("\"{}\"", escape_zig(url)));
                continue;
            }
            let id_upper = fixture_id.to_uppercase();
            setup_lines.push(format!(
                "const {name} = if (std.c.getenv(\"MOCK_SERVER_{id_upper}\")) |_pf| try std.fmt.allocPrint(allocator, \"{{s}}\", .{{std.mem.span(_pf)}}) else try std.fmt.allocPrint(allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}});"
            ));
            setup_lines.push(format!("defer allocator.free({name});"));
            parts.push(name);
            setup_needs_gpa = true;
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            let value = crate::e2e::codegen::resolve_urls_field(input, &arg.field);
            if let Some(urls) = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, value) {
                let values = urls
                    .iter()
                    .map(|url| format!("\"{}\"", escape_zig(url)))
                    .collect::<Vec<_>>()
                    .join(", ");
                parts.push(format!("&[_][]const u8{{{values}}}"));
                continue;
            }
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
            if let Some(trait_name) = &arg.trait_name
                && let Some(trait_bridge) = config
                    .trait_bridges_for(crate::core::config::Language::Zig)
                    .find(|tb| tb.trait_name == *trait_name)
            {
                let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                    .iter()
                    .find(|t| t.name == *trait_name)
                    .map(|t| t.methods.iter().collect())
                    .unwrap_or_default();
                let excluded_named = crate::e2e::codegen::recipe::trait_bridge_excluded_type_names(
                    config,
                    crate::core::config::Language::Zig,
                    type_defs,
                    &methods,
                );
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
            // A `test_backend` arg fills a required Zig stub parameter — there is no
            // compilable value to fall back to when the trait isn't configured. Fail
            // generation loudly instead of silently splicing a `null` argument with a
            // comment where the real stub belongs. ~keep
            panic!(
                "Zig e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate a Zig stub without a resolvable trait bridge",
                fixture.id, arg.name, arg.trait_name
            );
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
                    let docs_files = fixture.docs_files_for_arg(&arg.field);
                    if !docs_files.is_empty() {
                        let (lines, expression) = render_docs_json(&arg.name, v, &docs_files);
                        setup_lines.extend(lines);
                        parts.push(expression);
                        setup_needs_gpa = true;
                        continue;
                    }
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
                    if let serde_json::Value::String(path) = v {
                        let var_name = format!("{}_bytes", arg.name);
                        let io_name = format!("{var_name}_io");
                        let threaded_name = format!("{var_name}_threaded");
                        let epath = escape_zig(path);
                        setup_lines.push(format!(
                            "var {threaded_name} = std.Io.Threaded.init(std.heap.c_allocator, .{{}});"
                        ));
                        setup_lines.push(format!("defer {threaded_name}.deinit();"));
                        setup_lines.push(format!("const {io_name} = {threaded_name}.io();"));
                        setup_lines.push(format!(
                            "const {var_name} = try std.Io.Dir.cwd().readFileAlloc({io_name}, \"{epath}\", std.heap.c_allocator, .unlimited);"
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

fn render_docs_json(
    variable: &str,
    value: &serde_json::Value,
    files: &[crate::e2e::fixture::FixtureDocsFileInput],
) -> (Vec<String>, String) {
    let mut value = value.clone();
    let mut reads = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let marker = format!("__ALEF_DOC_FILE_{index}__");
        let target = if file.field.is_empty() {
            Some(&mut value)
        } else {
            value.pointer_mut(&file.field)
        };
        let Some(target) = target else { continue };
        *target = serde_json::Value::String(marker.clone());
        reads.push((index, marker, file.path.clone()));
    }
    let mut lines = Vec::new();
    let mut source = format!("\"{}\"", escape_zig(&serde_json::to_string(&value).unwrap_or_default()));
    for (index, marker, path) in reads {
        lines.push(
            crate::e2e::template_env::render(
                "zig/docs_file_read.jinja",
                minijinja::context! { variable => variable, index => index, path => escape_zig(&path) },
            )
            .trim_end()
            .to_string(),
        );
        lines.push(
            crate::e2e::template_env::render(
                "zig/docs_file_json.jinja",
                minijinja::context! { variable => variable, index => index },
            )
            .trim_end()
            .to_string(),
        );
        let output = format!("{variable}_json_{index}");
        lines.push(
            crate::e2e::template_env::render(
                "zig/docs_json_replace.jinja",
                minijinja::context! {
                    output => output,
                    source => source,
                    marker => escape_zig(&format!("\"{marker}\"")),
                    variable => variable,
                    index => index,
                },
            )
            .trim_end()
            .to_string(),
        );
        source = output;
    }
    (lines, source)
}

#[cfg(test)]
mod docs_file_tests {
    use super::render_docs_json;
    use crate::e2e::fixture::FixtureDocsFileInput;

    #[test]
    fn nested_typed_dto_files_become_runtime_json_byte_arrays() {
        let (lines, expression) = render_docs_json(
            "request",
            &serde_json::json!({"content": "ignored"}),
            &[FixtureDocsFileInput {
                field: "/content".into(),
                path: "document.pdf".into(),
            }],
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("readFileAlloc") && line.contains("document.pdf"))
        );
        assert!(lines.iter().any(|line| line.contains("emit_strings_as_arrays")));
        assert_eq!(expression, "request_json_0");
    }

    #[test]
    fn runtime_file_io_compiles_as_a_zig_binary() {
        let zig = std::process::Command::new("zig").arg("version").output();
        if zig.is_err() {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let source_path = directory.path().join("main.zig");
        let source = r#"const std = @import("std");
pub fn main(init: std.process.Init) !void {
    var content_bytes_threaded = std.Io.Threaded.init(std.heap.c_allocator, .{});
    defer content_bytes_threaded.deinit();
    const content_bytes_io = content_bytes_threaded.io();
    const content_bytes = try std.Io.Dir.cwd().readFileAlloc(content_bytes_io, "Cargo.toml", std.heap.c_allocator, .unlimited);
    defer std.heap.c_allocator.free(content_bytes);
    _ = init;
}
"#;
        std::fs::write(&source_path, source).expect("write Zig source");
        // `-lc` mirrors the `.link_libc = true` that `zig/build.rs` writes into every generated
        // e2e `build.zig`: the emitted body allocates through `std.heap.c_allocator`, which Zig
        // rejects outright unless libc is linked. Without the flag this check compiled the
        // snippet under settings no generated project actually uses, and passed only on macOS,
        // where libSystem is linked implicitly. ~keep
        let output = std::process::Command::new("zig")
            .args(["build-exe", "-fno-emit-bin", "-lc"])
            .arg(&source_path)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run Zig compiler");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
}
