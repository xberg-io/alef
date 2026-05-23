//! Swift visitor class emission for e2e test callbacks.
//!
//! The swift `HtmlVisitor` trait is bridged via the `HtmlVisitorProtocol` protocol and
//! `SwiftHtmlVisitorBox` class emitted by `alef-backend-swift`. Each visitor-bearing
//! fixture emits a local `final class LocalVisitor_<id>: HtmlVisitorProtocol { ... }`
//! that overrides the methods specified in the fixture's `visitor` configuration.
//! All other methods inherit the default implementation (returning `.continue_`).
//!
//! The canonical method list mirrors `dart_visitors.rs` — both are sourced from
//! `crates/html-to-markdown/src/visitor/traits.rs`.

use crate::e2e::fixture::{CallbackAction, VisitorSpec};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

/// All HtmlVisitor callback methods (Rust snake_case names) that
/// `HtmlVisitorProtocol` exposes. Order is stable across regenerations.
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

/// Build a visitor-handle setup block and append it to `setup_lines`.
/// Returns the Swift expression that evaluates to a `VisitorHandle`.
///
/// The emitted block:
/// 1. Declares `final class LocalVisitor_<fixture_id_camel>: HtmlVisitorProtocol` with
///    overrides for every method listed in `visitor_spec.callbacks`.
/// 2. Returns the expression `makeHtmlVisitorHandle(LocalVisitor_<id>())`.
pub(super) fn build_swift_visitor(
    setup_lines: &mut Vec<String>,
    visitor_spec: &VisitorSpec,
    fixture_id: &str,
) -> String {
    // Build a Swift-safe class name from the fixture id.
    let class_suffix = fixture_id.replace('-', "_").to_upper_camel_case();
    let class_name = format!("LocalVisitor_{class_suffix}");

    let mut block = String::new();
    let _ = writeln!(block, "final class {class_name}: HtmlVisitorProtocol {{");

    // Only emit method overrides for methods the fixture configures.
    for method in ALL_VISITOR_METHODS {
        let Some(action) = visitor_spec.callbacks.get(*method) else {
            continue; // default (`continue_`) — no override needed
        };

        let method_camel = method.to_lower_camel_case();
        let params = swift_visitor_params(method);
        let body = swift_action_body(action, method);
        let _ = writeln!(
            block,
            "    func {method_camel}({params}) -> VisitResult {{ return {body} }}"
        );
    }

    let _ = writeln!(block, "}}");
    setup_lines.push(block);

    format!("makeHtmlVisitorHandle({class_name}())")
}

/// Swift parameter list for each HtmlVisitor method on the protocol.
/// Types match what `emit_inbound_protocols` emits into the generated `HtmlVisitorProtocol`.
fn swift_visitor_params(method: &str) -> &'static str {
    match method {
        "visit_text" => "_ ctx: NodeContext, _ text: String",
        "visit_element_start" => "_ ctx: NodeContext",
        "visit_element_end" => "_ ctx: NodeContext, _ output: String",
        "visit_link" => "_ ctx: NodeContext, _ href: String, _ text: String, _ title: String?",
        "visit_image" => "_ ctx: NodeContext, _ src: String, _ alt: String, _ title: String?",
        "visit_heading" => "_ ctx: NodeContext, _ level: UInt32, _ text: String, _ id: String?",
        "visit_code_block" => "_ ctx: NodeContext, _ lang: String?, _ code: String",
        "visit_code_inline" => "_ ctx: NodeContext, _ code: String",
        "visit_list_item" => "_ ctx: NodeContext, _ ordered: Bool, _ marker: String, _ text: String",
        "visit_list_start" => "_ ctx: NodeContext, _ ordered: Bool",
        "visit_list_end" => "_ ctx: NodeContext, _ ordered: Bool, _ output: String",
        "visit_table_start" => "_ ctx: NodeContext",
        "visit_table_row" => "_ ctx: NodeContext, _ cells: RustVec<RustString>, _ isHeader: Bool",
        "visit_table_end" => "_ ctx: NodeContext, _ output: String",
        "visit_blockquote" => "_ ctx: NodeContext, _ content: String, _ depth: Int",
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
        | "visit_definition_description" => "_ ctx: NodeContext, _ text: String",
        "visit_line_break" | "visit_horizontal_rule" | "visit_definition_list_start" | "visit_figure_start" => {
            "_ ctx: NodeContext"
        }
        "visit_definition_list_end" => "_ ctx: NodeContext, _ output: String",
        "visit_custom_element" => "_ ctx: NodeContext, _ tagName: String, _ html: String",
        "visit_form" => "_ ctx: NodeContext, _ action: String?, _ method: String?",
        "visit_input" => "_ ctx: NodeContext, _ inputType: String, _ name: String?, _ value: String?",
        "visit_audio" | "visit_video" | "visit_iframe" => "_ ctx: NodeContext, _ src: String?",
        "visit_details" => "_ ctx: NodeContext, _ open: Bool",
        "visit_figure_end" => "_ ctx: NodeContext, _ output: String",
        _ => "_ ctx: NodeContext",
    }
}

