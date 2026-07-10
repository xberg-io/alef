use super::sanitize::{
    DocTarget, sanitize_rust_idioms, sanitize_rust_idioms_keep_sections, unlink_intradoc_references,
    wrap_bare_bracket_references,
};
use super::sections::{
    RustdocSections, example_for_target, parse_arguments_bullets, parse_rustdoc_sections, render_csharp_xml_sections,
    render_phpdoc_sections,
};

/// Emit PHPDoc-style comments (/** ... */)
/// Used for PHP classes, methods, and properties.
///
/// Sanitizes Rust-specific idioms before translating rustdoc sections
/// (`# Arguments` → `@param`, `# Returns` → `@return`, `# Errors` → `@throws`,
/// `# Example` → ` ```php ` fence) via [`render_phpdoc_sections`].
///
/// `exception_class` is the PHP exception class name to use in `@throws` tags.
pub fn emit_phpdoc(out: &mut String, doc: &str, indent: &str, exception_class: &str) {
    if doc.is_empty() {
        return;
    }
    let sanitized = sanitize_rust_idioms(doc, DocTarget::PhpDoc);
    let sections = parse_rustdoc_sections(&sanitized);
    let any_section = sections.arguments.is_some()
        || sections.returns.is_some()
        || sections.errors.is_some()
        || sections.example.is_some();
    let body = if any_section {
        render_phpdoc_sections(&sections, exception_class)
    } else {
        sanitized
    };
    out.push_str(indent);
    out.push_str("/**\n");
    for line in body.lines() {
        out.push_str(indent);
        out.push_str(" * ");
        out.push_str(&escape_phpdoc_line(line));
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Escape PHPDoc line: handle */ sequences that could close the comment early.
fn escape_phpdoc_line(s: &str) -> String {
    s.replace("*/", "* /")
}

/// Emit C# XML documentation comments (/// <summary> ... </summary>)
/// Used for C# classes, structs, methods, and properties.
///
/// Translates rustdoc sections (`# Arguments` → `<param>`,
/// `# Returns` → `<returns>`, `# Errors` → `<exception>`,
/// `# Example` → `<example><code>`) via [`render_csharp_xml_sections`].
///
/// `exception_class` is the C# exception class name to use in `<exception cref="...">` tags.
pub fn emit_csharp_doc(out: &mut String, doc: &str, indent: &str, exception_class: &str) {
    if doc.is_empty() {
        return;
    }
    let raw_sections = parse_rustdoc_sections(doc);
    let sections = RustdocSections {
        summary: sanitize_rust_idioms_keep_sections(&raw_sections.summary, DocTarget::CSharpDoc),
        arguments: raw_sections
            .arguments
            .as_deref()
            .map(|s| sanitize_rust_idioms_keep_sections(s, DocTarget::CSharpDoc)),
        returns: raw_sections
            .returns
            .as_deref()
            .map(|s| sanitize_rust_idioms_keep_sections(s, DocTarget::CSharpDoc)),
        errors: raw_sections
            .errors
            .as_deref()
            .map(|s| sanitize_rust_idioms_keep_sections(s, DocTarget::CSharpDoc)),
        panics: raw_sections
            .panics
            .as_deref()
            .map(|s| sanitize_rust_idioms_keep_sections(s, DocTarget::CSharpDoc)),
        safety: raw_sections
            .safety
            .as_deref()
            .map(|s| sanitize_rust_idioms_keep_sections(s, DocTarget::CSharpDoc)),
        example: None,
    };
    let any_section = sections.arguments.is_some()
        || sections.returns.is_some()
        || sections.errors.is_some()
        || sections.example.is_some();
    if !any_section {
        out.push_str(indent);
        out.push_str("/// <summary>\n");
        for line in sections.summary.lines() {
            out.push_str(indent);
            out.push_str("/// ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(indent);
        out.push_str("/// </summary>\n");
        return;
    }
    let rendered = render_csharp_xml_sections(&sections, exception_class);
    for line in rendered.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Elixir documentation comments (@doc)
/// Used for Elixir modules and functions.
pub fn emit_elixir_doc(out: &mut String, doc: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str("@doc \"\"\"\n");
    for line in doc.lines() {
        out.push_str(&escape_elixir_doc_line(line));
        out.push('\n');
    }
    out.push_str("\"\"\"\n");
}

/// Emit Rust `///` documentation comments.
///
/// Used by alef backends that emit Rust source (e.g., the Rustler NIF crate,
/// the swift-bridge wrapper crate, the FRB Dart bridge crate). Distinct from
/// `emit_swift_doc` only by intent — the syntax is identical (`/// ` per line).
pub fn emit_rustdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    // crate-level `#![allow]` cannot override it) would fail. Converting them to
    let delinked = unlink_intradoc_references(doc);
    for line in delinked.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Escape Elixir doc line: handle triple-quote sequences that could close the heredoc early.
fn escape_elixir_doc_line(s: &str) -> String {
    s.replace("\"\"\"", "\"\" \"")
}

/// Emit R roxygen2-style documentation comments (#')
/// Used for R functions.
pub fn emit_roxygen(out: &mut String, doc: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str("#' ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Swift-style documentation comments (///)
/// Used for Swift structs, enums, and functions.
pub fn emit_swift_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Javadoc-style documentation comments (/** ... */)
/// Used for Java classes, methods, and fields.
/// Handles XML escaping and Javadoc tag formatting.
pub fn emit_javadoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(indent);
    out.push_str("/**\n");
    for line in doc.lines() {
        let escaped = escape_javadoc_line(line);
        let trimmed = escaped.trim_end();
        if trimmed.is_empty() {
            out.push_str(indent);
            out.push_str(" *\n");
        } else {
            out.push_str(indent);
            out.push_str(" * ");
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Emit KDoc-style documentation comments (/** ... */)
/// Used for Kotlin classes, methods, and properties.
pub fn emit_kdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    out.push_str(indent);
    out.push_str("/**\n");
    for line in doc.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            out.push_str(indent);
            out.push_str(" *\n");
        } else {
            out.push_str(indent);
            out.push_str(" * ");
            out.push_str(&escape_kdoc_line(trimmed));
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Escape a KDoc line so embedded `/*` / `*/` cannot disturb the comment lexer.
///
/// Kotlin KDoc supports **nested** block comments — any `/*` inside a `/** … */`
/// block opens a new comment level that must be balanced by a matching `*/`
/// before the outer block can close. Rust source frequently uses
/// backtick-delimited inline code such as `` `"image/*"` `` in `///` comments,
/// which contains a stray `/*` but no matching `*/`. When that text flows
/// verbatim into a Kotlin KDoc, the embedded `/*` opens a nested comment that
/// is never closed and the Kotlin lexer reports cascading "Unclosed comment" /
/// "Missing '}'" errors past the end of the file. (Java's lexer does *not*
/// nest block comments, which is why `emit_javadoc` does not need this guard.)
///
/// Replacing `/*` with `/ *` and `*/` with `* /` keeps the rendered text
/// readable while preventing the lexer-level open/close tokens from matching.
pub(crate) fn escape_kdoc_line(s: &str) -> String {
    s.replace("*/", "* /").replace("/*", "/ *")
}

/// Emit KDoc-style documentation comments in ktfmt-canonical format.
///
/// ktfmt collapses short KDoc comments to single-line format (`/** ... */`)
/// when they fit within the 100-character line width limit. This function
/// generates KDoc in that canonical form to avoid unnecessary formatting
/// diffs when the generated code is passed through ktfmt.
///
/// - Single-line comments that fit in 100 chars: emitted as `/** content */`
/// - Multi-paragraph or longer comments: emitted with newlines and ` * ` prefixes
/// - Preserves indent and respects line width boundary at 100 chars
pub fn emit_kdoc_ktfmt_canonical(out: &mut String, doc: &str, indent: &str) {
    const KTFMT_LINE_WIDTH: usize = 100;

    if doc.is_empty() {
        return;
    }

    let lines: Vec<&str> = doc.lines().collect();

    let is_short_single_paragraph = lines.len() == 1 && !lines[0].contains('\n');

    if is_short_single_paragraph {
        let trimmed = lines[0].trim();
        let escaped = escape_kdoc_line(trimmed);
        let single_line_len = indent.len() + 4 + escaped.len() + 3;
        if single_line_len <= KTFMT_LINE_WIDTH {
            out.push_str(indent);
            out.push_str("/** ");
            out.push_str(&escaped);
            out.push_str(" */\n");
            return;
        }
    }

    out.push_str(indent);
    out.push_str("/**\n");
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            out.push_str(indent);
            out.push_str(" *\n");
        } else {
            out.push_str(indent);
            out.push_str(" * ");
            out.push_str(&escape_kdoc_line(trimmed));
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str(" */\n");
}

/// Emit Dartdoc-style documentation comments (///)
/// Used for Dart classes, methods, and properties.
pub fn emit_dartdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Gleam documentation comments (///)
/// Used for Gleam functions and types.
pub fn emit_gleam_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit Doxygen-style C documentation comments using `///`-prefixed lines.
///
/// Used by `alef-backend-ffi` above every `extern "C" fn`, the `*_len()`
/// companion, opaque-handle typedef, and (post-cbindgen) the type/enum
/// declarations cbindgen surfaces in the generated `.h`. cbindgen translates
/// `///` source lines into a single `/** ... */` Doxygen block per item, so we
/// only need to emit per-line `///` content here.
///
/// Translates rustdoc sections via `render_doxygen_sections`:
///
/// - `# Arguments` → `\param <name> <description>` (one per arg).
/// - `# Returns`   → `\return <description>`.
/// - `# Errors`    → `\note <description>` (Doxygen has no `\throws` for C;
///   `\note` is the convention).
/// - `# Safety`    → `\note SAFETY: <description>`.
/// - `# Example`   → `\code` ... `\endcode` block.
///
/// Markdown links (`[text](url)`) are flattened to `text (url)`. Body lines
/// are word-wrapped at ~100 columns so the rendered `/** */` block stays
/// readable in IDE tooltips and terminal viewers.
pub fn emit_c_doxygen(out: &mut String, doc: &str, indent: &str) {
    if doc.trim().is_empty() {
        return;
    }
    let sections = parse_rustdoc_sections(doc);
    let any_section = sections.arguments.is_some()
        || sections.returns.is_some()
        || sections.errors.is_some()
        || sections.safety.is_some()
        || sections.example.is_some();
    let mut body = if any_section {
        render_doxygen_sections_with_notes(&sections)
    } else {
        sections.summary.clone()
    };
    body = strip_markdown_links(&body);
    body = wrap_bare_bracket_references(&body);
    let wrapped = word_wrap(&body, DOXYGEN_WRAP_WIDTH);
    for line in wrapped.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

const DOXYGEN_WRAP_WIDTH: usize = 100;

/// Render `RustdocSections` as a Doxygen body but route `# Errors` and
/// `# Safety` to `\note` lines instead of plain prose. This is the variant
/// `emit_c_doxygen` uses; the public `render_doxygen_sections` keeps its
/// long-standing plain-prose semantics so existing callers don't shift.
fn render_doxygen_sections_with_notes(sections: &RustdocSections) -> String {
    let mut out = String::new();
    if !sections.summary.is_empty() {
        out.push_str(&sections.summary);
    }
    if let Some(args) = sections.arguments.as_deref() {
        for (name, desc) in parse_arguments_bullets(args) {
            if !out.is_empty() {
                out.push('\n');
            }
            if desc.is_empty() {
                out.push_str("\\param ");
                out.push_str(&name);
            } else {
                out.push_str("\\param ");
                out.push_str(&name);
                out.push(' ');
                out.push_str(&desc);
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("\\return ");
        out.push_str(ret.trim());
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("\\note ");
        out.push_str(err.trim());
    }
    if let Some(safety) = sections.safety.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("\\note SAFETY: ");
        out.push_str(safety.trim());
    }
    if let Some(example) = sections.example.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("\\code\n");
        for line in example.lines() {
            let t = line.trim_start();
            if t.starts_with("```") {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("\\endcode");
    }
    out
}

/// Flatten Markdown inline links `[text](url)` to `text (url)` so the rendered
/// Doxygen block stays readable when consumed without a Markdown filter.
fn strip_markdown_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(close) = bytes[i + 1..].iter().position(|&b| b == b']') {
                let text_end = i + 1 + close;
                if text_end + 1 < bytes.len() && bytes[text_end + 1] == b'(' {
                    if let Some(paren_close) = bytes[text_end + 2..].iter().position(|&b| b == b')') {
                        let url_start = text_end + 2;
                        let url_end = url_start + paren_close;
                        let text = &s[i + 1..text_end];
                        let url = &s[url_start..url_end];
                        out.push_str(text);
                        out.push_str(" (");
                        out.push_str(url);
                        out.push(')');
                        i = url_end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Word-wrap each input line at `width` columns. Lines starting with `\code`
/// or contained between `\code`/`\endcode` markers, as well as Markdown fence
/// blocks, are passed through verbatim to preserve example formatting.
fn word_wrap(input: &str, width: usize) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_code = false;
    for raw in input.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("\\code") {
            in_code = true;
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        if trimmed.starts_with("\\endcode") {
            in_code = false;
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        if in_code || trimmed.starts_with("```") {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        if raw.len() <= width {
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        let mut current = String::with_capacity(width);
        for word in raw.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() > width {
                out.push_str(&current);
                out.push('\n');
                current.clear();
                current.push_str(word);
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            out.push_str(&current);
            out.push('\n');
        }
    }
    out.trim_end_matches('\n').to_string()
}

pub fn emit_zig_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("/// ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Emit YARD documentation comments for Ruby.
/// Used for Ruby classes, methods, and attributes.
///
/// YARD syntax: each line prefixed with `# ` (with space). Translates rustdoc
/// sections (`# Arguments` → `@param`, `# Returns` → `@return`, `# Errors` → `@raise`)
/// via [`render_yard_sections`].
pub fn emit_yard_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let sections = parse_rustdoc_sections(doc);
    let any_section = sections.arguments.is_some()
        || sections.returns.is_some()
        || sections.errors.is_some()
        || sections.example.is_some();
    let body = if any_section {
        render_yard_sections(&sections)
    } else {
        doc.to_string()
    };
    for line in body.lines() {
        out.push_str(indent);
        out.push_str("# ");
        out.push_str(line);
        out.push('\n');
    }
}

/// Render `RustdocSections` as YARD documentation comment body.
///
/// - `# Arguments` → `@param name desc` (one per arg)
/// - `# Returns`   → `@return desc`
/// - `# Errors`    → `@raise desc`
/// - `# Example`   → `@example` block.
///
/// Output is a plain string with `\n` separators; the emitter wraps each line
/// in `# ` itself.
pub fn render_yard_sections(sections: &RustdocSections) -> String {
    let mut out = String::new();
    if !sections.summary.is_empty() {
        out.push_str(&sections.summary);
    }
    if let Some(args) = sections.arguments.as_deref() {
        for (name, desc) in parse_arguments_bullets(args) {
            if !out.is_empty() {
                out.push('\n');
            }
            if desc.is_empty() {
                out.push_str("@param ");
                out.push_str(&name);
            } else {
                out.push_str("@param ");
                out.push_str(&name);
                out.push(' ');
                out.push_str(&desc);
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("@return ");
        out.push_str(ret.trim());
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("@raise ");
        out.push_str(err.trim());
    }
    if let Some(example) = sections.example.as_deref() {
        if let Some(body) = example_for_target(example, "ruby") {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("@example\n");
            out.push_str(&body);
        }
    }
    out
}

/// Escape Javadoc line: handle XML special chars and backtick code blocks.
///
/// HTML entities (`<`, `>`, `&`) are also escaped *inside* `{@code …}` blocks.
/// Without that, content like `` `<pre><code>` `` would emit raw `<pre>`
/// inside the Javadoc tag — Eclipse-formatter Spotless then treats it as a
/// real `<pre>` block element and shatters the line across multiple `* `
/// rows, breaking `alef-verify`'s embedded hash. Escaped content is
/// rendered identically by Javadoc readers (the `{@code}` tag shows literal
/// characters) and is stable under any post-formatter pass.
fn escape_javadoc_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            let mut code = String::new();
            for c in chars.by_ref() {
                if c == '`' {
                    break;
                }
                code.push(c);
            }
            result.push_str("{@code ");
            result.push_str(&escape_javadoc_html_entities(&code));
            result.push('}');
        } else if ch == '<' {
            result.push_str("&lt;");
        } else if ch == '>' {
            result.push_str("&gt;");
        } else if ch == '&' {
            result.push_str("&amp;");
        } else {
            result.push(ch);
        }
    }
    result
}

/// Escape only the HTML special characters that would otherwise be parsed by
/// downstream Javadoc/Eclipse formatters as block-level HTML (e.g. `<pre>`).
fn escape_javadoc_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            other => out.push(other),
        }
    }
    out
}
