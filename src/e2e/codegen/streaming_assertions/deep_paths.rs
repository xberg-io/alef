use super::model::{StreamingFieldResolver, split_streaming_deep_path};
use super::python_typeddict_accessor::{python_tool_call_element_type, render_python_tool_calls_deep};
use super::renderers::{
    render_deep_tail, render_optional_chain_tail, render_rust_tool_calls_deep, render_swift_tool_calls_deep,
    render_zig_tool_calls_deep,
};
use crate::e2e::field_access::PythonTypedDictMap;

pub(super) fn render(
    field: &str,
    lang: &str,
    chunks_var: &str,
    module_qualifier: Option<&str>,
    item_type: Option<&str>,
    typeddict_map: Option<&PythonTypedDictMap>,
) -> Option<String> {
    let (root, tail) = split_streaming_deep_path(field)?;
    match (root, lang) {
        ("usage", _) => render_usage_deep(chunks_var, tail, lang),
        ("tool_calls", "rust") => Some(render_rust_tool_calls_deep(chunks_var, tail)),
        ("tool_calls", "swift") => Some(render_swift_tool_calls_deep(
            &StreamingFieldResolver::accessor(root, lang, chunks_var)?,
            tail,
        )),
        ("tool_calls", "zig") => render_zig_tool_calls_deep(chunks_var, tail),
        ("tool_calls", "python") => {
            let expression = StreamingFieldResolver::accessor_with_typeddict_map(
                root,
                lang,
                chunks_var,
                module_qualifier,
                item_type,
                typeddict_map,
            )?;
            let element = python_tool_call_element_type(item_type, typeddict_map);
            Some(render_python_tool_calls_deep(
                &expression,
                tail,
                element.as_deref(),
                typeddict_map,
            ))
        }
        _ => Some(render_deep_tail(
            &StreamingFieldResolver::accessor(root, lang, chunks_var)?,
            tail,
            lang,
        )),
    }
}

/// Render a path below the stream-level `usage` root, such as `usage.total_tokens`.
///
/// Every backend reads the counter off the LAST chunk of the collected list, which is where a
/// stream carries its usage payload. The base `usage` accessor cannot take a field in Rust (an
/// `Option`), Go (an untyped `interface{}`), Swift and Dart (an optional): those four build their
/// own root. Zig never reaches this function: it parses the last chunk's JSON and asserts through
/// its JSON-struct path (`zig/assertions.rs`).
fn render_usage_deep(chunks_var: &str, tail: &str, lang: &str) -> Option<String> {
    match lang {
        "rust" => {
            let root = format!(
                "{chunks_var}.last().and_then(|c| c.usage.as_ref()).expect(\"expected usage on the last chunk\")"
            );
            Some(render_deep_tail(&root, tail, lang))
        }
        "go" => {
            let root = format!("{chunks_var}[len({chunks_var})-1].Usage");
            Some(render_deep_tail(&root, tail, lang))
        }
        // `Usage?` on the chunk: a missing chunk or a missing usage reads as `nil` and fails the
        // comparison instead of trapping.
        "swift" => Some(render_optional_chain_tail(
            &format!("{chunks_var}.last?.usage"),
            tail,
            crate::core::config::Language::Swift,
        )),
        "dart" => {
            let root = StreamingFieldResolver::accessor("usage", lang, chunks_var)?;
            Some(render_optional_chain_tail(
                &root,
                tail,
                crate::core::config::Language::Dart,
            ))
        }
        _ => {
            let root = StreamingFieldResolver::accessor("usage", lang, chunks_var)?;
            Some(render_deep_tail(&root, tail, lang))
        }
    }
}
