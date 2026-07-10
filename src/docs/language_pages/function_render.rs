use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef, VersionAnnotation};
use crate::docs::descriptions::generate_param_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings, extract_param_docs};
use crate::docs::examples::render_function_example;
use crate::docs::formatting::{doc_type_with_optional, escape_table_cell, format_error_phrase};
use crate::docs::naming::{field_name, func_name, lang_code_fence};
use crate::docs::signatures::render_function_signature;
use crate::docs::{clean_doc, doc_type, template_env, version_labels};

pub(super) fn push_version_annotation(out: &mut String, version: &VersionAnnotation) {
    if let Some(ref since) = version.since {
        let since = version_labels::major_minor(since);
        out.push_str(&template_env::render(
            "since_badge.jinja",
            minijinja::context! { since => since },
        ));
        out.push('\n');
        out.push('\n');
    }
    if let Some(ref dep) = version.deprecated {
        let since = dep
            .since
            .as_deref()
            .map(version_labels::major_minor)
            .unwrap_or_default();
        out.push_str(&template_env::render(
            "deprecated_notice.jinja",
            minijinja::context! {
                since => since,
                note => dep.note.as_deref().unwrap_or(""),
            },
        ));
        out.push('\n');
        out.push('\n');
    }
}

pub(super) fn render_function(
    func: &FunctionDef,
    lang: Language,
    _config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let fn_name = func_name(&func.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => format!("{fn_name}()") },
    ));

    push_version_annotation(&mut out, &func.version);

    let param_docs = extract_param_docs(&func.doc);

    if !func.doc.is_empty() {
        let doc = clean_doc(&func.doc, lang);
        let doc = demote_headings(&doc, 2);
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    out.push_str("**Signature:**\n\n");
    let lang_code = lang_code_fence(lang);
    let sig = render_function_signature(func, lang, ffi_prefix);
    out.push_str(&template_env::render(
        "code_block.jinja",
        minijinja::context! { lang_code => lang_code, body => sig },
    ));
    out.push('\n');

    out.push_str(&render_function_example(func, lang, ffi_prefix));

    push_parameters_table(&mut out, &func.params, &param_docs, lang, ffi_prefix);

    push_returns(&mut out, &func.return_type, lang, ffi_prefix);
    push_errors(&mut out, func.error_type.as_deref(), lang);

    let _ = api;
    out
}

pub(super) fn push_parameters_table(
    out: &mut String,
    params: &[ParamDef],
    param_docs: &std::collections::HashMap<String, String>,
    lang: Language,
    ffi_prefix: &str,
) {
    if params.is_empty() {
        return;
    }
    out.push_str("**Parameters:**\n\n");
    out.push_str("| Name | Type | Required | Description |\n");
    out.push_str("|------|------|----------|-------------|\n");
    for param in params {
        let pname = field_name(&param.name, lang);
        let pty = doc_type_with_optional(&param.ty, lang, param.optional, ffi_prefix);
        let required = if param.optional { "No" } else { "Yes" };
        let pdoc = param_docs
            .get(param.name.as_str())
            .map(|s| clean_doc_inline(s, lang))
            .unwrap_or_else(|| generate_param_description(&param.name, &param.ty));
        out.push_str(&template_env::render(
            "param_row.jinja",
            minijinja::context! {
                name => escape_table_cell(&pname),
                ty => escape_table_cell(&pty),
                required => required,
                doc => escape_table_cell(&pdoc),
            },
        ));
    }
    out.push('\n');
}

pub(super) fn push_returns(out: &mut String, return_type: &TypeRef, lang: Language, ffi_prefix: &str) {
    push_returns_with_override(out, return_type, None, lang, ffi_prefix);
}

pub(super) fn push_returns_with_override(
    out: &mut String,
    return_type: &TypeRef,
    return_type_override: Option<&str>,
    lang: Language,
    ffi_prefix: &str,
) {
    if matches!(return_type, TypeRef::Unit) {
        out.push_str("**Returns:** No return value.\n");
        out.push('\n');
        return;
    }

    let ret_ty = return_type_override
        .map(str::to_string)
        .unwrap_or_else(|| doc_type(return_type, lang, ffi_prefix));
    if ret_ty.is_empty() {
        out.push_str("**Returns:** No return value.\n");
        out.push('\n');
    } else {
        out.push_str(&template_env::render(
            "returns.jinja",
            minijinja::context! { ty => ret_ty },
        ));
        out.push('\n');
    }
}

pub(super) fn push_errors(out: &mut String, error_type: Option<&str>, lang: Language) {
    if let Some(err) = error_type {
        let error_phrase = format_error_phrase(err, lang);
        out.push_str(&template_env::render(
            "errors_phrase.jinja",
            minijinja::context! { phrase => error_phrase },
        ));
        out.push('\n');
    }
}