/// Returns `true` if the named parameter on the given visitor method is
/// declared with an Optional type in the swift `HtmlVisitorProtocol`.
///
/// Swift string interpolation of `Optional<T>` renders as `Optional("value")`
/// (or `nil`), which sabotages fixture templates like `[VIDEO: {src}]` where
/// the asserted output is `[VIDEO: tutorial.mp4]`. For optional params we emit
/// `\(name ?? "")` instead of `\(name)` so the unwrapped string flows in.
///
/// The list mirrors the `?` suffix in [`swift_visitor_params`].
fn swift_visitor_param_is_optional(method: &str, snake_param: &str) -> bool {
    matches!(
        (method, snake_param),
        ("visit_link", "title")
            | ("visit_image", "title")
            | ("visit_heading", "id")
            | ("visit_code_block", "lang")
            | ("visit_form", "action")
            | ("visit_form", "method")
            | ("visit_input", "name")
            | ("visit_input", "value")
            | ("visit_audio", "src")
            | ("visit_video", "src")
            | ("visit_iframe", "src")
    )
}

/// Render the Swift expression for a fixture-driven callback action.
///
/// Variant naming mirrors the swift-backend emission of `VisitResult` (see
/// `alef-backend-swift::gen_bindings`):
/// - Unit variants are emitted by `swift_case_ident` (backtick-escape reserved
///   keywords, plain camelCase otherwise). `continue` is a Swift keyword so the
///   case is `` `continue` ``.
/// - Tuple variants with a single field carry the synthesised label `field0:`
///   in the Swift enum (`case custom(field0: String)`), so call sites MUST
///   provide that label.
fn swift_action_body(action: &CallbackAction, method: &str) -> String {
    match action {
        CallbackAction::Skip => ".skip".to_string(),
        CallbackAction::Continue => ".`continue`".to_string(),
        CallbackAction::PreserveHtml => ".preserveHtml".to_string(),
        CallbackAction::Custom { output } => {
            let escaped = escape_swift_str(output);
            format!(".custom(field0: \"{escaped}\")")
        }
        CallbackAction::CustomTemplate {
            template,
            return_form: _,
        } => {
            // Swift string interpolation: `{placeholder}` → `\(placeholder_camelCase)`,
            // or `\(placeholder_camelCase ?? "")` when the placeholder names an
            // Optional<String> parameter on the visitor method.
            let mut interpolated = String::with_capacity(template.len());
            let mut chars = template.chars().peekable();
            while let Some(ch) = chars.next() {
                match ch {
                    '{' => {
                        let mut name = String::new();
                        while let Some(&peek) = chars.peek() {
                            if peek == '}' {
                                chars.next();
                                break;
                            }
                            name.push(peek);
                            chars.next();
                        }
                        // Convert to camelCase to match Swift parameter names.
                        let is_optional = swift_visitor_param_is_optional(method, &name);
                        interpolated.push_str("\\(");
                        interpolated.push_str(&name.to_lower_camel_case());
                        if is_optional {
                            interpolated.push_str(" ?? \"\"");
                        }
                        interpolated.push(')');
                    }
                    '\\' => interpolated.push_str("\\\\"),
                    '"' => interpolated.push_str("\\\""),
                    '\n' => interpolated.push_str("\\n"),
                    '\r' => interpolated.push_str("\\r"),
                    '\t' => interpolated.push_str("\\t"),
                    other => interpolated.push(other),
                }
            }
            format!(".custom(field0: \"{interpolated}\")")
        }
    }
}

