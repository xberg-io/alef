//! Python visitor method generation for e2e test callbacks.

use crate::e2e::escape::escape_python;
use crate::e2e::fixture::{CallbackAction, TemplateReturnForm};

use super::visitor_context::VisitorContextProbe;

/// Emit the `_TestVisitor` members that record whether each callback's context argument exposes
/// the surface the generated binding declares for it.
///
/// One helper per distinct context type, so a crate with several visitor bridges probes each
/// callback against its own bridge's context type instead of one globally-chosen one.
///
/// The probe is a recorder rather than an in-callback assertion on purpose: every generated
/// visitor bridge catches host exceptions and substitutes the default visit result, so an
/// `AttributeError` raised inside a callback is invisible to the test process. The test body
/// asserts on the recorded results after the call returns
/// (see [`emit_python_visitor_context_assertions`]). ~keep
pub(super) fn emit_python_visitor_context_probes(out: &mut String, probes: &[&VisitorContextProbe]) {
    if probes.is_empty() {
        return;
    }
    out.push_str(&crate::e2e::template_env::render(
        "python/visitor_context_probe.jinja",
        crate::alef_context! { probes => probes },
    ));
}

/// Emit the test-body assertions over the probes recorded by [`emit_python_visitor_context_probes`].
pub(super) fn emit_python_visitor_context_assertions(out: &mut String) {
    out.push_str(&crate::e2e::template_env::render(
        "python/visitor_context_assertions.jinja",
        crate::alef_context! {},
    ));
}

