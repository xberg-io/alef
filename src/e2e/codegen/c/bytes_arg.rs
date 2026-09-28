//! Lowers a fixture `type = "bytes"` argument to the two-parameter C shape the FFI generator
//! emits for any Rust `&[u8]` parameter: `(const uint8_t *ptr, uintptr_t len)`. See
//! `backends::ffi::gen_bindings::types.rs`, which appends a synthesized `{name}_len: usize`
//! parameter immediately after a `TypeRef::Bytes` parameter -- this module is the C call-site
//! mirror of that same contract. Every other backend (`go/setup/args.rs::render_bytes_arg`,
//! `ruby/args.rs`, `csharp/setup.rs`, `java/args.rs`, `kotlin/args.rs`, `php/args.rs`,
//! `r/args.rs`, `swift/args.rs`, `zig/args.rs`, `elixir/args.rs`, `rust/args.rs`,
//! `gleam/args.rs`) already special-cases `arg_type == "bytes"`; the C free-function/raw-result
//! path had no such branch at all, so it rendered the fixture's file-path string as a bare C
//! string literal and dropped the length argument entirely -- `xberg_pdf_page_count`'s
//! generated snippet passed one argument to a two-argument-past-`self` C export.

use crate::e2e::escape::escape_c;

/// One `type = "bytes"` argument, lowered to C: `setup` lines to emit before the call that
/// consumes it, `ptr_expr`/`len_expr` to splice into the call's argument list in that order
/// (matching the FFI generator's pointer-then-length convention), and `cleanup` lines to emit
/// once the call has read the buffer.
pub(super) struct RenderedBytesArg {
    pub(super) setup: Vec<String>,
    pub(super) ptr_expr: String,
    pub(super) len_expr: String,
    pub(super) cleanup: Vec<String>,
}

/// Render a `type = "bytes"` arg's fixture value into a `(ptr, len)` C argument pair.
///
/// - A JSON string is a fixture-relative file path: read it into a heap buffer with
///   `fopen`/`fread`, matching the shape `c/docs_file_replace.jinja` already uses to load a file
///   for a `json_object` arg's embedded byte field (minus the JSON re-encoding that template
///   does, since this value is passed as a real byte buffer, not spliced into a JSON string).
/// - A non-empty JSON array of integers is an inline byte literal: emit a
///   `static const uint8_t[]` and use `sizeof(...)` for its length -- no I/O needed.
/// - Anything else (`null`, missing, an empty array) has no buffer to read: send `(NULL, 0)`.
///   The FFI's length parameter always exists regardless of whether the pointer argument is
///   optional, so this is the only sentinel pair that keeps the C parameter *count* fixed while
///   still representing "no bytes".
pub(super) fn render_bytes_arg(
    name: &str,
    val: Option<&serde_json::Value>,
    documentation_snippet: bool,
) -> RenderedBytesArg {
    match val {
        Some(serde_json::Value::String(path)) => render_file_bytes_arg(name, path, documentation_snippet),
        Some(serde_json::Value::Array(items)) if !items.is_empty() => render_inline_bytes_arg(name, items),
        _ => RenderedBytesArg {
            setup: Vec::new(),
            ptr_expr: "NULL".to_string(),
            len_expr: "0".to_string(),
            cleanup: Vec::new(),
        },
    }
}

fn render_file_bytes_arg(name: &str, path: &str, documentation_snippet: bool) -> RenderedBytesArg {
    let escaped_path = escape_c(path);
    let file_var = format!("{name}_file");
    let size_var = format!("{name}_size");
    let buf_var = format!("{name}_buf");

    let mut setup = vec![format!("FILE *{file_var} = fopen(\"{escaped_path}\", \"rb\");")];
    if documentation_snippet {
        setup.push(format!("if ({file_var} == NULL) return EXIT_FAILURE;"));
    } else {
        setup.push(format!("assert({file_var} != NULL);"));
    }
    setup.push(format!("fseek({file_var}, 0, SEEK_END);"));
    setup.push(format!("long {size_var} = ftell({file_var});"));
    if documentation_snippet {
        setup.push(format!(
            "if ({size_var} < 0) {{ fclose({file_var}); return EXIT_FAILURE; }}"
        ));
    }
    setup.push(format!("rewind({file_var});"));
    setup.push(format!(
        "uint8_t *{buf_var} = malloc({size_var} > 0 ? (size_t){size_var} : 1);"
    ));
    if documentation_snippet {
        setup.push(format!(
            "if ({buf_var} == NULL) {{ fclose({file_var}); return EXIT_FAILURE; }}"
        ));
    } else {
        setup.push(format!("assert({buf_var} != NULL);"));
    }
    if documentation_snippet {
        setup.push(format!(
            "if (fread({buf_var}, 1, (size_t){size_var}, {file_var}) != (size_t){size_var}) {{ free({buf_var}); fclose({file_var}); return EXIT_FAILURE; }}"
        ));
    } else {
        setup.push(format!(
            "assert(fread({buf_var}, 1, (size_t){size_var}, {file_var}) == (size_t){size_var});"
        ));
    }
    setup.push(format!("fclose({file_var});"));

    RenderedBytesArg {
        setup,
        ptr_expr: buf_var.clone(),
        len_expr: format!("(uintptr_t){size_var}"),
        cleanup: vec![format!("free({buf_var});")],
    }
}

fn render_inline_bytes_arg(name: &str, items: &[serde_json::Value]) -> RenderedBytesArg {
    let literal_var = format!("{name}_bytes");
    let literal = items
        .iter()
        .map(|item| item.as_u64().unwrap_or(0).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    RenderedBytesArg {
        setup: vec![format!("static const uint8_t {literal_var}[] = {{{literal}}};")],
        ptr_expr: literal_var.clone(),
        len_expr: format!("sizeof({literal_var})"),
        cleanup: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::render_bytes_arg;

    #[test]
    fn file_path_reads_into_a_heap_buffer_and_passes_ptr_and_len() {
        let val = serde_json::json!("pdf/sample_contract.pdf");
        let rendered = render_bytes_arg("pdf_bytes", Some(&val), false);
        assert!(
            rendered
                .setup
                .iter()
                .any(|line| line.contains("fopen(\"pdf/sample_contract.pdf\", \"rb\")"))
        );
        assert!(rendered.setup.iter().any(|line| line.contains("fread(")));
        assert_eq!(rendered.ptr_expr, "pdf_bytes_buf");
        assert_eq!(rendered.len_expr, "(uintptr_t)pdf_bytes_size");
        assert_eq!(rendered.cleanup, vec!["free(pdf_bytes_buf);".to_string()]);
    }

    #[test]
    fn missing_value_sends_null_and_zero_len() {
        let rendered = render_bytes_arg("pdf_bytes", None, false);
        assert!(rendered.setup.is_empty());
        assert_eq!(rendered.ptr_expr, "NULL");
        assert_eq!(rendered.len_expr, "0");
        assert!(rendered.cleanup.is_empty());
    }

    #[test]
    fn inline_byte_array_uses_a_static_literal_and_sizeof() {
        let val = serde_json::json!([1, 2, 3]);
        let rendered = render_bytes_arg("data", Some(&val), false);
        assert_eq!(
            rendered.setup,
            vec!["static const uint8_t data_bytes[] = {1, 2, 3};".to_string()]
        );
        assert_eq!(rendered.ptr_expr, "data_bytes");
        assert_eq!(rendered.len_expr, "sizeof(data_bytes)");
        assert!(rendered.cleanup.is_empty());
    }
}
