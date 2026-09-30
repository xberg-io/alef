use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, TypeRef};
use crate::e2e::codegen::call_ir::TargetParams;
use crate::e2e::escape::escape_java;
use heck::ToUpperCamelCase;

use super::values::{
    emit_java_object_array, is_java_builtin_type, is_numeric_type_hint, json_to_java, json_to_java_typed,
};

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
    /// The IR enum registry, needed to tell an enum-typed parameter from a struct-typed one:
    /// enums are not in `type_defs`, so without this a `Named` parameter that happens to be an
    /// enum looks like an unknown type and falls back to a quoted literal. ~keep
    pub(super) enums: &'a [EnumDef],
    /// What the core IR declares about the target's parameters -- see
    /// [`ir_typed_java_expression`]. [`TargetParams::IrAbsent`] keeps the pre-IR lowering, so a
    /// call site that has no IR to supply is unaffected. ~keep
    pub(super) target_params: TargetParams<'a>,
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
        enums,
        target_params,
        teardown_block,
    } = context;
    let fixture_id = &fixture.id;
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for (index, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let value = input.get(field).unwrap_or(&serde_json::Value::Null);
            if let Some(url) = crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value) {
                setup_lines.push(format!("String {} = \"{}\";", arg.name, escape_java(url)));
            } else if fixture.has_host_root_route() {
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
            let val = crate::e2e::codegen::resolve_urls_field(input, &arg.field);
            let name = &arg.name;

            if let Some(urls) = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, val) {
                let literals: Vec<String> = urls.iter().map(|url| format!("\"{}\"", escape_java(url))).collect();
                setup_lines.push(format!(
                    "java.util.List<String> {name} = java.util.List.of({});",
                    literals.join(", ")
                ));
                if let Some(req_type) = adapter_request_type {
                    let req_var = format!("{}Req", arg.name);
                    setup_lines.push(format!("var {req_var} = new {req_type}({});", arg.name));
                    parts.push(req_var);
                } else {
                    parts.push(name.clone());
                }
                continue;
            }

            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_java(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
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
                        "var {name}Config = MAPPER.readValue({}, {config_type}.class);",
                        super::values::java_string_literal(&json_str),
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
            if let Some(trait_name) = &arg.trait_name
                && let Some(trait_bridge) = config
                    .trait_bridges_for(crate::core::config::Language::Java)
                    .find(|tb| tb.trait_name == *trait_name)
            {
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
                if let Some(super_trait) = &trait_bridge.super_trait
                    && let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait)
                {
                    for method in &super_type.methods {
                        if !methods.iter().any(|m| m.name == method.name)
                            && !trait_bridge.ffi_skip_methods.contains(&method.name)
                            && !matches!(method.name.as_str(), "description" | "author")
                        {
                            methods.push(method);
                        }
                    }
                }

                // `gen_interface_file` declares `name()`/`version()` abstract on `I<Trait>`
                // unconditionally whenever `super_trait` is configured -- it never looks up the
                // super-trait's own `TypeDef` (see `trait_bridge_naming::SUPER_TRAIT_REQUIRED_METHODS`).
                // The lookup above does, by matching `rust_path`, and finds nothing for a
                // super-trait declared in a private module and re-exported via `pub use` (its
                // `rust_path` need not equal the configured value), silently leaving the stub
                // without either method and failing to compile against the interface's guarantee.
                // Synthesize whichever required method the lookup above did not already supply,
                // instead of re-deriving the same convention a second way. ~keep
                let synthetic_super_trait_methods: Vec<crate::core::ir::MethodDef> =
                    if trait_bridge.super_trait.is_some() {
                        crate::backends::java::gen_bindings::trait_bridge_naming::SUPER_TRAIT_REQUIRED_METHODS
                            .iter()
                            .filter(|required| !methods.iter().any(|m| m.name == required.name))
                            .map(|required| crate::core::ir::MethodDef {
                                name: required.name.to_string(),
                                return_type: crate::core::ir::TypeRef::String,
                                ..Default::default()
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                methods.extend(synthetic_super_trait_methods.iter());

                // `trait_bridge_excluded_type_names` (no enum registry) treats every enum-typed
                // signature as excluded by default -- enums live in `enums`, not `type_defs`, so
                // `collect_hidden_named_types` sees no `TypeDef` for one and falls back to
                // "unknown, therefore excluded". That cannot distinguish a real, non-excluded Java
                // enum from one the crate's own `exclude_types` really does marshal as `String`.
                // Passing the IR enum names, minus the ones this language's config excludes, is
                // exactly what `trait_bridge_excluded_type_names_with_enums`'s `known_enum_names`
                // parameter exists for -- see its doc comment. ~keep
                let configured_excluded_types = crate::docs::language_pages::excludes::language_excludes(
                    config,
                    crate::core::config::Language::Java,
                )
                .1;
                let known_enum_names: std::collections::HashSet<&str> = enums
                    .iter()
                    .map(|enum_def| enum_def.name.as_str())
                    .filter(|name| !configured_excluded_types.contains(*name))
                    .collect();
                let excluded_named = crate::e2e::codegen::recipe::trait_bridge_excluded_type_names_with_enums(
                    config,
                    crate::core::config::Language::Java,
                    type_defs,
                    &methods,
                    &known_enum_names,
                );

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
            // A `test_backend` arg fills a non-null Java stub parameter — there is no
            // compilable value to fall back to when the trait isn't configured. Fail
            // generation loudly instead of silently splicing a `null` argument with a
            // comment where the real stub belongs. ~keep
            panic!(
                "Java e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate a Java stub without a resolvable trait bridge",
                fixture.id, arg.name, arg.trait_name
            );
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
                            if elem_type == "String"
                                && crate::e2e::codegen::value_contains_mock_url_placeholder(v)
                                && let Some(items) = v.as_array()
                            {
                                let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                let base_var = format!("{}MockBaseUrl", arg.name);
                                setup_lines.push(format!(
                                    "String {base_var} = System.getProperty(\"mockServer.{fixture_id}\", System.getenv().getOrDefault(\"{env_key}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\"));"
                                ));
                                let item_exprs: Vec<String> = items
                                    .iter()
                                    .map(|item| {
                                        if let Some(raw) = item.as_str()
                                            && raw.contains(crate::e2e::codegen::MOCK_URL_PLACEHOLDER)
                                        {
                                            format!(
                                                "\"{}\".replace(\"{}\", {base_var})",
                                                escape_java(raw),
                                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                                            )
                                        } else {
                                            json_to_java_typed(item, Some(elem_type))
                                        }
                                    })
                                    .collect();
                                parts.push(format!("java.util.List.of({})", item_exprs.join(", ")));
                                continue;
                            }
                            // For complex types, deserialize each array element via JsonUtil.
                            if !is_numeric_type_hint(elem_type) && !is_java_builtin_type(elem_type) {
                                if crate::e2e::codegen::value_contains_mock_url_placeholder(v)
                                    && let Some(items) = v.as_array()
                                {
                                    let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                    let base_var = format!("{}MockBaseUrl", arg.name);
                                    setup_lines.push(format!(
                                        "String {base_var} = System.getProperty(\"mockServer.{fixture_id}\", System.getenv().getOrDefault(\"{env_key}\", System.getProperty(\"mockServerUrl\", System.getenv(\"MOCK_SERVER_URL\")) + \"/fixtures/{fixture_id}\"));"
                                    ));
                                    let item_exprs: Vec<String> = items
                                        .iter()
                                        .enumerate()
                                        .map(|(idx, item)| {
                                            let json_str = serde_json::to_string(item).unwrap_or_default();
                                            let escaped = escape_java(&json_str);
                                            let json_var = format!("{}Json{idx}", arg.name);
                                            setup_lines.push(format!(
                                                "String {json_var} = \"{escaped}\".replace(\"{}\", {base_var});",
                                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                                            ));
                                            format!("JsonUtil.fromJson({json_var}, {elem_type}.class)")
                                        })
                                        .collect();
                                    parts.push(format!("java.util.Arrays.asList({})", item_exprs.join(", ")));
                                    continue;
                                }
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
                    if let Some(typed) =
                        ir_typed_java_expression(arg, index, v, target_params, type_defs, enums, config)
                    {
                        parts.push(typed);
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
                if let Some(typed) = ir_typed_java_expression(arg, index, v, target_params, type_defs, enums, config) {
                    parts.push(typed);
                    continue;
                }
                parts.push(json_to_java(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

/// The Java expression for an argument whose *declared* parameter type the core IR resolved,
/// or `None` to keep the existing `arg_type`-only lowering.
///
/// This is the Java answer to the shared question [`TargetParams`] poses, not a shared verdict.
/// `ArgMapping::arg_type` defaults to `"string"`, so before the seam a fixture value bound for a
/// DTO- or enum-typed parameter reached [`json_to_java`], which renders an object as a *quoted
/// JSON string literal* and a string as a *quoted string literal* -- `javac` rejects both
/// against a `CompletionRequest` or a `Model` parameter. Jackson can build either from the same
/// JSON, and the Java binding already ships both entry points, so the fix is to use them:
/// `JsonUtil.fromJson` for a DTO, the generated enum's `@JsonCreator fromValue` for an enum
/// (which matches the wire value case-insensitively, unlike guessing the constant's spelling
/// from the value's camel case as the builder path does). Both are emitted fully qualified so
/// no import has to be predicted at the point the test file's import block is computed. ~keep
///
/// Deliberately narrow. Only a bare `TypeRef::Named` qualifies: an `Optional<T>` or `List<T>`
/// parameter wants a wrapper this expression does not build, and unwrapping to `T` there would
/// swap one compile error for another. Only a *value shape that matches* qualifies: an object
/// for a DTO, a string for an enum. Everything else -- an unknown name, an `IrAbsent` or
/// `Unresolvable` target -- keeps today's rendering exactly. ~keep
fn ir_typed_java_expression(
    arg: &crate::e2e::config::ArgMapping,
    index: usize,
    value: &serde_json::Value,
    target_params: TargetParams<'_>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[EnumDef],
    config: &ResolvedCrateConfig,
) -> Option<String> {
    let TypeRef::Named(declared) = &target_params.param_for(&arg.name, index)?.ty else {
        return None;
    };
    let package = config.java_package();
    let qualifier = if package.is_empty() {
        String::new()
    } else {
        format!("{package}.")
    };
    if enums.iter().any(|enum_def| &enum_def.name == declared)
        && let Some(text) = value.as_str()
    {
        return Some(format!("{qualifier}{declared}.fromValue(\"{}\")", escape_java(text)));
    }
    if type_defs.iter().any(|type_def| &type_def.name == declared) && value.is_object() {
        let json = serde_json::to_string(value).unwrap_or_default();
        return Some(format!(
            "{qualifier}JsonUtil.fromJson({}, {qualifier}{declared}.class)",
            super::values::java_string_literal(&json)
        ));
    }
    None
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
