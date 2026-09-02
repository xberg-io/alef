use super::*;

/// The core IR type of a `handle` argument's config object, or `None` when the IR does not
/// determine one.
///
/// Resolved from the constructor the emitted call actually names: a `handle` argument `engine`
/// renders as `createEngine(engineConfig)`, whose core symbol is the free function
/// `create_engine`, whose declared parameter is `config: Option<CrawlConfig>`. Asking the IR is
/// what closes the gap this generator had: the binding emitter derived `createEngine` from that
/// same signature, while the e2e emitter had no way to reach it and dumped the config object
/// untyped. `options_type` is deliberately NOT a fallback here — on the test-file path it
/// resolves to the file-level `[e2e.call.overrides.<lang>]` default, which is a claim about a
/// call's *options* parameter and not about the handle's constructor, so reading it as one would
/// re-type the config against a struct nobody said it was. ~keep
///
/// A signature the IR does not carry, a parameter that names no struct, or a name absent from
/// `type_defs` all yield `None`, which leaves the caller on exactly the untyped path it took
/// before this seam existed.
fn handle_config_ir_type<'a>(
    arg: &ArgMapping,
    ir: crate::e2e::codegen::call_ir::CallIr<'a>,
    type_defs: &'a [TypeDef],
) -> Option<&'a str> {
    let signature = ir.signature(&format!(
        "create_{}",
        heck::ToSnakeCase::to_snake_case(arg.name.as_str())
    ))?;
    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
    let param = signature
        .params
        .iter()
        .find(|param| param.name == field)
        .or(match signature.params {
            [only] => Some(only),
            _ => None,
        })?;
    let type_name = crate::e2e::codegen::call_ir::named_type(&param.ty)?;
    type_defs
        .iter()
        .any(|definition| definition.name == type_name)
        .then_some(type_name)
}