/// Emit a Python visitor method for a callback action.
pub(super) fn emit_python_visitor_method(
    out: &mut String,
    method_name: &str,
    action: &CallbackAction,
    probe_method: Option<&str>,
) {
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

    // Pre-compute action type and values
    let (action_type, action_value, action_template, return_form) = match action {
        CallbackAction::Skip => ("skip", String::new(), String::new(), "dict"),
        CallbackAction::Continue => ("continue", String::new(), String::new(), "dict"),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), String::new(), "dict"),
        CallbackAction::Custom { output } => {
            let escaped = escape_python(output);
            ("custom", escaped, String::new(), "dict")
        }
        CallbackAction::CustomTemplate { template, return_form } => {
            let escaped_template = template
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            let form = match return_form {
                TemplateReturnForm::Dict => "dict",
                TemplateReturnForm::BareString => "bare_string",
            };
            ("custom_template", String::new(), escaped_template, form)
        }
    };

    // A002 (argument shadows a builtin) only applies to `visit_heading`'s `id` parameter —
    // every other callback's parameter list is builtin-clean, so blanket-suppressing A002
    // on every visitor method left it an unused, RUF100-dirty directive on all the others. ~keep
    let needs_a002 = params.split(", ").any(|p| p == "id");

    let rendered = crate::e2e::template_env::render(
        "python/visitor_method.jinja",
        crate::alef_context! {
            method_name => method_name,
            params => params,
            needs_a002 => needs_a002,
            action_type => action_type,
            action_value => action_value,
            action_template => action_template,
            return_form => return_form,
            probe_method => probe_method,
        },
    );
    out.push_str(&rendered);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(probe_method: &str, attributes: &[&str], methods: &[&str]) -> VisitorContextProbe {
        VisitorContextProbe {
            probe_method: probe_method.to_string(),
            attributes: attributes.iter().map(|name| (*name).to_string()).collect(),
            methods: methods.iter().map(|name| (*name).to_string()).collect(),
        }
    }

    #[test]
    fn emit_python_visitor_method_skip_returns_skip() {
        let mut out = String::new();
        emit_python_visitor_method(&mut out, "visit_text", &CallbackAction::Skip, None);
        assert!(out.contains("return \"skip\""), "got: {out}");
    }

    #[test]
    fn emit_python_visitor_method_uses_method_name_as_is() {
        let mut out = String::new();
        emit_python_visitor_method(&mut out, "visit_list_item", &CallbackAction::Continue, None);
        assert!(out.contains("visit_list_item"), "got: {out}");
    }

    #[test]
    fn emit_python_custom_uses_adjacent_tag_and_output_payload() {
        let mut out = String::new();
        emit_python_visitor_method(
            &mut out,
            "visit_text",
            &CallbackAction::Custom {
                output: "replacement".to_string(),
            },
            None,
        );
        assert!(out.contains(r#"return {"type": "custom", "output": "replacement"}"#));
    }

    /// Without this call the generated visitor never touches `ctx`, so the whole e2e suite runs
    /// green no matter what shape the bridge hands the callback. ~keep
    #[test]
    fn emit_python_visitor_method_dereferences_context_when_probing() {
        let mut out = String::new();
        emit_python_visitor_method(
            &mut out,
            "visit_text",
            &CallbackAction::Skip,
            Some("_probe_node_context"),
        );
        assert!(out.contains("self._probe_node_context(ctx)"), "got: {out}");
    }

    /// A crate with two visitor bridges gets two probe helpers, and each callback calls its own.
    /// A single shared helper would check one bridge's context type against every fixture. ~keep
    #[test]
    fn emit_python_visitor_context_probes_emits_one_helper_per_context_type() {
        let first = probe("_probe_node_context", &["node_type"], &["attributes"]);
        let second = probe("_probe_frame_context", &["frame_id"], &[]);
        let mut out = String::new();
        emit_python_visitor_context_probes(&mut out, &[&first, &second]);

        assert!(out.contains("def _probe_node_context(self, ctx)"), "got: {out}");
        assert!(out.contains("def _probe_frame_context(self, ctx)"), "got: {out}");
        assert!(out.contains("\"node_type\","), "got: {out}");
        assert!(out.contains("\"frame_id\","), "got: {out}");
        assert_eq!(
            out.matches("def __init__").count(),
            1,
            "one recorder for both probes: {out}"
        );
    }

    /// `getattr` alone proves a name resolves, not that the declared method works -- a mapping
    /// answers `getattr(ctx, "items")` with its own `dict.items`. The probe must call. ~keep
    #[test]
    fn emit_python_visitor_context_probes_calls_zero_arg_methods() {
        let only = probe("_probe_node_context", &["node_type"], &["attributes"]);
        let mut out = String::new();
        emit_python_visitor_context_probes(&mut out, &[&only]);

        assert!(
            out.contains("getattr(ctx, name)()"),
            "declared methods must be called: {out}"
        );
        assert!(
            out.contains("getattr(ctx, name)\n"),
            "plain attributes must still be read: {out}"
        );
        assert!(out.contains("except AttributeError as exc:"), "got: {out}");
        assert!(out.contains("self.context_reads += 1"), "got: {out}");
    }

    #[test]
    fn emit_python_visitor_context_probes_is_silent_without_probes() {
        let mut out = String::new();
        emit_python_visitor_context_probes(&mut out, &[]);
        assert_eq!(out, "");
    }

    /// Calling a declared method is still not enough. When the declared names are `dict` API
    /// names, `getattr(ctx, "values")` and `getattr(ctx, "items")()` both succeed on the
    /// bridge's fallback dict, so every name probe passes on the wrong shape. The
    /// name-independent shape check has to run first, and it has to run on every probe. ~keep
    #[test]
    fn emit_python_visitor_context_probes_checks_the_shape_before_any_name() {
        let first = probe("_probe_node_context", &["values"], &["items"]);
        let second = probe("_probe_frame_context", &["frame_id"], &[]);
        let mut out = String::new();
        emit_python_visitor_context_probes(&mut out, &[&first, &second]);

        assert_eq!(
            out.matches("def _record_dict_shaped_context").count(),
            1,
            "one shape check shared by every probe: {out}"
        );
        assert_eq!(
            out.matches("if self._record_dict_shaped_context(").count(),
            2,
            "every probe must run the shape check, not just the first: {out}"
        );
        assert!(out.contains("if not isinstance(ctx, dict):"), "got: {out}");

        let shape_check = out
            .find(r#"self._record_dict_shaped_context("_probe_node_context", ctx)"#)
            .expect("the first probe must run the shape check");
        let first_name = out
            .find(r#""values","#)
            .expect("the first probe must read its declared field");
        assert!(
            shape_check < first_name,
            "the shape check must precede the name probes, or a colliding dict passes them first: {out}"
        );
    }

    /// The recorded failures have to be asserted in the test body: the bridge swallows callback
    /// exceptions, so an assertion inside the callback can never fail a test. ~keep
    #[test]
    fn emit_python_visitor_context_assertions_fails_the_test_on_recorded_errors() {
        let mut out = String::new();
        emit_python_visitor_context_assertions(&mut out);
        assert!(out.contains("assert not _visitor.context_errors"), "got: {out}");
        assert!(out.contains("assert _visitor.context_reads > 0"), "got: {out}");
    }
}
