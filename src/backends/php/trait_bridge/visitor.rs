use minijinja::context;

use crate::codegen::generators::trait_bridge::{bridge_param_type as param_type, visitor_param_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

use super::interfaces::named_type_name;

/// Generate a visitor-style bridge wrapping a PHP `Zval` object reference.
pub(super) fn gen_visitor_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    type_paths: &HashMap<String, String>,
    api: &ApiSurface,
) -> String {
    let mut out = String::with_capacity(4096);
    let core_crate = trait_path
        .split("::")
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| panic!("trait_path '{trait_path}' must be a qualified path of the form 'crate_name::...'; configure extension_name in alef.toml"))
        .to_string();
    let Some(result_type) = bridge_cfg.result_type.as_deref() else {
        eprintln!(
            "[alef] gen_visitor(php): skip visitor bridge `{}` because result_type is not configured",
            bridge_cfg.trait_name
        );
        return String::new();
    };
    let result_type_path = type_paths
        .get(result_type)
        .cloned()
        .unwrap_or_else(|| format!("{core_crate}::{result_type}"));
    let Some(context_type) = bridge_cfg.context_type.as_deref() else {
        eprintln!(
            "[alef] gen_visitor(php): skip visitor bridge `{}` because context_type is not configured",
            bridge_cfg.trait_name
        );
        return String::new();
    };
    let context_type_path = type_paths
        .get(context_type)
        .cloned()
        .unwrap_or_else(|| format!("{core_crate}::{context_type}"));
    let Some(result_metadata) = crate::codegen::visitor_result::visitor_result_metadata(api, bridge_cfg) else {
        eprintln!(
            "[alef] gen_visitor(php): skip visitor bridge `{}` because result_type `{result_type}` is not in IR",
            bridge_cfg.trait_name
        );
        return String::new();
    };
    let default_variant = result_metadata.default_variant.name.as_str();

    out.push_str(&crate::backends::php::template_env::render(
        "visitor_nodecontext_helper.jinja",
        context! {
            context_type_path => &context_type_path,
        },
    ));
    out.push('\n');

    out.push_str(&crate::backends::php::template_env::render(
        "visitor_zval_to_visitresult.jinja",
        context! {
            result_type_path => &result_type_path,
            default_variant => default_variant,
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &result_type_path,
                &result_metadata,
                "s.to_string()",
            ),
        },
    ));
    out.push('\n');

    out.push_str(&crate::backends::php::template_env::render(
        "php_visit_result_with_template.jinja",
        context! {
            result_type_path => &result_type_path,
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
        },
    ));
    out.push_str("\n\n");

    out.push_str(&crate::backends::php::template_env::render(
        "visitor_bridge_struct.jinja",
        context! {
            struct_name => struct_name,
        },
    ));
    out.push('\n');

    out.push_str(&crate::backends::php::template_env::render(
        "php_trait_impl_start.jinja",
        context! {
            trait_path => &trait_path,
            struct_name => struct_name,
        },
    ));
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        if named_type_name(&method.return_type) != bridge_cfg.result_type.as_deref()
            || !method
                .params
                .iter()
                .any(|param| named_type_name(&param.ty) == bridge_cfg.context_type.as_deref())
        {
            continue;
        }
        gen_visitor_method_php(&mut out, method, bridge_cfg, type_paths, default_variant);
    }
    out.push_str("}\n");
    out.push('\n');

    out
}

