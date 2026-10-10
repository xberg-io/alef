//! PHP e2e visitor helpers.

use crate::e2e::escape::escape_php;
use crate::e2e::fixture::{CallbackAction, VisitorSpec};

/// Build a PHP visitor object and add setup lines. The visitor is assigned to $visitor variable.
pub(super) fn build_php_visitor(setup_lines: &mut Vec<String>, visitor_spec: &VisitorSpec) {
    setup_lines.push("$visitor = new class {".to_string());
    for (method_name, action) in &visitor_spec.callbacks {
        emit_php_visitor_method(setup_lines, method_name, action);
    }
    setup_lines.push("};".to_string());
}

/// Emit a PHP visitor method for a callback action.
pub(super) fn emit_php_visitor_method(setup_lines: &mut Vec<String>, method_name: &str, action: &CallbackAction) {
    // The PHP bridge performs `{param_name}` template substitution on Custom/Error payloads
    // (see `php_visit_result_with_template`), so the callback returns the raw template and the
    // bridge interpolates from the Rust-side argument values. A variadic param list is sufficient.
    let params = "...$args";

    let (action_type, action_value, return_form) = match action {
        CallbackAction::Skip => ("skip", String::new(), "dict"),
        CallbackAction::Continue => ("continue", String::new(), "dict"),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), "dict"),
        CallbackAction::Custom { output } => ("custom", escape_php(output), "dict"),
        CallbackAction::CustomTemplate { template, return_form } => {
            let form = match return_form {
                crate::e2e::fixture::TemplateReturnForm::Dict => "dict",
                crate::e2e::fixture::TemplateReturnForm::BareString => "bare_string",
            };
            ("custom_template", escape_php(template), form)
        }
    };

    let rendered = crate::e2e::template_env::render(
        "php/visitor_method.jinja",
        crate::alef_context! {
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
