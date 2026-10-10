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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::visitor_result::required_visitor_result_metadata;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};

    /// `VisitResult` IR shape mirroring the consumer convention for visitor trait bridges:
    /// unit `Skip`/`Continue`/`PreserveHtml` variants plus a single-`String`-field `Custom`
    /// payload variant, with `#[serde(rename_all = "snake_case")]` on the enum — the
    /// convention the Magnus backend relies on to build lowercase wire names.
    fn visit_result_enum() -> EnumDef {
        EnumDef {
            name: "VisitResult".to_string(),
            rust_path: "sample_markdown_rs::visitor::VisitResult".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Continue".to_string(),
                    is_default: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Skip".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "PreserveHtml".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Custom".to_string(),
                    fields: vec![FieldDef {
                        name: "0".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            has_serde: true,
            serde_rename_all: Some("snake_case".to_string()),
            ..Default::default()
        }
    }

    fn visitor_bridge_cfg() -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "HtmlVisitor".to_string(),
            result_type: Some("VisitResult".to_string()),
            ..Default::default()
        }
    }

    fn render(action: &CallbackAction) -> String {
        let mut setup_lines = Vec::new();
        emit_ruby_visitor_method(&mut setup_lines, "visit_text", action);
        setup_lines.join("\n")
    }

    /// Pins the e2e template's emitted Ruby literals to the wire names computed by
    /// `visitor_result::required_visitor_result_metadata` — the exact function the Magnus
    /// backend's `gen_visitor_bridge` calls to build both its `match s.as_str()` arms and its
    /// `ruby.to_symbol(...)` hash key. Deriving the expected literal from that shared function,
    /// rather than hardcoding it here, means the e2e template and the real binding can't
    /// silently re-diverge: this is what let the two sides disagree in the first place, each
    /// looking self-consistent (the template's own hardcoded PascalCase vs the backend's
    /// wire-derived lowercase) until a real consumer's Ruby suite hit the mismatch at runtime.
    #[test]
    fn ruby_visitor_action_literals_match_magnus_wire_names() {
        let api = ApiSurface {
            enums: vec![visit_result_enum()],
            ..Default::default()
        };
        let metadata = required_visitor_result_metadata(&api, &visitor_bridge_cfg())
            .expect("VisitResult metadata should resolve for a well-formed visitor result enum");

        let unit_wire = |variant_name: &str| {
            metadata
                .unit_variants
                .iter()
                .find(|variant| variant.name == variant_name)
                .unwrap_or_else(|| panic!("expected unit variant `{variant_name}` in metadata"))
                .wire_name
                .clone()
        };
        let custom_wire = metadata
            .string_payload_variants
            .first()
            .expect("expected one string-payload variant (Custom)")
            .wire_name
            .clone();

        for (action, variant_name) in [
            (CallbackAction::Skip, "Skip"),
            (CallbackAction::Continue, "Continue"),
            (CallbackAction::PreserveHtml, "PreserveHtml"),
        ] {
            let rendered = render(&action);
            let expected_literal = format!("'{}'", unit_wire(variant_name));
            assert!(
                rendered.contains(&expected_literal),
                "expected Magnus-matching literal {expected_literal:?} in rendered Ruby:\n{rendered}"
            );
        }

        let rendered = render(&CallbackAction::Custom {
            output: "replacement".to_string(),
        });
        let expected_key = format!("{custom_wire}: ");
        assert!(
            rendered.contains(&expected_key),
            "expected Magnus-matching hash key {expected_key:?} in rendered Ruby:\n{rendered}"
        );
    }
}
