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

/// Every binding class this fixture's handle-config value will construct.
///
/// Delegates to `collect_used_handle_config_types`, which is the emitting traversal itself with
/// its output discarded — the import block therefore learns the class set from the code generator
/// rather than from a parallel re-derivation that can fall out of step with it. ~keep
fn handle_config_classes(
    fixture: &Fixture,
    call_config: &crate::core::config::e2e::CallConfig,
    override_config: &crate::core::config::e2e::CallOverride,
    handle_config_type: &str,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
) -> std::collections::BTreeSet<String> {
    let mut used_types = std::collections::BTreeSet::new();
    let effective_nested_types: std::collections::HashMap<String, String> = {
        let mut derived = derive_nested_types_for_wasm(handle_config_type, type_defs, wasm_type_prefix);
        for (key, value) in &override_config.nested_types {
            derived.insert(key.clone(), value.clone());
        }
        derived
    };
    // Only the class map and the JSON shape decide which classes get constructed; `enum_fields`
    // and `bigint_fields` steer scalar rendering, whose string this caller discards. ~keep
    let bigint_fields: std::collections::BTreeSet<String> = override_config.bigint_fields.iter().cloned().collect();
    let owner_type = handle_config_type
        .strip_prefix(wasm_type_prefix)
        .unwrap_or(handle_config_type);
    let context = HandleConfigContext {
        nested_types: &override_config.nested_types,
        effective_nested_types: &effective_nested_types,
        lang: "wasm",
        enum_fields: &override_config.enum_fields,
        bigint_fields: &bigint_fields,
        type_defs,
        enums,
        wasm_type_prefix,
        owner_type: Some(owner_type),
    };
    for arg in call_config.args.iter().filter(|arg| arg.arg_type == "handle") {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let Some(config_value) = fixture.input.get(field) else {
            continue;
        };
        let Some(config_object) = config_value.as_object() else {
            continue;
        };
        for (key, value) in config_object {
            collect_used_handle_config_types(key, value, &context, &mut used_types);
        }
    }
    used_types
}

