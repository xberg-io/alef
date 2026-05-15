//! Dart visitor method generation for e2e test callbacks.
//!
//! The dart `HtmlVisitor` trait is bridged through `flutter_rust_bridge`'s
//! `DartFnFuture` machinery. Every method of the trait must be supplied as a
//! closure to `createHtmlVisitor(...)` — the FRB generator requires
//! all callbacks to be passed positionally. Fixtures only configure a subset
//! of callbacks; for the rest we emit default closures that return
//! `VisitResult.continue_()`.
//!
//! The full list of callback methods is the union of every callback name that
//! ever appears across the fixture corpus, plus the static set declared by the
//! `HtmlVisitor` trait. We hardcode the canonical list here to keep the
//! generated code self-contained — see
//! `crates/html-to-markdown/src/visitor/traits.rs` for the source of truth.

use super::dart::escape_dart;
use crate::fixture::{CallbackAction, VisitorSpec};
use heck::ToLowerCamelCase;
use std::fmt::Write as FmtWrite;

/// All HtmlVisitor callback methods (Rust snake_case names) that
/// `createHtmlVisitor` requires. Order is not important — they are
/// passed as named arguments — but we keep a stable list to keep emitted code
/// deterministic and snapshot-friendly.
const ALL_VISITOR_METHODS: &[&str] = &[
    "visit_text",
    "visit_element_start",
    "visit_element_end",
    "visit_link",
    "visit_image",
    "visit_heading",
    "visit_code_block",
    "visit_code_inline",
    "visit_list_item",
    "visit_list_start",
    "visit_list_end",
    "visit_table_start",
    "visit_table_row",
    "visit_table_end",
    "visit_blockquote",
    "visit_strong",
    "visit_emphasis",
    "visit_strikethrough",
    "visit_underline",
    "visit_subscript",
    "visit_superscript",
    "visit_mark",
    "visit_line_break",
    "visit_horizontal_rule",
    "visit_custom_element",
    "visit_definition_list_start",
    "visit_definition_term",
    "visit_definition_description",
    "visit_definition_list_end",
    "visit_form",
    "visit_input",
    "visit_button",
    "visit_audio",
    "visit_video",
    "visit_iframe",
    "visit_details",
    "visit_summary",
    "visit_figure_start",
    "visit_figcaption",
    "visit_figure_end",
];

/// Build a visitor-handle setup block and append it to `setup_lines`. Returns
/// the dart variable name holding the visitor handle (always `_visitor`).
pub(super) fn build_dart_visitor(setup_lines: &mut Vec<String>, visitor_spec: &VisitorSpec) -> String {
    // Emit one named-arg per visitor method. Methods with fixture-supplied
    // callbacks return the action-specific VisitResult; all others return
    // the default `VisitResult.continue_()` so the conversion falls through
    // to the built-in markdown emitter.
    let mut named_args: Vec<String> = Vec::with_capacity(ALL_VISITOR_METHODS.len());
    for method in ALL_VISITOR_METHODS {
        let camel = method.to_lower_camel_case();
        let params = dart_visitor_params(method);
        let body = match visitor_spec.callbacks.get(*method) {
            Some(action) => dart_action_body(method, action),
            None => "VisitResult.continue_()".to_string(),
        };
        // Use an async closure so the callback signature matches
        // `DartFnFuture<VisitResult>` (FRB awaits the returned future).
        named_args.push(format!("{camel}: ({params}) async => {body}"));
    }

    // Render as a multi-line `createHtmlVisitor(...)` call. The
    // indentation matches the standard test-body indent (4 spaces inside the
    // test closure) so the emitted file reads cleanly.
    let mut block = String::from("final _visitor = await createHtmlVisitor(\n");
    for (i, arg) in named_args.iter().enumerate() {
        let sep = if i + 1 == named_args.len() { "" } else { "," };
        let _ = writeln!(block, "      {arg}{sep}");
    }
    block.push_str("    );");
    setup_lines.push(block);
    "_visitor".to_string()
}

