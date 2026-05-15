//! Zig visitor vtable emission for e2e test callbacks.
//!
//! Each visitor-bearing fixture emits a small `TestVisitor_<id>` struct
//! exposing one `callconv(.C)` thunk per fixture-configured callback. The
//! thunks return an `i32` discriminator (`0=Continue`, `1=Skip`,
//! `2=PreserveHtml`, `3=Custom`) and, for the `Custom` path, allocate a
//! heap-owned UTF-8 buffer that is written back through the
//! `out_custom`/`out_len` out-pointers. The FFI runtime takes ownership of
//! the buffer (it frees it after copying into the `VisitResult::Custom`
//! variant).
//!
//! Callbacks not present in the fixture stay null in the
//! `HTMHtmVisitorCallbacks` struct; the FFI defaults those to `Continue`
//! behaviour (see `crates/html-to-markdown-ffi/src/lib.rs` HtmVisitor impl).

use crate::fixture::{CallbackAction, VisitorSpec};
use heck::ToSnakeCase;
use std::fmt::Write as FmtWrite;

/// Parameter list (typed Zig signature) for the C callback that backs a
/// `HtmVisitorCallbacks::visit_*` slot. The shape mirrors the cbindgen-
/// emitted function pointer type — see
/// `crates/html-to-markdown-ffi/include/html_to_markdown.h` for the
/// canonical signatures.
fn callback_params(method: &str) -> &'static str {
    match method {
        "visit_text" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _text: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_element_start" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_element_end" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _output: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_link" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _href: [*c]const u8, _text: [*c]const u8, _title: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_image" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _src: [*c]const u8, _alt: [*c]const u8, _title: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_heading" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _level: u32, _text: [*c]const u8, _id: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_code_block" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _lang: [*c]const u8, _code: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_code_inline" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _code: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_list_item" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _ordered: i32, _marker: [*c]const u8, _text: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_list_start" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _ordered: i32, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_list_end" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _ordered: i32, _output: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_table_start" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_table_row" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _cells: [*c]const [*c]const u8, _cell_count: usize, _is_header: i32, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_table_end" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _output: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_blockquote" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _content: [*c]const u8, _depth: usize, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_line_break" | "visit_horizontal_rule" | "visit_definition_list_start" | "visit_figure_start" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_custom_element" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _tag_name: [*c]const u8, _html: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_form" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _action: [*c]const u8, _method: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_input" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _input_type: [*c]const u8, _name: [*c]const u8, _value: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_audio" | "visit_video" | "visit_iframe" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _src: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_details" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _open: i32, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        "visit_figure_end" | "visit_definition_list_end" => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _output: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
        // Default: single text payload (covers visit_strong/emphasis/strikethrough/
        // underline/subscript/superscript/mark/button/summary/figcaption/
        // definition_term/definition_description).
        _ => {
            "_ctx: [*c]const c.HTMHtmNodeContext, _user_data: ?*anyopaque, _text: [*c]const u8, out_custom: [*c]?[*c]u8, out_len: [*c]usize"
        }
    }
}

/// Build the body lines of a callback thunk. Returns one or more lines,
/// 8-space-indented to nest inside `pub fn ... {`.
fn callback_body(method: &str, action: &CallbackAction) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "        _ = _ctx;");
    let _ = writeln!(out, "        _ = _user_data;");
    // Silence each typed parameter that the action body does not use.
    let unused = unused_params_for(method);
    for name in &unused {
        let _ = writeln!(out, "        _ = {name};");
    }

    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "        _ = out_custom;");
            let _ = writeln!(out, "        _ = out_len;");
            let _ = writeln!(out, "        return 1;");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "        _ = out_custom;");
            let _ = writeln!(out, "        _ = out_len;");
            let _ = writeln!(out, "        return 0;");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "        _ = out_custom;");
            let _ = writeln!(out, "        _ = out_len;");
            let _ = writeln!(out, "        return 2;");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_zig_string(output);
            let _ = writeln!(
                out,
                "        const _buf = std.heap.c_allocator.dupeZ(u8, \"{escaped}\") catch return 0;"
            );
            let _ = writeln!(out, "        if (out_custom) |p| p.* = _buf.ptr;");
            let _ = writeln!(out, "        if (out_len) |p| p.* = _buf.len;");
            let _ = writeln!(out, "        return 3;");
        }
        CallbackAction::CustomTemplate { template, .. } => {
            // Convert `{name}` placeholders to `{s}` and collect placeholder names.
            let (zig_fmt, placeholders) = template_to_zig_fmt(template);
            let escaped_fmt = escape_zig_string(&zig_fmt);
            let placeholder_args = if placeholders.is_empty() {
                ".{}".to_string()
            } else {
                let args: Vec<String> = placeholders
                    .iter()
                    .map(|name| placeholder_to_zig_arg(method, name))
                    .collect();
                format!(".{{ {} }}", args.join(", "))
            };
            // allocPrintSentinel returns a `[:0]u8` so we can hand its `ptr` and
            // `len` straight to the FFI; the runtime takes ownership and frees
            // it via libc free, which matches `std.heap.c_allocator`.
            let _ = writeln!(
                out,
                "        const _buf = std.fmt.allocPrintSentinel(std.heap.c_allocator, \"{escaped_fmt}\", {placeholder_args}, 0) catch return 0;"
            );
            let _ = writeln!(out, "        if (out_custom) |p| p.* = _buf.ptr;");
            let _ = writeln!(out, "        if (out_len) |p| p.* = _buf.len;");
            let _ = writeln!(out, "        return 3;");
        }
    }
    out
}

