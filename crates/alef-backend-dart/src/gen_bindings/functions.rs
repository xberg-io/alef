use alef_core::ir::{FunctionDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::ident::dart_safe_ident;
use crate::template_env;

use super::render_type::{format_param, render_type};

/// The Dart literal for the default `ExtractionConfig`, matching `ExtractionConfig::default()`.
///
/// FRB generates `ExtractionConfig` with several required fields. Any wrapper that makes
/// `config` optional must supply a valid default when the caller omits it.
const DEFAULT_EXTRACTION_CONFIG: &str = "ExtractionConfig(\
    useCache: true, \
    enableQualityProcessing: true, \
    forceOcr: false, \
    disableOcr: false, \
    resultFormat: ResultFormat.unified, \
    outputFormat: OutputFormat.plain(), \
    includeDocumentStructure: false, \
    maxArchiveDepth: 3\
)";

/// Returns `true` if the parameter is the extraction config type.
///
/// Parameters named `config` with type `ExtractionConfig` are made optional in the
/// dart wrapper so callers can omit the config and get sensible defaults.
fn is_extraction_config_param(p: &alef_core::ir::ParamDef) -> bool {
    p.name == "config" && matches!(&p.ty, TypeRef::Named(n) if n == "ExtractionConfig")
}

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

    // Determine if any param is an optional ExtractionConfig.
    let has_config_param = f.params.iter().any(is_extraction_config_param);

    // Build the dart wrapper parameter list. If the function has an ExtractionConfig param,
    // split into required params and then `[ExtractionConfig? config]` optional positional.
    let params_str = if has_config_param {
        let required_params: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_extraction_config_param(p))
            .map(|p| format_param(p, imports))
            .collect();
        if required_params.is_empty() {
            "[ExtractionConfig? config]".to_string()
        } else {
            format!("{}, [ExtractionConfig? config]", required_params.join(", "))
        }
    } else {
        f.params.iter().map(|p| format_param(p, imports)).collect::<Vec<_>>().join(", ")
    };

    // FRB bridge functions use Dart named parameters (required keyword).
    // Call them with `name: value` named-argument syntax.
    // When config is optional, pass the default when the caller omits it.
    let call_args_str = if has_config_param {
        let non_config: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_extraction_config_param(p))
            .map(|p| {
                let ident = dart_safe_ident(&p.name.to_lower_camel_case());
                format!("{ident}: {ident}")
            })
            .collect();
        let config_arg = format!("config: config ?? {DEFAULT_EXTRACTION_CONFIG}");
        if non_config.is_empty() {
            config_arg
        } else {
            format!("{}, {config_arg}", non_config.join(", "))
        }
    } else {
        f.params
            .iter()
            .map(|p| {
                let ident = dart_safe_ident(&p.name.to_lower_camel_case());
                format!("{ident}: {ident}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

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
                params => params_str.as_str(),
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
                params => params_str.as_str(),
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
