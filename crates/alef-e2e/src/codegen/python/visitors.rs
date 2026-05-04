//! Python visitor method generation for e2e test callbacks.

use std::fmt::Write as FmtWrite;

use crate::escape::escape_python;
use crate::fixture::CallbackAction;

/// Emit a Python visitor method for a callback action.
pub(super) fn emit_python_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    let params = match method_name {
        "visit_link" => "self, ctx, href, text, title",
        "visit_image" => "self, ctx, src, alt, title",
        "visit_heading" => "self, ctx, level, text, id",
        "visit_code_block" => "self, ctx, lang, code",
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
        | "visit_definition_description" => "self, ctx, text",
        "visit_text" => "self, ctx, text",
        "visit_list_item" => "self, ctx, ordered, marker, text",
        "visit_blockquote" => "self, ctx, content, depth",
        "visit_table_row" => "self, ctx, cells, is_header",
        "visit_custom_element" => "self, ctx, tag_name, html",
        "visit_form" => "self, ctx, action_url, method",
        "visit_input" => "self, ctx, input_type, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "self, ctx, src",
        "visit_details" => "self, ctx, is_open",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            "self, ctx, output, *args"
        }
        "visit_list_start" => "self, ctx, ordered, *args",
        "visit_list_end" => "self, ctx, ordered, output, *args",
        _ => "self, ctx, *args",
    };

    let _ = writeln!(
        out,
        "        def {method_name}({params}):  # noqa: A002, ANN001, ANN202, ARG002"
    );
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "            return {{\"type\": \"Skip\"}}");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "            return {{\"type\": \"Continue\"}}");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "            return {{\"type\": \"PreserveHtml\"}}");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_python(output);
            let _ = writeln!(
                out,
                "            return {{\"type\": \"Custom\", \"_0\": \"{escaped}\"}}"
            );
        }
        CallbackAction::CustomTemplate { template } => {
            // Use single-quoted f-string so that double quotes inside the template
            // (e.g. `QUOTE: "{text}"`) are not misinterpreted as string delimiters.
            let escaped_template = template
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            let _ = writeln!(
                out,
                "            return {{\"type\": \"Custom\", \"_0\": f'{escaped_template}'}}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_python_visitor_method_skip_returns_skip() {
        let mut out = String::new();
        emit_python_visitor_method(&mut out, "visit_text", &CallbackAction::Skip);
        assert!(out.contains("return {\"type\": \"Skip\"}"), "got: {out}");
    }

    #[test]
    fn emit_python_visitor_method_uses_method_name_as_is() {
        let mut out = String::new();
        emit_python_visitor_method(&mut out, "visit_list_item", &CallbackAction::Continue);
        assert!(out.contains("visit_list_item"), "got: {out}");
    }
}
