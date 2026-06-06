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

/// Emit a Ruby visitor method for a callback action.
pub(super) fn emit_ruby_visitor_method(setup_lines: &mut Vec<String>, method_name: &str, action: &CallbackAction) {
    let params = "*args";

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
            let interpolated = template.replace('\\', "\\\\").replace('"', "\\\"");
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
