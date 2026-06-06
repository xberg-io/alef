use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_java;
use heck::ToUpperCamelCase;

use super::values::{emit_java_object_array, is_numeric_type_hint, json_to_java, json_to_java_typed};

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
pub(super) struct JavaArgsContext<'a> {
    pub(super) class_name: &'a str,
    pub(super) options_type: Option<&'a str>,
    pub(super) fixture: &'a crate::e2e::fixture::Fixture,
    pub(super) adapter_request_type: Option<&'a str>,
    pub(super) owner_handle_is_receiver: bool,
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) teardown_block: &'a mut String,
}

pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    context: JavaArgsContext<'_>,
) -> (Vec<String>, String) {
    let JavaArgsContext {
        class_name,
        options_type,
        fixture,
        adapter_request_type,
        owner_handle_is_receiver,
        config,
        type_defs,
        teardown_block,
    } = context;
    let fixture_id = &fixture.id;
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                setup_lines.push(format!(
                    "String {} = System.getProperty(\"mockServer.{fixture_id}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\");",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "String {} = System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\";",
                    arg.name,
                ));
            }
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}Req", arg.name);
                setup_lines.push(format!("var {req_var} = new {req_type}({});", arg.name));
                parts.push(req_var);
            } else {
                parts.push(arg.name.clone());
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // List<String> of URLs: each element is either a bare path (`/seed1`) -
            // prefixed with the per-fixture mock-server URL at runtime - or an absolute
            // URL kept as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>`
            // env var first, then `MOCK_SERVER_URL/fixtures/<id>`. Emitted as a typed
            // `java.util.List<String>` so it matches the binding signature.
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_java(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            // Per-fixture mock-server URL resolution order:
            //   1. System.getProperty("mockServer.<fixture_id>") - populated by
            //      MockServerListener from the mock-server's MOCK_SERVERS=
            //      announcement (preferred for host-root-route fixtures).
            //   2. System.getenv("MOCK_SERVER_<FIXTURE_ID>") - explicit env override
            //      for CI / external harnesses.
            //   3. System.getenv("MOCK_SERVER_URL") + "/fixtures/<fixture_id>" -
            //      fallback to the shared-route URL for fixtures without host-root
            //      routes.
            // Previous code skipped (1), so any fixture with per-fixture host-root
            // routes hit /fixtures/<id>/<path> on the shared host - which mock-server
            // doesn't serve - and returned 404 for every batch URL.
            setup_lines.push(format!(
                "String {name}Base = System.getProperty(\"mockServer.{fixture_id}\", System.getenv().getOrDefault(\"{env_key}\", (System.getProperty(\"mockServerUrl\") != null ? System.getProperty(\"mockServerUrl\") : (System.getenv(\"MOCK_SERVER_URL\") != null ? System.getenv(\"MOCK_SERVER_URL\") : \"http://localhost:8000\")) + \"/fixtures/{fixture_id}\"));"
            ));
            setup_lines.push(format!(
                "java.util.List<String> {name} = java.util.Arrays.stream(new String[]{{{paths_literal}}}).map(p -> p.startsWith(\"http\") ? p : {name}Base + p).collect(java.util.stream.Collectors.toList());"
            ));
            // Wrap in adapter request type if present (e.g., BatchedStreamItemsRequest).
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}Req", arg.name);
                setup_lines.push(format!("var {req_var} = new {req_type}({});", arg.name));
                parts.push(req_var);
            } else {
                parts.push(name.clone());
            }
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a createEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("var {} = {class_name}.{constructor_name}(null);", arg.name,));
            } else {
                let json_str = serde_json::to_string(config_value).unwrap_or_default();
                let name = &arg.name;
                if let Some(config_type) = resolve_handle_config_type(arg, options_type, type_defs) {
                    setup_lines.push(format!(
                        "var {name}Config = MAPPER.readValue(\"{}\", {config_type}.class);",
                        escape_java(&json_str),
                    ));
                    setup_lines.push(format!(
                        "var {} = {class_name}.{constructor_name}({name}Config);",
                        arg.name,
                        name = name,
                    ));
                } else {
                    setup_lines.push(format!("var {} = {class_name}.{constructor_name}(null);", arg.name,));
                }
            }
            // For streaming owner_type adapters the handle is the instance-method
            // receiver, not a positional argument - emit its construction but omit
            // it from the call's argument list.
            if owner_handle_is_receiver {
                continue;
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    // Filter to only methods that appear in the Java trait-bridge interface.
                    // Async methods (extract_bytes, extract_file) are handled by the FFI bridge internally.
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| {
                            t.methods
                                .iter()
                                .filter(|m| {
                                    // Skip methods in the ffi_skip_methods list
                                    if trait_bridge.ffi_skip_methods.contains(&m.name) {
                                        return false;
                                    }

                                    // Skip only known non-trait methods not in Java trait-bridge interfaces
                                    match m.name.as_str() {
                                        "description" | "author" => return false,
                                        _ => {}
                                    }

                                    // As of the trait method extraction fix, methods returning excluded types
                                    // are now kept in the interface with type substitution.
                                    // Methods like extract_bytes/extract_file and backend_type are now included.
                                    true
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    // Include super-trait methods so the stub can implement them.
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait) {
                            for method in &super_type.methods {
                                if !methods.iter().any(|m| m.name == method.name)
                                    && !trait_bridge.ffi_skip_methods.contains(&method.name)
                                    && !matches!(method.name.as_str(), "description" | "author")
                                {
                                    methods.push(method);
                                }
                            }
                        }
                    }

                    let excluded_named =
                        crate::e2e::codegen::recipe::trait_bridge_excluded_type_names(config, type_defs, &methods);

                    // Do NOT filter out methods that return excluded types. As of the trait method extraction
                    // fix, trait methods with excluded type signatures are now kept in the interface with type
                    // substitution (excluded types become String). The trait-bridge interface properly handles
                    // these via emit_test_backend_with_context, which uses excluded_named to substitute types.

                    // Call java::stubs::emit_test_backend_with_context so stubs handle excluded types correctly.
                    let emission = super::stubs::emit_test_backend_with_context(
                        trait_bridge,
                        &methods,
                        fixture,
                        &config.java_package(),
                        &excluded_named,
                        class_name,
                    );
                    setup_lines.push(emission.setup_block);
                    parts.push(emission.arg_expr);
                    teardown_block.push_str(&emission.teardown_block);
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("java");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
            continue;
        }

        let resolved = super::super::resolve_field(input, &arg.field);
        let val = if resolved.is_null() { None } else { Some(resolved) };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: emit positional null/default so the call
                // has the right arity. For json_object optional args, build an empty default object
                // so we get the right type rather than a raw null.
                if arg.arg_type == "json_object" {
                    if let Some(opts_type) = options_type {
                        parts.push(format!("{opts_type}.builder().build()"));
                    } else {
                        parts.push("null".to_string());
                    }
                } else {
                    parts.push("null".to_string());
                }
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
                let default_val = match arg.arg_type.as_str() {
                    "string" | "file_path" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0d".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "json_object" {
                    // Array json_object args: emit inline Java list expression.
                    if v.is_array() {
                        if let Some(elem_type) = &arg.element_type {
                            // For complex types, deserialize each array element via JsonUtil.
                            if !is_numeric_type_hint(elem_type) {
                                parts.push(emit_java_object_array(v, elem_type));
                                continue;
                            }
                        }
                        // Otherwise use element_type to emit the correct numeric literal suffix (f vs d).
                        let elem_type = arg.element_type.as_deref();
                        parts.push(json_to_java_typed(v, elem_type));
                        continue;
                    }
                    // Object json_object args with options_type: use pre-deserialized variable.
                    if options_type.is_some() {
                        parts.push(arg.name.clone());
                        continue;
                    }
                    parts.push(json_to_java(v));
                    continue;
                }
                // bytes args carry a relative file path (e.g. "docx/fake.docx") that the
                // e2e harness resolves against test_documents/. Read the file at runtime,
                // not the raw path string's UTF-8 bytes.
                if arg.arg_type == "bytes" {
                    let val = json_to_java(v);
                    parts.push(format!(
                        "java.nio.file.Files.readAllBytes(java.nio.file.Path.of({val}))"
                    ));
                    continue;
                }
                // file_path args must be wrapped in java.nio.file.Path.of().
                if arg.arg_type == "file_path" {
                    let val = json_to_java(v);
                    parts.push(format!("java.nio.file.Path.of({val})"));
                    continue;
                }
                parts.push(json_to_java(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
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
        let candidate = format!("{}Config", arg.name.to_upper_camel_case());
        type_defs.iter().any(|ty| ty.name == candidate).then_some(candidate)
    })
}
