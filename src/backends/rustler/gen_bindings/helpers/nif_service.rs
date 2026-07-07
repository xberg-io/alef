use crate::backends::rustler::template_env;
use crate::codegen::shared::binding_fields;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::TypeRef;
use ahash::AHashSet;
use heck::ToSnakeCase;

use super::json_values::elixir_safe_param_name;

/// Generate the `{AppModule}.Native` Elixir module with NIF stubs for all functions and methods.
pub(in crate::backends::rustler::gen_bindings) fn gen_native_ex(
    api: &crate::core::ir::ApiSurface,
    app_name: &str,
    app_module: &str,
    _crate_name: &str,
    config: &ResolvedCrateConfig,
    exclude_functions: &AHashSet<String>,
    exclude_types: &AHashSet<&str>,
) -> String {
    let mut out = String::with_capacity(1024);

    let repo_url = config.github_repo();
    // The env var that forces a local source build: {APP_NAME_UPPER}_BUILD
    let build_env_var = format!("{}_BUILD", app_name.to_uppercase());

    // RustlerPrecompiled targets list. Reads from `[languages.elixir]
    // nif_targets` in alef.toml; falls back to the historical default when
    // the consumer hasn't customized it. Must agree with the consumer's CI
    // matrix and `generate-elixir-checksums` action targets input.
    let default_nif_targets: &[&str] = &[
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-gnu",
    ];
    let nif_targets = match config.elixir.as_ref() {
        Some(elixir) if !elixir.nif_targets.is_empty() => elixir.nif_targets.join(" "),
        _ => default_nif_targets.join(" "),
    };

    // RustlerPrecompiled NIF ABI versions. Reads `[crates.publish.languages.elixir]
    // nif_versions` — the SAME key that `publish::package::elixir::resolve_nif_versions`
    // uses for tarball naming — so the generated `nif_versions:` list and the packaged
    // artifacts always agree (a mismatch makes on-load abort with a 404 at consumer
    // install). Falls back to the historical default when unset. Must also agree with
    // the consumer's CI `nif:` build matrix.
    let default_nif_versions: &[&str] = &["2.16", "2.17"];
    let nif_versions: Vec<String> = config
        .publish
        .as_ref()
        .and_then(|publish| publish.languages.get("elixir"))
        .and_then(|lang_cfg| lang_cfg.nif_versions.as_ref())
        .filter(|versions| !versions.is_empty())
        .cloned()
        .unwrap_or_else(|| default_nif_versions.iter().map(|version| version.to_string()).collect());
    let nif_versions_block = nif_versions
        .iter()
        .map(|version| format!("\"{version}\""))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&hash::header(CommentStyle::Hash));
    let nif_targets_list: Vec<&str> = nif_targets.split_whitespace().collect();
    let last_idx = nif_targets_list.len().saturating_sub(1);
    let nif_targets_block = nif_targets_list
        .iter()
        .enumerate()
        .map(|(idx, target)| {
            if idx == last_idx {
                format!("      \"{target}\"")
            } else {
                format!("      \"{target}\",")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let ctx = minijinja::context! {
        app_module => app_module,
        app_name => app_name,
        repo_url => repo_url,
        build_env_var => build_env_var,
        nif_targets_block => nif_targets_block,
        nif_versions_block => nif_versions_block,
    };
    out.push_str(&template_env::render("native_module_header.jinja", ctx));

    // Stubs for top-level API functions
    let mut last_was_multiline = true;
    let mut emitted_nif_stubs: AHashSet<String> = AHashSet::new();
    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let fn_name = if func.is_async {
            let n = func.name.as_str();
            if n.ends_with("_async") {
                n.to_string()
            } else {
                format!("{n}_async")
            }
        } else {
            func.name.clone()
        };
        let underscored_params: Vec<String> = func
            .params
            .iter()
            .map(|p| format!("_{}", p.name.to_snake_case()))
            .collect();
        if write_nif_doc(&mut out, &func.doc, last_was_multiline) {
            // @doc attaches to the next def with no blank line between them.
            last_was_multiline = true;
        }
        last_was_multiline = write_nif_stub(&mut out, &fn_name, &underscored_params, last_was_multiline);
        emitted_nif_stubs.insert(fn_name.clone());

        // For functions that have a visitor bridge (FunctionParam pattern), also emit the
        // async visitor variant stub plus the visitor_reply NIF stub (once).
        let has_visitor_bridge = config.trait_bridges.iter().any(|b| {
            b.bind_via != crate::core::config::BridgeBinding::OptionsField
                && func.params.iter().any(|p| {
                    b.param_name.as_deref() == Some(p.name.as_str()) || {
                        let named = match &p.ty {
                            TypeRef::Named(n) => Some(n.as_str()),
                            TypeRef::Optional(inner) => {
                                if let TypeRef::Named(n) = inner.as_ref() {
                                    Some(n.as_str())
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        named.map(|n| b.type_alias.as_deref() == Some(n)).unwrap_or(false)
                    }
                })
        });
        if has_visitor_bridge {
            // Params for convert_with_visitor: same as convert but visitor is required (not optional).
            let with_visitor_params: Vec<String> = func
                .params
                .iter()
                .map(|p| format!("_{}", p.name.to_snake_case()))
                .collect();
            last_was_multiline = write_nif_stub(
                &mut out,
                &format!("{fn_name}_with_visitor"),
                &with_visitor_params,
                last_was_multiline,
            );
            emitted_nif_stubs.insert(format!("{fn_name}_with_visitor"));
        }

        // For functions that have an options_field visitor bridge, emit
        // `{fn_name}_with_visitor` stub with the original params + `_visitor` appended.
        let has_options_field_bridge = config.trait_bridges.iter().any(|b| {
            b.bind_via == crate::core::config::BridgeBinding::OptionsField
                && func.params.iter().any(|p| {
                    let type_name = match &p.ty {
                        TypeRef::Named(n) => Some(n.as_str()),
                        TypeRef::Optional(inner) => {
                            if let TypeRef::Named(n) = inner.as_ref() {
                                Some(n.as_str())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    type_name.is_some_and(|n| b.options_type.as_deref() == Some(n))
                })
        });
        if has_options_field_bridge {
            // Params: all original params (options is Option<String>) + _visitor at the end.
            let mut with_visitor_params: Vec<String> = func
                .params
                .iter()
                .map(|p| format!("_{}", p.name.to_snake_case()))
                .collect();
            with_visitor_params.push("_visitor".to_string());
            last_was_multiline = write_nif_stub(
                &mut out,
                &format!("{fn_name}_with_visitor"),
                &with_visitor_params,
                last_was_multiline,
            );
            emitted_nif_stubs.insert(format!("{fn_name}_with_visitor"));
        }
    }

    // visitor_reply stub: emitted once when there are visitor bridges.
    if !config.trait_bridges.is_empty() {
        last_was_multiline = write_nif_stub(
            &mut out,
            "visitor_reply",
            &["_ref_id".to_string(), "_result".to_string()],
            last_was_multiline,
        );
        // Visitor trait call completion stubs (for async trait methods)
        last_was_multiline = write_nif_stub(
            &mut out,
            "complete_trait_call",
            &["_reply_id".to_string(), "_result_json".to_string()],
            last_was_multiline,
        );
        last_was_multiline = write_nif_stub(
            &mut out,
            "fail_trait_call",
            &["_reply_id".to_string(), "_error_message".to_string()],
            last_was_multiline,
        );
    }

    // Trait bridge registration stubs (register_fn, unregister_fn, clear_fn).
    // These are emitted for each trait bridge that doesn't exclude Elixir.
    for bridge in &config.trait_bridges {
        if bridge.exclude_languages.contains(&"elixir".to_string()) {
            continue;
        }

        // register_fn stub: takes (env, pid, name, implemented_methods) -> Atom
        if let Some(register_fn) = &bridge.register_fn {
            let params = vec![
                "_pid".to_string(),
                "_name".to_string(),
                "_implemented_methods".to_string(),
            ];
            if emitted_nif_stubs.insert(register_fn.clone()) {
                last_was_multiline = write_nif_stub(&mut out, register_fn, &params, last_was_multiline);
            }
        }

        // unregister_fn stub: takes (env, name) -> Atom
        if let Some(unregister_fn) = &bridge.unregister_fn {
            let params = vec!["_name".to_string()];
            if emitted_nif_stubs.insert(unregister_fn.clone()) {
                last_was_multiline = write_nif_stub(&mut out, unregister_fn, &params, last_was_multiline);
            }
        }

        // clear_fn stub: takes (env) -> Atom (no args besides env)
        if let Some(clear_fn) = &bridge.clear_fn {
            let params = vec![];
            if emitted_nif_stubs.insert(clear_fn.clone()) {
                last_was_multiline = write_nif_stub(&mut out, clear_fn, &params, last_was_multiline);
            }
        }
    }

    // Streaming-adapter method keys are emitted as start/next pairs below — skip
    // them in the regular method-stub loop.
    let streaming_method_keys: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
        .collect();

    // Stubs for type methods
    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
    {
        for method in typ
            .methods
            .iter()
            .filter(|m| !exclude_functions.contains(m.name.as_str()))
            .filter(|m| !streaming_method_keys.contains(&format!("{}.{}", typ.name, m.name)))
        {
            let nif_fn_name = if method.is_async {
                format!("{}_{}_async", typ.name.to_lowercase(), method.name)
            } else {
                format!("{}_{}", typ.name.to_lowercase(), method.name)
            };

            let mut underscored_params: Vec<String> = Vec::new();
            if method.receiver.is_some() {
                underscored_params.push("_obj".to_string());
            }
            for p in &method.params {
                underscored_params.push(format!("_{}", elixir_safe_param_name(&p.name)));
            }

            if write_nif_doc(&mut out, &method.doc, last_was_multiline) {
                last_was_multiline = true;
            }
            last_was_multiline = write_nif_stub(&mut out, &nif_fn_name, &underscored_params, last_was_multiline);
        }
    }

    // Stubs for streaming-adapter NIF pairs: `{owner_lc}_{name}_start(_obj, _req)`
    // and `{owner_lc}_{name}_next(_handle)`. Both NIFs are scheduled on DirtyCpu.
    // These are internal implementation details (delegated to by the public streaming wrapper
    // functions in the main module), so they are marked @doc false.
    for adapter in config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
    {
        let Some(owner) = adapter.owner_type.as_deref() else {
            continue;
        };
        let owner_lc = owner.to_lowercase();
        let start_fn = format!("{owner_lc}_{}_start", adapter.name);
        let next_fn = format!("{owner_lc}_{}_next", adapter.name);
        let mut start_params = vec!["_obj".to_string()];
        for p in &adapter.params {
            start_params.push(format!("_{}", elixir_safe_param_name(&p.name)));
        }
        // Streaming NIFs are internal — mark @doc false and skip inherited rustdoc.
        // The public wrapper functions in the main module expose the high-level API.
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("  @doc false\n");
        let _ = write_nif_stub(&mut out, &start_fn, &start_params, false);

        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("  @doc false\n");
        let _ = write_nif_stub(&mut out, &next_fn, &["_handle".to_string()], false);
    }

    // Stubs for *_from_json helper NIFs — only for types with NIF wrapper structs.
    // These are internal test utilities — mark @doc false.
    let nif_wrapped_types = collect_types_for_nif_derives(api, exclude_types);
    for typ in api.types.iter().filter(|t| {
        !t.is_trait
            && !t.is_opaque
            && !t.fields.is_empty()
            && t.has_serde
            && !exclude_types.contains(t.name.as_str())
            && nif_wrapped_types.contains(&t.name)
    }) {
        let from_json_fn_name = format!("{}_from_json", typ.name.to_snake_case());
        // *_from_json takes a JSON string and returns Result<Type, String>
        let params = vec!["_json".to_string()];
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("  @doc false\n");
        let _ = write_nif_stub(&mut out, &from_json_fn_name, &params, false);
    }

    // Stubs for service-API NIFs. `service.rs` (emitted by the service-API
    // codegen) declares the following `#[rustler::nif]` functions; every one
    // needs a matching Elixir stub or rustler-precompiled's on_load aborts
    // with `:bad_lib`:
    //   - `{service}_{ep.method}` per service × entrypoint (e.g. app_run, app_into_router)
    //   - `{service}_{variant.name}` per service × registration variant
    //     (e.g. app_get, app_post)
    //   - top-level `complete_trait_call(reply_id, response_json)` (forwards
    //     a GenServer-handled response back into the awaiting Rust bridge).
    if !api.services.is_empty() {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("  @doc false\n");
        last_was_multiline = write_nif_stub(
            &mut out,
            "complete_trait_call",
            &["_reply_id".to_string(), "_response_json".to_string()],
            last_was_multiline,
        );
        emitted_nif_stubs.insert("complete_trait_call".to_string());

        for service in &api.services {
            let service_snake = service.name.to_snake_case();
            for ep in &service.entrypoints {
                let fn_name = format!("{service_snake}_{}", ep.method);
                if emitted_nif_stubs.insert(fn_name.clone()) {
                    let mut params = vec!["_registrations".to_string()];
                    for p in &ep.params {
                        params.push(format!("_{}", elixir_safe_param_name(&p.name)));
                    }
                    if !out.is_empty() && !out.ends_with("\n\n") {
                        out.push('\n');
                    }
                    out.push_str("  @doc false\n");
                    last_was_multiline = write_nif_stub(&mut out, &fn_name, &params, last_was_multiline);
                }
            }
            for reg in &service.registrations {
                for variant in &reg.variants {
                    let fn_name = format!("{service_snake}_{}", variant.name);
                    if emitted_nif_stubs.insert(fn_name.clone()) {
                        let mut params = vec!["_registrations".to_string()];
                        for p in &variant.signature_params {
                            params.push(format!("_{}", elixir_safe_param_name(&p.name)));
                        }
                        params.push("_handler".to_string());
                        if !out.is_empty() && !out.ends_with("\n\n") {
                            out.push('\n');
                        }
                        out.push_str("  @doc false\n");
                        last_was_multiline = write_nif_stub(&mut out, &fn_name, &params, last_was_multiline);
                    }
                }
            }
        }
    }

    // Stubs for whitelisted error-introspection NIF shims (e.g. `<errname>_status_code`,
    // `<errname>_is_transient`, `<errname>_error_type`). These mirror the Rust NIFs
    // emitted by `generate_bindings` so rustler-precompiled's on_load can resolve every
    // declared NIF — without these stubs, BEAM aborts loading with `:bad_lib`.
    for error in &api.errors {
        for method in error.methods.iter().filter(|m| !m.sanitized) {
            let nif_fn_name = format!("{}_{}", error.name.to_lowercase(), method.name);
            let params = vec!["_msg".to_string()];
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str("  @doc false\n");
            let _ = write_nif_stub(&mut out, &nif_fn_name, &params, false);
        }
    }

    out.push_str(&template_env::render(
        "native_module_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Write an Elixir `@doc` attribute at the given two-space indent above a NIF stub.
///
/// - Empty `doc` → emits nothing (the next stub stays undocumented; this matches the
///   alef policy of omitting `@doc` rather than emitting `@doc false` for stubs without
///   propagated rustdoc — ExDoc will fall back to the `@moduledoc false` parent module).
/// - Single-line `doc` (no embedded newline) → `  @doc "text"` form, with embedded
///   double-quotes and backslashes escaped.
/// - Multi-line `doc` → `  @doc """` heredoc with each line indented by two spaces; any
///   `"""` sequence inside the body is broken up to avoid closing the heredoc early.
///
/// Mix-format compliance: an `@doc` attribute must attach directly to the next `def`
/// (no blank line between them) but the whole `@doc`/`def` block needs to be separated
/// from the previous stub by a blank line. The helper inspects the existing output to
/// add a leading blank line only when one isn't already present.
///
/// Returns `true` when a doc was emitted (so the caller can force the following stub
/// to be treated as "previous was multiline" for spacing purposes).
fn write_nif_doc(out: &mut String, doc: &str, _prev_was_multiline: bool) -> bool {
    if doc.is_empty() {
        return false;
    }
    // Ensure a blank line separates this @doc/def block from preceding content. If the
    // previous stub was multi-line the template already pushed a trailing blank line
    // (output ends with "\n\n"); otherwise we add one here.
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    if !doc.contains('\n') {
        // Single-line form: @doc "..." — escape backslashes then quotes.
        let escaped = doc.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str("  @doc \"");
        out.push_str(&escaped);
        out.push_str("\"\n");
    } else {
        // Multi-line heredoc form. Break up any embedded `"""` sequences so they don't
        // close the heredoc early (mirrors `emit_elixir_doc` in alef-codegen).
        out.push_str("  @doc \"\"\"\n");
        for line in doc.lines() {
            let safe = line.replace("\"\"\"", "\"\" \"");
            if safe.is_empty() {
                out.push('\n');
            } else {
                out.push_str("  ");
                out.push_str(&safe);
                out.push('\n');
            }
        }
        out.push_str("  \"\"\"\n");
    }
    true
}

/// Write a NIF stub line, splitting onto two lines when the single-line form exceeds 120 chars.
///
/// `prev_was_multiline` should be `true` when the previous stub was multi-line. This is used
/// to insert a single blank separator line around multi-line defs (mix format requirement):
/// - single → multi: blank before multi
/// - multi → single: blank before single
/// - multi → multi: single blank between them (not double)
/// - single → single: no blank
///
/// Returns `true` when this stub was written in multi-line form.
///
/// Single-line form:  `  def fn_name(args), do: :erlang.nif_error(:nif_not_loaded)`
/// Two-line form:
/// ```elixir
///   def fn_name(args),
///     do: :erlang.nif_error(:nif_not_loaded)
/// ```
fn write_nif_stub(out: &mut String, fn_name: &str, params: &[String], prev_was_multiline: bool) -> bool {
    let args = params.join(", ");
    // Elixir convention: omit parens on zero-arg defs
    let sig = if args.is_empty() {
        fn_name.to_string()
    } else {
        format!("{fn_name}({args})")
    };
    // "  def <sig>, do: :erlang.nif_error(:nif_not_loaded)"
    let single_line_len = 6 + sig.len() + 40;
    if single_line_len > 120 {
        let ctx = minijinja::context! { sig => sig, prev_was_multiline => prev_was_multiline };
        out.push_str(&template_env::render("nif_stub_multi_line.jinja", ctx));
        true
    } else {
        let ctx = minijinja::context! { sig => sig };
        out.push_str(&template_env::render("nif_stub_single_line.jinja", ctx));
        false
    }
}

pub(in crate::backends::rustler::gen_bindings) fn collect_types_for_nif_derives(
    api: &crate::core::ir::ApiSurface,
    exclude_types: &AHashSet<&str>,
) -> AHashSet<String> {
    let mut types = AHashSet::new();

    // Seed with types from function signatures (params and returns)
    for func in &api.functions {
        collect_named_types_from_ref(&func.return_type, &mut types);
        for param in &func.params {
            collect_named_types_from_ref(&param.ty, &mut types);
        }
    }

    // Seed with types from method signatures
    for typ in api.types.iter().filter(|t| !t.is_trait) {
        // Seed the owning type itself when it has receiver methods: gen_nif_method emits
        // `obj: TypeName` for every non-opaque type with a receiver, so the binding struct
        // must be defined even when the type does not appear in any function signature.
        if !typ.is_opaque && typ.methods.iter().any(|m| m.receiver.is_some()) {
            types.insert(typ.name.clone());
        }
        for method in &typ.methods {
            collect_named_types_from_ref(&method.return_type, &mut types);
            for param in &method.params {
                collect_named_types_from_ref(&param.ty, &mut types);
            }
        }
    }

    // Seed with types from enum variants
    for enum_def in &api.enums {
        for variant in &enum_def.variants {
            for field in &variant.fields {
                collect_named_types_from_ref(&field.ty, &mut types);
            }
        }
    }

    // Transitive closure: walk field types recursively
    let mut changed = true;
    while changed {
        changed = false;
        let snapshot: Vec<String> = types.iter().cloned().collect();
        for type_name in &snapshot {
            if let Some(typ) = api.types.iter().find(|t| t.name == *type_name) {
                for field in binding_fields(&typ.fields) {
                    if collect_named_types_from_ref(&field.ty, &mut types) {
                        changed = true;
                    }
                }
            }
        }
    }

    // Remove excluded and opaque types
    types.retain(|name| {
        !exclude_types.contains(name.as_str()) && !api.types.iter().any(|t| t.name == *name && t.is_opaque)
    });
    types
}

/// Helper: collect named types from a TypeRef. Returns true if any new types were added.
fn collect_named_types_from_ref(ty: &TypeRef, out: &mut AHashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => out.insert(name.clone()),
        TypeRef::Optional(inner) => collect_named_types_from_ref(inner, out),
        TypeRef::Vec(inner) => collect_named_types_from_ref(inner, out),
        TypeRef::Map(k, v) => {
            let k_added = collect_named_types_from_ref(k, out);
            let v_added = collect_named_types_from_ref(v, out);
            k_added || v_added
        }
        _ => false,
    }
}
