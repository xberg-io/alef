use alef_core::ir::{FunctionDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::ident::dart_safe_ident;
use crate::template_env;

use super::render_type::{format_param, render_type};

pub(super) fn emit_function(f: &FunctionDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !f.doc.is_empty() {
        for line in f.doc.lines() {
            out.push_str("  /// ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(ref error_ty) = f.error_type {
        out.push_str(&template_env::render(
            "function_throws_annotation.jinja",
            minijinja::context! {
                error_ty => error_ty.as_str(),
            },
        ));
    }

    let fn_name = dart_safe_ident(&f.name.to_lower_camel_case());
    let params: Vec<String> = f.params.iter().map(|p| format_param(p, imports)).collect();
    // FRB bridge functions use Dart named parameters (required keyword).
    // Call them with `name: value` named-argument syntax.
    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let ident = dart_safe_ident(&p.name.to_lower_camel_case());
            format!("{ident}: {ident}")
        })
        .collect();
    let call_args_str = call_args.join(", ");

    if f.is_async {
        let return_ty = if matches!(f.return_type, TypeRef::Unit) {
            "Future<void>".to_string()
        } else {
            format!("Future<{}>", render_type(&f.return_type, imports))
        };
        out.push_str(&template_env::render(
            "function_signature_async.jinja",
            minijinja::context! {
                return_ty => return_ty,
                fn_name => fn_name.as_str(),
                params => params.join(", "),
            },
        ));
        out.push_str(&template_env::render(
            "function_await_return.jinja",
            minijinja::context! {
                fn_name => fn_name.as_str(),
                call_args_str => call_args_str.as_str(),
            },
        ));
        out.push_str("  }\n");
    } else {
        let return_ty = render_type(&f.return_type, imports);
        out.push_str(&template_env::render(
            "function_signature_sync.jinja",
            minijinja::context! {
                return_ty => return_ty,
                fn_name => fn_name.as_str(),
                params => params.join(", "),
            },
        ));
        out.push_str(&template_env::render(
            "function_sync_return.jinja",
            minijinja::context! {
                fn_name => fn_name.as_str(),
                call_args_str => call_args_str.as_str(),
            },
        ));
        out.push_str("  }\n");
    }
}
