//! TypeScript visitor generation for e2e test callbacks.

use std::fmt::Write as FmtWrite;

use crate::escape::escape_js;
use crate::fixture::CallbackAction;
use heck::ToLowerCamelCase;

/// Build a TypeScript visitor object and add setup line. Returns the visitor variable name.
pub(super) fn build_typescript_visitor(
    setup_lines: &mut Vec<String>,
    visitor_spec: &crate::fixture::VisitorSpec,
) -> String {
    let mut visitor_obj = String::new();
    let _ = writeln!(visitor_obj, "{{");
    for (method_name, action) in &visitor_spec.callbacks {
        emit_typescript_visitor_method(&mut visitor_obj, method_name, action);
    }
    let _ = writeln!(visitor_obj, "    }}");

    setup_lines.push(format!("const _testVisitor = {visitor_obj}"));
    "_testVisitor".to_string()
}

/// Emit a TypeScript visitor method for a callback action.
pub(super) fn emit_typescript_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    let camel_method = method_name.to_lower_camel_case();
    // All parameters are typed as `any` — visitor methods are untyped in e2e tests
    // because `JsNodeContext` is not importable without the built native module.
    let params = match method_name {
        "visit_link" => "ctx: any, href: any, text: any, title: any",
        "visit_image" => "ctx: any, src: any, alt: any, title: any",
        "visit_heading" => "ctx: any, level: any, text: any, id: any",
        "visit_code_block" => "ctx: any, lang: any, code: any",
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => "ctx: any, text: any",
        "visit_text" => "ctx: any, text: any",
        "visit_list_item" => "ctx: any, ordered: any, marker: any, text: any",
        "visit_blockquote" => "ctx: any, content: any, depth: any",
        "visit_table_row" => "ctx: any, cells: any, isHeader: any",
        "visit_custom_element" => "ctx: any, tagName: any, html: any",
        "visit_form" => "ctx: any, actionUrl: any, method: any",
        "visit_input" => "ctx: any, inputType: any, name: any, value: any",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx: any, src: any",
        "visit_details" => "ctx: any, isOpen: any",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            "ctx: any, output: any"
        }
        "visit_list_start" => "ctx: any, ordered: any",
        "visit_list_end" => "ctx: any, ordered: any, output: any",
        _ => "ctx: any",
    };

    let _ = writeln!(out, "    {camel_method}({params}): string | {{ custom: string }} {{");
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "        return \"skip\";");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "        return \"continue\";");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "        return \"preserve_html\";");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_js(output);
            let _ = writeln!(out, "        return {{ custom: \"{escaped}\" }};");
        }
        CallbackAction::CustomTemplate { template } => {
            let _ = writeln!(out, "        return {{ custom: `{template}` }};");
        }
    }
    let _ = writeln!(out, "    }},");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::CallbackAction;

    #[test]
    fn emit_typescript_visitor_method_skip_returns_skip() {
        let mut out = String::new();
        emit_typescript_visitor_method(&mut out, "visit_text", &CallbackAction::Skip);
        assert!(out.contains("return \"skip\""), "got: {out}");
    }

    #[test]
    fn emit_typescript_visitor_method_uses_camel_case_name() {
        let mut out = String::new();
        emit_typescript_visitor_method(&mut out, "visit_list_item", &CallbackAction::Continue);
        assert!(out.contains("visitListItem"), "got: {out}");
    }
}
