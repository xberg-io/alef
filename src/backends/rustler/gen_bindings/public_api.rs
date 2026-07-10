use crate::backends::rustler::gen_bindings::helpers::{
    elixir_return_typespec, elixir_safe_param_name, elixir_typespec,
};
use crate::backends::rustler::gen_bindings::public_api_args::{
    emit_tagged_enum_encoder, json_encode_param_indices, keyword_nif_arg, nif_arg, tagged_enum_param_map,
};
use crate::backends::rustler::gen_bindings::public_api_delegates::append_trait_bridge_delegates;
use crate::backends::rustler::gen_bindings::public_api_patches::patch_native_stub_module;
use crate::backends::rustler::gen_bindings::public_files::{self, PublicFileContext};
use crate::backends::rustler::template_env;
use crate::core::backend::GeneratedFile;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use ahash::{AHashMap, AHashSet};
use heck::{ToPascalCase, ToSnakeCase};
use std::path::PathBuf;

pub(super) fn generate_public_api(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let deduped_api = api.with_deduped_functions();
    let api = &deduped_api;

    let app_name = config.elixir_app_name();
    let app_module = app_name.to_pascal_case();
    let native_mod = format!("{app_module}.Native");
    let crate_name = config.name.replace('-', "_");

    let elixir_config = config.elixir.as_ref();
    let exclude_functions: AHashSet<String> = elixir_config
        .map(|c| c.exclude_functions.iter().cloned().collect())
        .unwrap_or_default();

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

    let default_types: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.is_opaque)
        .map(|t| t.name.clone())
        .collect();

    // Index serde-tagged enums (`#[serde(tag = "...")]`) by name. The wrapper layer
    let enum_lookup: AHashMap<String, &crate::core::ir::EnumDef> =
        api.enums.iter().map(|e| (e.name.clone(), e)).collect();

    let mut tagged_enums_used: AHashSet<String> = AHashSet::new();

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

    let mut content = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    content.push_str(&template_env::render(
        "elixir_module_header.jinja",
        minijinja::context! {
            app_module => &app_module,
            moduledoc => &format!("High-level API for {app_name}"),
        },
    ));

    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let public_fn_name = func.name.to_snake_case();
        let nif_fn_name = if func.is_async {
            let s = public_fn_name.clone();
            if s.ends_with("_async") { s } else { format!("{s}_async") }
        } else {
            public_fn_name.clone()
        };
        let doc_line_raw = if func.doc.is_empty() {
            "Function".to_string()
        } else {
            crate::codegen::doc_emission::doc_first_paragraph_joined(&func.doc)
        };
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

        let trailing_optional_count = func
            .params
            .iter()
            .rev()
            .zip(param_types.iter().rev())
            .take_while(|(p, type_str)| p.optional || type_str.contains("| nil"))
            .count();

        let json_encode_params = json_encode_param_indices(&func.params, &opaque_types, &default_types);
        let tagged_enum_params = tagged_enum_param_map(&func.params, &enum_lookup);
        tagged_enums_used.extend(tagged_enum_params.values().map(|param| param.enum_name.clone()));

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

        let visitor_bridge_idx =
            visitor_bridge_param_idx.or_else(|| options_field_bridge.as_ref().map(|(idx, _)| *idx));
        let trailing_keyword_count = if visitor_bridge_idx.is_some() {
            0
        } else {
            trailing_optional_count
        };
        let use_keyword_opts = trailing_keyword_count >= 2;

        let arity_variants: Vec<usize> = if !use_keyword_opts && trailing_optional_count > 0 {
            ((all_params.len() - trailing_optional_count)..=all_params.len()).collect()
        } else if use_keyword_opts {
            vec![]
        } else {
            vec![all_params.len()]
        };

        if use_keyword_opts {
            let required_count = all_params.len() - trailing_keyword_count;
            let required_params = &all_params[..required_count];
            let required_types = &param_types[..required_count];
            let optional_ir_params = &func.params[required_count..];

            if !content.is_empty() && !content.ends_with("\n\n") {
                content.push('\n');
            }
            content.push_str(&template_env::render(
                "elixir_doc_line.jinja",
                minijinja::context! { doc_line => doc_line },
            ));

            let mut spec_types: Vec<String> = required_types.to_vec();
            spec_types.push("keyword()".to_string());
            let spec_inline = format!("  @spec {public_fn_name}({}) :: {return_spec}", spec_types.join(", "));
            if spec_inline.len() > 98 {
                let spec_broken = format!(
                    "  @spec {public_fn_name}({}) ::\n          {return_spec}",
                    spec_types.join(", ")
                );
                if spec_broken.lines().all(|l| l.len() <= 98) {
                    content.push_str(&spec_broken);
                    content.push('\n');
                } else {
                    content.push_str(&template_env::render(
                        "elixir_spec_multiline.jinja",
                        minijinja::context! {
                            func_name => &public_fn_name,
                            param_types => &spec_types,
                            return_spec => &return_spec,
                        },
                    ));
                }
            } else {
                content.push_str(&spec_inline);
                content.push('\n');
            }

            let mut def_parts: Vec<String> = required_params.to_vec();
            def_parts.push("opts \\\\ []".to_string());
            let def_params = def_parts.join(", ");

            let mut nif_call_parts: Vec<String> = required_params
                .iter()
                .enumerate()
                .map(|(idx, req_param)| nif_arg(idx, req_param, &json_encode_params, &tagged_enum_params))
                .collect();
            nif_call_parts.extend(optional_ir_params.iter().enumerate().map(|(param_offset, opt_p)| {
                let opt_idx = required_count + param_offset;
                let safe_name = elixir_safe_param_name(&opt_p.name);
                keyword_nif_arg(opt_idx, &safe_name, &json_encode_params, &tagged_enum_params)
            }));
            let nif_call_str = nif_call_parts.join(",\n      ");
            content.push_str(&template_env::render(
                "elixir_keyword_opts_wrapper.ex.jinja",
                minijinja::context! {
                    public_func_name => &public_fn_name,
                    nif_func_name => &nif_fn_name,
                    params => &def_params,
                    native_mod => &native_mod,
                    nif_call_args => &nif_call_str,
                },
            ));
        } else if arity_variants.is_empty() && trailing_optional_count == 0 && !all_params.is_empty() {
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

            if !content.is_empty() && !content.ends_with("\n\n") {
                content.push('\n');
            }
            content.push_str(&template_env::render(
                "elixir_doc_line.jinja",
                minijinja::context! { doc_line => doc_line },
            ));
            let spec_inline = format!("  @spec {public_fn_name}({}) :: {return_spec}", param_types.join(", "));
            if spec_inline.len() > 98 {
                let spec_broken = format!(
                    "  @spec {public_fn_name}({}) ::\n          {return_spec}",
                    param_types.join(", ")
                );
                if spec_broken.lines().all(|l| l.len() <= 98) {
                    content.push_str(&spec_broken);
                    content.push('\n');
                } else {
                    content.push_str(&template_env::render(
                        "elixir_spec_multiline.jinja",
                        minijinja::context! {
                            func_name => &public_fn_name,
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
                    func_name => &public_fn_name,
                    params => &param_with_defaults.join(", "),
                },
            ));
            let single_arity_nif_args: Vec<String> = all_params
                .iter()
                .enumerate()
                .map(|(i, p)| nif_arg(i, p, &json_encode_params, &tagged_enum_params))
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

            let required_count = all_params.len() - trailing_optional_count;
            let single_clause = arity_variants.len() == 1;
            let arity_params: Vec<String> = arity_params_slice
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let has_nil_option = param_types.get(i).map(|t| t.contains("| nil")).unwrap_or(false);
                    if single_clause && ((i >= required_count && i < *arity) || has_nil_option) {
                        format!("{p} \\\\ nil")
                    } else {
                        p.clone()
                    }
                })
                .collect();

            if !content.is_empty() && !content.ends_with("\n\n") {
                content.push('\n');
            }
            content.push_str(&template_env::render(
                "elixir_doc_line.jinja",
                minijinja::context! {
                    doc_line => doc_line,
                },
            ));
            let spec_inline = format!("  @spec {public_fn_name}({}) :: {return_spec}", arity_types.join(", "));
            if spec_inline.len() > 98 {
                let spec_broken = format!(
                    "  @spec {public_fn_name}({}) ::\n          {return_spec}",
                    arity_types.join(", ")
                );
                if spec_broken.lines().all(|l| l.len() <= 98) {
                    content.push_str(&spec_broken);
                    content.push('\n');
                } else {
                    content.push_str(&template_env::render(
                        "elixir_spec_multiline.jinja",
                        minijinja::context! {
                            func_name => &public_fn_name,
                            param_types => &arity_types,
                            return_spec => &return_spec,
                        },
                    ));
                }
            } else {
                content.push_str(&spec_inline);
                content.push('\n');
            }

            let nif_call_args: Vec<String> = all_params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i < *arity {
                        nif_arg(i, p, &json_encode_params, &tagged_enum_params)
                    } else {
                        "nil".to_string()
                    }
                })
                .collect();

            if let Some((opts_idx, ref field_name)) = options_field_bridge {
                if *arity > opts_idx {
                    let opts_param = &all_params[opts_idx];
                    content.push_str(&template_env::render(
                        "elixir_def_with_guard.jinja",
                        minijinja::context! {
                            func_name => &public_fn_name,
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
                    content.push('\n');
                    content.push_str("    if is_map(visitor) do\n");
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
                    let single_line = format!(
                        "      {{:ok, _}} = {native_mod}.{nif_fn_name}_with_visitor({with_visitor_args_str})\n"
                    );
                    if single_line.len() > 98 {
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
                    content.push('\n');
                    content.push_str(&template_env::render(
                        "elixir_visitor_receive.jinja",
                        minijinja::context! {
                            visitor_param => "visitor",
                        },
                    ));
                    content.push_str("    else\n");
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
                            func_name => &public_fn_name,
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

            if let Some(vis_idx) = visitor_bridge_param_idx {
                if *arity > vis_idx {
                    let vis_param = &all_params[vis_idx];
                    content.push_str(&template_env::render(
                        "elixir_def_with_guard.jinja",
                        minijinja::context! {
                            func_name => &public_fn_name,
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
                    content.push_str(&template_env::render(
                        "elixir_doc_line.jinja",
                        minijinja::context! {
                            doc_line => &doc_line,
                        },
                    ));
                    let spec_inline = format!("  @spec {public_fn_name}({}) :: {return_spec}", arity_types.join(", "));
                    if spec_inline.len() > 98 {
                        let spec_broken = format!(
                            "  @spec {public_fn_name}({}) ::\n          {return_spec}",
                            arity_types.join(", ")
                        );
                        if spec_broken.lines().all(|l| l.len() <= 98) {
                            content.push_str(&spec_broken);
                        } else {
                            content.push_str(&template_env::render(
                                "elixir_spec_multiline.jinja",
                                minijinja::context! {
                                    func_name => &public_fn_name,
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
                            func_name => &public_fn_name,
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
                        func_name => &public_fn_name,
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
                        func_name => &public_fn_name,
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

    crate::backends::rustler::gen_bindings::public_api_delegates::append_visitor_receive_loop(
        &mut content,
        api,
        config,
        &native_mod,
    );

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

    let streaming_adapters: Vec<_> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .collect();

    for adapter in streaming_adapters {
        let Some(owner) = adapter.owner_type.as_deref() else {
            continue;
        };
        let owner_lc = owner.to_lowercase();
        let start_fn = format!("{owner_lc}_{}_start", adapter.name);
        let next_fn = format!("{owner_lc}_{}_next", adapter.name);
        let stream_fn = adapter.name.to_snake_case();

        let mut start_param_names: Vec<String> = vec!["client".to_string()];
        for p in &adapter.params {
            start_param_names.push(elixir_safe_param_name(&p.name));
        }
        let start_call_args = start_param_names.join(", ");

        content.push_str(&template_env::render(
            "elixir_streaming_start_wrapper.jinja",
            minijinja::context! {
                core_path => &adapter.core_path,
                start_fn => &start_fn,
                start_call_args => &start_call_args,
                native_mod => &native_mod,
            },
        ));
        content.push('\n');

        content.push_str(&template_env::render(
            "elixir_streaming_next_wrapper.jinja",
            minijinja::context! {
                next_fn => &next_fn,
                native_mod => &native_mod,
            },
        ));
        content.push('\n');

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
        content.push('\n');
    }

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
            content.push('\n');
        }
    }

    let api_fn_names: AHashSet<String> = api.functions.iter().map(|f| f.name.clone()).collect();
    append_trait_bridge_delegates(
        &mut content,
        config,
        &crate::backends::rustler::gen_bindings::public_api_delegates::TraitDelegateCtx {
            api,
            app_module: &app_module,
            opaque_types: &opaque_types,
            default_types: &default_types,
            api_fn_names: &api_fn_names,
            native_mod: &native_mod,
        },
    );

    if !tagged_enums_used.is_empty() {
        let mut sorted: Vec<&String> = tagged_enums_used.iter().collect();
        sorted.sort();
        for enum_name in sorted {
            if let Some(enum_def) = enum_lookup.get(enum_name) {
                if !content.ends_with("\n\n") {
                    content.push('\n');
                }
                content.push_str(&emit_tagged_enum_encoder(enum_def));
            }
        }
    }

    let trimmed = content.trim_end_matches('\n');
    content = format!("{trimmed}\nend\n");

    public_files::append_stream_error_exception(&mut content, config, &app_module);

    content = content.replace(
        "do nil -> nil; v -> Jason.encode!(v) end",
        "do nil -> nil; v when is_binary(v) -> v; v -> Jason.encode!(v) end",
    );

    files.push(GeneratedFile {
        path: PathBuf::from(&output_dir).join(format!("{}.ex", app_name.to_snake_case())),
        content,
        generated_header: false,
    });

    patch_native_stub_module(&mut files, config);

    Ok(files)
}
