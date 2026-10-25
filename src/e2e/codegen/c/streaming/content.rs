//! Reads the first choice of each streamed chunk through the C ABI, so the fixture's
//! `stream_content`, `finish_reason`, `stream_complete`, `tool_calls` and
//! `tool_calls[0].function.name` assertions run instead of rendering a skip.
//!
//! The probe walks the IR the way the usage probe does: the chunk type must carry a `choices`
//! list and the choice type the field a pseudo-field reads (`delta` with a text `content`,
//! `finish_reason`, or a `delta` with a `tool_calls` list). When a link is missing the probe
//! stays empty for that field and the assertion keeps its registered skip.

use crate::core::ir::{TypeDef, TypeRef};
use crate::e2e::fixture::{Assertion, Fixture};

#[derive(Default)]
pub(super) struct ContentProbe {
    item: String,
    choice: String,
    delta: String,
    /// The delta text is read through the ABI and appended to `stream_content`.
    text: bool,
    /// The last non-null `finish_reason` of the first choice is kept.
    finish: bool,
    /// The tool calls of the first choice's delta are counted and the first call's name is kept.
    tools: bool,
}

/// The streaming pseudo-fields this probe reads from the chunks.
fn asserts(fixture: &Fixture, matches: impl Fn(&str) -> bool) -> bool {
    fixture
        .assertions
        .iter()
        .any(|assertion| assertion.field.as_deref().is_some_and(&matches))
}

impl ContentProbe {
    pub(super) fn has_content(&self) -> bool {
        !self.item.is_empty()
    }

    pub(super) fn resolve(fixture: &Fixture, types: &[TypeDef], item: &str) -> Self {
        let wants_text = asserts(fixture, |field| field == "stream_content");
        let wants_finish = asserts(fixture, |field| field == "finish_reason" || field == "stream_complete");
        let wants_tools = asserts(fixture, |field| {
            field == "tool_calls" || field.starts_with("tool_calls[") || field.starts_with("tool_calls.")
        });
        if !(wants_text || wants_finish || wants_tools) {
            return Self::default();
        }
        let find = |name: &str| types.iter().find(|ty| ty.name == name && !ty.binding_excluded);
        let Some(choice) = find(item)
            .and_then(|chunk| field_type(chunk, "choices"))
            .and_then(list_element)
            .and_then(find)
        else {
            return Self::default();
        };
        let delta = field_type(choice, "delta").and_then(named).and_then(find);
        let text = wants_text && delta.is_some_and(|delta| field_type(delta, "content").is_some_and(is_text));
        // The finish reason and the tool calls are read from the choice's JSON, so the wire
        // name must be the declared field name.
        let finish = wants_finish && has_plain_field(choice, "finish_reason");
        let tools = wants_tools
            && delta.is_some_and(|delta| {
                has_plain_field(delta, "tool_calls")
                    && field_type(delta, "tool_calls").is_some_and(|ty| list_element(ty).is_some())
            });
        if !(text || finish || tools) {
            return Self::default();
        }
        Self {
            item: item.to_string(),
            choice: choice.name.clone(),
            delta: delta.map(|delta| delta.name.clone()).unwrap_or_default(),
            text,
            finish,
            tools,
        }
    }

    /// Declares the buffers the loop fills. The loop frees nothing it does not own: the strings
    /// are released by [`Self::render_release`] after the assertions.
    pub(super) fn render_declarations(&self, out: &mut String) {
        if self.finish {
            out.push_str("    char *stream_finish_reason = NULL;\n");
        }
        if self.tools {
            out.push_str("    int stream_tool_calls_count = 0;\n");
            out.push_str("    char *stream_tool_call_name = NULL;\n");
        }
    }

    pub(super) fn render_release(&self, out: &mut String) {
        if self.finish {
            out.push_str("    free(stream_finish_reason);\n");
        }
        if self.tools {
            out.push_str("    free(stream_tool_call_name);\n    (void)stream_tool_calls_count;\n");
        }
    }

