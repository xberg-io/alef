use super::*;
use crate::e2e::codegen::client_factory::js_client_construction;

pub(crate) struct SnippetContext<'a> {
    pub lang: &'a str,
    pub fixture: &'a Fixture,
    pub module: &'a str,
    pub client_factory: Option<&'a str>,
    pub e2e_config: &'a E2eConfig,
    pub type_defs: &'a [TypeDef],
    pub enums: &'a [EnumDef],
    /// `ApiSurface::functions`. Empty for a caller with no free-function registry in scope,
    /// which leaves the presentation resolver's field facts unanchored — the pre-existing
    /// behaviour — rather than changing any verdict. ~keep
    pub functions: &'a [crate::core::ir::FunctionDef],
    pub wasm_type_prefix: &'a str,
    pub config: &'a crate::core::config::ResolvedCrateConfig,
}

/// The node docs-snippet body, assembling the [`SnippetContext`] from the `node` call overrides.
///
/// Lives beside the renderer rather than in the backend's `mod.rs` so the module that owns
/// `SnippetContext` also owns how a node context is built — `wasm/snippet.rs` builds its own the
/// same way, and `mod.rs` is a remediation target that must not absorb more of this. ~keep
pub(crate) fn render_node_snippet_body(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> String {
    let overrides = e2e_config.call.overrides.get("node");
    let module = e2e_config
        .resolve_package("node")
        .and_then(|package| package.name)
        .or_else(|| overrides.and_then(|value| value.module.clone()))
        .unwrap_or_else(|| e2e_config.call.module.clone());
    render_snippet_body(SnippetContext {
        lang: "node",
        fixture,
        module: &module,
        client_factory: overrides.and_then(|value| value.client_factory.as_deref()),
        e2e_config,
        type_defs,
        enums,
        functions,
        wasm_type_prefix: "",
        config,
    })
}

pub(crate) fn render_snippet_body(context: SnippetContext<'_>) -> String {
    let SnippetContext {
        lang,
        fixture,
        module,
        client_factory,
        e2e_config,
        type_defs,
        enums,
        functions,
        wasm_type_prefix,
        config,
    } = context;
    let wasm_surface = (lang == "wasm").then(|| effective_wasm_e2e_surface(type_defs, enums, &[], config));
    let emitted_wasm_types = wasm_surface.as_ref().map(|surface| surface.emitted_types());
    let wasm_excluded_classes = wasm_surface
        .as_ref()
        .map(|surface| wasm_excluded_class_names(surface, wasm_type_prefix))
        .unwrap_or_default();
    let filtered_e2e_config = wasm_surface
        .as_ref()
        .map(|_| without_excluded_wasm_nested_types(e2e_config, &wasm_excluded_classes));
    let type_defs = emitted_wasm_types.as_deref().unwrap_or(type_defs);
    let e2e_config = filtered_e2e_config.as_ref().unwrap_or(e2e_config);
    let docs_fixture = fixture.docs_call_fixture();
    let fixture = &docs_fixture;
    let mut call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    call = crate::e2e::codegen::select_best_matching_call(call, e2e_config, fixture);
    // A `const` loop binding is in its own initializer's scope, so a loop named after the result
    // it iterates is `TS2448`, not a shadow. Decided before anything renders — the per-item field
    // accessors below are rooted at this name. ~keep
    let unshadowed =
        crate::e2e::codegen::loop_binding::without_shadowed_loop_bindings(fixture, &[call.effective_result_var()]);
    let fixture = unshadowed.as_ref();
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve(lang, fixture, call, type_defs)
        .with_functions(functions);
    let override_config = recipe.override_config;
    let options_type = recipe
        .options_type
        .map(|name| canonical_ts_type_name(lang, name, config));
    let mut nested_types = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|value| value.nested_types.clone())
        .unwrap_or_default();
    let mut enum_fields = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|value| value.enum_fields.clone())
        .unwrap_or_default();
    let mut bigint_fields: std::collections::BTreeSet<String> = e2e_config
        .call
        .overrides
        .get(lang)
        .map(|value| value.bigint_fields.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(value) = override_config {
        nested_types.extend(value.nested_types.clone());
        enum_fields.extend(value.enum_fields.clone());
        bigint_fields.extend(value.bigint_fields.iter().cloned());
    }
    infer_enum_fields(recipe.options_type, type_defs, enums, &mut enum_fields);
    for argument in recipe.args {
        infer_enum_fields(argument.element_type.as_deref(), type_defs, enums, &mut enum_fields);
    }
    let handle_config_type = override_config.and_then(|value| value.handle_config_type.as_deref());
    let (mut setup_lines, mut args) = build_args_and_setup(
        &fixture.input,
        recipe.args,
        options_type.as_deref(),
        fixture,
        &nested_types,
        lang,
        &enum_fields,
        &bigint_fields,
        handle_config_type,
        type_defs,
        enums,
        wasm_type_prefix,
        config,
        true,
        &mut Default::default(),
        recipe.target_params(lang),
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    // Same attribution the e2e test-file path does, with the fallback key this path actually
    // reads -- see `fixture_refusal::call_level_source`. ~keep
    crate::e2e::codegen::fixture_refusal::attribute(
        lang,
        &fixture.id,
        crate::e2e::codegen::fixture_refusal::resolved_call_key(e2e_config, call),
        crate::e2e::codegen::fixture_refusal::call_level_source(
            override_config.and_then(|value| value.options_type.as_deref()),
            call.options_type.as_deref(),
        ),
    );
    if !recipe.extra_args.is_empty() {
        let extras = recipe.extra_args.join(", ");
        args = if args.is_empty() {
            extras
        } else {
            format!("{args}, {extras}")
        };
    }
    let mut visitor_imports = Vec::new();
    if let Some(visitor_spec) = &fixture.visitor {
        let visitor_arg = build_typescript_visitor(&mut setup_lines, visitor_spec);
        if lang == "wasm"
            && let Some(binding) = wasm_visitor_binding(config, options_type.as_deref())
        {
            visitor_imports.extend([binding.options_type.clone(), binding.handle_type.clone()]);
            args = apply_wasm_visitor_arg(&args, &visitor_arg, &binding);
        } else if lang == "node" {
            args = node_visitor_args(&args, &visitor_arg);
        }
    }

    let function_name = resolve_js_function_name(lang, call);
    let effective_factory = override_config
        .and_then(|value| value.client_factory.as_deref())
        .or(client_factory);
    let call_expr = if effective_factory.is_some() {
        format!("client.{function_name}({args})")
    } else {
        format!("{function_name}({args})")
    };
    let client = effective_factory.map(|factory| js_client_construction(fixture, e2e_config, call, lang, factory));
    let client_setup = client.as_ref().map(|(_, setup)| setup.clone()).unwrap_or_default();
    let client_release = wasm_client_release(lang, effective_factory);
    let expects_error = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "error");
    // Both langs this renders for ignore a crate `error_type` name: node throws a plain
    // global `Error`, and wasm renders `String(error)` via `thrown_value_is_opaque` instead
    // of an `instanceof` check (see that template flag below), so it is never read. ~keep
    let error_type_name = "Error".to_string();
    let mut imports = std::collections::BTreeSet::new();
    imports.insert(client.map_or_else(|| function_name.clone(), |(imported, _)| imported));
    // No `else` branch imports an error type here: node throws a plain global
    // `Error` (nothing to import) and wasm-bindgen throws a bare JS string
    // (also nothing to import, and no named error export exists to import in
    // the first place -- see the `thrown_value_is_opaque` template branch).
    imports.extend(visitor_imports);
    let referenced_code = format!("{}\n{args}\n{client_setup}", setup_lines.join("\n"));
    // Every imported type name goes through the same prefixing helper the body
    // uses. For wasm the emitted code constructs prefixed classes
    // (`WasmExtractInput`), so an unprefixed import names a symbol the package
    // does not export -- `render_test_file` prefixes at its own import sites for
    // exactly this reason. Non-wasm languages pass through unchanged.
    let import_name = |name: &str| wasm_prefixed_wrapped_type(lang, name, type_defs, enums, wasm_type_prefix);
    if let Some(name) = options_type.as_deref().map(import_name)
        && references_identifier(&referenced_code, &name)
    {
        imports.insert(name);
    }
    imports.extend(
        nested_types
            .into_values()
            .chain(enum_fields.into_values())
            .map(|name| import_name(&name))
            .filter(|name| references_identifier(&referenced_code, name)),
    );
    // WASM builder expressions construct nested classes recursively
    // (`ts_builder_expression_inner`): `_u0.config = (() => { const _u1 =
    // WasmFileExtractionConfig.default(); ... })()`. `nested_types` above is only the
    // config-authored map (`[crates.e2e.calls.<call>.overrides.wasm].nested_types`), so a
    // class reached solely through the options type's own IR fields — with no per-call
    // config entry naming it, as a call with no wasm override at all has none — was built
    // into the snippet body but never imported, failing at typecheck with "Cannot find
    // name". `render_test_file`'s import builder already walks the IR transitively for the
    // same reason (`collect_transitive_nested_types_for_wasm`); this standalone-snippet path
    // lacked the same walk. Filtered by `references_identifier`, matching every other
    // entry in this list, so a reachable-but-unused class is never imported.
    //
    // The walk must also seed from every `json_object` arg's array `element_type`
    // (`extractBatch(inputs: ExtractInput[])`), not just the call's single `options_type`:
    // once each array element is built via the same typed wasm builder (see
    // `build_args_and_setup`'s array branch), a class reached only through *that* element
    // type's own fields — e.g. `ExtractInput.config: FileExtractionConfig` — is exactly as
    // unreachable from `options_type` as the options-type case above, and was built into the
    // snippet body but never imported for the identical reason. ~keep
    if lang == "wasm" {
        let mut seeds: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        if let Some(seed) = options_type.as_deref() {
            seeds.insert(seed.to_string());
        }
        for arg in recipe.args {
            if arg.arg_type == "json_object"
                && let Some(element_type) = &arg.element_type
            {
                seeds.insert(canonical_ts_type_name(lang, element_type, config));
            }
        }
        if !seeds.is_empty() {
            // No override `nested_types` map is threaded into this walk: unlike
            // `render_test_file`, this path already sweeps every `type_defs` entry against
            // `referenced_code` below (the struct-typed twin of the enum sweep just above), so a
            // class reachable only through an override-introduced edge (e.g. `SsrfPolicy`
            // reached through an `AuthConfig` an override alone names) is still caught by that
            // sweep even though this BFS cannot follow the override edge to find it. ~keep
            imports.extend(
                collect_transitive_nested_types_for_wasm(
                    &seeds,
                    type_defs,
                    wasm_type_prefix,
                    &std::collections::HashMap::new(),
                )
                .into_iter()
                .map(|name| import_name(&name))
                .filter(|name| references_identifier(&referenced_code, name)),
            );
        }
    }
    // A trait-bridge stub method returning a named enum annotates its signature with that
    // enum and casts through it (`(): ProcessingStage { return "\"Early\"" as unknown as
    // ProcessingStage; }` — see `emit_test_backend`'s `type_imports`). The enum is a
    // top-level type, so it is reached by neither `nested_types` nor `enum_fields`, and
    // without an import the emitted snippet does not type-check. Both uses are type
    // positions, but the module is imported as values here, which covers them.
    imports.extend(
        enums
            .iter()
            .map(|enum_def| import_name(&enum_def.name))
            .filter(|name| references_identifier(&referenced_code, name)),
    );
    // The struct-typed twin of the enum sweep above: a trait-bridge stub method returning a
    // plain struct (e.g. `OcrBackend.processImage(): Promise<ExtractedDocument>`) annotates
    // its signature with that struct and casts its JSON-string body through it the same way
    // (`(): ExtractedDocument { return "{}" as unknown as ExtractedDocument; }` — see
    // `emit_test_backend`'s `type_imports`). Also a top-level type reached by neither
    // `nested_types` nor `enum_fields`, so it needs the same referenced-code sweep or the
    // emitted snippet fails to typecheck on the unimported cast target. ~keep
    imports.extend(
        type_defs
            .iter()
            .map(|type_def| import_name(&type_def.name))
            .filter(|name| references_identifier(&referenced_code, name)),
    );
    for arg in recipe.args {
        if arg.arg_type == "json_object"
            && let Some(type_name) = &arg.element_type
        {
            let type_name = import_name(&canonical_ts_type_name(lang, type_name, config));
            if references_identifier(&referenced_code, &type_name) {
                imports.insert(type_name);
            }
        }
        if arg.arg_type == "handle" {
            imports.insert(format!("create{}", arg.name.to_upper_camel_case()));
        }
    }
    if let Some(name) = handle_config_type
        && references_identifier(&referenced_code, name)
    {
        imports.insert(name.to_string());
    }

    let stream_item_binding = super::snippet_stream_resources::item_binding(fixture, e2e_config, call, lang);
    let wasm_stream_core_type = if lang == "wasm" && stream_item_binding.is_some() {
        super::snippet_stream_resources::item_type(call, config, &function_name, type_defs)
    } else {
        None
    };
    let wasm_stream_item_type = wasm_stream_core_type.map(import_name);
    if let Some(name) = &wasm_stream_item_type {
        imports.insert(name.clone());
    }
    let presentation =
        crate::e2e::codegen::presentation::resolve(fixture, e2e_config, lang, type_defs, enums, functions);
    let wasm_stream_resources = match (wasm_stream_core_type, stream_item_binding.as_deref()) {
        (Some(item_type), Some(root)) => {
            super::snippet_stream_resources::plans(&presentation, item_type, type_defs, root)
        }
        _ => Vec::new(),
    };
    if lang == "wasm" {
        imports.retain(|name| wasm_import_is_exported(name, &wasm_excluded_classes));
    }
    crate::e2e::template_env::render(
        "typescript/snippet_body.jinja",
        minijinja::context! {
            imports => imports.into_iter().collect::<Vec<_>>(), module => module,
            setup_lines => setup_lines, client_setup => client_setup, call_expr => call_expr,
            result_var => call.effective_result_var(),
            is_async => stream_item_binding.is_some() || override_config.and_then(|value| value.r#async).unwrap_or(call.r#async),
            stream_item_binding => stream_item_binding,
            wasm_stream => lang == "wasm",
            wasm_stream_item_type => wasm_stream_item_type,
            wasm_stream_resources => wasm_stream_resources,
            expects_error => expects_error, client_release => client_release,
            error_type => error_type_name.clone(),
            thrown_value_is_opaque => lang == "wasm",
            returns_void => call.returns_void,
            presentation => presentation,
        },
    )
}

/// The statement a WASM snippet must run to release the client it constructed, if any.
///
/// This renderer serves both `node` and `wasm`, and only `wasm` has anything to release:
/// wasm-bindgen gives every exported class a `free()`, while alef's napi wrapper
/// (`backends/napi/templates/config_opaque_wrapper.rs.jinja`) emits an empty `impl` with no
/// release surface at all. Returning `None` for node therefore keeps every node snippet
/// byte-identical, and so does a snippet with no `client_factory` in either language.
///
/// Only the client is released, deliberately. wasm-bindgen passes an owned class argument by
/// calling `__destroy_into_raw()` on it at the call site, which zeroes the JS wrapper's pointer,
/// and the generated `free()` carries no null guard — so releasing a request DTO the snippet
/// already handed to the call is a free of a null pointer, not a fix. The client is never
/// consumed that way (its methods read `this.__wbg_ptr` and leave it intact), so it is the one
/// object the snippet still owns when the body ends.
///
/// Emitted as an explicit `free()` rather than a TypeScript 5.2 `using` declaration so a
/// published snippet imposes no TypeScript version floor on the consumer's docs site. ~keep
fn wasm_client_release(lang: &str, effective_factory: Option<&str>) -> Option<&'static str> {
    (lang == "wasm" && effective_factory.is_some()).then_some("client.free();")
}

