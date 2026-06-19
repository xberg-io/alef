//! Swift visitor class emission for e2e test callbacks.
//!
//! Swift visitor traits are bridged via the generated protocol and box class emitted
//! by `alef-backend-swift`. Each visitor-bearing fixture emits a local visitor class
//! that overrides the methods specified in the fixture's `visitor` configuration.
//! All other methods inherit the generated protocol's default implementation.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{MethodDef, TypeRef};
use crate::e2e::fixture::{CallbackAction, VisitorSpec};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

pub(super) struct SwiftVisitorConfig {
    pub(super) trait_name: String,
    pub(super) context_type: String,
    pub(super) result_type: String,
    pub(super) methods: Vec<String>,
}

pub(super) fn resolve_swift_visitor_config(
    config: &ResolvedCrateConfig,
    call_override: Option<&crate::e2e::config::CallOverride>,
    type_defs: &[crate::core::ir::TypeDef],
    visitor_spec: &VisitorSpec,
) -> SwiftVisitorConfig {
    let trait_name = call_override
        .and_then(|override_config| override_config.visitor_trait.clone())
        .or_else(|| {
            type_defs
                .iter()
                .find(|type_def| {
                    type_def.is_trait && visitor_spec.callbacks.keys().any(|name| has_method(type_def, name))
                })
                .map(|type_def| type_def.name.clone())
        })
        .unwrap_or_else(|| "Visitor".to_string());

    let methods = type_defs
        .iter()
        .find(|type_def| type_def.name == trait_name)
        .map(|type_def| type_def.methods.iter().map(|method| method.name.clone()).collect())
        .unwrap_or_else(|| visitor_spec.callbacks.keys().cloned().collect());

    let bridge = config
        .trait_bridges
        .iter()
        .find(|bridge| bridge.trait_name == trait_name);
    let callback_methods = callback_methods(type_defs, visitor_spec, &trait_name);
    let context_type = bridge
        .and_then(|bridge| bridge.context_type.clone())
        .or_else(|| {
            callback_methods
                .iter()
                .find_map(|method| first_named_param_type(method))
        })
        .unwrap_or_else(|| "VisitorContext".to_string());
    let result_type = bridge
        .and_then(|bridge| bridge.result_type.clone())
        .or_else(|| {
            callback_methods
                .iter()
                .find_map(|method| named_type(&method.return_type))
        })
        .unwrap_or_else(|| "VisitorResult".to_string());

    SwiftVisitorConfig {
        trait_name,
        context_type,
        result_type,
        methods,
    }
}

fn has_method(type_def: &crate::core::ir::TypeDef, method_name: &str) -> bool {
    type_def.methods.iter().any(|method| method.name == method_name)
}

fn callback_methods<'a>(
    type_defs: &'a [crate::core::ir::TypeDef],
    visitor_spec: &VisitorSpec,
    trait_name: &str,
) -> Vec<&'a MethodDef> {
    type_defs
        .iter()
        .find(|type_def| type_def.name == trait_name)
        .map(|type_def| {
            visitor_spec
                .callbacks
                .keys()
                .filter_map(|name| type_def.methods.iter().find(|method| method.name == *name))
                .collect()
        })
        .unwrap_or_default()
}

fn first_named_param_type(method: &MethodDef) -> Option<String> {
    method.params.iter().find_map(|param| named_type(&param.ty))
}

fn named_type(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(name) => Some(name.clone()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => named_type(inner),
        TypeRef::Map(key, value) => named_type(key).or_else(|| named_type(value)),
        _ => None,
    }
}