    /// Reads the first choice of `result` into the buffers. The text goes through the ABI's own
    /// accessors, so it arrives JSON-decoded.
    pub(super) fn render_capture(&self, out: &mut String, prefix: &str, result: &str, prefix_upper: &str) {
        if !self.has_content() {
            return;
        }
        let symbols =
            |type_name: &str, field: &str| crate::codegen::naming::ffi_field_accessor_symbol(prefix, type_name, field);
        out.push_str(&crate::e2e::template_env::render(
            "c/stream_content_capture.jinja",
            crate::alef_context! {
                prefix => prefix, prefix_upper => prefix_upper, result => result,
                text => self.text, finish => self.finish, tools => self.tools,
                choices_getter => symbols(&self.item, "choices"),
                choice_type => crate::codegen::naming::pascal_to_snake(&self.choice),
                delta_getter => symbols(&self.choice, "delta"),
                delta_type => crate::codegen::naming::pascal_to_snake(&self.delta),
                content_getter => symbols(&self.delta, "content"),
            },
        ));
    }

    /// Renders the assertion against the captured stream state. Returns `false` when the field
    /// was not read or the assertion type has no C spelling here, so the caller falls through to
    /// the registered skip.
    pub(super) fn render_assertion(&self, out: &mut String, assertion: &Assertion) -> bool {
        let Some(field) = assertion.field.as_deref() else {
            return false;
        };
        let kind = assertion.assertion_type.as_str();
        let text = assertion.value.as_ref().and_then(|value| value.as_str());
        let count = assertion.value.as_ref().and_then(|value| value.as_u64());
        let expression = match (field, kind) {
            ("stream_content", "equals") if self.text && text.is_some() => {
                format!(
                    "strcmp(stream_content, \"{}\") == 0",
                    escape_c(text.unwrap_or_default())
                )
            }
            ("stream_content", "contains") if self.text && text.is_some() => {
                format!(
                    "strstr(stream_content, \"{}\") != NULL",
                    escape_c(text.unwrap_or_default())
                )
            }
            ("stream_content", "not_empty") if self.text => "stream_content_len > 0".to_string(),
            ("finish_reason", "equals") if self.finish && text.is_some() => format!(
                "stream_finish_reason != NULL && strcmp(stream_finish_reason, \"{}\") == 0",
                escape_c(text.unwrap_or_default())
            ),
            ("finish_reason", "not_empty") if self.finish => "stream_finish_reason != NULL".to_string(),
            ("stream_complete", "is_true") if self.finish => "stream_finish_reason != NULL".to_string(),
            ("stream_complete", "is_false") if self.finish => "stream_finish_reason == NULL".to_string(),
            ("tool_calls", "not_empty") if self.tools => "stream_tool_calls_count > 0".to_string(),
            ("tool_calls", "count_equals") if self.tools && count.is_some() => {
                format!("stream_tool_calls_count == {}", count.unwrap_or_default())
            }
            ("tool_calls", "count_min") if self.tools && count.is_some() => {
                format!("stream_tool_calls_count >= {}", count.unwrap_or_default())
            }
            ("tool_calls[0].function.name", "equals") if self.tools && text.is_some() => format!(
                "stream_tool_call_name != NULL && strcmp(stream_tool_call_name, \"{}\") == 0",
                escape_c(text.unwrap_or_default())
            ),
            _ => return false,
        };
        out.push_str(&format!(
            "    assert(({expression}) && \"{kind} assertion failed for {field}\");\n"
        ));
        true
    }
}

fn escape_c(text: &str) -> String {
    crate::e2e::escape::escape_c(text)
}

fn has_plain_field(owner: &TypeDef, name: &str) -> bool {
    crate::codegen::shared::binding_fields(&owner.fields)
        .any(|field| field.name == name && field.serde_rename.is_none())
}

fn field_type<'a>(owner: &'a TypeDef, name: &str) -> Option<&'a TypeRef> {
    crate::codegen::shared::binding_fields(&owner.fields)
        .find(|field| field.name == name)
        .map(|field| &field.ty)
}

fn named(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => name.rsplit("::").next(),
        TypeRef::Optional(inner) => named(inner),
        _ => None,
    }
}

fn list_element(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Vec(inner) => named(inner),
        TypeRef::Optional(inner) => list_element(inner),
        _ => None,
    }
}

fn is_text(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String => true,
        TypeRef::Optional(inner) => is_text(inner),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
