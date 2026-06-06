//! C# argument setup rendering for generated e2e tests.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_csharp;
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::collections::HashMap;

use super::stubs::emit_test_backend_with_class_name;
use super::{classify_bytes_value_csharp, json_to_csharp, resolve_handle_config_type};

/// Build setup lines (e.g. handle creation) and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    class_name: &str,
    options_type: Option<&str>,
    options_via: Option<&str>,
    enum_fields: &HashMap<String, String>,
    nested_types: &HashMap<String, String>,
    fixture: &crate::e2e::fixture::Fixture,
    adapter_request_type: Option<&str>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    class_decls: &mut Vec<String>,
    teardown_lines: &mut Vec<String>,
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "bytes" {
            // bytes args must be passed as byte[] in C#.
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            match val {
                None | Some(serde_json::Value::Null) if arg.optional => {
                    parts.push("null".to_string());
                }
                None | Some(serde_json::Value::Null) => {
                    parts.push("System.Array.Empty<byte>()".to_string());
                }
                Some(v) => {
                    // Classify the value to determine how to interpret it:
                    // - File paths (like "pdf/fake.pdf") → File.ReadAllBytes(path)
                    // - Inline text → System.Text.Encoding.UTF8.GetBytes()
                    // - Base64 → Convert.FromBase64String()
                    if let Some(s) = v.as_str() {
                        let bytes_code = classify_bytes_value_csharp(s);
                        parts.push(bytes_code);
                    } else {
                        // Literal arrays or other non-string types: use as-is
                        let cs_str = json_to_csharp(v);
                        parts.push(format!("System.Text.Encoding.UTF8.GetBytes({cs_str})"));
                    }
                }
            }
            continue;
        }

        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
                setup_lines.push(format!(
                    "var _pfUrl_{name} = Environment.GetEnvironmentVariable(\"{env_key}\");",
                    name = arg.name,
                ));
                setup_lines.push(format!(
                    "var {} = !string.IsNullOrEmpty(_pfUrl_{name}) ? _pfUrl_{name} : Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\";",
                    arg.name,
                    name = arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "var {} = Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\";",
                    arg.name,
                ));
            }
            if let Some(req_type) = adapter_request_type {
                let req_var = format!("{}Req", arg.name);
                setup_lines.push(format!("var {req_var} = new {req_type} {{ Url = {} }};", arg.name));
                parts.push(req_var);
            } else {
                parts.push(arg.name.clone());
            }
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // List<string> of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL kept as-is.
            // Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first, then
            // `MOCK_SERVER_URL/fixtures/<id>`. Emitted as a typed `List<string>` so it matches
            // the C# binding signature (`Task<BatchScrapeResults> BatchScrapeAsync(handle, List<string> urls)`),
            // which does not accept `string[]`.
            let env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            // Try both the declared field and common aliases (batch_urls, urls, etc.)
            let val = if let Some(v) = input.get(field).filter(|v| !v.is_null()) {
                v.clone()
            } else {
                crate::e2e::codegen::resolve_urls_field(input, &arg.field).clone()
            };
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_csharp(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            let name = &arg.name;
            setup_lines.push(format!(
                "var _pfBase_{name} = Environment.GetEnvironmentVariable(\"{env_key}\");"
            ));
            setup_lines.push(format!(
                "var _base_{name} = !string.IsNullOrEmpty(_pfBase_{name}) ? _pfBase_{name} : Environment.GetEnvironmentVariable(\"MOCK_SERVER_URL\") + \"/fixtures/{fixture_id}\";"
            ));
            setup_lines.push(format!(
                "var {name} = new System.Collections.Generic.List<string>(new string[] {{ {paths_literal} }}.Select(p => p.StartsWith(\"http\") ? p : _base_{name} + p));"
            ));
            parts.push(name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            // Generate a CreateEngine (or equivalent) call and pass the variable.
            let constructor_name = format!("Create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("var {} = {class_name}.{constructor_name}(null);", arg.name,));
            } else {
                // Sort discriminator fields ("type") to appear first in nested objects so
                // System.Text.Json [JsonPolymorphic] can find the type discriminator before
                // reading other properties (a requirement as of .NET 8).
                let sorted = sort_discriminator_first(config_value.clone());
                let json_str = serde_json::to_string(&sorted).unwrap_or_default();
                let name = &arg.name;
                if let Some(config_type) = resolve_handle_config_type(arg, options_type, type_defs) {
                    setup_lines.push(format!(
                        "var {name}Config = JsonSerializer.Deserialize<{config_type}>(\"{}\", ConfigOptions)!;",
                        escape_csharp(&json_str),
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
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    // Collect methods from both the main trait and its super-trait (if present).
                    // The super-trait methods are needed so stubs implement the full interface.
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();

                    // If there's a super-trait, also collect its methods.
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        // Extract the simple name from the full path (e.g., "Plugin" from "crate::plugins::Plugin").
                        let super_trait_simple = super_trait.rsplit("::").next().unwrap_or(super_trait.as_str());
                        if let Some(super_type) = type_defs.iter().find(|t| t.name == super_trait_simple) {
                            for method in &super_type.methods {
                                // Only add if not already present (avoid duplicates).
                                if !methods.iter().any(|m| m.name == method.name) {
                                    methods.push(method);
                                }
                            }
                        }
                    }

                    let enum_names: std::collections::HashSet<&str> = enums.iter().map(|e| e.name.as_str()).collect();
                    let excluded_named = crate::e2e::codegen::recipe::trait_bridge_excluded_type_names_with_enums(
                        config,
                        type_defs,
                        &methods,
                        &enum_names,
                    );
                    let emission =
                        emit_test_backend_with_class_name(trait_bridge, &methods, fixture, class_name, &excluded_named);
                    // setup_block is a private nested class declaration — must be at class
                    // scope in C#, not inside the method body.
                    class_decls.push(emission.setup_block);
                    parts.push(emission.arg_expr);
                    if !emission.teardown_block.is_empty() {
                        teardown_lines.push(emission.teardown_block);
                    }
                    continue;
                }
            }
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("csharp");
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
            continue;
        }

        // When field is exactly "input", treat the entire input object as the value.
        // This matches the convention used by other language generators (e.g. Go).
        let val: Option<&serde_json::Value> = if arg.field == "input" {
            Some(input)
        } else {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: pass null explicitly since
                // C# nullable parameters still require an argument at the call site.
                parts.push("null".to_string());
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                // Required arg with no fixture value: pass a language-appropriate default.
                // For json_object args with a known options_type, use `new OptionsType()`
                // so the generated code compiles when the method parameter is non-nullable.
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0d".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    "json_object" => {
                        if arg.optional {
                            // Optional parameter can be null
                            "null".to_string()
                        } else if let Some(opts_type) = options_type {
                            // Required parameter: construct default instance
                            format!("new {opts_type}()")
                        } else if let Some(elem_type) = &arg.element_type {
                            // Try element_type as fallback for config type
                            format!("new {elem_type}()")
                        } else {
                            // Infer from argument name: try directly (e.g., "config" → "Config"),
                            // then try with "Config" suffix (e.g., "config" → "ConfigConfig").
                            let name_upper = arg.name.to_upper_camel_case();
                            let candidates = [name_upper.clone(), format!("{name_upper}Config")];
                            let matched = candidates
                                .iter()
                                .find(|cand| type_defs.iter().any(|ty| ty.name == **cand))
                                .cloned();
                            if let Some(inferred_type) = matched {
                                format!("new {inferred_type}()")
                            } else {
                                // Cannot determine type; pass null (will fail at runtime with ArgumentNullException)
                                "null".to_string()
                            }
                        }
                    }
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                if arg.arg_type == "json_object" {
                    // `options_via = "from_json"`: deserialize the entire value (object,
                    // array, or scalar) as the options type. This sidesteps per-field
                    // type ambiguity — e.g. `JsonElement?` (untagged unions) or
                    // `List<NamedRecord>` whose element type cannot be inferred from
                    // JSON shape alone — by delegating to System.Text.Json.
                    if options_via == Some("from_json")
                        && let Some(opts_type) = options_type
                    {
                        let sorted = sort_discriminator_first(v.clone());
                        let json_str = serde_json::to_string(&sorted).unwrap_or_default();
                        let escaped = escape_csharp(&json_str);
                        // Use the binding-emitted `<Type>.FromJson(...)` factory so any
                        // System.Text.Json deserialization failure is wrapped in
                        // `<Crate>Exception`, allowing error fixtures asserting
                        // `Assert.ThrowsAny<<Crate>Exception>(...)` to catch the parse
                        // failure (e.g. `Unknown FilePurpose value: invalid-purpose`).
                        parts.push(format!("{opts_type}.FromJson(\"{escaped}\")",));
                        continue;
                    }
                    // Array value: generate a typed List<T> based on element_type.
                    if let Some(arr) = v.as_array() {
                        parts.push(json_array_to_csharp_list(arr, arg.element_type.as_deref()));
                        continue;
                    }
                    // Object value with known type: generate idiomatic C# object initializer.
                    if let Some(opts_type) = options_type {
                        if let Some(obj) = v.as_object() {
                            parts.push(csharp_object_initializer(obj, opts_type, enum_fields, nested_types));
                            continue;
                        }
                    }
                }
                parts.push(json_to_csharp(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

/// Convert a JSON array to a typed C# `List<T>` expression.
///
/// Mapping from `ArgMapping::element_type`:
/// - `None` or any string type → `List<string>`
/// - `"f32"` → `List<float>` with `(float)` casts
/// - `"(String, String)"` → `List<List<string>>` for key-value pair arrays
fn json_array_to_csharp_list(arr: &[serde_json::Value], element_type: Option<&str>) -> String {
    match element_type {
        Some("f32") => {
            let items: Vec<String> = arr.iter().map(|v| format!("(float){}", json_to_csharp(v))).collect();
            format!("new List<float>() {{ {} }}", items.join(", "))
        }
        Some("(String, String)") => {
            let items: Vec<String> = arr
                .iter()
                .map(|v| {
                    let strs: Vec<String> = v
                        .as_array()
                        .map_or_else(Vec::new, |a| a.iter().map(json_to_csharp).collect());
                    format!("new List<string>() {{ {} }}", strs.join(", "))
                })
                .collect();
            format!("new List<List<string>>() {{ {} }}", items.join(", "))
        }
        Some(et) if et != "f32" && et != "(String, String)" && et != "string" => {
            // Class/record types: deserialize each element from JSON
            let items: Vec<String> = arr
                .iter()
                .map(|v| {
                    let json_str = serde_json::to_string(v).unwrap_or_default();
                    let escaped = escape_csharp(&json_str);
                    format!("JsonSerializer.Deserialize<{et}>(\"{escaped}\", ConfigOptions)!")
                })
                .collect();
            format!("new List<{et}>() {{ {} }}", items.join(", "))
        }
        _ => {
            let items: Vec<String> = arr.iter().map(json_to_csharp).collect();
            format!("new List<string>() {{ {} }}", items.join(", "))
        }
    }
}

/// Recursively sort JSON objects so that any key named `"type"` appears first.
///
/// System.Text.Json's `[JsonPolymorphic]` requires the type discriminator to be
/// the first property when deserializing polymorphic types. Fixture config values
/// serialised via serde_json preserve insertion/alphabetical order, which may put
/// `"type"` after other keys (e.g. `"password"` before `"type"` in auth configs).
fn sort_discriminator_first(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted = serde_json::Map::with_capacity(map.len());
            // Insert "type" first if present.
            if let Some(type_val) = map.get("type") {
                sorted.insert("type".to_string(), sort_discriminator_first(type_val.clone()));
            }
            for (k, v) in map {
                if k != "type" {
                    sorted.insert(k, sort_discriminator_first(v));
                }
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_discriminator_first).collect())
        }
        other => other,
    }
}

/// Emit a C# object initializer for a JSON options object.
///
/// - camelCase fixture keys → PascalCase C# property names
/// - Enum fields (from `enum_fields`) → `EnumType.Member`
/// - Nested objects with known type (from `nested_types`) → `JsonSerializer.Deserialize<T>(...)`
/// - Arrays → `new List<string> { ... }`
/// - Primitives → C# literals via `json_to_csharp`
fn csharp_object_initializer(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    enum_fields: &HashMap<String, String>,
    nested_types: &HashMap<String, String>,
) -> String {
    if obj.is_empty() {
        return format!("new {type_name}()");
    }

    // Snake_case fixture keys for fields that are real C# enums in the binding.
    // The fixture string value (e.g. "markdown") maps to `EnumType.Member` (e.g. `OutputFormat.Markdown`).
    static IMPLICIT_ENUM_FIELDS: &[(&str, &str)] = &[("output_format", "OutputFormat")];

    let props: Vec<String> = obj
        .iter()
        .map(|(key, val)| {
            let pascal_key = key.to_upper_camel_case();
            let implicit_enum_type = IMPLICIT_ENUM_FIELDS
                .iter()
                .find(|(k, _)| *k == key.as_str())
                .map(|(_, t)| *t);
            // Check enum_fields both with the original snake_case key AND with camelCase key.
            // The alef.toml config uses camelCase keys (e.g., "codeBlockStyle"), but fixture
            // JSON uses snake_case keys (e.g., "code_block_style"). So we check both.
            let camel_key = key.to_lower_camel_case();
            let cs_val = if let Some(enum_type) = enum_fields
                .get(key.as_str())
                .or_else(|| enum_fields.get(camel_key.as_str()))
                .map(String::as_str)
                .or(implicit_enum_type)
            {
                // Enum: EnumType.Member
                if val.is_null() {
                    "null".to_string()
                } else {
                    let member = val
                        .as_str()
                        .map(|s| s.to_upper_camel_case())
                        .unwrap_or_else(|| "null".to_string());
                    format!("{enum_type}.{member}")
                }
            } else if let Some(nested_type) = nested_types
                .get(key.as_str())
                .or_else(|| nested_types.get(camel_key.as_str()))
            {
                // Nested type: deserialize via JsonSerializer using the binding's custom converters.
                // This handles sealed records, custom JsonConverters, and sealed unions correctly.
                let normalized = normalize_csharp_enum_values(val, enum_fields);
                let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                let escaped = escape_csharp(&json_str);
                format!("JsonSerializer.Deserialize<{nested_type}>(\"{escaped}\", ConfigOptions)!")
            } else if let Some(arr) = val.as_array() {
                // Array: List<string>
                let items: Vec<String> = arr.iter().map(json_to_csharp).collect();
                format!("new List<string> {{ {} }}", items.join(", "))
            } else {
                json_to_csharp(val)
            };
            format!("{pascal_key} = {cs_val}")
        })
        .collect();
    format!("new {} {{ {} }}", type_name, props.join(", "))
}

/// Convert enum values in a JSON object to lowercase to match C# [JsonPropertyName] attributes.
/// The JSON deserialization uses JsonPropertyName("lowercase_value"), so fixture enum values
/// (typically PascalCase like "Tildes") must be converted to lowercase ("tildes") for correct
/// deserialization with JsonStringEnumConverter.
fn normalize_csharp_enum_values(value: &serde_json::Value, enum_fields: &HashMap<String, String>) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut result = map.clone();
            for (key, val) in result.iter_mut() {
                // Check both snake_case and camelCase keys, since alef.toml uses camelCase
                // but fixture JSON uses snake_case.
                let camel_key = key.to_lower_camel_case();
                if enum_fields.contains_key(key) || enum_fields.contains_key(camel_key.as_str()) {
                    // This is an enum field; convert the string value to lowercase.
                    if let Some(s) = val.as_str() {
                        *val = serde_json::Value::String(s.to_lowercase());
                    }
                }
            }
            serde_json::Value::Object(result)
        }
        other => other.clone(),
    }
}