/// Build a visitor-handle setup block and append it to `setup_lines`.
/// Returns the Swift expression that evaluates to a `VisitorHandle`.
///
/// The emitted block:
/// 1. Declares `final class LocalVisitor_<fixture_id_camel>: <TraitName>Protocol` with
///    overrides for every method listed in `visitor_spec.callbacks`.
/// 2. Returns the generated visitor-handle factory expression.
pub(super) fn build_swift_visitor(
    setup_lines: &mut Vec<String>,
    visitor_spec: &VisitorSpec,
    fixture_id: &str,
    visitor_config: &SwiftVisitorConfig,
    module_name: &str,
) -> String {
    // Build a Swift-safe class name from the fixture id.
    let class_suffix = fixture_id.replace('-', "_").to_upper_camel_case();
    let class_name = format!("LocalVisitor_{class_suffix}");

    // The test file imports both the host module (for example, `SampleModule`) and
    // `RustBridge`. Visitor context types like `NodeContext` are emitted into
    // both modules (the user-facing wrapper in the host, the swift-bridge raw
    // type in RustBridge), so an unqualified reference is ambiguous and Swift
    // rejects the test with `'NodeContext' is ambiguous for type lookup`.
    // Qualifying the context type with the host module name resolves it.
    let qualified_context = format!("{module_name}.{}", visitor_config.context_type);

    let mut block = String::new();
    let protocol_name = format!("{}Protocol", visitor_config.trait_name);
    if visitor_config.context_type == "VisitorContext" || visitor_config.result_type == "VisitorResult" {
        let _ = writeln!(
            block,
            "#error(\"Swift visitor fixtures require trait_bridge context_type and result_type metadata; add it to alef.toml or skip visitor fixtures for Swift\")"
        );
    }
    let _ = writeln!(block, "final class {class_name}: {protocol_name} {{");

    // Only emit method overrides for methods the fixture configures.
    for method in &visitor_config.methods {
        let Some(action) = visitor_spec.callbacks.get(method.as_str()) else {
            continue; // default (`continue_`) — no override needed
        };

        let method_camel = method.to_lower_camel_case();
        let params = swift_visitor_params(method, &qualified_context);
        let body = swift_action_body(action, method, &visitor_config.result_type);
        let _ = writeln!(
            block,
            "    func {method_camel}({params}) -> {} {{ return {body} }}",
            visitor_config.result_type
        );
    }

    let _ = writeln!(block, "}}");
    setup_lines.push(block);

    format!(
        "make{}Handle({class_name}())",
        visitor_config.trait_name.to_upper_camel_case()
    )
}

/// Swift parameter list for visitor methods on the protocol.
/// Known visitor-special associated types use concrete signatures; unknown methods
/// fall back to the single context parameter shape used by defaultable callbacks.
fn swift_visitor_params(method: &str, context_type: &str) -> String {
    let template = match method {
        "visit_text" => "_ ctx: {context_type}, _ text: String",
        "visit_element_start" => "_ ctx: {context_type}",
        "visit_element_end" => "_ ctx: {context_type}, _ output: String",
        "visit_link" => "_ ctx: {context_type}, _ href: String, _ text: String, _ title: String?",
        "visit_image" => "_ ctx: {context_type}, _ src: String, _ alt: String, _ title: String?",
        "visit_heading" => "_ ctx: {context_type}, _ level: UInt32, _ text: String, _ id: String?",
        "visit_code_block" => "_ ctx: {context_type}, _ lang: String?, _ code: String",
        "visit_code_inline" => "_ ctx: {context_type}, _ code: String",
        "visit_list_item" => "_ ctx: {context_type}, _ ordered: Bool, _ marker: String, _ text: String",
        "visit_list_start" => "_ ctx: {context_type}, _ ordered: Bool",
        "visit_list_end" => "_ ctx: {context_type}, _ ordered: Bool, _ output: String",
        "visit_table_start" => "_ ctx: {context_type}",
        "visit_table_row" => "_ ctx: {context_type}, _ cells: RustVec<RustString>, _ isHeader: Bool",
        "visit_table_end" => "_ ctx: {context_type}, _ output: String",
        "visit_blockquote" => "_ ctx: {context_type}, _ content: String, _ depth: UInt",
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
        | "visit_definition_description" => "_ ctx: {context_type}, _ text: String",
        "visit_line_break" | "visit_horizontal_rule" | "visit_definition_list_start" | "visit_figure_start" => {
            "_ ctx: {context_type}"
        }
        "visit_definition_list_end" => "_ ctx: {context_type}, _ output: String",
        "visit_custom_element" => "_ ctx: {context_type}, _ tagName: String, _ html: String",
        "visit_form" => "_ ctx: {context_type}, _ action: String?, _ method: String?",
        "visit_input" => "_ ctx: {context_type}, _ inputType: String, _ name: String?, _ value: String?",
        "visit_audio" | "visit_video" | "visit_iframe" => "_ ctx: {context_type}, _ src: String?",
        "visit_details" => "_ ctx: {context_type}, _ open: Bool",
        "visit_figure_end" => "_ ctx: {context_type}, _ output: String",
        _ => "_ ctx: {context_type}",
    };
    template.replace("{context_type}", context_type)
}