/// Generate a single visitor method that checks for a snake_case PHP method and calls it.
fn gen_visitor_method_php(
    out: &mut String,
    method: &MethodDef,
    bridge_cfg: &TraitBridgeConfig,
    type_paths: &HashMap<String, String>,
    default_variant: &str,
) {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths.get(n.as_str()).cloned().unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    out.push_str(&crate::backends::php::template_env::render(
        "php_visitor_method_signature.jinja",
        context! {
            name => name,
            sig => &sig,
            ret_ty => &ret_ty,
        },
    ));

    // SAFETY: php_obj pointer is valid for the lifetime of the PHP call frame.
    out.push_str("        // SAFETY: php_obj is a valid ZendObject pointer for the duration of this call.\n");
    out.push_str("        let php_obj_ref = unsafe { &mut *self.php_obj };\n");

    let has_args = !method.params.is_empty();
    if has_args {
        out.push_str("        let mut args: Vec<ext_php_rs::types::Zval> = Vec::new();\n");
        for p in &method.params {
            if let TypeRef::Named(n) = &p.ty {
                if Some(n.as_str()) == bridge_cfg.context_type.as_deref() {
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_visitor_arg_nodecontext.jinja",
                        context! {
                            name => &p.name,
                            ref => if p.is_ref { "" } else { "&" },
                        },
                    ));
                    out.push('\n');
                    continue;
                }
            }
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                out.push_str(&crate::backends::php::template_env::render(
                    "php_visitor_arg_optional_string_ref.jinja",
                    context! {
                        name => &p.name,
                    },
                ));
                out.push('\n');
                continue;
            }
            if matches!(&p.ty, TypeRef::String) {
                if p.is_ref {
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_visitor_arg_string_ref.jinja",
                        context! {
                            name => &p.name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_visitor_arg_string_owned.jinja",
                        context! {
                            name => &p.name,
                        },
                    ));
                }
                out.push('\n');
                continue;
            }
            if matches!(&p.ty, TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)) {
                out.push_str(&crate::backends::php::template_env::render(
                    "php_visitor_arg_bool.jinja",
                    context! {
                        name => &p.name,
                    },
                ));
                out.push('\n');
                continue;
            }
            out.push_str(&crate::backends::php::template_env::render(
                "php_visitor_arg_default.jinja",
                context! {
                    name => &p.name,
                },
            ));
            out.push('\n');
        }
    }

    if has_args {
        out.push_str("        let dyn_args: Vec<&dyn ext_php_rs::convert::IntoZvalDyn> = args.iter().map(|z| z as &dyn ext_php_rs::convert::IntoZvalDyn).collect();\n");
    }
    let args_expr = if has_args { "dyn_args" } else { "vec![]" };
    out.push_str(&crate::backends::php::template_env::render(
        "php_visitor_method_php_call.jinja",
        context! {
            name => name,
            args_expr => args_expr,
        },
    ));

    let mut tmpl_var_names: Vec<String> = Vec::new();
    for p in &method.params {
        if let TypeRef::Named(n) = &p.ty {
            if Some(n.as_str()) == bridge_cfg.context_type.as_deref() {
                continue;
            }
        }
        if matches!(&p.ty, TypeRef::Vec(_)) {
            continue;
        }
        let key = p.name.strip_prefix('_').unwrap_or(&p.name);
        let owned_var = format!("_{key}_s");
        let expr: String = if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
            format!("{}.map(|s| s.to_string()).unwrap_or_default()", p.name)
        } else if matches!(&p.ty, TypeRef::String) && p.is_ref {
            format!("{}.to_string()", p.name)
        } else if matches!(&p.ty, TypeRef::String) {
            format!("{}.clone()", p.name)
        } else if matches!(&p.ty, TypeRef::Optional(_)) {
            format!("{}.map(|v| v.to_string()).unwrap_or_default()", p.name)
        } else {
            format!("{}.to_string()", p.name)
        };
        out.push_str(&crate::backends::php::template_env::render(
            "php_visitor_template_var_let_binding.jinja",
            context! {
                owned_var => &owned_var,
                expr => &expr,
            },
        ));
        out.push('\n');
        tmpl_var_names.push(format!("(\"{key}\", {owned_var}.as_str())"));
    }
    let tmpl_vars_expr = if tmpl_var_names.is_empty() {
        "&[]".to_string()
    } else {
        format!("&[{}]", tmpl_var_names.join(", "))
    };

    out.push_str(&crate::backends::php::template_env::render(
        "php_visitor_method_result_match.jinja",
        context! {
            ret_ty => &ret_ty,
            default_variant => format!("{ret_ty}::{default_variant}"),
            tmpl_vars_expr => &tmpl_vars_expr,
        },
    ));
    out.push('\n');
}
