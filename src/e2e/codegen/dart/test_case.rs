//! Dart ordinary function-call e2e test rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::resolve_field;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::assertions::{render_assertion_dart, render_streaming_assertion_dart, snake_to_camel};
use super::stubs::emit_test_backend;
use super::values::{escape_dart, mime_from_extension, type_name_to_create_from_json_dart};

pub(super) struct DartTestCaseContext<'a> {
    pub(super) e2e_config: &'a E2eConfig,
    pub(super) lang: &'a str,
    pub(super) bridge_class: &'a str,
    pub(super) dart_first_class_map: &'a crate::e2e::field_access::DartFirstClassMap,
    pub(super) adapters: &'a [crate::core::config::extras::AdapterConfig],
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) enums: &'a [crate::core::ir::EnumDef],
}

pub(super) fn render_test_case(out: &mut String, fixture: &Fixture, context: DartTestCaseContext<'_>) {
    let DartTestCaseContext {
        e2e_config,
        lang,
        bridge_class,
        dart_first_class_map,
        adapters,
        config,
        type_defs,
        enums,
    } = context;
    // HTTP fixtures: hit the mock server.
    if let Some(http) = &fixture.http {
        super::http::render_http_test_case(out, fixture, http);
        return;
    }

    // Non-HTTP fixtures: render a call-based test using the resolved call config.
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let call_recipe = crate::e2e::codegen::recipe::E2eCallRecipe::resolve(lang, fixture, call_config, type_defs);
    // Build per-call field resolver using the effective field sets for this call.
    let call_field_resolver = FieldResolver::new_with_dart_first_class(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        e2e_config.effective_fields_method_calls(call_config),
        &HashMap::new(),
        dart_first_class_map.clone(),
    )
    .with_display_as_text_fields(e2e_config.effective_fields_display_as_text(call_config).clone())
    .with_dart_root_type(super::dart_call_result_type(call_config).or_else(|| dart_first_class_map.root_type.clone()));
    let field_resolver = &call_field_resolver;
    let enum_fields_base = e2e_config.effective_fields_enum(call_config);

    // Merge per-language enum_fields from the Dart override into the effective enum set so that
    // fields like "status" (BatchStatus on BatchObject) are treated as enum-typed
    // even when they are not globally listed in fields_enum (they are context-
    // dependent — BatchStatus on BatchObject but plain String on ResponseObject).
    let effective_enum_fields: HashSet<String> = {
        let dart_overrides = call_config.overrides.get("dart");
        if let Some(overrides) = dart_overrides {
            let mut merged = enum_fields_base.clone();
            merged.extend(overrides.enum_fields.keys().cloned());
            merged
        } else {
            enum_fields_base.clone()
        }
    };
    let enum_fields = &effective_enum_fields;
    let call_overrides = call_config.overrides.get(lang);
    let mut function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    // Convert snake_case function names to camelCase for Dart conventions.
    function_name = function_name
        .split('_')
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_string()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("");
    let result_var = &call_config.result_var;
    let description = escape_dart(&fixture.description);
    let fixture_id = &fixture.id;
    // `is_async` retained for future use (e.g. non-FRB backends); unused with FRB since
    // all wrappers return Future<T>.
    let _is_async = call_overrides.and_then(|o| o.r#async).unwrap_or(call_config.r#async);

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");
    let is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call_config.streaming_enabled());
    // `result_is_simple = true` means the dart return is a scalar/bytes value
    // (e.g. `Uint8List` for speech/file_content), not a struct. Field-based
    // assertions like `audio.not_empty` collapse to whole-result checks so we
    // don't emit `result.audio` against a `Uint8List` receiver.
    let result_is_simple = call_overrides.is_some_and(|o| o.result_is_simple) || call_config.result_is_simple;

    // Resolve options_type and options_via from per-fixture → per-call → default.
    // These drive how `json_object` args are constructed:
    //   options_via = "from_json" — call `createTypeNameFromJson(json: r'...')` bridge
    //                               helper and pass the result as a named parameter `req:`.
    //   All other values (or absent) — existing behaviour (batch arrays, config objects,
    //   generic JSON arrays, or nothing).
    let options_type: Option<&str> = call_recipe.options_type;
    let options_via: &str = call_recipe.options_via;

    // Build argument list from fixture.input and resolved args (fixture.args or call_config.args).
    // Use `resolve_field` (respects the `field` path like "input.data") rather than
    // looking up by `arg_def.name` directly — the name and the field key may differ.
    //
    // For `extract_file_sync` / `extract_file` fixtures that omit `mime_type`,
    // derive the MIME from the path extension so `extractBytesSync`/`extractBytes`
    // can be called (both require an explicit MIME type).
    let file_path_for_mime: Option<&str> = fixture
        .resolved_args(call_config)
        .iter()
        .find(|a| a.arg_type == "file_path")
        .and_then(|a| resolve_field(&fixture.input, &a.field).as_str());

    // Detect whether this call converts a file_path arg to bytes at test-run time.
    // Most dart fixtures take the bytes path historically (`extractFile` →
    // `extractBytes`), but source-code files need the path-based variant to reach
    // CodeExtractor's `extract_file` implementation (its `extract_bytes` path
    // requires a shebang line for language detection, so `code/hello.py` fed as
    // bytes errors out with "Cannot detect programming language from content").
    // We therefore keep the bytes remap for ordinary file types and skip it for
    // source-code extensions, letting the binding's own `extractFileSync` /
    // `extractFile` accept the path directly.
    let has_file_path_arg = fixture
        .resolved_args(call_config)
        .iter()
        .any(|a| a.arg_type == "file_path");
    let routes_to_source_code = file_path_for_mime
        .and_then(mime_from_extension)
        .map(|m| m == "text/x-source-code")
        .unwrap_or(false);
    // Apply the remap only when no per-fixture dart override has already specified the
    // function — if the fixture author set a dart-specific function name we trust it.
    let caller_supplied_override = call_overrides.and_then(|o| o.function.as_ref()).is_some();
    if has_file_path_arg && !caller_supplied_override && !routes_to_source_code {
        function_name = match function_name.as_str() {
            "extractFile" => "extractBytes".to_string(),
            "extractFileSync" => "extractBytesSync".to_string(),
            other => other.to_string(),
        };
    }

    // Resolve client_factory early so the per-arg builders below can pick the
    // calling convention. When `client_factory` is set the test calls methods on
    // an FRB-generated client instance, and FRB
    // emits every non-`config` parameter as a Dart named-required parameter. When
    // unset the call routes through a hand-written facade whose required args are
    // positional. See the `"string"` arg handler below.
    let client_factory_for_args: Option<&str> =
        call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
            e2e_config
                .call
                .overrides
                .get(lang)
                .and_then(|o| o.client_factory.as_deref())
        });

    // Dart e2e currently emits all args positionally; FRB-direct-call named-arg dispatch
    // is parked behind this flag until the codegen can distinguish facade vs bridge call
    // shapes precisely.
    let is_frb_bridge_call = false;

    // Resolve adapter request type for streaming methods.
    let adapter = adapters.iter().find(|a| a.name == call_config.function.as_str());
    let adapter_request_type: Option<String> = adapter
        .and_then(|a| a.request_type.as_deref())
        .map(|rt| rt.rsplit("::").next().unwrap_or(rt).to_string());

    // setup_lines holds per-test statements that must precede the main call:
    // engine construction (handle args) and URL building (mock_url args).
    let mut setup_lines: Vec<String> = Vec::new();
    let mut args = Vec::new();

    for arg_def in call_recipe.args {
        match arg_def.arg_type.as_str() {
            "mock_url" => {
                let name = arg_def.name.clone();
                setup_lines.push(format!(r#"final {name} = _fixtureUrl("{fixture_id}");"#));
                // For streaming adapters with a request_type, wrap the URL in the request constructor.
                if let Some(ref req_type) = adapter_request_type {
                    let req_var = format!("{}Req", name);
                    // Extract just the type name (last segment after ::).
                    let req_type_name = req_type.rsplit("::").next().unwrap_or(req_type.as_str());
                    setup_lines.push(format!("final {req_var} = {req_type_name}(url: {name});"));
                    args.push(req_var);
                } else {
                    args.push(name);
                }
                continue;
            }
            "handle" => {
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
                if config_value.is_null()
                    || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty())
                {
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
                    let config_type_name = call_recipe.handle_config_type(arg_def).unwrap_or(&arg_def.name);
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
                continue;
            }
            "mock_url_list" => {
                // List<String> of URLs: each element is either a bare path (`/seed1`) — prefixed
                // with the SUT URL at runtime — or an absolute URL kept as-is.
                let field = arg_def.field.strip_prefix("input.").unwrap_or(&arg_def.field);
                let val = fixture.input.get(field).unwrap_or(&serde_json::Value::Null);

                let paths: Vec<String> = if let Some(arr) = val.as_array() {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| format!("'{}'", escape_dart(s)))
                        .collect()
                } else {
                    Vec::new()
                };

                let var_name = &arg_def.name;
                let paths_literal = paths.join(", ");

                setup_lines.push(format!(r#"final {var_name}Base = _fixtureUrl("{fixture_id}");"#));
                setup_lines.push(format!(
                    r#"final {var_name} = <String>[{paths_literal}].map((p) => p.startsWith('http') ? p : {var_name}Base + p).toList();"#
                ));

                // For streaming adapters with a request_type, wrap the URL list in the request constructor.
                if let Some(ref req_type) = adapter_request_type {
                    let req_var = format!("{}Req", var_name);
                    // Extract just the type name (last segment after ::).
                    let req_type_name = req_type.rsplit("::").next().unwrap_or(req_type.as_str());
                    setup_lines.push(format!("final {req_var} = {req_type_name}(urls: {var_name});"));
                    args.push(req_var);
                } else {
                    args.push(var_name.to_string());
                }
                continue;
            }
            "test_backend" => {
                if let Some(trait_name) = &arg_def.trait_name {
                    if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                        let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                            .iter()
                            .find(|t| t.name == *trait_name)
                            .map(|t| t.methods.iter().collect())
                            .unwrap_or_default();
                        let emission = emit_test_backend(trait_bridge, &methods, fixture, enums);
                        // Dart class definitions are emitted at module-level (before void main)
                        // in collect_dart_test_stub_classes, so we only push the instantiation here.
                        args.push(emission.arg_expr);
                        continue;
                    }
                }
                let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("dart");
                setup_lines.push(format!("// {}", emission.arg_expr));
                args.push("null".to_string());
                continue;
            }
            _ => {}
        }

        let arg_value = resolve_field(&fixture.input, &arg_def.field);
        match arg_def.arg_type.as_str() {
            "bytes" | "file_path" => {
                // `bytes`: value is a file path string; load file contents at test-run time.
                // `file_path`: for dart, normally remapped to bytes via the extract
                // facade convention. The exception is source-code paths — those
                // route through extractFile/extractFileSync directly (see
                // `routes_to_source_code` above), so the path string must be
                // passed verbatim instead of materialised as bytes.
                if let serde_json::Value::String(file_path) = arg_value {
                    let arg_expr = if arg_def.arg_type == "file_path" && routes_to_source_code {
                        format!("'{file_path}'")
                    } else {
                        format!("File('{}').readAsBytesSync()", file_path)
                    };
                    if is_frb_bridge_call {
                        let dart_param_name = snake_to_camel(&arg_def.name);
                        args.push(format!("{dart_param_name}: {arg_expr}"));
                    } else {
                        args.push(arg_expr);
                    }
                }
            }
            "int" | "integer" | "i64" => {
                // Scalar integer argument. Direct FRB calls use named parameters.
                let dart_param_name = snake_to_camel(&arg_def.name);
                match arg_value {
                    serde_json::Value::Number(n) => {
                        if is_frb_bridge_call {
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
                        if is_frb_bridge_call {
                            args.push(format!("{dart_param_name}: 0"));
                        } else {
                            args.push("0".to_string());
                        }
                    }
                }
            }
            "float" | "number" => {
                // Scalar float/number argument. Direct FRB calls use named parameters.
                let dart_param_name = snake_to_camel(&arg_def.name);
                match arg_value {
                    serde_json::Value::Number(n) => {
                        if is_frb_bridge_call {
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
                        if is_frb_bridge_call {
                            args.push(format!("{dart_param_name}: 0.0"));
                        } else {
                            args.push("0.0".to_string());
                        }
                    }
                }
            }
            "bool" | "boolean" => {
                // Scalar boolean argument. Direct FRB calls use named parameters.
                let dart_param_name = snake_to_camel(&arg_def.name);
                match arg_value {
                    serde_json::Value::Bool(b) => {
                        let bool_str = if *b { "true" } else { "false" };
                        if is_frb_bridge_call {
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
                        if is_frb_bridge_call {
                            args.push(format!("{dart_param_name}: false"));
                        } else {
                            args.push("false".to_string());
                        }
                    }
                }
            }
            "string" => {
                // Dart FRB bridge methods emit all parameters as named-required.
                // Hand-written facades use positional required and named optional.
                // Direct FRB bridge calls (is_frb_bridge_call = true) should emit all as named.
                // Facade methods (extractBytes, extractFile) keep required args positional.
                //
                // The `mime_type` parameter is special: it's positional in facade extract methods
                // but named in direct FRB bridge calls. The `client_factory` path is for stateful
                // clients (e.g., demo-client) which always use named parameters.
                let dart_param_name = snake_to_camel(&arg_def.name);
                let mime_type_is_positional =
                    arg_def.name == "mime_type" && !is_frb_bridge_call && client_factory_for_args.is_none();
                match arg_value {
                    serde_json::Value::String(s) => {
                        let literal = format!("'{}'", escape_dart(s));
                        // Direct FRB bridge calls: all parameters are named-required.
                        // Client factory methods: all non-config parameters are named-required.
                        // Facade methods: required positional, optional named.
                        if is_frb_bridge_call || client_factory_for_args.is_some() || arg_def.optional {
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
                        let inferred = file_path_for_mime
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
            "json_object" => {
                if let Some(elem_type) = &arg_def.element_type {
                    if arg_value.is_object() {
                        let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
                        let escaped_json = escape_dart(&json_str);
                        let var_name = format!("_{}", arg_def.name);
                        let dart_fn = type_name_to_create_from_json_dart(elem_type);
                        let json_source = if crate::e2e::codegen::value_contains_mock_url_placeholder(arg_value) {
                            setup_lines.push(format!(
                                "final {var_name}MockBaseUrl = _fixtureUrl(\"{}\");",
                                fixture.id
                            ));
                            setup_lines.push(format!(
                                "final {var_name}Json = '{escaped_json}'.replaceAll(r'{}', {var_name}MockBaseUrl);",
                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                            ));
                            format!("{var_name}Json")
                        } else {
                            format!("'{escaped_json}'")
                        };
                        setup_lines.push(format!("final {var_name} = await {dart_fn}(json: {json_source});"));
                        if is_frb_bridge_call {
                            let dart_param_name = snake_to_camel(&arg_def.name);
                            args.push(format!("{dart_param_name}: {var_name}"));
                        } else {
                            args.push(var_name);
                        }
                    } else if elem_type == "String" && arg_value.is_array() {
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
                        if is_frb_bridge_call {
                            let dart_param_name = snake_to_camel(&arg_def.name);
                            args.push(format!("{dart_param_name}: {list_literal}"));
                        } else {
                            args.push(list_literal);
                        }
                    } else if arg_value.is_array() {
                        // Generic typed array (e.g. `actions: [PageAction]` for interact,
                        // or `items: [BatchBytesItem]` for batch). Decode via jsonDecode at
                        // test-run time and convert to typed instances.
                        let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
                        let var_name = arg_def.name.clone();
                        let json_source = if crate::e2e::codegen::value_contains_mock_url_placeholder(arg_value) {
                            setup_lines.push(format!(
                                "final {var_name}MockBaseUrl = _fixtureUrl(\"{}\");",
                                fixture.id
                            ));
                            setup_lines.push(format!(
                                "final {var_name}Json = r'{json_str}'.replaceAll(r'{}', {var_name}MockBaseUrl);",
                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                            ));
                            format!("{var_name}Json")
                        } else {
                            format!("r'{json_str}'")
                        };
                        if elem_type == "PageAction" {
                            setup_lines.push(format!(
                                "final {var_name} = (jsonDecode({json_source}) as List<dynamic>).map((e) => _parsePageAction(e as Map<String, dynamic>)).toList();"
                            ));
                        } else {
                            // FRB-generated `create<ElementType>FromJson(json:)` factory
                            // takes a JSON string per item. Map each map to its typed
                            // instance and await the futures together so the typed list
                            // matches the binding's parameter type (e.g. List<BatchBytesItem>).
                            let dart_fn = type_name_to_create_from_json_dart(elem_type);
                            setup_lines.push(format!(
                                "final {var_name} = await Future.wait((jsonDecode({json_source}) as List<dynamic>).cast<Map<String, dynamic>>().map((m) => {dart_fn}(json: jsonEncode(m))));"
                            ));
                        }
                        // For generic arrays, emit named parameter if it's a direct FRB call
                        if is_frb_bridge_call {
                            let dart_param_name = snake_to_camel(&arg_def.name);
                            args.push(format!("{dart_param_name}: {var_name}"));
                        } else {
                            args.push(var_name);
                        }
                    }
                } else if options_via == "from_json" {
                    // `from_json` path: construct a typed mirror-struct via the generated
                    // `create<TypeName>FromJson(json: '...')` bridge helper, then pass it
                    // as the named FRB parameter `req: _var`.
                    //
                    // The helper is generated by `emit_from_json_fn` in the dart bridge-crate
                    // generator and made available as a top-level function via the exported
                    // the generated bridge package. The parameter name used in the
                    // bridge method call is always `req:` for single-request-object methods
                    // (derived from the Rust IR param name).
                    if let Some(opts_type) = call_recipe
                        .json_object_constructor_type(arg_def, arg_value)
                        .or(options_type)
                    {
                        if !arg_value.is_null() {
                            let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
                            // Escape for Dart single-quoted string literal (handles embedded quotes,
                            // backslashes, and interpolation markers).
                            let escaped_json = escape_dart(&json_str);
                            let var_name = format!("_{}", arg_def.name);
                            let dart_fn = type_name_to_create_from_json_dart(opts_type);
                            setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{escaped_json}');"));
                            // FRB bridge method param name is `req` for all single-request methods.
                            // Use `req:` as the named argument label.
                            args.push(format!("req: {var_name}"));
                        }
                    }
                } else if call_recipe.should_materialize_json_object(arg_def, arg_value) && arg_value.is_null() {
                    if let Some(opts_type) = options_type {
                        let var_name = format!("_{}", arg_def.name);
                        let dart_fn = type_name_to_create_from_json_dart(opts_type);
                        setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{{}}');"));
                        // Facade methods (e.g. `embedTextsAsync`, `rerankAsync`, `classifyText`)
                        // declare config as a required positional parameter; extraction
                        // facades declare it as a named optional. Emit positional when the
                        // resolved options type indicates a required positional config.
                        let is_config_positional = opts_type.contains("Embedding")
                            || opts_type.contains("Reranker")
                            || opts_type.contains("Classification")
                            || opts_type.contains("Translation")
                            || opts_type.contains("Keyword")
                            || opts_type.contains("Redaction");
                        if is_config_positional {
                            args.push(var_name);
                        } else {
                            let dart_param_name = snake_to_camel(&arg_def.name);
                            args.push(format!("{dart_param_name}: {var_name}"));
                        }
                    }
                } else if arg_def.name == "config" {
                    // Helper to check if a config type should be emitted as positional in Dart.
                    // Config parameters are positional-required in facades like rerankAsync,
                    // embedTextsAsync, classifyText, etc. They are named-optional in extraction facades
                    // like extractBytes which declare { ExtractionConfig? config }.
                    let is_config_positional = |opts_type: &str| -> bool {
                        opts_type.contains("Embedding")
                            || opts_type.contains("Reranker")
                            || opts_type.contains("Classification")
                            || opts_type.contains("Translation")
                            || opts_type.contains("Keyword")
                            || opts_type.contains("Redaction")
                    };

                    if let serde_json::Value::Object(map) = &arg_value {
                        if !map.is_empty() {
                            // Round-trip object config JSON through a generated helper.
                            // Resolve config type from explicit element_type first, then fall back
                            // to options_type from the call recipe, then to the arg name as a last resort.
                            let opts_type = call_recipe
                                .json_object_constructor_type(arg_def, arg_value)
                                .or(arg_def.element_type.as_deref())
                                .or(options_type)
                                .unwrap_or(&arg_def.name);
                            let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
                            let escaped_json = escape_dart(&json_str);
                            let var_name = format!("_{}", arg_def.name);
                            let dart_fn = type_name_to_create_from_json_dart(opts_type);
                            setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{escaped_json}');"));
                            if is_config_positional(opts_type) {
                                args.push(var_name);
                            } else {
                                let dart_param_name = snake_to_camel(&arg_def.name);
                                args.push(format!("{dart_param_name}: {var_name}"));
                            }
                        } else {
                            // Empty config object: construct a default instance via FRB's
                            // `create<Type>FromJson(json: '{}')` helper (supports all
                            // configured config types). This ensures the
                            // call signature matches the binding, which expects a required
                            // config parameter even when all fields use their defaults.
                            // Resolve config type from element_type, options_type, or arg name.
                            let opts_type = call_recipe
                                .json_object_constructor_type(arg_def, arg_value)
                                .or(arg_def.element_type.as_deref())
                                .or(options_type);
                            if let Some(opts_type) = opts_type {
                                let var_name = format!("_{}", arg_def.name);
                                let dart_fn = type_name_to_create_from_json_dart(opts_type);
                                setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{{}}');"));
                                if is_config_positional(opts_type) {
                                    args.push(var_name);
                                } else {
                                    let dart_param_name = snake_to_camel(&arg_def.name);
                                    args.push(format!("{dart_param_name}: {var_name}"));
                                }
                            }
                        }
                    } else if arg_def.optional {
                        // Fixture has no config block (null/absent) but the Dart facade
                        // declares the arg as a required-positional non-nullable type
                        // (e.g. `embed_texts_async(texts, settings)` with `SampleSettings`).
                        // Construct a default instance via FRB's
                        // `create<Type>FromJson(json: '{}')` helper when IR metadata says
                        // the configured type has a default.
                        let opts_type = call_recipe
                            .json_object_constructor_type(arg_def, arg_value)
                            .or(arg_def.element_type.as_deref())
                            .or(options_type);
                        if let Some(opts_type) = opts_type.filter(|_| {
                            call_recipe.json_object_arg_has_default(arg_def)
                                || call_recipe.should_materialize_json_object(arg_def, arg_value)
                        }) {
                            let var_name = format!("_{}", arg_def.name);
                            let dart_fn = type_name_to_create_from_json_dart(opts_type);
                            setup_lines.push(format!("final {var_name} = await {dart_fn}(json: '{{}}');"));
                            if is_config_positional(opts_type) {
                                args.push(var_name);
                            } else {
                                let dart_param_name = snake_to_camel(&arg_def.name);
                                args.push(format!("{dart_param_name}: {var_name}"));
                            }
                        }
                    }
                } else if arg_value.is_array() {
                    // Generic JSON array (e.g. batch_urls: ["/page1", "/page2"]).
                    // Decode via jsonDecode and cast to List<String> at test-run time.
                    let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
                    let var_name = arg_def.name.clone();
                    setup_lines.push(format!(
                        "final {var_name} = (jsonDecode(r'{json_str}') as List<dynamic>).cast<String>();"
                    ));
                    // Direct FRB bridge calls use named parameters
                    if is_frb_bridge_call {
                        let dart_param_name = snake_to_camel(&arg_def.name);
                        args.push(format!("{dart_param_name}: {var_name}"));
                    } else {
                        args.push(var_name);
                    }
                } else if let serde_json::Value::Object(map) = &arg_value {
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
                    // + `options_field` binding). The `_visitor` variable is materialised
                    // in the visitor block below — its setup line is inserted ahead of
                    // this options call by `build_dart_visitor`.
                    if !map.is_empty() {
                        if let Some(opts_type) = call_recipe
                            .json_object_constructor_type(arg_def, arg_value)
                            .or(options_type)
                        {
                            let json_str = serde_json::to_string(&arg_value).unwrap_or_default();
                            let escaped_json = escape_dart(&json_str);
                            let dart_param_name = snake_to_camel(&arg_def.name);
                            let var_name = format!("_{}", arg_def.name);
                            let dart_fn = type_name_to_create_from_json_dart(opts_type);
                            if fixture.visitor.is_some() {
                                setup_lines.push(format!(
                                    "final {var_name} = await {dart_fn}WithVisitor(json: '{escaped_json}', visitor: _visitor);"
                                ));
                            } else {
                                setup_lines
                                    .push(format!("final {var_name} = await {dart_fn}(json: '{escaped_json}');"));
                            }
                            // Dart bridge method declares options as keyword-only parameter.
                            // Always emit as named argument regardless of optionality.
                            args.push(format!("{dart_param_name}: {var_name}"));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Fixture-driven visitor handle. When `fixture.visitor` is set we build a
    // `_visitor` via the generated visitor factory (emitted by
    // `alef-backend-dart`'s trait-bridge generator in the `type_alias` mode)
    // and thread it into the options blob via the
    // `create<OptionsType>FromJsonWithVisitor(json, visitor)` helper (handled
    // a few lines above in the json_object arg branch).
    //
    // The visitor setup line is INSERTED at the front of `setup_lines` so
    // `_visitor` is defined before any `_options` line that references it.
    // Fixtures without an `options` json_object in input still need an options
    // blob to carry the visitor through to the configured call — we synthesise an empty
    // options call with the configured options type here when no `options` arg was emitted in the loop
    // above.
    if let Some(visitor_spec) = &fixture.visitor {
        let mut visitor_setup: Vec<String> = Vec::new();
        let visitor_config = crate::e2e::codegen::dart_visitors::resolve_dart_visitor_config(
            config,
            call_overrides,
            type_defs,
            visitor_spec,
        );
        let _ =
            crate::e2e::codegen::dart_visitors::build_dart_visitor(&mut visitor_setup, visitor_spec, &visitor_config);
        // Prepend the visitor block so `_visitor` is in scope by the time the
        // options call (which may reference it) runs.
        for line in visitor_setup.into_iter().rev() {
            setup_lines.insert(0, line);
        }

        // If no `options` arg was emitted by the loop above (the fixture has no
        // input.options block), build an empty options-with-visitor and add it as
        // an `options:` named arg so the visitor reaches the convert call.
        let already_has_options = args.iter().any(|a| a.starts_with("options:") || a == "_options");
        if !already_has_options {
            if let Some(opts_type) = options_type {
                let dart_fn = type_name_to_create_from_json_dart(opts_type);
                setup_lines.push(format!(
                    "final _options = await {dart_fn}WithVisitor(json: '{{}}', visitor: _visitor);"
                ));
                args.push("options: _options".to_string());
            }
        } else if let Some(opts_type) = options_type {
            // The args loop already emitted a non-WithVisitor options call (e.g.
            // for `options: {}` or `options: {some: value}`). Without the visitor
            // attached the convert call ignores `_visitor` — rewrite the
            // emitted call to its `WithVisitor` sibling so the visitor reaches
            // the converter.
            let dart_fn = type_name_to_create_from_json_dart(opts_type);
            let needle = format!("await {dart_fn}(json:");
            let replacement = format!("await {dart_fn}WithVisitor(visitor: _visitor, json:");
            for line in setup_lines.iter_mut() {
                if line.contains(&needle) {
                    *line = line.replace(&needle, &replacement);
                }
            }
        }
    }

    // Resolve client_factory: when set, tests create a client instance and call
    // methods on it rather than using static bridge-class calls. This mirrors the
    // go/python/zig pattern for stateful clients (e.g. demo-client).
    let client_factory: Option<&str> = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    // Convert factory name to camelCase (same rule as function_name above).
    let client_factory_camel: Option<String> = client_factory.map(|f| {
        f.split('_')
            .enumerate()
            .map(|(i, part)| {
                if i == 0 {
                    part.to_string()
                } else {
                    let mut chars = part.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("")
    });

    // All bridge methods return Future<T> because FRB v2 wraps every Rust
    // function as async in Dart — even "sync" Rust functions. Always emit an async
    // test body and await the call so the test framework waits for the future.
    let _ = writeln!(out, "  test('{description}', () async {{");

    let args_str = args.join(", ");
    let receiver_class = call_overrides
        .and_then(|o| o.class.as_ref())
        .cloned()
        .unwrap_or_else(|| bridge_class.to_string());

    // When client_factory is set, determine the mock URL and emit client instantiation.
    // The mock URL derivation follows the same has_host_root_route / plain-fixture split
    // used by the mock_url arg handler above.
    let (receiver, extra_setup): (String, Option<String>) = if let Some(factory) = &client_factory_camel {
        let has_mock_url = fixture
            .resolved_args(call_config)
            .iter()
            .any(|a| a.arg_type == "mock_url");
        let mock_url_setup = if !has_mock_url {
            // No explicit mock_url arg — derive the URL inline.
            Some(format!(r#"final _mockUrl = _fixtureUrl("{fixture_id}");"#))
        } else {
            None
        };
        let url_expr = if has_mock_url {
            // A mock_url arg was emitted into setup_lines already — reuse the variable name
            // from the first mock_url arg definition so we don't duplicate the URL.
            call_config
                .args
                .iter()
                .find(|a| a.arg_type == "mock_url")
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "_mockUrl".to_string())
        } else {
            "_mockUrl".to_string()
        };
        let create_line = format!("final _client = await {receiver_class}.{factory}('test-key', baseUrl: {url_expr});");
        let full_setup = if let Some(url_line) = mock_url_setup {
            Some(format!("{url_line}\n    {create_line}"))
        } else {
            Some(create_line)
        };
        ("_client".to_string(), full_setup)
    } else {
        (receiver_class.clone(), None)
    };

    if expects_error && (!setup_lines.is_empty() || extra_setup.is_some()) {
        // Wrap setup + call in an async lambda so any exception at any step is caught.
        // flutter_rust_bridge 2.x decodes Rust errors as raw String values (not Exception
        // subtypes), so throwsException will not match. Use throwsA(anything) instead.
        let _ = writeln!(out, "    await expectLater(() async {{");
        for line in &setup_lines {
            // Handle multi-line setup blocks (e.g., class definitions from emit_test_backend).
            // Each embedded newline in `line` needs proper indentation.
            for inner_line in line.lines() {
                let _ = writeln!(out, "      {inner_line}");
            }
        }
        if let Some(extra) = &extra_setup {
            for line in extra.lines() {
                let _ = writeln!(out, "      {line}");
            }
        }
        if is_streaming {
            let _ = writeln!(out, "      return {receiver}.{function_name}({args_str}).toList();");
        } else {
            let _ = writeln!(out, "      return {receiver}.{function_name}({args_str});");
        }
        let _ = writeln!(out, "    }}(), throwsA(anything));");
    } else if expects_error {
        // No setup lines, direct call — same throwsA(anything) rationale as above.
        if let Some(extra) = &extra_setup {
            for line in extra.lines() {
                let _ = writeln!(out, "    {line}");
            }
        }
        if is_streaming {
            let _ = writeln!(
                out,
                "    await expectLater({receiver}.{function_name}({args_str}).toList(), throwsA(anything));"
            );
        } else {
            let _ = writeln!(
                out,
                "    await expectLater({receiver}.{function_name}({args_str}), throwsA(anything));"
            );
        }
    } else {
        for line in &setup_lines {
            // Handle multi-line setup blocks (e.g., class definitions from emit_test_backend).
            // Each embedded newline in `line` needs proper indentation.
            for inner_line in line.lines() {
                let _ = writeln!(out, "    {inner_line}");
            }
        }
        if let Some(extra) = &extra_setup {
            for line in extra.lines() {
                let _ = writeln!(out, "    {line}");
            }
        }
        if is_streaming {
            let _ = writeln!(
                out,
                "    final {result_var} = await {receiver}.{function_name}({args_str}).toList();"
            );
        } else {
            let _ = writeln!(
                out,
                "    final {result_var} = await {receiver}.{function_name}({args_str});"
            );
        }
        for assertion in &fixture.assertions {
            if is_streaming {
                render_streaming_assertion_dart(out, assertion, result_var);
            } else {
                render_assertion_dart(
                    out,
                    assertion,
                    result_var,
                    result_is_simple,
                    field_resolver,
                    enum_fields,
                );
            }
        }
    }

    let _ = writeln!(out, "  }});");
    let _ = writeln!(out);
}
