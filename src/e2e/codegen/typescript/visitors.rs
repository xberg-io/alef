//! TypeScript visitor generation for e2e test callbacks.

use std::fmt::Write as FmtWrite;

use crate::e2e::escape::escape_js;
use crate::e2e::fixture::{CallbackAction, TemplateReturnForm};
use heck::ToLowerCamelCase;

/// Build a TypeScript visitor object and add setup line. Returns the visitor variable name.
pub(super) fn build_typescript_visitor(
    setup_lines: &mut Vec<String>,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
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
        "visit_code_block" => "ctx: any, code: any, lang: any",
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
        "visit_input" => "ctx: any, input_type: any, name: any, value: any",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx: any, src: any",
        "visit_details" => "ctx: any, isOpen: any",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => {
            "ctx: any, output: any"
        }
        "visit_list_start" => "ctx: any, ordered: any",
        "visit_list_end" => "ctx: any, ordered: any, output: any",
        _ => "ctx: any",
    };

    let (action_type, action_value, action_template, return_form) = match action {
        CallbackAction::Skip => ("skip", String::new(), String::new(), "dict"),
        CallbackAction::Continue => ("continue", String::new(), String::new(), "dict"),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), String::new(), "dict"),
        CallbackAction::Custom { output } => {
            let escaped = escape_js(output);
            ("custom", escaped, String::new(), "dict")
        }
        CallbackAction::CustomTemplate { template, return_form } => {
            // Convert {placeholder} to ${placeholder} for JavaScript template literals
            let mut processed = String::new();
            for ch in template.chars() {
                match ch {
                    '{' => processed.push_str("${"),
                    '}' => processed.push('}'),
                    _ => processed.push(ch),
                }
            }
            let form = match return_form {
                TemplateReturnForm::Dict => "dict",
                TemplateReturnForm::BareString => "bare_string",
            };
            ("custom_template", String::new(), processed, form)
        }
    };

    let rendered = crate::e2e::template_env::render(
        "typescript/visitor_method.jinja",
        crate::alef_context! {
            camel_method => camel_method,
            params => params,
            action_type => action_type,
            action_value => action_value,
            action_template => action_template,
            return_form => return_form,
        },
    );
    let _ = writeln!(out, "{rendered}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::visitor_result::required_visitor_result_metadata;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};
    use crate::e2e::fixture::CallbackAction;

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

    #[test]
    fn emit_typescript_visitor_method_uses_adjacent_custom_payload() {
        let mut out = String::new();
        emit_typescript_visitor_method(
            &mut out,
            "visit_code_block",
            &CallbackAction::Custom {
                output: "replacement".to_string(),
            },
        );

        assert!(
            out.contains("visitCodeBlock(ctx: any, code: any, lang: any)"),
            "got: {out}"
        );
        assert!(out.contains(r#"return { custom: "replacement" };"#), "got: {out}");
    }

    /// `VisitResult` IR shape mirroring the consumer convention for visitor trait bridges:
    /// unit `Skip`/`Continue`/`PreserveHtml` variants plus a single-`String`-field `Custom`
    /// payload variant, with `#[serde(rename_all = "snake_case")]` on the enum — the
    /// convention the napi backend relies on to build lowercase wire names.
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

    /// Pins the e2e template's emitted TypeScript object key to the flat key that
    /// `visitor_result::required_visitor_result_metadata` computes — the exact function the
    /// napi backend's `gen_visitor_bridge` calls to build its
    /// `obj.get_named_property(prop_name)` lookup. Deriving the expected key from that shared
    /// function, rather than hardcoding it here, means the e2e template and the real binding
    /// can't silently re-diverge: a nested `{ type: "custom", output: ... }` envelope satisfies
    /// neither `get_named_property("custom")` nor any other flat lookup, so the previous shape
    /// looked self-consistent while never being deliverable to the napi backend.
    #[test]
    fn typescript_visitor_custom_payload_key_matches_napi_wire_name() {
        let api = ApiSurface {
            enums: vec![visit_result_enum()],
            ..Default::default()
        };
        let metadata = required_visitor_result_metadata(&api, &visitor_bridge_cfg())
            .expect("VisitResult metadata should resolve for a well-formed visitor result enum");
        let custom_wire = metadata
            .string_payload_variants
            .first()
            .expect("expected one string-payload variant (Custom)")
            .wire_name
            .clone();

        let mut out = String::new();
        emit_typescript_visitor_method(
            &mut out,
            "visit_text",
            &CallbackAction::Custom {
                output: "replacement".to_string(),
            },
        );

        let expected_key = format!("{{ {custom_wire}: ");
        assert!(
            out.contains(&expected_key),
            "expected napi-matching key {expected_key:?} in rendered TypeScript:\n{out}"
        );
        assert!(
            !out.contains("type:") && !out.contains("output:"),
            "TypeScript visitor payload must not emit the dead nested `type`/`output` envelope:\n{out}"
        );
    }
}