/// Escape a string for use as a Swift string literal (double-quoted).
fn escape_swift_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::fixture::TemplateReturnForm;
    use std::collections::BTreeMap;

    fn spec(method: &str, action: CallbackAction) -> VisitorSpec {
        let mut callbacks = BTreeMap::new();
        callbacks.insert(method.to_string(), action);
        VisitorSpec { callbacks }
    }

    #[test]
    fn build_swift_visitor_returns_make_handle_expr() {
        let mut lines = Vec::new();
        let expr = build_swift_visitor(
            &mut lines,
            &spec(
                "visit_audio",
                CallbackAction::Custom {
                    output: "[AUDIO]".to_string(),
                },
            ),
            "audio_skip",
        );
        assert!(expr.starts_with("makeHtmlVisitorHandle("), "got: {expr}");
        assert_eq!(lines.len(), 1);
        let block = &lines[0];
        assert!(block.contains("LocalVisitor_AudioSkip"), "got: {block}");
        assert!(block.contains("HtmlVisitorProtocol"), "got: {block}");
        assert!(block.contains("visitAudio"), "got: {block}");
        assert!(block.contains(".custom(field0: \"[AUDIO]\")"), "got: {block}");
    }

    #[test]
    fn build_swift_visitor_skip_action() {
        let mut lines = Vec::new();
        build_swift_visitor(&mut lines, &spec("visit_button", CallbackAction::Skip), "btn_skip");
        assert!(lines[0].contains(".skip"), "got: {}", lines[0]);
    }

    #[test]
    fn build_swift_visitor_preserve_html_action() {
        let mut lines = Vec::new();
        build_swift_visitor(
            &mut lines,
            &spec("visit_iframe", CallbackAction::PreserveHtml),
            "iframe_preserve",
        );
        assert!(lines[0].contains(".preserveHtml"), "got: {}", lines[0]);
    }

    #[test]
    fn build_swift_visitor_continue_action() {
        let mut lines = Vec::new();
        build_swift_visitor(
            &mut lines,
            &spec("visit_strong", CallbackAction::Continue),
            "strong_cont",
        );
        assert!(lines[0].contains(".`continue`"), "got: {}", lines[0]);
    }

    #[test]
    fn build_swift_visitor_template_interpolation() {
        let mut lines = Vec::new();
        build_swift_visitor(
            &mut lines,
            &spec(
                "visit_link",
                CallbackAction::CustomTemplate {
                    template: "[LINK:{text}:{href}]".to_string(),
                    return_form: TemplateReturnForm::Dict,
                },
            ),
            "link_template",
        );
        // Placeholder names should be camelCased and use Swift interpolation syntax.
        assert!(
            lines[0].contains(".custom(field0: \"[LINK:\\(text):\\(href)]\""),
            "got: {}",
            lines[0]
        );
    }

    #[test]
    fn build_swift_visitor_no_override_for_unconfigured_methods() {
        // A spec with one method should not emit override for others.
        let mut lines = Vec::new();
        build_swift_visitor(&mut lines, &spec("visit_text", CallbackAction::Skip), "text_only");
        let block = &lines[0];
        // Only visit_text is overridden.
        assert!(block.contains("visitText"), "got: {block}");
        // visitAudio is NOT in the spec — no override should appear.
        assert!(!block.contains("visitAudio"), "got: {block}");
    }
}
