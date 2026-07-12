use super::*;

/// Emit environment variable setup code for test file.
/// Returns a TypeScript code snippet with `process.env.VAR ??= "value"` assignments,
/// or an empty string if no env vars are configured. Keys are sorted alphabetically.
pub(crate) fn render_env_setup(env: &std::collections::HashMap<String, String>) -> String {
    if env.is_empty() {
        return String::new();
    }
    let mut keys: Vec<&String> = env.keys().collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        let v = &env[k];
        out.push_str(&format!("process.env.{} ??= \"{}\";\n", k, v));
    }
    out
}

/// Render a complete test file for the given category.
///
/// `lang` is the language key used for per-fixture call override resolution
/// (e.g. `"node"` for TypeScript, `"wasm"` for WASM tests).
///
/// `type_defs` is the IR type registry from the source crate. For the WASM
/// language path it is used to auto-derive `nested_types` (class-typed field
/// mappings) so plain object literals are not passed where wasm-bindgen expects
/// class instances. Pass an empty slice when not available; the generator
/// falls back to explicit call-override mappings.
///
/// `enums` is the IR enum registry from the source crate. For WASM, it is used
/// to identify tagged-data enums so they are emitted as plain JS object literals
/// instead of wrapper factories. Pass an empty slice when not available.
#[allow(clippy::too_many_arguments)]
pub fn render_test_file(
    lang: &str,
    category: &str,
    fixtures: &[&Fixture],
    module_path: &str,
    pkg_name: &str,
    function_name: &str,
    args: &[ArgMapping],
    options_type: Option<&str>,
    client_factory: Option<&str>,
    e2e_config: &E2eConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
) -> String {
    // `lang` is used for wasm visitor arg placement and override routing
    let (needs_cache_isolation, has_configure) = detect_cache_isolation_needs(fixtures, e2e_config);

    let import_vitest = if needs_cache_isolation && has_configure {
        "import { describe, expect, it, beforeAll, afterAll } from \"vitest\";"
    } else {
        "import { describe, expect, it } from \"vitest\";"
    };

    let has_non_http_fixtures = fixtures.iter().any(|f| !f.is_http_test() && !f.assertions.is_empty());

    // `_alefE2eDecompressAndParseJson` is also referenced by `http_test.jinja` when an HTTP
    // fixture declares a non-string JSON body, a partial body, or validation errors.
    // Emit the helper for HTTP-only files that would trigger these branches so the
    // generated test file compiles without "cannot find function" errors.
    let http_fixtures_need_decompress_helper = fixtures.iter().any(|f| {
        let Some(http) = &f.http else { return false };
        let has_json_body = http
            .expected_response
            .body
            .as_ref()
            .is_some_and(|b| !b.is_null() && !b.is_string());
        let has_partial_body = http
            .expected_response
            .body_partial
            .as_ref()
            .is_some_and(|b| b.is_object());
        let has_validation_errors = http
            .expected_response
            .validation_errors
            .as_ref()
            .is_some_and(|v| !v.is_empty());
        has_json_body || has_partial_body || has_validation_errors
    });

    // Extract nested_types and enum_fields from the call override if available.
    let override_config = e2e_config.call.overrides.get(lang);
    let nested_types = override_config.map(|o| o.nested_types.clone()).unwrap_or_default();
    let enum_fields = override_config.map(|o| o.enum_fields.clone()).unwrap_or_default();
    let result_enum_fields = override_config
        .map(|o| o.result_enum_fields.clone())
        .unwrap_or_default();

    // Per-fixture wasm/node overrides may add their own options_type / nested_types /
    // enum_fields (each call exposes a different request struct in WASM, e.g.
    // `WasmEmbeddingRequest` vs `WasmChatCompletionRequest`). Aggregate every class
    // referenced across this file's fixtures so the import line covers them all.
    // The global `options_type` parameter remains the default fallback when a
    // per-call override is absent.
    let mut all_options_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut all_nested_types: std::collections::HashMap<String, String> = nested_types.clone();
    let mut all_enum_fields: std::collections::HashMap<String, String> = enum_fields.clone();
    let mut all_result_enum_classes: std::collections::BTreeSet<String> =
        result_enum_fields.values().cloned().collect();
    if let Some(opts) = options_type {
        all_options_types.insert(canonical_ts_type_name(lang, opts, config));
    }
    for fixture in fixtures.iter() {
        let cc = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        if let Some(o) = cc.overrides.get(lang) {
            if let Some(opts) = &o.options_type {
                all_options_types.insert(canonical_ts_type_name(lang, opts, config));
            }
            for (k, v) in &o.nested_types {
                all_nested_types.entry(k.clone()).or_insert_with(|| v.clone());
            }
            for (k, v) in &o.enum_fields {
                all_enum_fields.entry(k.clone()).or_insert_with(|| v.clone());
            }
            for v in o.result_enum_fields.values() {
                all_result_enum_classes.insert(v.clone());
            }
            // For WASM, also collect handle_config_type so its nested types are imported
            if lang == "wasm" {
                if let Some(handle_type) = &o.handle_config_type {
                    all_options_types.insert(handle_type.clone());
                }
            }
        }
        if lang == "wasm" {
            for arg in &cc.args {
                if arg.arg_type == "json_object"
                    && let Some(element_type) = &arg.element_type
                    && !is_typescript_primitive_element_type(element_type)
                {
                    // Prefix bare wasm-wrapped element types so the import and
                    // the constructor reference agree (`ExtractInput` ->
                    // `WasmExtractInput`). See `wasm_prefixed_wrapped_type`.
                    all_options_types.insert(wasm_prefixed_wrapped_type(
                        lang,
                        &canonical_ts_type_name(lang, element_type, config),
                        type_defs,
                        enums,
                        wasm_type_prefix,
                    ));
                }
            }
        }
        if lang == "wasm" && fixture.visitor.is_some() {
            if let Some(binding) = wasm_visitor_binding(config, options_type) {
                all_options_types.insert(binding.options_type);
                all_options_types.insert(binding.handle_type);
            }
        }
    }

    // For the WASM path, auto-derive additional nested_types from the IR
    // registry so their class names are included in the import statement.
    // This mirrors the derivation in `ts_builder_expression_inner` — we
    // collect from every options_type seen in this file. The walk is
    // transitive: when a derived class itself has class-typed fields
    // (e.g. WasmChatCompletionRequest.tools[].function: WasmFunctionDefinition),
    // those second-level classes are also referenced by the test body's
    // builder expressions and must appear in the import statement, or the
    // test fails at runtime with `ReferenceError: WasmFunctionDefinition
    // is not defined`. The BFS uses a seen-set to terminate on cycles.
    if lang == "wasm" {
        let derived_all = collect_transitive_nested_types_for_wasm(&all_options_types, type_defs, wasm_type_prefix);
        for (k, v) in derived_all {
            all_nested_types.entry(k).or_insert(v);
        }
    }

    // For WASM, we need to import the options type when:
    // 1. There are json_object args with values, OR
    // 2. There are visitor specs (which require a configured options bridge)
    let has_visitor_fixtures = lang == "wasm" && fixtures.iter().any(|f| f.visitor.is_some());
    let needs_options_import = !all_options_types.is_empty()
        && (has_visitor_fixtures
            || fixtures.iter().any(|f| {
                let cc = e2e_config.resolve_call_for_fixture(
                    f.call.as_deref(),
                    &f.id,
                    &f.resolved_category(),
                    &f.tags,
                    &f.input,
                );
                cc.args.iter().any(|arg| {
                    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                    let val = if field == "input" {
                        Some(f.input.get("extract_input").unwrap_or(&f.input))
                    } else {
                        f.input.get(field)
                    };
                    arg.arg_type == "json_object" && val.is_some_and(|v| !v.is_null())
                })
            }));

    // Collect handle constructor function names that need to be imported.
    let handle_constructors: Vec<String> = args
        .iter()
        .filter(|arg| arg.arg_type == "handle")
        .map(|arg| format!("create{}", arg.name.to_upper_camel_case()))
        .collect();

    let mut import_modules = String::new();
    let mut import_node_fs = String::new();

    if has_non_http_fixtures {
        let mut imports: Vec<String> = if let Some(factory) = client_factory {
            vec![factory.to_string()]
        } else {
            vec![function_name.to_string()]
        };

        // Also import any additional function names used by per-fixture call overrides or
        // select_when auto-selected calls.
        for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
            let call_config = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            let fixture_fn = resolve_node_function_name(call_config);
            if client_factory.is_none() && !imports.contains(&fixture_fn) {
                imports.push(fixture_fn);
            }
        }

        // Collect tree helper function names needed by method_result assertions.
        for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
            for assertion in &fixture.assertions {
                if assertion.assertion_type == "method_result" {
                    if let Some(method_name) = &assertion.method {
                        if let Some(helper_fn) = ts_method_helper_import(method_name) {
                            if !imports.contains(&helper_fn) {
                                imports.push(helper_fn);
                            }
                        }
                    }
                }
            }
        }

        // Collect unregister function names for trait bridge cleanup (Node.js only).
        if lang == "node" {
            for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
                // For trait-bridge fixtures, args are defined at fixture level, not call level.
                // Check fixture.args directly (not call_config.args, which may be empty for trait-bridge calls).
                for arg in &fixture.args {
                    if arg.arg_type == "test_backend"
                        && let Some(trait_name) = arg.trait_name.as_ref()
                    {
                        // The ArgMapping.trait_name specifies the trait name (e.g., "OcrBackend")
                        let unregister_fn = format!("unregister{}", trait_name);
                        if !imports.contains(&unregister_fn) {
                            imports.push(unregister_fn);
                        }
                    }
                }
            }
        }

        for ctor in &handle_constructors {
            if !imports.contains(ctor) {
                imports.push(ctor.clone());
            }
        }

        // Import named element types used by typed json_object arrays.
        for fixture in fixtures.iter() {
            let cc = e2e_config.resolve_call_for_fixture(
                fixture.call.as_deref(),
                &fixture.id,
                &fixture.resolved_category(),
                &fixture.tags,
                &fixture.input,
            );
            for arg in &cc.args {
                if let Some(elem_type) = &arg.element_type {
                    // Prefix bare wasm-wrapped element types (e.g. `ExtractInput` ->
                    // `WasmExtractInput`) so the import matches the constructor
                    // reference emitted by `build_args_and_setup`. Non-wasm langs
                    // and primitives / host types pass through unchanged.
                    let elem_type = wasm_prefixed_wrapped_type(lang, elem_type, type_defs, enums, wasm_type_prefix);
                    if !is_typescript_primitive_element_type(&elem_type) && !imports.contains(&elem_type) {
                        imports.push(elem_type);
                    }
                }
                if lang == "node" && arg.arg_type == "json_object" {
                    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                    let val = if field == "input" {
                        Some(fixture.input.get("extract_input").unwrap_or(&fixture.input))
                    } else {
                        fixture.input.get(field)
                    };
                    if val.is_some_and(|v| !v.is_null()) {
                        if let Some(override_type) = cc
                            .overrides
                            .get("node")
                            .and_then(|o| o.options_type.as_deref())
                            .or(cc.options_type.as_deref())
                        {
                            let type_import = format!("type {}", canonical_ts_type_name(lang, override_type, config));
                            if !imports.contains(&type_import) {
                                imports.push(type_import);
                            }
                        }
                    }
                }
            }
        }

        let _ = module_path; // retained in signature for potential future use
        if needs_options_import {
            if lang == "node" {
                // Configured options types can be TypeScript interfaces — use type-only imports.
                // No Update class exists; options are constructed as plain object literals.
                for opts_type in &all_options_types {
                    let type_import = format!("type {opts_type}");
                    if !imports.contains(&type_import) {
                        imports.push(type_import);
                    }
                }
            } else {
                // WASM: value import needed for runtime construction. The
                // alef-backend-wasm codegen does not emit `*Update` builder
                // classes, so we construct the main type directly via its
                // all-optional positional constructor and then assign each
                // present field through generated setters. Nested types use
                // the same pattern. See `ts_builder_expression_inner`.
                for opts_type in &all_options_types {
                    if !imports.contains(opts_type) {
                        imports.push(opts_type.clone());
                    }
                }
                // Sort values for deterministic import ordering — HashMap
                // iteration order is non-deterministic and would thrash git
                // on each regen.
                let mut nested_type_values: Vec<&String> = all_nested_types.values().collect();
                nested_type_values.sort();
                for nested_type in nested_type_values {
                    if !imports.contains(nested_type) {
                        imports.push(nested_type.clone());
                    }
                }
                // Also import enum types referenced in this test file
                let mut enum_field_values: Vec<&String> = all_enum_fields.values().collect();
                enum_field_values.sort();
                for enum_type in enum_field_values {
                    if !imports.contains(enum_type) {
                        imports.push(enum_type.clone());
                    }
                }
            }
        }

        // Result-enum classes are imported even when no options-type imports
        // are needed — assertions on enum-typed result fields reference the
        // enum class by name (e.g. `WasmFinishReason.Stop`).
        if lang == "wasm" {
            for enum_class in &all_result_enum_classes {
                if !imports.contains(enum_class) {
                    imports.push(enum_class.clone());
                }
            }
            // Also import handle config types for WASM
            for fixture in fixtures.iter() {
                let cc = e2e_config.resolve_call_for_fixture(
                    fixture.call.as_deref(),
                    &fixture.id,
                    &fixture.resolved_category(),
                    &fixture.tags,
                    &fixture.input,
                );
                if let Some(o) = cc.overrides.get("wasm") {
                    if let Some(config_type) = &o.handle_config_type {
                        if !imports.contains(config_type) {
                            imports.push(config_type.clone());
                        }
                    }
                }
            }
        }

        let imports_str = imports.join(", ");
        import_modules = format!("import {{ {imports_str} }} from \"{pkg_name}\";");

        if needs_cache_isolation && has_configure {
            import_node_fs = "import { mkdtempSync, rmSync } from \"node:fs\";\nimport { join } from \"node:path\";\nimport { tmpdir } from \"node:os\";".to_string();
        }
    }

    // WASM: even if needs_options_import is false, if we have nested types
    // (e.g., from handle_config_type), we should import them because they're
    // used in handle config construction in setup lines. Example: WasmAuthConfig
    // is used when building WasmCrawlConfig fields, even if there's no direct
    // json_object arg in the fixture input.
    if lang == "wasm" && !all_nested_types.is_empty() {
        let mut additional_imports: Vec<String> = Vec::new();
        // Sort values for deterministic import ordering — HashMap iteration
        // order is non-deterministic and would thrash git on each regen.
        let mut nested_type_values: Vec<&String> = all_nested_types.values().collect();
        nested_type_values.sort();
        for nested_type in nested_type_values {
            if !import_modules.contains(nested_type) && !additional_imports.contains(nested_type) {
                additional_imports.push(nested_type.clone());
            }
        }
        // Also import enum types that might be used in handle config
        let mut enum_field_values: Vec<&String> = all_enum_fields.values().collect();
        enum_field_values.sort();
        for enum_type in enum_field_values {
            if !import_modules.contains(enum_type) && !additional_imports.contains(enum_type) {
                additional_imports.push(enum_type.clone());
            }
        }
        if !additional_imports.is_empty() {
            if import_modules.is_empty() {
                let imports_str = additional_imports.join(", ");
                import_modules = format!("import {{ {imports_str} }} from \"{pkg_name}\";");
            } else {
                // Append to existing imports
                let existing_import_start = "import { ".len();
                let existing_import_end = import_modules.rfind(" } from").unwrap_or(import_modules.len());
                let existing_part = &import_modules[existing_import_start..existing_import_end];
                let mut all_imports: Vec<&str> = existing_part.split(", ").collect();
                for imp in &additional_imports {
                    all_imports.push(imp);
                }
                let imports_str = all_imports.join(", ");
                import_modules = format!("import {{ {imports_str} }} from \"{pkg_name}\";");
            }
        }
    }

    // Build helper functions string.
    // Emit for non-HTTP fixtures (tree assertions) AND for HTTP-only files that reference
    // `_alefE2eDecompressAndParseJson` (JSON body / partial body / validation error assertions).
    let helper_functions = if has_non_http_fixtures || http_fixtures_need_decompress_helper {
        crate::e2e::template_env::render("typescript/helpers.jinja", minijinja::context! {})
    } else {
        String::new()
    };

    // Build cache isolation setup
    let mut cache_isolation_setup = String::new();
    if needs_cache_isolation && has_configure {
        emit_cache_isolation_setup(&mut cache_isolation_setup);
    }

    // Build env var setup
    let env_setup = render_env_setup(&e2e_config.env);

    // Build fixtures body
    let mut fixtures_body = String::new();
    for (i, fixture) in fixtures.iter().enumerate() {
        if fixture.is_http_test() {
            render_http_test_case(&mut fixtures_body, fixture);
        } else {
            render_test_case(
                &mut fixtures_body,
                fixture,
                client_factory,
                options_type,
                e2e_config,
                lang,
                &nested_types,
                &enum_fields,
                &result_enum_fields,
                type_defs,
                enums,
                wasm_type_prefix,
                config,
            );
        }
        if i + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }

    let ctx = minijinja::context! {
        header => hash::header(CommentStyle::DoubleSlash),
        import_vitest => import_vitest,
        import_modules => import_modules,
        import_node_fs => import_node_fs,
        helper_functions => helper_functions,
        category => category,
        env_setup => env_setup,
        cache_isolation_setup => cache_isolation_setup,
        fixtures_body => fixtures_body,
    };
    crate::e2e::template_env::render("typescript/test_file.jinja", ctx)
}