/// List of typed parameter names that should be discarded with `_ = name;`
/// when the callback body doesn't reference them. Mirrors `callback_params`
/// but excludes `_ctx`/`_user_data` (already discarded unconditionally) and
/// `out_custom`/`out_len` (action-specific).
fn unused_params_for(method: &str) -> Vec<&'static str> {
    match method {
        "visit_text" => vec!["_text"],
        "visit_element_start"
        | "visit_table_start"
        | "visit_line_break"
        | "visit_horizontal_rule"
        | "visit_definition_list_start"
        | "visit_figure_start" => vec![],
        "visit_element_end" | "visit_table_end" | "visit_figure_end" | "visit_definition_list_end" => vec!["_output"],
        "visit_link" => vec!["_href", "_text", "_title"],
        "visit_image" => vec!["_src", "_alt", "_title"],
        "visit_heading" => vec!["_level", "_text", "_id"],
        "visit_code_block" => vec!["_lang", "_code"],
        "visit_code_inline" => vec!["_code"],
        "visit_list_item" => vec!["_ordered", "_marker", "_text"],
        "visit_list_start" => vec!["_ordered"],
        "visit_list_end" => vec!["_ordered", "_output"],
        "visit_table_row" => vec!["_cells", "_cell_count", "_is_header"],
        "visit_blockquote" => vec!["_content", "_depth"],
        "visit_custom_element" => vec!["_tag_name", "_html"],
        "visit_form" => vec!["_action", "_method"],
        "visit_input" => vec!["_input_type", "_name", "_value"],
        "visit_audio" | "visit_video" | "visit_iframe" => vec!["_src"],
        "visit_details" => vec!["_open"],
        // Default: text-only methods.
        _ => vec!["_text"],
    }
}

/// Map a fixture placeholder (e.g. `href`, `text`) to the typed zig
/// expression that yields its string-formattable value at the callsite.
/// For `[*c]const u8` C strings, we wrap with `std.mem.span` to get a
/// `[:0]const u8` slice that `{s}` formats as the underlying bytes.
fn placeholder_to_zig_arg(method: &str, raw_name: &str) -> String {
    // Specialise integer placeholders (e.g. `level` on visit_heading,
    // `depth` on visit_blockquote, `ordered` on visit_list_*).
    let int_placeholder = matches!(
        (method, raw_name),
        ("visit_heading", "level")
            | ("visit_blockquote", "depth")
            | ("visit_list_item", "ordered")
            | ("visit_list_start", "ordered")
            | ("visit_list_end", "ordered")
            | ("visit_details", "open")
            | ("visit_table_row", "is_header")
    );
    if int_placeholder {
        return format!("_{raw_name}");
    }
    // C-string placeholder: wrap with std.mem.span. Map fixture-side names
    // to typed-parameter names by prefixing with "_" (matching callback_params).
    format!("std.mem.span(_{raw_name})")
}

/// Convert a fixture template (`{href}`, `{text}`) into a Zig format
/// string (with `{s}` or `{d}` placeholders) plus the ordered list of
/// fixture placeholder names. Literal `{` / `}` produce `{{` / `}}`.
fn template_to_zig_fmt(template: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(template.len());
    let mut placeholders: Vec<String> = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    out.push_str("{{");
                    continue;
                }
                let mut name = String::new();
                while let Some(&peek) = chars.peek() {
                    if peek == '}' {
                        chars.next();
                        break;
                    }
                    name.push(peek);
                    chars.next();
                }
                // Use {d} for known integer placeholders so std.fmt formats
                // them as decimals; default to {s} for everything else.
                let is_int = matches!(name.as_str(), "level" | "depth" | "ordered" | "open" | "is_header");
                if is_int {
                    out.push_str("{d}");
                } else {
                    out.push_str("{s}");
                }
                placeholders.push(name);
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    out.push_str("}}");
                } else {
                    // Stray `}` — zig fmt requires escaping.
                    out.push_str("}}");
                }
            }
            other => out.push(other),
        }
    }
    (out, placeholders)
}

/// Escape a string for embedding in a Zig double-quoted string literal.
fn escape_zig_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Zig accepts plain UTF-8 in string literals; only escape control chars.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Build the per-fixture visitor block (struct declaration + callbacks
/// initialisation + visitor handle setup). Returns the rendered Zig
/// source as a single multi-line string ready to splice into the test
/// body. The caller is expected to bracket the test with
/// `defer c.htm_visitor_free(_visitor);`.
pub(super) fn build_zig_visitor(fixture_id: &str, spec: &VisitorSpec) -> String {
    let struct_id = fixture_id.to_snake_case();
    let mut out = String::new();
    // Per-fixture container struct with one pub thunk per fixture-configured method.
    let _ = writeln!(out, "    const TestVisitor_{struct_id} = struct {{");
    // Stable iteration order: sort callback names alphabetically.
    let mut callbacks: Vec<(&String, &CallbackAction)> = spec.callbacks.iter().collect();
    callbacks.sort_by(|a, b| a.0.cmp(b.0));
    for (method, action) in &callbacks {
        let params = callback_params(method);
        let body = callback_body(method, action);
        let _ = writeln!(out, "        pub fn {method}({params}) callconv(.C) i32 {{");
        out.push_str(&body);
        let _ = writeln!(out, "        }}");
    }
    let _ = writeln!(out, "    }};");

    // Build a zero-initialised callbacks struct and wire each configured slot.
    let _ = writeln!(
        out,
        "    var _callbacks: c.HTMHtmVisitorCallbacks = std.mem.zeroes(c.HTMHtmVisitorCallbacks);"
    );
    for (method, _) in &callbacks {
        let _ = writeln!(out, "    _callbacks.{method} = &TestVisitor_{struct_id}.{method};");
    }
    out
}