/// Whether `code` uses `identifier` as a whole identifier, not merely as a substring of some
/// longer one.
///
/// Every import above is gated on "does the rendered body mention this name" -- a plain
/// `code.contains(identifier)` says yes whenever `identifier` is a *prefix* of a longer name the
/// body legitimately uses, e.g. the wasm-prefixed `WasmOcrBackend` is a substring of
/// `WasmOcrBackendType`. A crate-wide enum registry search (the trait-bridge enum-import loop
/// below) can then add an import for a symbol the binding never exports, because some unrelated
/// IR name happens to prefix-match a real one -- `import { ..., WasmOcrBackend, ... }` from a
/// package that only exports `WasmOcrBackendType`. `wasm/snippet.rs` already has this exact
/// check under the name `names_identifier`; this file needs its own copy rather than a
/// cross-module `pub(crate)` reach, per this repo's third-repetition rule. ~keep
pub(super) fn references_identifier(code: &str, identifier: &str) -> bool {
    let is_identifier_char = |character: char| character.is_alphanumeric() || character == '_' || character == '$';
    code.match_indices(identifier).any(|(start, matched)| {
        let before_is_identifier = code[..start].chars().next_back().is_some_and(is_identifier_char);
        let after_is_identifier = code[start + matched.len()..]
            .chars()
            .next()
            .is_some_and(is_identifier_char);
        !before_is_identifier && !after_is_identifier
    })
}

fn infer_enum_fields(
    type_name: Option<&str>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    fields: &mut std::collections::HashMap<String, String>,
) {
    let Some(type_name) = type_name else { return };
    let mut pending = vec![type_name.to_string()];
    let mut visited = std::collections::HashSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(type_def) = type_defs.iter().find(|definition| definition.name == name) else {
            continue;
        };
        for field in &type_def.fields {
            let Some(named) = crate::e2e::codegen::call_ir::named_type(&field.ty) else {
                continue;
            };
            if enums.iter().any(|definition| definition.name == named) {
                // Key by owning-type + field, not the bare field name: this map
                // accumulates entries from every type reachable in the call's whole
                // type graph (see the two `infer_enum_fields` calls in
                // `render_snippet_body`), so two unrelated structs that happen to
                // share a field name (e.g. `TranscriptionConfig.model: WhisperModel`
                // and `LlmConfig.model: String`) must not collide on one key.
                fields
                    .entry(enum_field_key(&type_def.name, &field.name))
                    .or_insert_with(|| named.to_string());
            } else if type_defs.iter().any(|definition| definition.name == named) {
                pending.push(named.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests;
