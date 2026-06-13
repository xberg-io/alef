use crate::backends::rustler::gen_bindings::helpers::{
    elixir_return_typespec, elixir_safe_param_name, elixir_typespec,
};
use crate::backends::rustler::gen_bindings::public_api_args::{json_encode_param_indices, keyword_nif_arg, nif_arg};
use crate::backends::rustler::gen_bindings::public_api_delegates::append_trait_bridge_delegates;
use crate::backends::rustler::gen_bindings::public_files::{self, PublicFileContext};
use crate::backends::rustler::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

pub(super) fn generate_public_api(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let app_name = config.elixir_app_name();
    let app_module = app_name.to_pascal_case();
    let native_mod = format!("{app_module}.Native");
    let crate_name = config.name.replace('-', "_");

    let elixir_config = config.elixir.as_ref();
    let exclude_functions: AHashSet<String> = elixir_config
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

    // Skip binding-excluded types (service owners / handler-contract traits) — they are
    // emitted/exported by the service-API codegen, not the generic public-API listing.
    let binding_excluded_names: Vec<String> = api
        .types
        .iter()
        .filter(|t| t.binding_excluded)
        .map(|t| t.name.clone())
        .collect();
    let mut exclude_types: AHashSet<&str> = elixir_config
        .map(|c| c.exclude_types.iter().map(String::as_str).collect())
        .unwrap_or_default();
    exclude_types.extend(binding_excluded_names.iter().map(String::as_str));

    let opaque_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Types whose NIF params are JSON strings (has_default = true, non-opaque).
    let default_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    let (output_dir, mut files) = public_files::generated_module_files(
        api,
        config,
        PublicFileContext {
            app_name: &app_name,
            app_module: &app_module,
            crate_name: &crate_name,
            exclude_functions: &exclude_functions,
            exclude_types: &exclude_types,
            opaque_types: &opaque_types,
        },
    );

    // ── 4. Main wrapper module ────────────────────────────────────────────
    let mut content = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    content.push_str(&template_env::render(
        "elixir_module_header.jinja",
        minijinja::context! {
            app_module => &app_module,
            moduledoc => &format!("High-level API for {app_name}"),
        },
    ));

    // Wrapper functions for top-level API functions
    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let nif_fn_name = if func.is_async {
            let s = func.name.to_snake_case();
            if s.ends_with("_async") { s } else { format!("{s}_async") }
        } else {
            func.name.to_snake_case()
        };
        let doc_line_raw = if func.doc.is_empty() {
            "Function".to_string()
        } else {
            crate::codegen::doc_emission::doc_first_paragraph_joined(&func.doc)
        };
        // Elixir @doc strings use double-quote delimiters; escape any embedded quotes.
        let doc_line = doc_line_raw.replace('"', "\\\"");
        let doc_line = doc_line.as_str();

        let param_types: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                let base = elixir_typespec(&p.ty, &opaque_types, &default_types);
                if p.optional && !base.ends_with("| nil") {
                    format!("{base} | nil")
                } else {
                    base
                }
            })
            .collect();
        let return_spec = elixir_return_typespec(
            &func.return_type,
            func.error_type.is_some(),
            &opaque_types,
            &default_types,
        );
        let all_params: Vec<String> = func.params.iter().map(|p| elixir_safe_param_name(&p.name)).collect();

        // Count how many trailing parameters are optional (either p.optional=true or typespec has "| nil").
        // This ensures we catch Option<T> params that may have .optional=false but emit "| nil" typespecs.
        let trailing_optional_count = func
            .params
            .iter()
            .rev()
            .zip(param_types.iter().rev())
            .take_while(|(p, type_str)| p.optional || type_str.contains("| nil"))
            .count();

        // Mirror the NIF-side detection: every Vec<Named> over a non-opaque struct
        // crosses the boundary as Option<String> JSON, regardless of whether the
        // inner type has a Default impl.
        let json_encode_params = json_encode_param_indices(&func.params, &opaque_types);

        // Detect if this function has a visitor bridge param.
        let visitor_bridge_param_idx: Option<usize> = func.params.iter().position(|p| {
            config.trait_bridges.iter().any(|b| {
                b.param_name.as_deref() == Some(p.name.as_str()) || {
                    let named = match &p.ty {
                        crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                        crate::core::ir::TypeRef::Optional(inner) => {
                            if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
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

        // Detect options_field visitor bridge: visitor is embedded in the options struct.
        // Returns (options_param_idx, field_name) when matched.
        let options_field_bridge: Option<(usize, String)> = func.params.iter().enumerate().find_map(|(idx, p)| {
            let type_name = match &p.ty {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                crate::core::ir::TypeRef::Optional(inner) => {
                    if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            config.trait_bridges.iter().find_map(|b| {
                if b.bind_via == BridgeBinding::OptionsField
                    && type_name.is_some_and(|n| b.options_type.as_deref() == Some(n))
                {
                    let field = b.resolved_options_field().unwrap_or("visitor").to_string();
                    Some((idx, field))
                } else {
                    None
                }
            })
        });

        // Determine whether trailing optional params should be collapsed into a single
        // `opts \\ []` keyword argument (Elixir idiom) rather than N arity overloads.
        // Visitor-bridge params keep their positional form (handled below).
        let visitor_bridge_idx =
            visitor_bridge_param_idx.or_else(|| options_field_bridge.as_ref().map(|(idx, _)| *idx));
        let trailing_keyword_count = if visitor_bridge_idx.is_some() {
            // Visitor bridge present — no keyword collapsing for safety.
            0
        } else {
            trailing_optional_count
        };
        // Use keyword-opts collapsing (`opts \\ []`) for multiple trailing optionals only.
        // Single trailing optional params (e.g., `config: Option<T>`) stay positional with `\\ nil`
        // so e2e codegen can pass them as positional arguments. This preserves the common
        // config-parameter pattern where a single JSON string or nil is passed directly.
        let use_keyword_opts = trailing_keyword_count >= 2;

        // Emit one @spec/@doc per arity variant (shortest to longest).
        // The shortest arity fills optional params with nil.
        let arity_variants: Vec<usize> = if !use_keyword_opts && trailing_optional_count > 0 {
            ((all_params.len() - trailing_optional_count)..=all_params.len()).collect()
        } else if use_keyword_opts {
            // Keyword-opts path: single arity (required params + opts).
            vec![]
        } else {
            vec![all_params.len()]
        };

        // Keyword-opts path: emit a single `def f(required, opts \\ []) do` with
        // `Keyword.get(opts, :param)` for each trailing optional param.
        if use_keyword_opts {
            let required_count = all_params.len() - trailing_keyword_count;
            let required_params = &all_params[..required_count];
            let required_types = &param_types[..required_count];
            let optional_ir_params = &func.params[required_count..];

            // Ensure blank line before @doc (mix format requirement between defs)
            if !content.is_empty() && !content.ends_with("\n\n") {
                content.push('\n');
            }
            content.push_str(&template_env::render(
                "elixir_doc_line.jinja",
                minijinja::context! { doc_line => doc_line },
            ));

            // @spec: required types + keyword()
            let mut spec_types: Vec<String> = required_types.to_vec();
            spec_types.push("keyword()".to_string());
            let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", spec_types.join(", "));
            if spec_inline.len() > 98 {
                let spec_broken = format!(
                    "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                    spec_types.join(", ")
                );
                if spec_broken.lines().all(|l| l.len() <= 98) {
                    content.push_str(&spec_broken);
                    content.push('\n');
                } else {
                    content.push_str(&template_env::render(
                        "elixir_spec_multiline.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            param_types => &spec_types,
                            return_spec => &return_spec,
                        },
                    ));
                }
            } else {
                content.push_str(&spec_inline);
                content.push('\n');
            }

            // def fn_name(req_param, opts \\ []) do
            let mut def_parts: Vec<String> = required_params.to_vec();
            def_parts.push("opts \\\\ []".to_string());
            let def_params = def_parts.join(", ");

            // NIF call args: required positionally, optional via Keyword.get.
            let mut nif_call_parts: Vec<String> = required_params
                .iter()
                .enumerate()
                .map(|(idx, req_param)| nif_arg(idx, req_param, &json_encode_params))
                .collect();
            nif_call_parts.extend(optional_ir_params.iter().enumerate().map(|(param_offset, opt_p)| {
                let opt_idx = required_count + param_offset;
                let safe_name = elixir_safe_param_name(&opt_p.name);
                keyword_nif_arg(opt_idx, &safe_name, &json_encode_params)
            }));
            let nif_call_str = nif_call_parts.join(",\n      ");
            content.push_str(&template_env::render(
                "elixir_keyword_opts_wrapper.ex.jinja",
                minijinja::context! {
                    func_name => &nif_fn_name,
                    params => &def_params,
                    native_mod => &native_mod,
                    nif_call_args => &nif_call_str,
                },
            ));
        } else if arity_variants.is_empty() && trailing_optional_count == 0 && !all_params.is_empty() {
            // Single-arity, no keyword opts, no optional trailing params, but may have
            // optional (| nil) params in the typespec. Emit the def with defaults for
            // all params that have "| nil" in their typespec.
            let param_with_defaults: Vec<String> = param_types
                .iter()
                .zip(&all_params)
                .map(|(type_str, param_name)| {
                    if type_str.contains("| nil") {
                        format!("{param_name} \\\\ nil")
                    } else {
                        param_name.clone()
                    }
                })
                .collect();

            // Ensure blank line before @doc (mix format requirement between defs)
            if !content.is_empty() && !content.ends_with("\n\n") {
                content.push('\n');
            }
            content.push_str(&template_env::render(
                "elixir_doc_line.jinja",
                minijinja::context! { doc_line => doc_line },
            ));
            let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", param_types.join(", "));
            if spec_inline.len() > 98 {
                let spec_broken = format!(
                    "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                    param_types.join(", ")
                );
                if spec_broken.lines().all(|l| l.len() <= 98) {
                    content.push_str(&spec_broken);
                    content.push('\n');
                } else {
                    content.push_str(&template_env::render(
                        "elixir_spec_multiline.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            param_types => &param_types,
                            return_spec => &return_spec,
                        },
                    ));
                }
            } else {
                content.push_str(&spec_inline);
                content.push('\n');
            }

            content.push_str(&template_env::render(
                "elixir_def_simple.jinja",
                minijinja::context! {
                    func_name => &nif_fn_name,
                    params => &param_with_defaults.join(", "),
                },
            ));
            // JSON-encode any batch parameters in the single-arity non-visitor path.
            let single_arity_nif_args: Vec<String> = all_params
                .iter()
                .enumerate()
                .map(|(i, p)| nif_arg(i, p, &json_encode_params))
                .collect();
            content.push_str(&template_env::render(
                "elixir_def_nif_call.jinja",
                minijinja::context! {
                    native_mod => &native_mod,
                    func_name => &nif_fn_name,
                    args => &single_arity_nif_args.join(", "),
                },
            ));
            content.push_str("  end\n\n");
        }

        for arity in &arity_variants {
            let arity_params_slice = &all_params[..*arity];
            let arity_types = &param_types[..*arity];

            // For arity variants with positional defaults, append `\\ nil` to params
            // that have "| nil" in their typespec OR are trailing optional.
            // This allows fixtures to call functions with any intermediate arity.
            //
            // Defaults are only safe when this function emits a SINGLE clause. When
            // multiple arity variants are emitted, each shorter arity is already an
            // explicit clause; a `\\ nil` default on a longer clause would generate
            // an implicit lower-arity head that collides with it, producing a
            // "this clause cannot match" warning (fatal under --warnings-as-errors).
            let required_count = all_params.len() - trailing_optional_count;
            let single_clause = arity_variants.len() == 1;
            let arity_params: Vec<String> = arity_params_slice
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let has_nil_option = param_types.get(i).map(|t| t.contains("| nil")).unwrap_or(false);
                    if single_clause && ((i >= required_count && i < *arity) || has_nil_option) {
                        // Trailing optional param or param with | nil typespec: add default
                        format!("{p} \\\\ nil")
                    } else {
                        p.clone()
                    }
                })
                .collect();

            // Ensure blank line before @doc (mix format requirement between defs)
            if !content.is_empty() && !content.ends_with("\n\n") {
                content.push('\n');
            }
            content.push_str(&template_env::render(
                "elixir_doc_line.jinja",
                minijinja::context! {
                    doc_line => doc_line,
                },
            ));
            let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", arity_types.join(", "));
            if spec_inline.len() > 98 {
                let spec_broken = format!(
                    "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                    arity_types.join(", ")
                );
                if spec_broken.lines().all(|l| l.len() <= 98) {
                    content.push_str(&spec_broken);
                    content.push('\n');
                } else {
                    content.push_str(&template_env::render(
                        "elixir_spec_multiline.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            param_types => &arity_types,
                            return_spec => &return_spec,
                        },
                    ));
                }
            } else {
                content.push_str(&spec_inline);
                content.push('\n');
            }

            // Build the call: fill missing optional params with nil.
            // JSON-encode parameters that are Vec<Named> with has_default=true.
            let nif_call_args: Vec<String> = all_params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i < *arity {
                        nif_arg(i, p, &json_encode_params)
                    } else {
                        "nil".to_string()
                    }
                })
                .collect();

            // options_field bridge: visitor is embedded in the options map.
            // Extract `:visitor` from options before calling the NIF.
            if let Some((opts_idx, ref field_name)) = options_field_bridge {
                if *arity > opts_idx {
                    let opts_param = &all_params[opts_idx];
                    // Single clause handles both visitor and no-visitor by inspecting the map.
                    content.push_str(&template_env::render(
                        "elixir_def_with_guard.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            params => &arity_params.join(", "),
                            guard_param => opts_param,
                        },
                    ));
                    content.push_str(&template_env::render(
                        "elixir_map_pop_unpack.jinja",
                        minijinja::context! {
                            opts_param => opts_param,
                            field_name => field_name,
                        },
                    ));
                    // mix format: blank line after Map.pop before if block.
                    content.push('\n');
                    content.push_str("    if is_map(visitor) do\n");
                    // Build NIF args: replace opts param with JSON-encoded clean opts, then append visitor.
                    // The _with_visitor NIF has arity = base NIF arity + 1; the trailing arg is the popped visitor map.
                    let mut with_visitor_args: Vec<String> = nif_call_args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            if i == opts_idx {
                                "if(map_size(clean_opts) == 0, do: nil, else: Jason.encode!(clean_opts))".to_string()
                            } else {
                                a.clone()
                            }
                        })
                        .collect();
                    with_visitor_args.push("visitor".to_string());
                    let with_visitor_args_str = with_visitor_args.join(", ");
                    // Emit visitor NIF call. Check line length to decide between single-line
                    // and multi-line format (mix format wraps at 98 chars).
                    let single_line = format!(
                        "      {{:ok, _}} = {native_mod}.{nif_fn_name}_with_visitor({with_visitor_args_str})\n"
                    );
                    if single_line.len() > 98 {
                        // Multi-line format that mix format produces for long calls:
                        // every positional arg on its own line. Splitting on the first
                        // ", " only would leave the 2nd+ args concatenated on one line
                        // which mix format would then rewrap on every check, breaking
                        // prek's mix-format hook.
                        content.push_str(&template_env::render(
                            "elixir_visitor_call_multiline.ex.jinja",
                            minijinja::context! {
                                native_mod => &native_mod,
                                func_name => &nif_fn_name,
                                args => &with_visitor_args,
                            },
                        ));
                    } else {
                        content.push_str(&single_line);
                    }
                    content.push('\n'); // mix format: blank line before do_visitor_receive_loop.
                    content.push_str(&template_env::render(
                        "elixir_visitor_receive.jinja",
                        minijinja::context! {
                            visitor_param => "visitor",
                        },
                    ));
                    content.push_str("    else\n");
                    // No visitor: call regular NIF with options as JSON.
                    // mix format indents else body to 6 spaces (same as if body).
                    // Use clean_opts (visitor already popped) to avoid sending unknown fields to Rust.
                    let plain_args: Vec<String> = nif_call_args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            if i == opts_idx {
                                "if(map_size(clean_opts) == 0, do: nil, else: Jason.encode!(clean_opts))".to_string()
                            } else {
                                a.clone()
                            }
                        })
                        .collect();
                    let plain_args_str = plain_args.join(", ");
                    content.push_str(&template_env::render(
                        "elixir_visitor_plain_call.ex.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => &plain_args_str,
                        },
                    ));
                    content.push_str("    end\n");
                    content.push_str("  end\n\n");

                    // Nil clause: options is nil — pass nil directly to the NIF.
                    let nil_clause_params: Vec<String> = arity_params
                        .iter()
                        .enumerate()
                        .map(|(i, p)| if i == opts_idx { "nil".to_string() } else { p.clone() })
                        .collect();
                    let nil_nif_args: Vec<String> = nif_call_args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| if i == opts_idx { "nil".to_string() } else { a.clone() })
                        .collect();
                    content.push_str(&template_env::render(
                        "elixir_def_simple.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            params => &nil_clause_params.join(", "),
                        },
                    ));
                    content.push_str(&template_env::render(
                        "elixir_def_nif_call.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => &nil_nif_args.join(", "),
                        },
                    ));
                    content.push_str("  end\n\n");
                    continue;
                }
            }

            // function_param bridge: visitor is a direct positional parameter.
            // When a visitor is provided (non-nil at the bridge param index), delegate to
            // the async visitor variant which drives a receive loop.
            if let Some(vis_idx) = visitor_bridge_param_idx {
                if *arity > vis_idx {
                    // Full-arity def: visitor param is present in signature.
                    let vis_param = &all_params[vis_idx];
                    // Emit a two-clause definition: visitor map → receive loop, nil → direct.
                    content.push_str(&template_env::render(
                        "elixir_def_with_guard.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            params => &arity_params.join(", "),
                            guard_param => vis_param,
                        },
                    ));
                    let with_visitor_args = nif_call_args.join(", ");
                    content.push_str(&template_env::render(
                        "elixir_visitor_call.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => &with_visitor_args,
                        },
                    ));
                    content.push_str(&template_env::render(
                        "elixir_visitor_receive.jinja",
                        minijinja::context! {
                            visitor_param => vis_param,
                        },
                    ));
                    content.push_str("  end\n\n");
                    // Nil/no-visitor clause
                    content.push_str(&template_env::render(
                        "elixir_doc_line.jinja",
                        minijinja::context! {
                            doc_line => &doc_line,
                        },
                    ));
                    let spec_inline = format!("  @spec {nif_fn_name}({}) :: {return_spec}", arity_types.join(", "));
                    if spec_inline.len() > 98 {
                        let spec_broken = format!(
                            "  @spec {nif_fn_name}({}) ::\n          {return_spec}",
                            arity_types.join(", ")
                        );
                        if spec_broken.lines().all(|l| l.len() <= 98) {
                            content.push_str(&spec_broken);
                        } else {
                            content.push_str(&template_env::render(
                                "elixir_spec_multiline.jinja",
                                minijinja::context! {
                                    func_name => &nif_fn_name,
                                    param_types => &arity_types,
                                    return_spec => &return_spec,
                                },
                            ));
                        }
                    } else {
                        content.push_str(&spec_inline);
                    }
                    content.push('\n');
                    content.push_str(&template_env::render(
                        "elixir_def_simple.jinja",
                        minijinja::context! {
                            func_name => &nif_fn_name,
                            params => &arity_params.join(", "),
                        },
                    ));
                    content.push_str(&template_env::render(
                        "elixir_def_nif_call.jinja",
                        minijinja::context! {
                            native_mod => &native_mod,
                            func_name => &nif_fn_name,
                            args => &nif_call_args.join(", "),
                        },
                    ));
                    content.push_str("  end\n\n");
                    continue;
                }
            }

            if arity_params.is_empty() {
                content.push_str(&template_env::render(
                    "elixir_def_zero_arity.jinja",
                    minijinja::context! {
                        func_name => &nif_fn_name,
                    },
                ));
                content.push_str(&template_env::render(
                    "elixir_def_nif_call.jinja",
                    minijinja::context! {
                        native_mod => &native_mod,
                        func_name => &nif_fn_name,
                        args => &nif_call_args.join(", "),
                    },
                ));
            } else {
                content.push_str(&template_env::render(
                    "elixir_def_simple.jinja",
                    minijinja::context! {
                        func_name => &nif_fn_name,
                        params => &arity_params.join(", "),
                    },
                ));
                content.push_str(&template_env::render(
                    "elixir_def_nif_call.jinja",
                    minijinja::context! {
                        native_mod => &native_mod,
                        func_name => &nif_fn_name,
                        args => &nif_call_args.join(", "),
                    },
                ));
            }
            content.push_str("  end\n\n");
        }
    }

    // Emit the visitor receive loop helper if any function has a visitor bridge
    // (function_param or options_field mode).
    let has_visitor_bridges = api.functions.iter().any(|func| {
        func.params.iter().any(|p| {
            let named = match &p.ty {
                crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
                crate::core::ir::TypeRef::Optional(inner) => {
                    if let crate::core::ir::TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            config.trait_bridges.iter().any(|b| {
                // function_param: match by param_name or type_alias
                let is_function_param = b.param_name.as_deref() == Some(p.name.as_str())
                    || named.map(|n| b.type_alias.as_deref() == Some(n)).unwrap_or(false);
                // options_field: match when the param type is the configured options_type
                let is_options_field = b.bind_via == BridgeBinding::OptionsField
                    && named.is_some_and(|n| b.options_type.as_deref() == Some(n));
                is_function_param || is_options_field
            })
        })
    });

    if has_visitor_bridges {
        let visitor_result_metadata = config.trait_bridges.iter().find_map(|bridge_cfg| {
            match crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg) {
                Ok(metadata) => Some(metadata),
                Err(err) => {
                    eprintln!(
                        "[alef] gen_bindings(rustler): skip visitor helper metadata for trait bridge `{}`: {err}",
                        bridge_cfg.trait_name
                    );
                    None
                }
            }
        });
        if let Some(visitor_result_metadata) = visitor_result_metadata {
            let unit_result_variants = visitor_result_metadata
                .unit_variants
                .iter()
                .map(|variant| {
                    let atom_name = variant
                        .wire_name
                        .chars()
                        .all(|c| c == '_' || c.is_ascii_alphanumeric())
                        .then(|| variant.wire_name.clone());
                    minijinja::context! {
                        wire_name => variant.wire_name.clone(),
                        atom_name => atom_name,
                    }
                })
                .collect::<Vec<_>>();
            content.push_str(&template_env::render(
                "elixir_visitor_helper_functions.jinja",
                minijinja::context! {
                    native_mod => &native_mod,
                    default_result_wire_name => visitor_result_metadata.default_variant.wire_name,
                    unit_result_variants => unit_result_variants,
                },
            ));
        } else {
            eprintln!(
                "[alef] gen_bindings(rustler): skip visitor helper functions because no configured result enum metadata is available"
            );
        }
    }

    // Streaming-adapter method keys — these methods are emitted as start/next
    // Type methods are now emitted in their respective type modules
    // (gen_elixir_struct_module for structs, gen_elixir_opaque_module for opaque types).
    // This avoids emitting Rust-idiomatic wrappers with no Elixir equivalents.

    // Elixir public-API wrappers for whitelisted error introspection methods.
    // Each emits an `@spec` + `def` that delegates to the corresponding NIF shim.
    for error in &api.errors {
        for method in error.methods.iter().filter(|m| !m.sanitized) {
            let nif_fn_name = format!("{}_{}", error.name.to_lowercase(), method.name);
            let return_spec = elixir_return_typespec(&method.return_type, false, &opaque_types, &default_types);
            let doc_line = if method.doc.is_empty() {
                format!("Returns the `{}` value for the given error message.", method.name)
            } else {
                crate::codegen::doc_emission::doc_first_paragraph_joined(&method.doc).replace('"', "\\\"")
            };
            content.push_str(&template_env::render(
                "elixir_doc_line.jinja",
                minijinja::context! {
                    doc_line => &doc_line,
                },
            ));
            content.push_str(&template_env::render(
                "elixir_error_spec.ex.jinja",
                minijinja::context! {
                    func_name => &nif_fn_name,
                    return_spec => &return_spec,
                },
            ));
            content.push_str(&template_env::render(
                "elixir_def_simple.jinja",
                minijinja::context! {
                    func_name => &nif_fn_name,
                    params => "msg",
                },
            ));
            content.push_str(&template_env::render(
                "elixir_def_nif_call.jinja",
                minijinja::context! {
                    native_mod => &native_mod,
                    func_name => &nif_fn_name,
                    args => "msg",
                },
            ));
            content.push_str("  end\n\n");
        }
    }

    // Streaming-adapter wrappers: emit the underlying `_start` / `_next` defs
    // (delegating to NIFs) plus a high-level `{name}/2` (or `/3`) function
    // returning an Elixir `Stream` driven by `Stream.unfold/2`.
    let streaming_adapters: Vec<_> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .collect();

    // StreamError exception module is emitted AFTER the outer `defmodule
    // <AppModule>` closes (see post-trim block below). Emitting `defmodule
    // <AppModule>.StreamError` INSIDE `defmodule <AppModule>` produces a
    // doubly-namespaced `Elixir.<AppModule>.<AppModule>.StreamError` because
    // Elixir treats nested `defmodule <Outer>.<Suffix>` as relative — and
    // the rebind to the doubly-nested name also breaks every plain
    // `<AppModule>.Native.X` reference in the wrapper bodies, producing
    // `Elixir.<AppModule>.<AppModule>.Native.X is undefined` warnings.

    for adapter in streaming_adapters {
        let Some(owner) = adapter.owner_type.as_deref() else {
            continue;
        };
        let owner_lc = owner.to_lowercase();
        let start_fn = format!("{owner_lc}_{}_start", adapter.name);
        let next_fn = format!("{owner_lc}_{}_next", adapter.name);
        // The high-level Stream.unfold wrapper is the public streaming entry
        // point — it must be named after the adapter (`crawl_stream`), not the
        // owner-prefixed internal form (`crawlenginehandle_crawl_stream`), so
        // callers reach it as `Module.crawl_stream/2` like every other binding.
        let stream_fn = adapter.name.to_snake_case();

        // Build the wrapper-arg list: receiver + adapter params (binding type
        // gets JSON-encoded via Jason for the NIF boundary).
        let mut start_param_names: Vec<String> = vec!["client".to_string()];
        for p in &adapter.params {
            start_param_names.push(elixir_safe_param_name(&p.name));
        }
        let start_call_args = start_param_names.join(", ");

        // _start delegate
        content.push_str(&template_env::render(
            "elixir_streaming_start_wrapper.jinja",
            minijinja::context! {
                core_path => &adapter.core_path,
                start_fn => &start_fn,
                start_call_args => &start_call_args,
                native_mod => &native_mod,
            },
        ));
        // mix-format requires a blank line before each `@doc`. The template
        // source's trailing newlines get stripped by end-of-file-fixer, so
        // insert the separator explicitly here.
        content.push('\n');

        // _next delegate
        content.push_str(&template_env::render(
            "elixir_streaming_next_wrapper.jinja",
            minijinja::context! {
                next_fn => &next_fn,
                native_mod => &native_mod,
            },
        ));
        content.push('\n');

        // High-level Stream.unfold wrapper. The request map is passed directly
        // to the NIF (Rustler decodes via NifMap); the NIF returns chunk JSON
        // which is decoded back into a map by the wrapper.
        let req_param = adapter
            .params
            .first()
            .map(|p| elixir_safe_param_name(&p.name))
            .unwrap_or_else(|| "request".to_string());
        let exception_module = format!("{app_module}.StreamError");
        content.push_str(&template_env::render(
            "elixir_streaming_unfold_wrapper.jinja",
            minijinja::context! {
                core_path => &adapter.core_path,
                stream_fn => &stream_fn,
                req_param => &req_param,
                native_mod => &native_mod,
                start_fn => &start_fn,
                next_fn => &next_fn,
                exception_module => &exception_module,
            },
        ));
        // mix-format requires a blank line before each top-level def.
        // The next adapter iteration will emit `@doc` for the _start wrapper,
        // so insert the separator here.
        content.push('\n');
    }

    // Top-level flat wrappers for non-streaming methods on opaque types
    // (e.g. `defaultclient_chat_async/2`). The idiomatic Elixir API is exposed
    // via per-type submodules (`SampleLlm.DefaultClient.chat/2`), but consumers —
    // including the e2e fixture suite — also call the underlying NIFs through
    // flat top-level functions on the main module to mirror the streaming-wrapper
    // convention (`defaultclient_chat_stream/2`). These delegates are intentionally
    // thin: each `def` forwards directly to the corresponding `Native.*` NIF.
    let opaque_type_names: AHashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !exclude_types.contains(t.name.as_str()))
        .map(|t| t.name.as_str())
        .collect();
    let streaming_method_keys: AHashSet<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter_map(|a| a.owner_type.as_deref().map(|owner| format!("{owner}.{}", a.name)))
        .collect();
    for typ in api.types.iter().filter(|t| opaque_type_names.contains(t.name.as_str())) {
        let type_lc = typ.name.to_lowercase();
        for method in typ
            .methods
            .iter()
            .filter(|m| !exclude_functions.contains(m.name.as_str()))
            .filter(|m| !streaming_method_keys.contains(&format!("{}.{}", typ.name, m.name)))
        {
            let method_name = method.name.to_snake_case();
            let nif_fn = if method.is_async {
                if method.name.ends_with("_async") {
                    format!("{type_lc}_{method_name}")
                } else {
                    format!("{type_lc}_{method_name}_async")
                }
            } else {
                format!("{type_lc}_{method_name}")
            };

            let mut def_args: Vec<String> = Vec::new();
            if method.receiver.is_some() {
                def_args.push("obj".to_string());
            }
            for p in &method.params {
                def_args.push(elixir_safe_param_name(&p.name));
            }
            let args_str = def_args.join(", ");
            let doc_first = method.doc.lines().next().unwrap_or("").replace('"', "\\\"");
            content.push_str(&template_env::render(
                "elixir_top_level_opaque_method.ex.jinja",
                minijinja::context! {
                    doc_first => &doc_first,
                    func_name => &nif_fn,
                    args => &args_str,
                    native_mod => &native_mod,
                },
            ));
            // Template ends with newline; add blank line for mix format compatibility
            content.push('\n');
        }
    }

    // Trait bridge lifecycle functions are emitted by the native codegen but must
    // also be surfaced in the public module for e2e and user code.
    let api_fn_names: AHashSet<String> = api.functions.iter().map(|f| f.name.clone()).collect();
    append_trait_bridge_delegates(&mut content, config, &api_fn_names, &native_mod);

    // Trim trailing blank lines so `mix format` doesn't see an extra blank before `end`.
    let trimmed = content.trim_end_matches('\n');
    content = format!("{trimmed}\nend\n");

    public_files::append_stream_error_exception(&mut content, config, &app_module);

    files.push(GeneratedFile {
        path: PathBuf::from(&output_dir).join(format!("{}.ex", app_name.to_snake_case())),
        content,
        generated_header: false,
    });

    Ok(files)
}
