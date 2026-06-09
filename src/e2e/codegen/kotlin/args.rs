//! Kotlin argument construction and setup helpers.

use heck::ToUpperCamelCase;

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::ArgMapping;
use crate::e2e::escape::escape_kotlin;
use crate::e2e::fixture::Fixture;

/// Build setup lines and the argument list for the function call.
///
/// Returns `(setup_lines, args_string)`.
///
/// `kotlin_android_style = true` switches the optional-`json_object` default
/// from `OptionsType.builder().build()` to `null`. The Java-facade-backed
/// JVM target emits a Java-style builder for every `json_object` type, but
/// the kotlin_android backend emits plain Kotlin data classes with no
/// `.builder()` companion (every field is declared without a default), so a
/// builder call would not compile. The Android facade signatures declare the
/// optional argument as `T? = null`, making `null` the idiomatic positional
/// default that matches the call arity.
pub(super) struct KotlinArgsContext<'a> {
    pub(super) fixture: &'a Fixture,
    pub(super) class_name: &'a str,
    pub(super) options_type: Option<&'a str>,
    pub(super) fixture_id: &'a str,
    pub(super) kotlin_android_style: bool,
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
}

pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[ArgMapping],
    context: KotlinArgsContext<'_>,
) -> (Vec<String>, String) {
    let KotlinArgsContext {
        fixture,
        class_name,
        options_type,
        fixture_id,
        kotlin_android_style,
        config,
        type_defs,
    } = context;
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            if fixture.has_host_root_route() {
                setup_lines.push(format!(
                    "val {} = System.getProperty(\"mockServer.{fixture_id}\", (System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            } else {
                setup_lines.push(format!(
                    "val {} = (System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\") ?: \"\") ?: \"\") + \"/fixtures/{fixture_id}\"",
                    arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "handle" {
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
            {
                setup_lines.push(format!("val {} = {class_name}.{constructor_name}(null)", arg.name,));
            } else {
                let json_str = serde_json::to_string(config_value).unwrap_or_default();
                let name = &arg.name;
                if let Some(config_type) = super::test_file::resolve_handle_config_type(arg, options_type, type_defs) {
                    setup_lines.push(format!(
                        "val {name}Config = MAPPER.readValue(\"{}\", {config_type}::class.java)",
                        escape_kotlin(&json_str),
                    ));
                    setup_lines.push(format!(
                        "val {} = {class_name}.{constructor_name}({name}Config)",
                        arg.name,
                        name = name,
                    ));
                } else {
                    setup_lines.push(format!("val {} = {class_name}.{constructor_name}(null)", arg.name,));
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
                        // Extract the simple name from the full path (e.g., "Plugin" from "sample_core::plugins::Plugin").
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

                    // For kotlin_android, filter out methods whose return type or parameters
                    // reference types in the `exclude_types` list.  The binding generator
                    // omits those methods from the generated interface, so the test stub
                    // must not attempt to implement them.
                    if kotlin_android_style {
                        let excluded: std::collections::HashSet<&str> = config
                            .kotlin_android
                            .as_ref()
                            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
                            .unwrap_or_default();
                        if !excluded.is_empty() {
                            methods.retain(|m| {
                                !excluded.iter().any(|ex| m.return_type.references_named(ex))
                                    && m.params
                                        .iter()
                                        .all(|p| !excluded.iter().any(|ex| p.ty.references_named(ex)))
                            });
                        }
                    }

                    let lang = if kotlin_android_style {
                        "kotlin_android"
                    } else {
                        "kotlin"
                    };
                    let emission = crate::e2e::codegen::emit_test_backend(lang, trait_bridge, &methods, fixture);
                    setup_lines.push(emission.setup_block);
                    parts.push(emission.arg_expr);
                    continue;
                }
            }
            let lang = if kotlin_android_style {
                "kotlin_android"
            } else {
                "kotlin"
            };
            let emission = crate::e2e::codegen::TestBackendEmission::unimplemented(lang);
            setup_lines.push(format!("// {}", emission.arg_expr));
            parts.push("null".to_string());
            continue;
        }

        // Use resolve_field so field = "input" resolves to the whole fixture input.
        let val_resolved = crate::e2e::codegen::resolve_field(input, &arg.field);
        let val: Option<&serde_json::Value> = if val_resolved.is_null() {
            None
        } else {
            Some(val_resolved)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Optional arg with no fixture value: emit positional default so the
                // call has the right arity for the facade.
                //
                // For json_object optional args:
                // - If options_type is set, use `OptionsType()` for kotlin_android (data class
                //   constructor with defaults) or `OptionsType.builder().build()` for Java facade.
                // - If no options_type, infer the type from arg.name and emit default constructor
                //   (e.g., a configured default constructor for an options arg). This handles both Java facade
                //   (which requires non-null) and kotlin_android (which also declares non-null).
                if arg.arg_type == "json_object" {
                    let default_constructor = if let Some(opts_type) = options_type {
                        if kotlin_android_style {
                            format!("{}()", opts_type)
                        } else {
                            format!("{}.builder().build()", opts_type)
                        }
                    } else {
                        let candidate = format!("{}Config", arg.name.to_upper_camel_case());
                        let inferred_type = if type_defs.iter().any(|t| t.name == candidate) {
                            candidate
                        } else {
                            arg.name.to_upper_camel_case()
                        };
                        format!("{}()", inferred_type)
                    };
                    parts.push(default_constructor);
                } else {
                    parts.push("null".to_string());
                }
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // Typed arrays carry `element_type` and are materialised as `listOf(...)`.
                // For kotlin_android batch APIs the element type is a binding class
                // (e.g. FileBytesItem) that wraps file bytes + content type, so string
                // path entries must be read into ByteArray and wrapped in a constructor call.
                // For JVM bindings the element type matches the JSON literal shape, so we emit
                // the raw `listOf(literals)` form.
                if arg.arg_type == "json_object" && v.is_array() && arg.element_type.is_some() {
                    if let Some(element_type) = arg.element_type.as_deref().filter(|_| kotlin_android_style) {
                        let items: Vec<String> = v
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .map(|item| {
                                        if let Some(path) = item.as_str() {
                                            format!(
                                                "{element_type}(java.nio.file.Files.readAllBytes(java.nio.file.Paths.get(\"{}\")), java.nio.charset.StandardCharsets.UTF_8)",
                                                escape_kotlin(path)
                                            )
                                        } else {
                                            super::values::json_to_kotlin(item)
                                        }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        parts.push(format!("listOf({})", items.join(", ")));
                        continue;
                    }
                    let items: Vec<String> = v
                        .as_array()
                        .map(|arr| arr.iter().map(super::values::json_to_kotlin).collect())
                        .unwrap_or_default();
                    parts.push(format!("listOf({})", items.join(", ")));
                    continue;
                }
                // For json_object args with options_type, use the pre-deserialized variable.
                if arg.arg_type == "json_object" && options_type.is_some() {
                    parts.push(arg.name.clone());
                    continue;
                }
                // bytes args in Kotlin binding carry a relative file path (e.g. "docx/fake.docx")
                // that the Kotlin API resolves and reads internally.
                // - JVM binding: pass the path string directly
                // - android binding: need to read bytes and wrap in ByteArray
                if arg.arg_type == "bytes" {
                    let val = super::values::json_to_kotlin(v);
                    if kotlin_android_style {
                        // kotlin_android needs ByteArray, not String path
                        // Emit code to read the file as bytes
                        if v.is_string() {
                            parts.push(format!(
                                "java.nio.file.Files.readAllBytes(java.nio.file.Paths.get({val}))"
                            ));
                        } else {
                            parts.push("byteArrayOf()".to_string());
                        }
                    } else {
                        parts.push(val);
                    }
                    continue;
                }
                // file_path args: Kotlin module wraps the Java facade (which takes Path),
                // but kotlin_android has a different signature that takes a plain String.
                if arg.arg_type == "file_path" {
                    let val = super::values::json_to_kotlin(v);
                    if kotlin_android_style {
                        // kotlin_android binding takes a plain String path
                        parts.push(val);
                    } else {
                        // Kotlin (JVM) binding re-exports Java facade which takes java.nio.file.Path
                        parts.push(format!("java.nio.file.Path.of({val})"));
                    }
                    continue;
                }
                parts.push(super::values::json_to_kotlin(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}