/// The identifiers an already-built `import { .. } from ".."` line brings into scope.
///
/// Returns an empty set for an empty or unrecognised line, so a caller that finds nothing simply
/// re-adds what it needs — a duplicate import name is a compile error the generated file would
/// surface immediately, whereas a missing one only fails at run time.
fn imported_identifiers(import_line: &str) -> std::collections::BTreeSet<String> {
    let Some(open) = import_line.find("import { ") else {
        return std::collections::BTreeSet::new();
    };
    let rest = &import_line[open + "import { ".len()..];
    let Some(close) = rest.rfind(" } from") else {
        return std::collections::BTreeSet::new();
    };
    rest[..close]
        .split(", ")
        .map(|entry| {
            let entry = entry.trim();
            entry.strip_prefix("type ").unwrap_or(entry).to_string()
        })
        .collect()
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
    functions: &[crate::core::ir::FunctionDef],
    wasm_type_prefix: &str,
    config: &crate::core::config::ResolvedCrateConfig,
    errors: &[crate::core::ir::ErrorDef],
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

    let mut referenced_enums = std::collections::BTreeSet::new();
    let mut fixtures_body = String::new();
    for (index, fixture) in fixtures.iter().enumerate() {
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
                functions,
                wasm_type_prefix,
                config,
                &mut referenced_enums,
                errors,
            );
        }
        if index + 1 < fixtures.len() {
            fixtures_body.push('\n');
        }
    }

    // Per-fixture wasm/node overrides may add their own options_type / nested_types /
    // enum_fields (each call exposes a different request struct in WASM, e.g.
    // `WasmEmbeddingRequest` vs `WasmChatCompletionRequest`). Aggregate every class
    // referenced across this file's fixtures so the import line covers them all.
    // The global `options_type` parameter remains the default fallback when a
    // per-call override is absent.
    let mut all_options_types: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut all_nested_types: std::collections::HashMap<String, String> = nested_types.clone();
    // Unlike `referenced_enums` (filled by the emitter as it writes each request-side
    // `EnumType.Member`, see `builders::enum_member_reference`), this set is derived purely from
    // `result_enum_fields` config, never from what an assertion body actually names. That is safe
    // only because no TypeScript/WASM assertion path ever emits a class-name reference for a
    // result-side enum field, config-derived or IR-derived: `render_wasm_enum_assertion` compares
    // the field against the plain wire-format string (`expect(result.kind).toBe("uri")`), and its
    // `enum_class` parameter is intentionally unused (`_enum_class`). There is no body reference
    // this parallel derivation could ever fail to cover, because the renderer that would produce
    // one does not exist on this path. Pinned by
    // `result_enum_import_invariant_tests::wasm_result_enum_field_assertion_never_references_the_configured_class`;
    // if a future change makes an assertion reference the class by name, route it through
    // `referenced_enums` the way the request-side builders do, rather than trusting this set. ~keep
    let mut all_result_enum_classes: std::collections::BTreeSet<String> =
        result_enum_fields.values().cloned().collect();
    // The body's constructor reference for a `json_object` "config" arg is resolved through
    // `wasm_prefixed_wrapped_type(lang, canonical_ts_type_name(lang, ..), ..)` (see
    // `json_object_constructor_type` in `args.rs`), not through `canonical_ts_type_name` alone.
    // Every `options_type` collected into the import set here must go through the same two-step
    // chain, or a bare (unprefixed) IR type name configured as `options_type` renders a body that
    // calls `WasmFoo.default()` while the import statement names the unprefixed `Foo` — a
    // `ReferenceError` at runtime that `tsc` cannot catch, since both are valid identifiers.
    // For `lang != "wasm"`, `wasm_prefixed_wrapped_type` is a no-op, so this changes nothing for
    // the node backend, whose options types are TypeScript interfaces, not wasm-bindgen classes. ~keep
    if let Some(opts) = options_type {
        let canonical = canonical_ts_type_name(lang, opts, config);
        let resolved = wasm_prefixed_wrapped_type(lang, &canonical, type_defs, enums, wasm_type_prefix);
        all_options_types.insert(resolved);
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
                let canonical = canonical_ts_type_name(lang, opts, config);
                let resolved = wasm_prefixed_wrapped_type(lang, &canonical, type_defs, enums, wasm_type_prefix);
                all_options_types.insert(resolved);
            }
            for (k, v) in &o.nested_types {
                all_nested_types.entry(k.clone()).or_insert_with(|| v.clone());
            }
            for v in o.result_enum_fields.values() {
                all_result_enum_classes.insert(v.clone());
            }
            // For WASM, also collect handle_config_type so its nested types are imported
            if lang == "wasm"
                && let Some(handle_type) = &o.handle_config_type
            {
                all_options_types.insert(handle_type.clone());
                // Ask the renderer which classes the handle config actually constructs rather
                // than re-deriving the answer here. `collect_used_handle_config_types` runs the
                // same traversal `build_handle_config_value` runs and throws the string away, so
                // a class the body constructs cannot be one the import block never heard of —
                // the two-independent-walks split that leaves a nested class emitted but
                // undefined at run time. ~keep
                for class in handle_config_classes(fixture, cc, o, handle_type, type_defs, enums, wasm_type_prefix) {
                    all_nested_types.entry(class.clone()).or_insert(class);
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

        if lang == "wasm"
            && fixture.visitor.is_some()
            && let Some(binding) = wasm_visitor_binding(config, options_type)
        {
            all_options_types.insert(binding.options_type);
            all_options_types.insert(binding.handle_type);
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
        // `all_nested_types` is, by this point, the union of every call override's
        // `nested_types` seen across this file's fixtures (merged in by the per-fixture loop
        // above) plus the file-level default — exactly the override map the emitter itself
        // consults at every recursion depth. Passing it lets the BFS follow an
        // override-introduced edge (a fixture-authoring key with no direct IR-field
        // correspondence) the same way the body's own construction does, instead of being
        // limited to real `type_defs` struct fields alone. ~keep
        let derived_all = collect_transitive_nested_types_for_wasm(
            &all_options_types,
            type_defs,
            wasm_type_prefix,
            &all_nested_types,
        );
        // `derived_all` is a set of class names (not field-name keyed — two
        // distinct classes can share a field name, see the comment on
        // `collect_transitive_nested_types_for_wasm`). `all_nested_types` is
        // only ever read via `.values()` below for the import list, so a
        // synthetic key (the class name itself) is fine here.
        for class_name in derived_all {
            all_nested_types.entry(class_name.clone()).or_insert(class_name);
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
                if assertion.assertion_type == "method_result"
                    && let Some(method_name) = &assertion.method
                    && let Some(helper_fn) = ts_method_helper_import(method_name)
                    && !imports.contains(&helper_fn)
                {
                    imports.push(helper_fn);
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
                    let node_enum = (lang == "node")
                        .then(|| enums.iter().find(|definition| definition.name == elem_type))
                        .flatten();
                    let elem_import = match node_enum {
                        Some(definition) if crate::backends::napi::is_tagged_data_enum(definition) => {
                            Some(format!("type {elem_type}"))
                        }
                        Some(definition) if crate::backends::napi::is_untagged_data_enum(definition) => None,
                        _ => Some(elem_type),
                    };
                    if let Some(elem_import) = elem_import
                        && !is_typescript_primitive_element_type(&elem_import)
                        && !imports.contains(&elem_import)
                    {
                        imports.push(elem_import);
                    }
                }
                if lang == "node" && arg.arg_type == "json_object" {
                    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                    let val = if field == "input" {
                        Some(fixture.input.get("extract_input").unwrap_or(&fixture.input))
                    } else {
                        fixture.input.get(field)
                    };
                    if val.is_some_and(|v| !v.is_null())
                        && let Some(override_type) = cc
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
                // Enum classes are deliberately absent from this branch: they come from
                // `referenced_enums` below, which the builder fills as it emits. ~keep
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
                if let Some(o) = cc.overrides.get("wasm")
                    && let Some(config_type) = &o.handle_config_type
                    && !imports.contains(config_type)
                {
                    imports.push(config_type.clone());
                }
            }
        }

        // Every enum class the rendered bodies above actually named, as they named it.
        //
        // `referenced_enums` is filled by `builders::enum_member_reference` at the moment it
        // formats an `EnumType.Member` expression, so this is not a second opinion about which
        // enums a fixture touches — it is the emitter's own record. Deriving the list instead
        // from the `enum_fields` config (as the wasm branch above used to) was wrong twice over:
        // it missed a field whose enum type comes from the IR with no hand-written entry, and it
        // imported the bare IR name where the body emits the `wasm_type_prefix`-ed one. Both
        // failures look identical at run time — `ReferenceError: WasmFoo is not defined`. ~keep
        for enum_type in &referenced_enums {
            if !imports.contains(enum_type) {
                imports.push(enum_type.clone());
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
    if lang == "wasm" && (!all_nested_types.is_empty() || !referenced_enums.is_empty()) {
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
        // The enum classes the rendered bodies named — again the emitter's own record, not a
        // re-derivation from `enum_fields` (see the `referenced_enums` loop above). Membership is
        // tested against the parsed import list rather than `import_modules.contains`: a
        // substring test reports `WasmKind` as already imported when only `WasmKindDetail` is,
        // and a silently-dropped enum import is exactly the failure this is here to prevent. ~keep
        let already_imported = imported_identifiers(&import_modules);
        for enum_type in &referenced_enums {
            if !already_imported.contains(enum_type.as_str()) && !additional_imports.contains(enum_type) {
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