/// Returns `true` if the named parameter on the given visitor method is
/// declared with an Optional type in the generated visitor protocol.
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
/// Variant naming mirrors the swift-backend emission for simple result enums:
/// - Unit variants are emitted by `swift_case_ident` (backtick-escape reserved
///   keywords, plain camelCase otherwise). `continue` is a Swift keyword so the
///   case is `` `continue` ``.
/// - Tuple variants with a single field carry the synthesised label `field0:`
///   in the Swift enum (`case custom(field0: String)`), so call sites MUST
///   provide that label.
fn swift_action_body(action: &CallbackAction, method: &str, result_type: &str) -> String {
    if result_type == "VisitorResult" {
        return format!(
            "fatalError(\"Swift visitor fixture callback {method} requires explicit e2e result-action metadata for result type {result_type}\")"
        );
    }
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

    fn visitor_config(methods: &[&str]) -> SwiftVisitorConfig {
        SwiftVisitorConfig {
            trait_name: "RenderVisitor".to_string(),
            context_type: "SyntaxContext".to_string(),
            result_type: "WalkDecision".to_string(),
            methods: methods.iter().map(|method| method.to_string()).collect(),
        }
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
            &visitor_config(&["visit_audio"]),
            "MyModule",
        );
        assert!(expr.starts_with("makeRenderVisitorHandle("), "got: {expr}");
        assert_eq!(lines.len(), 1);
        let block = &lines[0];
        assert!(block.contains("LocalVisitor_AudioSkip"), "got: {block}");
        assert!(block.contains("RenderVisitorProtocol"), "got: {block}");
        assert!(block.contains("visitAudio"), "got: {block}");
        assert!(block.contains("_ ctx: MyModule.SyntaxContext"), "got: {block}");
        assert!(block.contains(".custom(field0: \"[AUDIO]\")"), "got: {block}");
    }

    #[test]
    fn build_swift_visitor_skip_action() {
        let mut lines = Vec::new();
        build_swift_visitor(
            &mut lines,
            &spec("visit_button", CallbackAction::Skip),
            "btn_skip",
            &visitor_config(&["visit_button"]),
            "MyModule",
        );
        assert!(lines[0].contains(".skip"), "got: {}", lines[0]);
    }

    #[test]
    fn build_swift_visitor_preserve_html_action() {
        let mut lines = Vec::new();
        build_swift_visitor(
            &mut lines,
            &spec("visit_iframe", CallbackAction::PreserveHtml),
            "iframe_preserve",
            &visitor_config(&["visit_iframe"]),
            "MyModule",
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
            &visitor_config(&["visit_strong"]),
            "MyModule",
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
            &visitor_config(&["visit_link"]),
            "MyModule",
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
        build_swift_visitor(
            &mut lines,
            &spec("visit_text", CallbackAction::Skip),
            "text_only",
            &visitor_config(&["visit_text", "visit_audio"]),
            "MyModule",
        );
        let block = &lines[0];
        // Only visit_text is overridden.
        assert!(block.contains("visitText"), "got: {block}");
        // visitAudio is NOT in the spec — no override should appear.
        assert!(!block.contains("visitAudio"), "got: {block}");
    }
}
