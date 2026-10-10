use crate::backends::rustler::template_env;
use crate::core::ir::TypeDef;
use ahash::AHashMap;

/// Resolve the fully-qualified core path for a named type.
///
/// When the TypeDef is found in `types_by_name` its `rust_path` is used (which
/// may be `sample_core::extraction::docx::drawing::DrawingType`). When not found
/// (e.g. a type that doesn't appear in the API surface but is referenced as a
/// param type) the short fallback `{core_import}::{name}` is used.
pub(super) fn resolve_core_type_path<'a>(
    name: &str,
    types_by_name: &AHashMap<&'a str, &'a TypeDef>,
    core_import: &str,
) -> String {
    if let Some(typ) = types_by_name.get(name) {
        crate::codegen::conversions::core_type_path(typ, core_import)
    } else {
        format!("{core_import}::{name}")
    }
}

pub(super) fn render_deser_line(template_name: &str, name: &str, core_type: &str) -> String {
    template_env::render(
        template_name,
        crate::alef_context! {
            name => name,
            core_type => core_type,
        },
    )
    .trim_end()
    .to_string()
}

pub(super) fn render_named_deser_line(template_name: &str, name: &str) -> String {
    template_env::render(
        template_name,
        crate::alef_context! {
            name => name,
        },
    )
    .trim_end()
    .to_string()
}

pub(super) fn render_preamble(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", lines.join("\n    "))
    }
}

pub(super) fn render_result_body(preamble: &str, core_call: &str, wrap: &str) -> String {
    template_env::render(
        "nif_result_body.rs.jinja",
        crate::alef_context! {
            preamble => preamble,
            core_call => core_call,
            wrap => wrap,
        },
    )
}

pub(super) fn render_wrapped_body(preamble: &str, wrap: &str) -> String {
    template_env::render(
        "nif_wrapped_body.rs.jinja",
        crate::alef_context! {
            preamble => preamble,
            wrap => wrap,
        },
    )
}

pub(super) fn render_async_body(template_name: &str, preamble: &str, core_call: &str, result_wrap: &str) -> String {
    template_env::render(
        template_name,
        crate::alef_context! {
            preamble => preamble,
            core_call => core_call,
            result_wrap => result_wrap,
        },
    )
}

pub(super) fn render_method_call(template_name: &str, core_path: &str, method_name: &str, call_args: &str) -> String {
    template_env::render(
        template_name,
        crate::alef_context! {
            core_path => core_path,
            method_name => method_name,
            call_args => call_args,
        },
    )
    .trim_end()
    .to_string()
}

pub(super) fn render_method_call_with_preamble(
    preamble: &str,
    core_path: &str,
    method_name: &str,
    call_args: &str,
) -> String {
    template_env::render(
        "rust_method_static_call_with_preamble.rs.jinja",
        crate::alef_context! {
            preamble => preamble,
            core_path => core_path,
            method_name => method_name,
            call_args => call_args,
        },
    )
    .trim_end()
    .to_string()
}
