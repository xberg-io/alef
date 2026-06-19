//! Ruby e2e visitor helpers.

use crate::e2e::escape::ruby_string_literal;
use crate::e2e::fixture::{CallbackAction, TemplateReturnForm, VisitorSpec};

/// Build a Ruby visitor object and add setup lines. Returns the visitor expression.
pub(super) fn build_ruby_visitor(setup_lines: &mut Vec<String>, visitor_spec: &VisitorSpec) -> String {
    setup_lines.push("visitor = Class.new do".to_string());
    for (method_name, action) in &visitor_spec.callbacks {
        emit_ruby_visitor_method(setup_lines, method_name, action);
    }
    setup_lines.push("end.new".to_string());
    "visitor".to_string()
}

/// Ruby parameter list for a visitor method, mirroring the core trait signature so that
/// `{placeholder}` template interpolation can resolve named arguments (e.g. `text`, `href`).
/// Names match the Python e2e codegen so the same fixtures interpolate identically.
fn ruby_visitor_params(method_name: &str) -> &'static str {
    match method_name {
        "visit_link" => "ctx, href, text, title",
        "visit_image" => "ctx, src, alt, title",
        "visit_heading" => "ctx, level, text, id",
        "visit_code_block" => "ctx, lang, code",
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
        | "visit_definition_description"
        | "visit_text" => "ctx, text",
        "visit_list_item" => "ctx, ordered, marker, text",
        "visit_blockquote" => "ctx, content, depth",
        "visit_table_row" => "ctx, cells, is_header",
        "visit_custom_element" => "ctx, tag_name, html",
        "visit_form" => "ctx, action_url, method",
        "visit_input" => "ctx, input_type, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx, src",
        "visit_details" => "ctx, is_open",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            "ctx, output, *args"
        }
        "visit_list_start" => "ctx, ordered, *args",
        "visit_list_end" => "ctx, ordered, output, *args",
        _ => "*args",
    }
}

/// Convert `{name}` template placeholders into Ruby `#{name}` interpolation, after escaping
/// backslashes and double quotes for a double-quoted Ruby string literal.
fn ruby_interpolate_template(template: &str) -> String {
    template.replace('\\', "\\\\").replace('"', "\\\"").replace('{', "#{")
}

/// Emit a Ruby visitor method for a callback action.
pub(super) fn emit_ruby_visitor_method(setup_lines: &mut Vec<String>, method_name: &str, action: &CallbackAction) {
    let params = ruby_visitor_params(method_name);

    // Pre-compute action type and values
    let (action_type, action_value, return_form) = match action {
        CallbackAction::Skip => ("skip", String::new(), "dict"),
        CallbackAction::Continue => ("continue", String::new(), "dict"),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), "dict"),
        CallbackAction::Custom { output } => {
            let escaped = ruby_string_literal(output);
            ("custom", escaped, "dict")
        }
        CallbackAction::CustomTemplate { template, return_form } => {
            let interpolated = ruby_interpolate_template(template);
            let form = match return_form {
                TemplateReturnForm::Dict => "dict",
                TemplateReturnForm::BareString => "bare_string",
            };
            ("custom_template", format!("\"{interpolated}\""), form)
        }
    };

    let rendered = crate::e2e::template_env::render(
        "ruby/visitor_method.jinja",
        minijinja::context! {
            method_name => method_name,
            params => params,
            action_type => action_type,
            action_value => action_value,
            return_form => return_form,
        },
    );
    for line in rendered.lines() {
        setup_lines.push(line.to_string());
    }
}
