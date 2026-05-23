use crate::core::ir::{FunctionDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

use crate::backends::dart::ident::dart_safe_ident;
use crate::backends::dart::template_env;

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
    useLayoutForMarkdown: false, \
    maxArchiveDepth: 3\
)";

/// Returns `true` if the parameter is a config type that should be made optional in Dart.
///
/// Parameters named `config` with types like `ExtractionConfig` or `PackConfig` are
/// made optional in the dart wrapper so callers can omit the config and get sensible
/// defaults. This applies to config structs that have `Default` implementations in Rust
/// and are conventionally used with optional semantics at binding boundaries.
fn is_optional_config_param(p: &crate::core::ir::ParamDef) -> bool {
    p.name == "config"
        && matches!(
            &p.ty,
            TypeRef::Named(n) if n == "ExtractionConfig" || n == "PackConfig"
        )
}

pub(super) fn emit_function(f: &FunctionDef, out: &mut String, imports: &mut BTreeSet<String>) {
    if !f.doc.is_empty() {
        let doc_lines: Vec<String> = f.doc.lines().map(ToString::to_string).collect();
        out.push_str(&template_env::render(
            "doc_comment.jinja",
            minijinja::context! {
                indent => "  ",
                lines => doc_lines,
            },
        ));
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

    // Find the optional config param if present, and determine its type.
    let config_param = f.params.iter().find(|p| is_optional_config_param(p));
    let config_type = config_param.and_then(|p| match &p.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        _ => None,
    });

    // Build the dart wrapper parameter list. If the function has an optional config param
    // (e.g., ExtractionConfig or PackConfig), split into required params and then
    // `[ConfigType? config]` optional positional.
    //
    // For all other functions, emit required (non-optional) params as positional and
    // optional params inside a `{...}` named-parameter block. This matches the natural
    // Dart calling convention `createClient('key', baseUrl: ...)` and mirrors the
    // underlying FRB binding which is itself named-only.
    let params_str = if let Some(cfg_type) = config_type {
        let required_params: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_optional_config_param(p))
            .map(|p| format_param(p, imports))
            .collect();
        let optional_sig = format!("[{cfg_type}? config]");
        if required_params.is_empty() {
            optional_sig
        } else {
            format!("{}, {optional_sig}", required_params.join(", "))
        }
    } else {
        let required: Vec<String> = f
            .params
            .iter()
            .filter(|p| !p.optional)
            .map(|p| format_param(p, imports))
            .collect();
        let optional: Vec<String> = f
            .params
            .iter()
            .filter(|p| p.optional)
            .map(|p| format_param(p, imports))
            .collect();
        match (required.is_empty(), optional.is_empty()) {
            (true, true) => String::new(),
            (false, true) => required.join(", "),
            (true, false) => format!("{{{}}}", optional.join(", ")),
            (false, false) => format!("{}, {{{}}}", required.join(", "), optional.join(", ")),
        }
    };

    // FRB bridge functions use Dart named parameters (required keyword).
    // Call them with `name: value` named-argument syntax.
    // When config is optional, pass the default when the caller omits it.
    let call_args_str = if let Some(cfg_type) = config_type {
        let non_config: Vec<String> = f
            .params
            .iter()
            .filter(|p| !is_optional_config_param(p))
            .map(|p| {
                let ident = dart_safe_ident(&p.name.to_lower_camel_case());
                format!("{ident}: {ident}")
            })
            .collect();
        // For ExtractionConfig, use the hardcoded DEFAULT_EXTRACTION_CONFIG constant.
        // For other config types (e.g., PackConfig), use the default constructor.
        let config_default = if cfg_type == "ExtractionConfig" {
            format!("config ?? {DEFAULT_EXTRACTION_CONFIG}")
        } else {
            format!("config ?? {cfg_type}()")
        };
        let config_arg = format!("config: {config_default}");
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

    // FRB v2 wraps ALL Rust functions as `Future<T>` in Dart, including sync ones.
    // Therefore all wrapper methods must be `async` and `await` the bridge call.
    {
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
    }
}