/// Dart closure parameter list for each visitor method. Parameter names mirror
/// the Rust trait signature, lowerCamelCased — same convention FRB uses for
/// the generated `BoxFn...` callback typedefs.
fn dart_visitor_params(method: &str) -> &'static str {
    match method {
        "visit_text" => "ctx, text",
        "visit_element_start" => "ctx",
        "visit_element_end" => "ctx, output",
        "visit_link" => "ctx, href, text, title",
        "visit_image" => "ctx, src, alt, title",
        "visit_heading" => "ctx, level, text, id",
        "visit_code_block" => "ctx, lang, code",
        "visit_code_inline" => "ctx, code",
        "visit_list_item" => "ctx, ordered, marker, text",
        "visit_list_start" => "ctx, ordered",
        "visit_list_end" => "ctx, ordered, output",
        "visit_table_start" => "ctx",
        "visit_table_row" => "ctx, cells, isHeader",
        "visit_table_end" => "ctx, output",
        "visit_blockquote" => "ctx, content, depth",
        "visit_strong"
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
        | "visit_definition_list_end" => "ctx, text",
        "visit_line_break" | "visit_horizontal_rule" | "visit_definition_list_start" | "visit_figure_start" => "ctx",
        "visit_custom_element" => "ctx, tagName, html",
        "visit_form" => "ctx, action, method",
        "visit_input" => "ctx, inputType, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx, src",
        "visit_details" => "ctx, open",
        "visit_figure_end" => "ctx, output",
        _ => "ctx",
    }
}

/// Render the Dart expression for a fixture-driven callback action.
fn dart_action_body(method: &str, action: &CallbackAction) -> String {
    match action {
        CallbackAction::Skip => "VisitResult.skip()".to_string(),
        CallbackAction::Continue => "VisitResult.continue_()".to_string(),
        CallbackAction::PreserveHtml => "VisitResult.preserveHtml()".to_string(),
        CallbackAction::Custom { output } => {
            format!("VisitResult.custom(field0: '{}')", escape_dart(output))
        }
        CallbackAction::CustomTemplate { template, return_form } => {
            // Convert `{placeholder}` segments to Dart string-interpolation
            // syntax (`${placeholder}`). Visitor method parameters are bound
            // in the enclosing closure so the interpolation resolves at
            // call-time. Template return form is ignored for Dart — the
            // bridge only carries `VisitResult::Custom(String)` and there is
            // no struct/dict variant.
            let _ = return_form;
            let mut interpolated = String::with_capacity(template.len());
            for ch in template.chars() {
                match ch {
                    '{' => interpolated.push_str("${"),
                    '\\' => interpolated.push_str("\\\\"),
                    '\'' => interpolated.push_str("\\'"),
                    '\n' => interpolated.push_str("\\n"),
                    '\r' => interpolated.push_str("\\r"),
                    '\t' => interpolated.push_str("\\t"),
                    other => interpolated.push(other),
                }
            }
            let _ = method;
            format!("VisitResult.custom(field0: '{interpolated}')")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::TemplateReturnForm;
    use std::collections::HashMap;

    fn spec(method: &str, action: CallbackAction) -> VisitorSpec {
        let mut callbacks = HashMap::new();
        callbacks.insert(method.to_string(), action);
        VisitorSpec { callbacks }
    }

    #[test]
    fn build_dart_visitor_emits_visitor_variable() {
        let mut lines = Vec::new();
        let name = build_dart_visitor(
            &mut lines,
            &spec(
                "visit_audio",
                CallbackAction::Custom {
                    output: "[AUDIO]".to_string(),
                },
            ),
        );
        assert_eq!(name, "_visitor");
        assert_eq!(lines.len(), 1);
        let block = &lines[0];
        assert!(block.contains("createHtmlVisitor("), "got: {block}");
        assert!(block.contains("visitAudio:"), "got: {block}");
        assert!(block.contains("VisitResult.custom(field0: '[AUDIO]')"), "got: {block}");
        // Methods without fixture callbacks default to `continue_()`.
        assert!(block.contains("visitText:"), "got: {block}");
        assert!(block.contains("VisitResult.continue_()"), "got: {block}");
    }

    #[test]
    fn build_dart_visitor_maps_skip_to_skip_variant() {
        let mut lines = Vec::new();
        build_dart_visitor(&mut lines, &spec("visit_button", CallbackAction::Skip));
        assert!(lines[0].contains("VisitResult.skip()"), "got: {}", lines[0]);
    }

    #[test]
    fn build_dart_visitor_maps_continue_to_continue_variant() {
        let mut lines = Vec::new();
        build_dart_visitor(&mut lines, &spec("visit_strong", CallbackAction::Continue));
        // Continue is the default, so we can't distinguish — but the method
        // body should still be `continue_()` (the action mirrors the default).
        assert!(lines[0].contains("visitStrong: (ctx, text) async => VisitResult.continue_()"));
    }

    #[test]
    fn build_dart_visitor_interpolates_custom_template() {
        let mut lines = Vec::new();
        build_dart_visitor(
            &mut lines,
            &spec(
                "visit_link",
                CallbackAction::CustomTemplate {
                    template: "[LINK:{text}:{href}]".to_string(),
                    return_form: TemplateReturnForm::Dict,
                },
            ),
        );
        assert!(
            lines[0].contains("VisitResult.custom(field0: '[LINK:${text}:${href}]')"),
            "got: {}",
            lines[0]
        );
    }
}