/// `target` is what the core IR declares about the parameters these arguments fill. Both the
/// documentation-snippet caller and the e2e test-file caller supply a real one (via
/// `ResolvedE2eCallRecipe::target_params`, opted into IR-aware lowering with `.with_functions`);
/// a handful of unit tests below still pass
/// [`crate::e2e::codegen::call_ir::TargetParams::IrAbsent`] directly, which licenses no claim, so
/// every rendering falls back to exactly what it emitted before the seam existed for those.
/// Mirrors the `go` emitter, which threads the same seam to the same two kinds of caller. ~keep
#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[ArgMapping],
    options_type: Option<&str>,
    fixture: &crate::e2e::fixture::Fixture,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    handle_config_type: Option<&str>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    bind_typed_json_objects: bool,
    referenced_enums: &mut std::collections::BTreeSet<String>,
    target: crate::e2e::codegen::call_ir::TargetParams<'_>,
    ir: crate::e2e::codegen::call_ir::CallIr<'_>,
) -> (Vec<String>, String) {
    let fixture_id = &fixture.id;
    if args.is_empty() {
        // When the call has no configured args and the fixture input is an
        // empty object, emit no positional arguments. This lets `extra_args`
        // (e.g. `undefined`) become the sole call argument — matching the
        // shape expected by zero-arg or single-optional-arg functions like
        // `listFiles(query?)` in WASM, where passing `{}` would fail the
        // `instanceof` check.
        let runtime_input = strip_setup_metadata(input);
        if runtime_input
            .as_object()
            .map(|m| m.is_empty())
            .unwrap_or_else(|| runtime_input.is_null())
        {
            return (Vec::new(), String::new());
        }
        return (Vec::new(), json_to_js(&runtime_input));
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    // Check if any later arg (after current) is a json_object that will get a default value
    // (needed to insert undefineds as placeholders for earlier missing optional args)
    fn has_later_json_object_default(args: &[ArgMapping], from_idx: usize, input: &serde_json::Value) -> bool {
        args[from_idx..].iter().any(|arg| {
            if arg.arg_type != "json_object" || !arg.optional {
                return false;
            }
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            input.get(field).is_none() || input.get(field).map(|v| v.is_null()).unwrap_or(true)
        })
    }

    for (idx, arg) in args.iter().enumerate() {
        if arg.arg_type == "mock_url" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let value = input.get(field).unwrap_or(&serde_json::Value::Null);
            let url_expr =
                if let Some(url) = crate::e2e::codegen::preserved_url_literal(fixture.preserve_input_urls, value) {
                    format!("\"{}\"", escape_js(url))
                } else if fixture.has_host_root_route() {
                    format!(
                        "process.env.MOCK_SERVER_{} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`",
                        fixture_id.to_uppercase()
                    )
                } else {
                    format!("`${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`")
                };
            setup_lines.push(format!("const {} = {url_expr};", arg.name));
            parts.push(arg.name.clone());
            continue;
        }

        if arg.arg_type == "mock_url_list" {
            // string[] of URLs: each element is either a bare path (`/seed1`) — prefixed
            // with the per-fixture mock-server URL at runtime — or an absolute URL kept
            // as-is. Mirrors `mock_url` resolution: `MOCK_SERVER_<FIXTURE_ID>` first,
            // then `MOCK_SERVER_URL/fixtures/<id>`. Without this branch the codegen
            // falls back to a JSON-array literal of bare relative paths and the Rust
            // HTTP client rejects them.
            let fixture_id = &fixture.id;
            let env_upper = fixture_id.to_uppercase();
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            // Try both the declared field and common aliases (batch_urls, urls, etc.)
            let val = if let Some(v) = input.get(field).filter(|v| !v.is_null()) {
                v.clone()
            } else {
                crate::e2e::codegen::resolve_urls_field(input, &arg.field).clone()
            };
            let name = &arg.name;
            if let Some(urls) = crate::e2e::codegen::preserved_url_list(fixture.preserve_input_urls, &val) {
                let literals = urls
                    .iter()
                    .map(|url| format!("\"{}\"", escape_js(url)))
                    .collect::<Vec<_>>()
                    .join(", ");
                // A bare `const urls = [];` never has an element pushed onto it in this scope, so
                // under `strict`/`noImplicitAny` TypeScript cannot resolve its element type and
                // reports TS7034 at the declaration and TS7005 at every read (the "evolving array"
                // diagnostic, which only fires for an unannotated `[]` initializer -- a non-empty
                // literal already carries `string[]` from its own elements, so this must not touch
                // that case). `mock_url_list` is always a URL string list by construction -- the
                // same fact `mock_url`'s scalar sibling already leans on above -- so `string[]` is
                // not a guess. ~keep
                let declaration = if urls.is_empty() {
                    format!("const {name}: string[] = [];")
                } else {
                    format!("const {name} = [{literals}];")
                };
                setup_lines.push(declaration);
                parts.push(name.clone());
                continue;
            }
            let paths: Vec<String> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", escape_js(s))))
                    .collect()
            } else {
                Vec::new()
            };
            let paths_literal = paths.join(", ");
            setup_lines.push(format!(
                "const {name}Base = process.env.MOCK_SERVER_{env_upper} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;"
            ));
            setup_lines.push(format!(
                "const {name} = [{paths_literal}].map((p) => p.startsWith(\"http\") ? p : {name}Base + p);"
            ));
            parts.push(name.clone());
            continue;
        }

        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name
                && let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name)
            {
                let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                    .iter()
                    .find(|t| t.name == *trait_name)
                    .map(|t| t.methods.iter().collect())
                    .unwrap_or_default();
                let emission = crate::e2e::codegen::emit_test_backend(
                    lang,
                    trait_bridge,
                    &methods,
                    fixture,
                    enums,
                    wasm_type_prefix,
                );
                setup_lines.push(emission.setup_block);
                // Assign the bridge to a variable for NAPI cleanup
                if lang == "node" {
                    let bridge_var = format!("_bridge_{}", arg.name);
                    setup_lines.push(format!("const {} = {};", bridge_var, emission.arg_expr));
                    parts.push(bridge_var);
                } else {
                    parts.push(emission.arg_expr);
                }
                continue;
            }
            // A `test_backend` arg fills a required stub parameter — there is no
            // compilable value to fall back to when the trait isn't configured. Fail
            // generation loudly instead of silently splicing a `null` argument with a
            // comment where the real stub belongs. ~keep
            panic!(
                "{lang} e2e generator: fixture `{}` declares a `test_backend` arg `{}` with trait `{:?}`, but either it has no `trait_name` configured or no `[[crates.trait_bridges]]` entry matches it; cannot generate a {lang} stub without a resolvable trait bridge",
                fixture.id, arg.name, arg.trait_name
            );
        }

        if arg.arg_type == "handle" {
            let constructor_name = format!("create{}", arg.name.to_upper_camel_case());
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let config_value = input.get(field).unwrap_or(&serde_json::Value::Null);
            let is_null_config = config_value.is_null()
                || config_value.is_object() && config_value.as_object().is_some_and(|o| o.is_empty());
            // WASM: std::env::var is unavailable on wasm32 so SsrfPolicy::from_env()
            // always returns deny_private=true. E2e suites target localhost (mock server),
            // so we must override ssrf.denyPrivate=false on every engine config.
            // Detect whether the config type exposes an `ssrf` field by checking the
            // WASM type-prefix: if the type_defs include an SsrfPolicy struct, we know
            // the binding exposes it. Emit the override whenever lang=="wasm" and the
            // handle has a config type.
            let wasm_has_ssrf_field = lang == "wasm"
                && handle_config_type.is_some()
                && type_defs.iter().any(|td| {
                    (td.name == "SsrfPolicy" || td.name.ends_with("SsrfPolicy"))
                        && td.fields.iter().any(|f| f.name == "deny_private")
                });
            if is_null_config && !wasm_has_ssrf_field {
                setup_lines.push(format!("const {} = {constructor_name}(null);", arg.name));
            } else if is_null_config && wasm_has_ssrf_field {
                // Null config but WASM needs SSRF override — materialise a default config.
                let config_type = handle_config_type.unwrap();
                setup_lines.push(format!(
                    "const {name}Config = {config_type}.default();",
                    name = arg.name
                ));
                setup_lines.push(format!("{name}Config.ssrf.denyPrivate = false;", name = arg.name));
                setup_lines.push(format!(
                    "const {} = {constructor_name}({name}Config);",
                    arg.name,
                    name = arg.name,
                ));
            } else {
                // WASM: if handle_config_type is set, use factory pattern + setters
                if let Some(config_type) = handle_config_type {
                    // Construct config object with setters
                    setup_lines.push(format!(
                        "const {name}Config = {config_type}.default();",
                        name = arg.name
                    ));
                    if let Some(obj) = config_value.as_object() {
                        // Derive nested types for the handle config type so nested objects
                        // are wrapped with their proper class constructors
                        let derived_nested = derive_nested_types_for_wasm(config_type, type_defs, wasm_type_prefix);
                        let effective_nested: std::collections::HashMap<String, String> = {
                            let mut m = derived_nested;
                            for (k, v) in nested_types {
                                m.insert(k.clone(), v.clone());
                            }
                            m
                        };

                        // One traversal owns the whole value, at every depth and through arrays —
                        // see `handle_values`. The three-way `Object`/`else` split this replaced
                        // consulted the class map only for a directly nested object, so an object
                        // inside a list fell to `json_to_js_camel` and stayed a bare literal that
                        // wasm-bindgen rejects, even though the map already held the element's
                        // class (`derive_nested_types_for_wasm` unwraps `Vec<Named>`). ~keep
                        let context = HandleConfigContext {
                            nested_types,
                            effective_nested_types: &effective_nested,
                            lang,
                            enum_fields,
                            bigint_fields,
                            type_defs,
                            enums,
                            wasm_type_prefix,
                        };
                        for (key, val) in obj {
                            let camel_key = underscore_camel_case(key);
                            let value_expr = build_handle_config_value(key, val, &context, &mut *referenced_enums);
                            setup_lines.push(format!("{name}Config.{camel_key} = {value_expr};", name = arg.name));
                        }
                    }
                    // WASM: inject ssrf.denyPrivate=false if the binding exposes SsrfPolicy.
                    // E2e suites hit localhost; std::env::var is unavailable on wasm32 so
                    // SsrfPolicy::from_env() cannot read private-network override environment.
                    if wasm_has_ssrf_field {
                        setup_lines.push(format!("{name}Config.ssrf.denyPrivate = false;", name = arg.name));
                    }
                } else {
                    // Other languages: pass config object directly or via constructor.
                    //
                    // node routes through the typed renderer whenever the IR determines the
                    // constructor's config struct — see `node_typed_value_expression` for the
                    // enum literal the untyped dump got wrong. Everything else, and node with an
                    // unresolvable config type, keeps the plain key-casing dump.
                    let literal = match handle_config_ir_type(arg, ir, type_defs).filter(|_| lang == "node") {
                        Some(config_type) => node_typed_value_expression(
                            config_value,
                            config_type,
                            enum_fields,
                            type_defs,
                            enums,
                            &mut *referenced_enums,
                        ),
                        None => json_to_js_camel(config_value),
                    };
                    setup_lines.push(format!("const {name}Config = {literal};", name = arg.name));
                }
                setup_lines.push(format!(
                    "const {} = {constructor_name}({name}Config);",
                    arg.name,
                    name = arg.name,
                ));
            }
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let runtime_input;
        let val = if field == "input" {
            runtime_input = strip_setup_metadata(input);
            Some(&runtime_input)
        } else {
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // For an absent optional arg, pass `undefined` so the arguments that follow keep
                // their positions. The previous `{} as OptionsType` pattern broke wasm-bindgen,
                // whose runtime `instanceof` check rejects plain object literals — wasm exposes
                // options as opaque classes, not interfaces.
                //
                // Nothing following means nothing to hold in place: an `arg_type == "json_object"`
                // disjunct used to force the placeholder regardless of position, so the trailing
                // options argument — the overwhelmingly common shape — rendered as
                // `convert(html, undefined)` against a signature that already reads `options?:`.
                // Only a real later argument justifies it. ~keep
                //
                // ...or the target declaring the parameter required. `optional` here is the
                // *fixture author's* claim that the value may be left out of the input, which is
                // not a claim about any binding's signature: node's `.d.ts` widens a parameter
                // whose type derives `Default` to `settings?:`, and wasm-bindgen widens nothing.
                // Reading the fixture's flag as if it were both targets' arity is what emitted the
                // node call shape into a wasm snippet — `TS2554: Expected 2 arguments, but got 1`
                // under the same `tsc` that accepts the node one. ~keep
                let target_requires_argument =
                    target.declares_param_optional(lang, &arg.name, idx, type_defs) == Some(false);
                if target_requires_argument
                    || has_later_arg_value(args, idx + 1, input)
                    || has_later_json_object_default(args, idx + 1, input)
                {
                    parts.push("undefined".to_string());
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
                if arg.arg_type == "bytes" {
                    // For bytes type, if value is a string path, read the file
                    if let Some(path) = v.as_str() {
                        let var_name = format!("_{}_content", sanitize_ident(&arg.name));
                        setup_lines.push(format!(
                            "const {var_name} = await (await import(\"node:fs/promises\")).readFile(\"{}\");",
                            escape_js(path)
                        ));
                        parts.push(var_name);
                    } else {
                        // Binary array fallback
                        parts.push(format!("Buffer.from({})", json_to_js(v)));
                    }
                } else if arg.arg_type == "json_object" {
                    if v.is_array() {
                        // Array args use fixture-shaped object literals; element_type is
                        // still used by typed bindings/imports, not product-specific constructors.
                        if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                            let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                            let var_prefix = sanitize_ident(&arg.name);
                            setup_lines.push(format!(
                                "const {var_prefix}MockBaseUrl = process.env.{env_key} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;"
                            ));
                            let json_literal = json_to_js_camel(v);
                            setup_lines.push(format!(
                                "const {var_prefix}Json = JSON.stringify({json_literal}).replaceAll(\"{}\", {var_prefix}MockBaseUrl);",
                                crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                            ));
                            let array_type = arg
                                .element_type
                                .as_deref()
                                .map(|raw| format!("{}[]", canonical_ts_type_name(lang, raw, config)))
                                .unwrap_or_else(|| "unknown[]".to_string());
                            setup_lines.push(format!(
                                "const {name} = JSON.parse({var_prefix}Json) as {array_type};",
                                name = arg.name
                            ));
                            parts.push(arg.name.clone());
                            continue;
                        }
                        // A raw `json_to_js_camel` dump only camelCases keys — it does not know
                        // that a node/napi element type can be a tagged-data enum whose payload
                        // napi nests under a synthesized per-variant field (see
                        // `build_node_tagged_enum_variant_literal`), nor that a field typed as
                        // `bytes` or as an enum needs a real `Uint8Array`/host identifier rather
                        // than the bare wire value. Route each element through the same typed
                        // builder single objects use, so an array of e.g. `Message` matches the
                        // shape napi's `.d.ts` union actually declares.
                        //
                        // wasm needs the identical routing for a second reason: every
                        // wasm-bindgen struct is lowered to a JS *class* with a positional
                        // constructor (`gen_new_method` in backends/wasm/gen_bindings/types.rs),
                        // never a plain interface -- so a bare object literal fails
                        // wasm-bindgen's `instanceof` guard at runtime and `tsc` at compile time
                        // (`TS2739: missing properties`). The single-object branch below already
                        // prefixes wasm element types via `wasm_prefixed_wrapped_type`; array
                        // elements must do the same rather than falling through to
                        // `json_to_js_camel`'s plain literal. ~keep
                        let element_type = arg
                            .element_type
                            .as_deref()
                            .map(|raw| canonical_ts_type_name(lang, raw, config));
                        let is_known_element_type = element_type.as_deref().is_some_and(|name| {
                            type_defs.iter().any(|t| t.name == name) || enums.iter().any(|e| e.name == name)
                        });
                        if (lang == "node" || lang == "wasm")
                            && let Some(element_type) = element_type.filter(|_| is_known_element_type)
                        {
                            let builder_type_name = if lang == "wasm" {
                                wasm_prefixed_wrapped_type(lang, &element_type, type_defs, enums, wasm_type_prefix)
                            } else {
                                element_type
                            };
                            let items: Vec<String> = v
                                .as_array()
                                .expect("checked is_array above")
                                .iter()
                                .map(|item| match item.as_object() {
                                    Some(item_obj) => ts_builder_expression(
                                        item_obj,
                                        &builder_type_name,
                                        nested_types,
                                        lang,
                                        enum_fields,
                                        bigint_fields,
                                        type_defs,
                                        enums,
                                        wasm_type_prefix,
                                        &fixture.docs_files_for_arg(&arg.field),
                                        &mut *referenced_enums,
                                    ),
                                    None if lang == "node" => item.as_str().map_or_else(
                                        || json_to_js(item),
                                        |wire_value| {
                                            node_enum_string_literal(
                                                &builder_type_name,
                                                enums,
                                                wire_value,
                                                &mut *referenced_enums,
                                            )
                                        },
                                    ),
                                    None => json_to_js(item),
                                })
                                .collect();
                            parts.push(format!("[{}]", items.join(", ")));
                        } else {
                            parts.push(json_to_js_camel(v));
                        }
                    } else if let Some(raw_type) =
                        crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, v)
                    {
                        // The wasm binding exposes every wrapped struct/enum under the
                        // `wasm_type_prefix` (e.g. `ExtractInput` -> `WasmExtractInput`).
                        // Config option types already arrive prefixed via the
                        // `options_type` override, but a bare input-builder type
                        // (`ExtractInput`) does not, so `new ExtractInput()` throws
                        // "not a constructor" at runtime. The import statement is
                        // prefixed with the same helper in `render_test_file`, so the
                        // constructor reference and its import stay in sync.
                        let opts_type = wasm_prefixed_wrapped_type(
                            lang,
                            &canonical_ts_type_name(lang, raw_type, config),
                            type_defs,
                            enums,
                            wasm_type_prefix,
                        );
                        // Object value with known options type — construct properly for wasm-bindgen.
                        if v.is_object() && v.as_object().is_some_and(|o| o.is_empty()) {
                            // Empty options: pass undefined so wasm-bindgen's instanceof
                            // guard accepts the call (a `{}` cast produces a plain literal
                            // that fails the runtime class check) -- but only when a later
                            // argument needs the position held, exactly as the absent-value arm
                            // above. A trailing `{}` is the same nothing an omitted argument is. ~keep
                            if has_later_arg_value(args, idx + 1, input)
                                || has_later_json_object_default(args, idx + 1, input)
                            {
                                parts.push("undefined".to_string());
                            }
                        } else if let Some(obj) = v.as_object() {
                            if crate::e2e::codegen::value_contains_mock_url_placeholder(v) {
                                let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
                                let var_prefix = sanitize_ident(&arg.name);
                                setup_lines.push(format!(
                                    "const {var_prefix}MockBaseUrl = process.env.{env_key} ?? `${{process.env.MOCK_SERVER_URL}}/fixtures/{fixture_id}`;"
                                ));
                                let json_literal = json_to_js_camel(v);
                                setup_lines.push(format!(
                                    "const {var_prefix}Json = JSON.stringify({json_literal}).replaceAll(\"{}\", {var_prefix}MockBaseUrl);",
                                    crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                                ));
                                setup_lines.push(format!(
                                    "const {name} = JSON.parse({var_prefix}Json) as {opts_type};",
                                    name = arg.name
                                ));
                                parts.push(arg.name.clone());
                                continue;
                            }
                            // Build TypeScript code to construct the options object properly,
                            // handling nested types via their static factory methods.
                            let ts_code = ts_builder_expression(
                                obj,
                                &opts_type,
                                nested_types,
                                lang,
                                enum_fields,
                                bigint_fields,
                                type_defs,
                                enums,
                                wasm_type_prefix,
                                &fixture.docs_files_for_arg(&arg.field),
                                &mut *referenced_enums,
                            );
                            if bind_typed_json_objects {
                                let suffix = format!(" as {opts_type}");
                                let expression = ts_code.strip_suffix(&suffix).unwrap_or(&ts_code);
                                setup_lines.push(crate::e2e::template_env::render(
                                    "typescript/typed_binding.jinja",
                                    minijinja::context! { name => arg.name, type_name => opts_type, expression => expression },
                                ).trim_end().to_string());
                                parts.push(arg.name.clone());
                            } else {
                                parts.push(ts_code);
                            }
                        } else {
                            parts.push(format!("{} as {opts_type}", json_to_js_camel(v)));
                        }
                    } else {
                        // No `options_type`/`element_type` was configured for this
                        // call/language -- e.g. node's `chat` call, whose binding needs no
                        // runtime constructor for a plain `ChatCompletionRequest` object
                        // literal, so nobody configured one. Ask the core IR what this
                        // argument's parameter is actually declared as, so the fallback
                        // converter below can resolve a serde-renamed field's key correctly
                        // instead of blindly camelCasing the fixture's wire key -- see
                        // `json_to_js_camel_with_types`'s doc for why it is that (narrower)
                        // converter and not the full `ts_builder_expression` typed-object
                        // path. A resolved name absent from `type_defs` (an external/opaque
                        // type) is filtered out: there is no declared field set to resolve
                        // against, so `json_to_js_camel_with_types` would behave identically
                        // to the blind converter anyway. ~keep
                        let declared_type_name = target
                            .declared_type_name(&arg.name, idx)
                            .filter(|declared| type_defs.iter().any(|definition| &definition.name == declared));
                        if lang == "node" {
                            // For node (napi-rs), tagged-data enum discriminants are
                            // always exposed as `"kind"` in TypeScript, regardless of the
                            // original Rust serde_tag attribute. Pre-process the JSON to
                            // rename serde_tag keys (e.g. `role`, `type`) to `"kind"` when
                            // the value matches a known enum variant, then convert to JS.
                            let preprocessed = rename_napi_serde_tags_to_kind(v, enums);
                            parts.push(json_to_js_camel_with_types(
                                &preprocessed,
                                declared_type_name,
                                type_defs,
                            ));
                        } else {
                            parts.push(json_to_js_camel_with_types(v, declared_type_name, type_defs));
                        }
                    }
                    continue;
                } else {
                    parts.push(json_to_js(v));
                }
            }
        }
    }

    (setup_lines, parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_enum_def() -> EnumDef {
        EnumDef {
            name: "Message".into(),
            serde_tag: Some("role".into()),
            serde_rename_all: Some("snake_case".into()),
            variants: vec![crate::core::ir::EnumVariant {
                name: "User".into(),
                is_tuple: true,
                fields: vec![crate::core::ir::FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("UserMessage".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn user_message_type_def() -> TypeDef {
        TypeDef {
            name: "UserMessage".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "content".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn fixture() -> crate::e2e::fixture::Fixture {
        crate::e2e::fixture::Fixture {
            id: "chat".to_string(),
            description: "Chat".to_string(),
            ..Default::default()
        }
    }

    /// Regression for the E3 message-shape defect: an array-typed `json_object` arg (the real
    /// site of the 108 x TS2353 failures, e.g. `messages: Message[]`) used to skip the typed
    /// builder entirely and dump each element through `json_to_js_camel` — a pure key-casing
    /// pass with no notion of a tagged-data enum's variant nesting. Each element must instead
    /// go through `ts_builder_expression`, the same builder a single typed object uses, so an
    /// array of `Message` gets the same `{ role: 'user', user: { content } }` nesting a lone
    /// `Message` argument does.
    #[test]
    fn node_array_of_tagged_enum_elements_nests_each_payload() {
        let enums = [message_enum_def()];
        let type_defs = [user_message_type_def()];
        let fixture = fixture();
        let input = serde_json::json!({ "messages": [{ "role": "user", "content": "Hello" }] });
        let args = [ArgMapping {
            name: "messages".into(),
            field: "input.messages".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: true,
            element_type: Some("Message".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        let config = crate::core::config::ResolvedCrateConfig::default();

        let (_setup_lines, call_args) = build_args_and_setup(
            &input,
            &args,
            None,
            &fixture,
            &Default::default(),
            "node",
            &Default::default(),
            &Default::default(),
            None,
            &type_defs,
            &enums,
            "",
            &config,
            true,
            &mut Default::default(),
            crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
            crate::e2e::codegen::call_ir::CallIr::default(),
        );

        assert_eq!(
            call_args, "[{ role: \"user\", user: { content: \"Hello\" } } as Message]",
            "array element must nest the payload under the synthesized variant field, not flatten it"
        );
    }

    fn extract_input_type_def() -> TypeDef {
        TypeDef {
            name: "ExtractInput".into(),
            fields: vec![
                crate::core::ir::FieldDef {
                    name: "bytes".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                },
                crate::core::ir::FieldDef {
                    name: "kind".into(),
                    ty: TypeRef::Named("ExtractInputKind".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// Regression for the #468 byte-payload typing defect (the wasm half of the 8 failing
    /// `docs-site` snippets, e.g. `extractBatch`'s `inputs: WasmExtractInput[]` argument): an
    /// array-typed `json_object` arg whose element type is a known IR struct used to route
    /// through the typed builder for node only — wasm fell back to `json_to_js_camel`, a pure
    /// key-casing dump with no notion of a `bytes` field or of wasm-bindgen's class-instance
    /// requirement. That dump produced a plain object literal carrying the fixture's raw
    /// `bytes` value (`{ bytes: [72, 105], kind: "bytes" }`), which is neither a
    /// `WasmExtractInput` instance nor assignable to its `Uint8Array` field. Each element must
    /// instead go through `ts_builder_expression`, exactly as node's array elements already do.
    #[test]
    fn wasm_array_of_known_element_type_lowers_bytes_field_via_builder() {
        let enums = [EnumDef {
            name: "ExtractInputKind".into(),
            ..Default::default()
        }];
        let type_defs = [extract_input_type_def()];
        let fixture = fixture();
        let input = serde_json::json!({ "inputs": [{ "bytes": [72, 105], "kind": "bytes" }] });
        let args = [ArgMapping {
            name: "inputs".into(),
            field: "input.inputs".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: true,
            element_type: Some("ExtractInput".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        let config = crate::core::config::ResolvedCrateConfig::default();

        let (_setup_lines, call_args) = build_args_and_setup(
            &input,
            &args,
            None,
            &fixture,
            &Default::default(),
            "wasm",
            &Default::default(),
            &Default::default(),
            None,
            &type_defs,
            &enums,
            "Wasm",
            &config,
            true,
            &mut Default::default(),
            crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
            crate::e2e::codegen::call_ir::CallIr::default(),
        );

        assert!(
            call_args.contains("_u0.bytes = Uint8Array.from([72, 105]);"),
            "array element must build a real Uint8Array via the typed builder, not dump the raw fixture literal: {call_args}"
        );
        assert!(
            call_args.contains("WasmExtractInput.default()"),
            "the wasm element must be constructed as a real binding-class instance, not a plain object literal: {call_args}"
        );
        assert!(
            !call_args.contains("bytes: [72, 105]"),
            "must not fall back to the untyped json_to_js_camel dump: {call_args}"
        );
    }

    /// Regression: a preserved (`preserve_input_urls: true`) `mock_url_list` argument whose
    /// fixture declares zero URLs used to render the bare `const urls = [];` -- no element is
    /// ever pushed onto it in this scope, so under `strict`/`noImplicitAny` `tsc` cannot resolve
    /// its element type and reports TS7034 at the declaration and TS7005 at the call site that
    /// reads it. The element type is always `string` for this arg type, so the fix annotates
    /// only the empty case as `string[]`.
    #[test]
    fn node_empty_preserved_url_list_declares_a_typed_empty_array() {
        let mut fixture = fixture();
        fixture.preserve_input_urls = true;
        let input = serde_json::json!({ "urls": [] });
        let args = [ArgMapping {
            name: "urls".into(),
            field: "input.urls".into(),
            arg_type: "mock_url_list".into(),
            optional: false,
            owned: true,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        let config = crate::core::config::ResolvedCrateConfig::default();

        let (setup_lines, call_args) = build_args_and_setup(
            &input,
            &args,
            None,
            &fixture,
            &Default::default(),
            "node",
            &Default::default(),
            &Default::default(),
            None,
            &[],
            &[],
            "",
            &config,
            true,
            &mut Default::default(),
            crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
            crate::e2e::codegen::call_ir::CallIr::default(),
        );

        assert_eq!(
            setup_lines,
            vec!["const urls: string[] = [];".to_string()],
            "an empty URL list must declare an explicit string[] type, not a bare [] TypeScript cannot infer"
        );
        assert_eq!(call_args, "urls");
    }

    /// Control for `node_empty_preserved_url_list_declares_a_typed_empty_array`: a non-empty
    /// preserved URL list already infers `string[]` from its own elements, so the fix must not
    /// touch this rendering at all.
    #[test]
    fn node_non_empty_preserved_url_list_is_unchanged() {
        let mut fixture = fixture();
        fixture.preserve_input_urls = true;
        let input = serde_json::json!({ "urls": ["https://a.example/x", "https://b.example/y"] });
        let args = [ArgMapping {
            name: "urls".into(),
            field: "input.urls".into(),
            arg_type: "mock_url_list".into(),
            optional: false,
            owned: true,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        let config = crate::core::config::ResolvedCrateConfig::default();

        let (setup_lines, call_args) = build_args_and_setup(
            &input,
            &args,
            None,
            &fixture,
            &Default::default(),
            "node",
            &Default::default(),
            &Default::default(),
            None,
            &[],
            &[],
            "",
            &config,
            true,
            &mut Default::default(),
            crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
            crate::e2e::codegen::call_ir::CallIr::default(),
        );

        assert_eq!(
            setup_lines,
            vec!["const urls = [\"https://a.example/x\", \"https://b.example/y\"];".to_string()],
            "a non-empty preserved URL list must keep its untyped literal -- TypeScript already infers string[]"
        );
        assert_eq!(call_args, "urls");
    }

    /// The wasm half of `node_empty_preserved_url_list_declares_a_typed_empty_array`. Wasm and
    /// node share `build_args_and_setup` for this argument kind -- neither branches on `lang` --
    /// so this pins that the shared code path is fixed for both targets, not only node's.
    #[test]
    fn wasm_empty_preserved_url_list_declares_a_typed_empty_array() {
        let mut fixture = fixture();
        fixture.preserve_input_urls = true;
        let input = serde_json::json!({ "urls": [] });
        let args = [ArgMapping {
            name: "urls".into(),
            field: "input.urls".into(),
            arg_type: "mock_url_list".into(),
            optional: false,
            owned: true,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        let config = crate::core::config::ResolvedCrateConfig::default();

        let (setup_lines, call_args) = build_args_and_setup(
            &input,
            &args,
            None,
            &fixture,
            &Default::default(),
            "wasm",
            &Default::default(),
            &Default::default(),
            None,
            &[],
            &[],
            "Wasm",
            &config,
            true,
            &mut Default::default(),
            crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
            crate::e2e::codegen::call_ir::CallIr::default(),
        );

        assert_eq!(
            setup_lines,
            vec!["const urls: string[] = [];".to_string()],
            "the wasm target must render the same typed empty-array declaration as node"
        );
        assert_eq!(call_args, "urls");
    }
}
