//! Language-native documentation comment emission.
//! Provides standardized functions for emitting doc comments in different languages.

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
    // Sanitize Rust-specific idioms before processing sections.
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
    // Parse sections from the raw rustdoc first (so `# Examples` / `# Arguments`
    // / `# Returns` / `# Errors` are routed into structured XML tags), then
    // sanitise each section body to strip Rust idioms and XML-escape `<`/`>`/`&`.
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
        // Examples typically contain Rust code that doesn't compile as C#; drop the body
        // entirely rather than risk leaking unparseable code into `<example>`.
        example: None,
    };
    let any_section = sections.arguments.is_some()
        || sections.returns.is_some()
        || sections.errors.is_some()
        || sections.example.is_some();
    if !any_section {
        // Backwards-compatible path: plain `<summary>` for prose-only docs.
        out.push_str(indent);
        out.push_str("/// <summary>\n");
        for line in sections.summary.lines() {
            out.push_str(indent);
            out.push_str("/// ");
            // Note: sanitise_rust_idioms_keep_sections already XML-escaped <, >, & for
            // the CSharpDoc target. We deliberately do NOT call escape_csharp_doc_line
            // here because that would double-encode (e.g. `&amp;` → `&amp;amp;`).
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
        // The rendered tags already contain the canonical chars; we only
        // escape XML special chars that aren't part of our tag syntax. Since
        // render_csharp_xml_sections produces well-formed XML, raw passthrough
        // is correct.
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
    for line in doc.lines() {
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
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.push_str(indent);
    out.push_str(" */\n");
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

    // Check if this is a short, single-paragraph comment that fits on one line.
    let is_short_single_paragraph = lines.len() == 1 && !lines[0].contains('\n');

    if is_short_single_paragraph {
        let trimmed = lines[0].trim();
        // Calculate total length: indent + "/** " + content + " */"
        let single_line_len = indent.len() + 4 + trimmed.len() + 3; // 4 for "/** ", 3 for " */"
        if single_line_len <= KTFMT_LINE_WIDTH {
            // Fits on one line in ktfmt-canonical format
            out.push_str(indent);
            out.push_str("/** ");
            out.push_str(trimmed);
            out.push_str(" */\n");
            return;
        }
    }

    // Multi-line format (default for long or multi-paragraph comments)
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
            out.push_str(trimmed);
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
/// Translates rustdoc sections via [`render_doxygen_sections`]:
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
            // Find matching closing bracket on the same logical span (no nested brackets).
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

/// Emit Zig documentation comments (///)
/// Used for Zig functions, types, and declarations.
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

/// A parsed rustdoc comment broken out into the sections binding emitters
/// care about.
///
/// `summary` is the leading prose paragraph(s) before any `# Heading`.
/// Sections are stored verbatim (without the `# Heading` line itself);
/// each binding is responsible for translating bullet lists and code
/// fences into its host-native conventions.
///
/// Trailing/leading whitespace inside each field is trimmed so emitters
/// can concatenate without producing `* ` lines containing only spaces.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RustdocSections {
    /// Prose before the first `# Section` heading.
    pub summary: String,
    /// Body of the `# Arguments` section, if present.
    pub arguments: Option<String>,
    /// Body of the `# Returns` section, if present.
    pub returns: Option<String>,
    /// Body of the `# Errors` section, if present.
    pub errors: Option<String>,
    /// Body of the `# Panics` section, if present.
    pub panics: Option<String>,
    /// Body of the `# Safety` section, if present.
    pub safety: Option<String>,
    /// Body of the `# Example` / `# Examples` section, if present.
    pub example: Option<String>,
}

/// Parse a rustdoc string into [`RustdocSections`].
///
/// Recognises level-1 ATX headings whose name matches one of the standard
/// rustdoc section names (`Arguments`, `Returns`, `Errors`, `Panics`,
/// `Safety`, `Example`, `Examples`). Anything before the first heading
/// becomes `summary`. Unrecognised headings are folded into the
/// preceding section verbatim, so unconventional rustdoc isn't lost.
///
/// The input is expected to already have rustdoc-hidden lines stripped
/// and intra-doc-link syntax rewritten by
/// [`crate::extractor::helpers::normalize_rustdoc`].
pub fn parse_rustdoc_sections(doc: &str) -> RustdocSections {
    if doc.trim().is_empty() {
        return RustdocSections::default();
    }
    let mut summary = String::new();
    let mut arguments: Option<String> = None;
    let mut returns: Option<String> = None;
    let mut errors: Option<String> = None;
    let mut panics: Option<String> = None;
    let mut safety: Option<String> = None;
    let mut example: Option<String> = None;
    let mut current: Option<&'static str> = None;
    let mut buf = String::new();
    let mut in_fence = false;
    let flush = |target: Option<&'static str>,
                 buf: &mut String,
                 summary: &mut String,
                 arguments: &mut Option<String>,
                 returns: &mut Option<String>,
                 errors: &mut Option<String>,
                 panics: &mut Option<String>,
                 safety: &mut Option<String>,
                 example: &mut Option<String>| {
        let body = std::mem::take(buf).trim().to_string();
        if body.is_empty() {
            return;
        }
        match target {
            None => {
                if !summary.is_empty() {
                    summary.push('\n');
                }
                summary.push_str(&body);
            }
            Some("arguments") => *arguments = Some(body),
            Some("returns") => *returns = Some(body),
            Some("errors") => *errors = Some(body),
            Some("panics") => *panics = Some(body),
            Some("safety") => *safety = Some(body),
            Some("example") => *example = Some(body),
            _ => {}
        }
    };
    for line in doc.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            buf.push_str(line);
            buf.push('\n');
            continue;
        }
        if !in_fence {
            if let Some(rest) = trimmed.strip_prefix("# ") {
                let head = rest.trim().to_ascii_lowercase();
                let target = match head.as_str() {
                    "arguments" | "args" => Some("arguments"),
                    "returns" => Some("returns"),
                    "errors" => Some("errors"),
                    "panics" => Some("panics"),
                    "safety" => Some("safety"),
                    "example" | "examples" => Some("example"),
                    _ => None,
                };
                if target.is_some() {
                    flush(
                        current,
                        &mut buf,
                        &mut summary,
                        &mut arguments,
                        &mut returns,
                        &mut errors,
                        &mut panics,
                        &mut safety,
                        &mut example,
                    );
                    current = target;
                    continue;
                }
            }
        }
        buf.push_str(line);
        buf.push('\n');
    }
    flush(
        current,
        &mut buf,
        &mut summary,
        &mut arguments,
        &mut returns,
        &mut errors,
        &mut panics,
        &mut safety,
        &mut example,
    );
    RustdocSections {
        summary,
        arguments,
        returns,
        errors,
        panics,
        safety,
        example,
    }
}

/// Parse `# Arguments` body into `(name, description)` pairs.
///
/// Recognises both Markdown bullet styles `*` and `-`, with optional
/// backticks around the name: `* `name` - description` or
/// `- name: description`. Continuation lines indented under a bullet
/// are appended to the previous entry's description.
///
/// Used by emitters that translate to per-parameter documentation tags
/// (`@param`, `<param>`, `\param`).
pub fn parse_arguments_bullets(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        let is_bullet = trimmed.starts_with("* ") || trimmed.starts_with("- ");
        if is_bullet {
            let after = &trimmed[2..];
            // Accept `name`, `name:` or `name -` separator forms.
            let (name, desc) = if let Some(idx) = after.find(" - ") {
                (after[..idx].trim(), after[idx + 3..].trim())
            } else if let Some(idx) = after.find(": ") {
                (after[..idx].trim(), after[idx + 2..].trim())
            } else if let Some(idx) = after.find(' ') {
                (after[..idx].trim(), after[idx + 1..].trim())
            } else {
                (after.trim(), "")
            };
            let name = name.trim_matches('`').trim_matches('*').to_string();
            out.push((name, desc.to_string()));
        } else if !trimmed.is_empty() {
            if let Some(last) = out.last_mut() {
                if !last.1.is_empty() {
                    last.1.push(' ');
                }
                last.1.push_str(trimmed);
            }
        }
    }
    out
}

/// Detect the language tag on the first code fence in `body`.
///
/// Scans `body` for the first line that starts with ` ``` ` and returns the
/// tag that follows (e.g. `"rust"`, `"php"`, `"typescript"`). A bare ` ``` `
/// with no tag returns `"rust"` because rustdoc treats unlabelled fences as
/// Rust by default. Returns `"rust"` when no fence is found at all.
fn detect_first_fence_lang(body: &str) -> &str {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            let tag = rest.split(',').next().unwrap_or("").trim();
            return if tag.is_empty() { "rust" } else { tag };
        }
    }
    "rust"
}

/// Return `Some(transformed_example)` if the example should be emitted for
/// `target_lang`, or `None` when the example is Rust source that would be
/// meaningless in the foreign language.
///
/// When the original fence language is `rust` (including bare ` ``` ` which
/// rustdoc defaults to Rust) and the target is not `rust`, the example is
/// suppressed entirely — better absent than misleading. Cross-language
/// transliteration of example bodies is intentionally out of scope.
pub fn example_for_target(example: &str, target_lang: &str) -> Option<String> {
    let trimmed = example.trim();
    let source_lang = detect_first_fence_lang(trimmed);
    if source_lang == "rust" && target_lang != "rust" {
        None
    } else {
        Some(replace_fence_lang(trimmed, target_lang))
    }
}

/// Strip a single ` ```lang ` fence pair from `body`, returning the inner
/// code lines. Replaces the leading ` ```rust ` (or any other tag) with
/// `lang_replacement`, leaving the rest of the body unchanged.
///
/// When no fence is present the body is returned unchanged. Used by
/// emitters that need to convert ` ```rust ` examples into
/// ` ```typescript ` / ` ```python ` / ` ```swift ` etc.
pub fn replace_fence_lang(body: &str, lang_replacement: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            // Replace the language tag (everything up to the next comma or
            // end of line). Preserve indentation.
            let indent = &line[..line.len() - trimmed.len()];
            let after_lang = rest.find(',').map(|i| &rest[i..]).unwrap_or("");
            out.push_str(indent);
            out.push_str("```");
            out.push_str(lang_replacement);
            out.push_str(after_lang);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end_matches('\n').to_string()
}

/// Render `RustdocSections` as a JSDoc comment body (without the `/**` /
/// ` */` wrappers — those are added by the caller's emitter, which knows
/// the indent/escape conventions).
///
/// - `# Arguments` → `@param name - desc`
/// - `# Returns`   → `@returns desc`
/// - `# Errors`    → `@throws desc`
/// - `# Example`   → `@example` block. Replaces ` ```rust ` fences with
///   ` ```typescript ` so the example highlights properly in TypeDoc.
///
/// Output is a plain string with `\n` separators; emitters wrap each line
/// in ` * ` themselves.
pub fn render_jsdoc_sections(sections: &RustdocSections) -> String {
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
                out.push_str(&crate::template_env::render(
                    "doc_jsdoc_param.jinja",
                    minijinja::context! { name => &name },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "doc_jsdoc_param_desc.jinja",
                    minijinja::context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_jsdoc_returns.jinja",
            minijinja::context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_jsdoc_throws.jinja",
            minijinja::context! { content => err.trim() },
        ));
    }
    if let Some(example) = sections.example.as_deref() {
        if let Some(body) = example_for_target(example, "typescript") {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("@example\n");
            out.push_str(&body);
        }
    }
    out
}

/// Render `RustdocSections` as a JavaDoc comment body.
///
/// - `# Arguments` → `@param name desc` (one per param)
/// - `# Returns`   → `@return desc`
/// - `# Errors`    → `@throws KreuzbergRsException desc`
/// - `# Example`   → `<pre>{@code ...}</pre>` block.
///
/// `throws_class` is the FQN/simple name of the exception class to use in
/// the `@throws` tag (e.g. `"KreuzbergRsException"`).
pub fn render_javadoc_sections(sections: &RustdocSections, throws_class: &str) -> String {
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
                out.push_str(&crate::template_env::render(
                    "doc_javadoc_param.jinja",
                    minijinja::context! { name => &name },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "doc_javadoc_param_desc.jinja",
                    minijinja::context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_javadoc_return.jinja",
            minijinja::context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_javadoc_throws.jinja",
            minijinja::context! { throws_class => throws_class, content => err.trim() },
        ));
    }
    out
}

/// Render `RustdocSections` as a C# XML doc comment body (without the
/// `/// ` line prefixes — the emitter adds those).
///
/// - summary  → `<summary>...</summary>`
/// - args     → `<param name="x">desc</param>` (one per arg)
/// - returns  → `<returns>desc</returns>`
/// - errors   → `<exception cref="KreuzbergException">desc</exception>`
/// - example  → `<example><code language="csharp">...</code></example>`
pub fn render_csharp_xml_sections(sections: &RustdocSections, exception_class: &str) -> String {
    let mut out = String::new();
    out.push_str("<summary>\n");
    let summary = if sections.summary.is_empty() {
        ""
    } else {
        sections.summary.as_str()
    };
    for line in summary.lines() {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("</summary>");
    if let Some(args) = sections.arguments.as_deref() {
        for (name, desc) in parse_arguments_bullets(args) {
            out.push('\n');
            if desc.is_empty() {
                out.push_str(&crate::template_env::render(
                    "doc_csharp_param.jinja",
                    minijinja::context! { name => &name },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "doc_csharp_param_desc.jinja",
                    minijinja::context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        out.push('\n');
        out.push_str(&crate::template_env::render(
            "doc_csharp_returns.jinja",
            minijinja::context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        out.push('\n');
        out.push_str(&crate::template_env::render(
            "doc_csharp_exception.jinja",
            minijinja::context! {
                exception_class => exception_class,
                content => err.trim(),
            },
        ));
    }
    if let Some(example) = sections.example.as_deref() {
        out.push('\n');
        out.push_str("<example><code language=\"csharp\">\n");
        // Drop fence markers, keep code.
        for line in example.lines() {
            let t = line.trim_start();
            if t.starts_with("```") {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("</code></example>");
    }
    out
}

/// Render `RustdocSections` as a PHPDoc comment body.
///
/// - `# Arguments` → `@param mixed $name desc`
/// - `# Returns`   → `@return desc`
/// - `# Errors`    → `@throws KreuzbergException desc`
/// - `# Example`   → ` ```php ` fence (replaces ` ```rust `).
pub fn render_phpdoc_sections(sections: &RustdocSections, throws_class: &str) -> String {
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
                out.push_str(&crate::template_env::render(
                    "doc_phpdoc_param.jinja",
                    minijinja::context! { name => &name },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "doc_phpdoc_param_desc.jinja",
                    minijinja::context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_phpdoc_return.jinja",
            minijinja::context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_phpdoc_throws.jinja",
            minijinja::context! { throws_class => throws_class, content => err.trim() },
        ));
    }
    if let Some(example) = sections.example.as_deref() {
        if let Some(body) = example_for_target(example, "php") {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&body);
        }
    }
    out
}

/// Render `RustdocSections` as a Doxygen comment body for the C header.
///
/// - args    → `\param name desc`
/// - returns → `\return desc`
/// - errors  → prose paragraph (Doxygen has no semantic tag for FFI errors)
/// - example → `\code` ... `\endcode`
pub fn render_doxygen_sections(sections: &RustdocSections) -> String {
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
                out.push_str(&crate::template_env::render(
                    "doc_doxygen_param.jinja",
                    minijinja::context! { name => &name },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "doc_doxygen_param_desc.jinja",
                    minijinja::context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_doxygen_return.jinja",
            minijinja::context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::template_env::render(
            "doc_doxygen_errors.jinja",
            minijinja::context! { content => err.trim() },
        ));
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

/// Return the first paragraph of a doc comment as a single joined line.
///
/// Collects lines until the first blank line, trims each, then joins with a
/// space. This handles wrapped sentences like:
///
/// ```text
/// Convert HTML to Markdown, returning
/// a `ConversionResult`.
/// ```
///
/// which would otherwise be truncated at the comma when callers use
/// `.lines().next()`.
pub fn doc_first_paragraph_joined(doc: &str) -> String {
    doc.lines()
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Target language for [`sanitize_rust_idioms`].
///
/// Each variant selects the idiomatic mapping for Rust constructs that do not
/// translate directly to foreign-language doc syntax.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DocTarget {
    /// PHPDoc (`/** ... */`), e.g. phpstan-typed prose.
    PhpDoc,
    /// Javadoc (`/** ... */`), e.g. OpenJDK-style annotations.
    JavaDoc,
    /// TSDoc (`/** ... */`), TypeScript variant of JSDoc.
    TsDoc,
    /// JSDoc (`/** ... */`), JavaScript variant.
    JsDoc,
    /// C# XML doc (`/// <summary>...</summary>`).
    ///
    /// Strips Rust code fences and section headings (`# Examples`,
    /// `# Arguments`, `# Returns`, etc.), drops Rust trait-bound prose,
    /// and XML-escapes any remaining `<` / `>` / `&` so the result is
    /// safe to embed inside a `<summary>` element.
    CSharpDoc,
}

/// Sanitize Rust-specific idioms in a prose string for the given foreign-language
/// documentation target.
///
/// Transformations are applied **outside** backtick spans and code fences only,
/// so inline code examples and fenced blocks are never mutated (except that
/// ` ```rust ` fences and unmarked ` ``` ` code blocks are dropped entirely
/// for all targets [`DocTarget::TsDoc`], [`DocTarget::JsDoc`], [`DocTarget::PhpDoc`],
/// [`DocTarget::JavaDoc`], and [`DocTarget::CSharpDoc`]).
///
/// # Transformations
///
/// - Intra-doc links `` [`Type::method`] `` → `` `Type.method` ``.
/// - `[`Foo`]` (backtick inside square brackets) → `` `Foo` ``.
/// - `None` (word boundary) → `null` (PHP/Java) or `undefined` (TS/JS).
/// - `Some(x)` → `the value (x)`.
/// - `Option<T>` → `T?` (PHP) / `T | null` (Java) / `T | undefined` (TS/JS).
/// - `Vec<u8>` → `string` (PHP) / `byte[]` (Java) / `Uint8Array` (TS/JS).
/// - `Vec<T>` → `T[]` (all targets).
/// - `HashMap<K, V>` → `array<K, V>` (PHP) / `Map<K, V>` (Java) / `Record<K, V>` (TS/JS).
/// - `Arc<T>`, `Box<T>`, `Mutex<T>`, `RwLock<T>`, `Rc<T>`, `Cell<T>`, `RefCell<T>` → `T`.
/// - `Send + Sync`, `Send`, `Sync`, `'static` → stripped.
/// - Standalone `::` between identifiers → `.`.
/// - `pub fn `, `crate::`, `&self`, `&mut self` → stripped.
/// - `#[…]` attribute macros on their own line or inline → stripped.
/// - `.unwrap()`, `.expect("…")` → stripped.
/// - ` ```rust ` and unmarked ` ``` ` code fences → dropped entirely.
pub fn sanitize_rust_idioms(text: &str, target: DocTarget) -> String {
    // For C# XML doc the default is to drop rustdoc section headings
    // (`# Examples`, `# Arguments`, …) and the remainder of the comment,
    // because those bodies routinely contain content that cannot be embedded
    // safely inside `<summary>`. Callers that have already extracted sections
    // (`emit_csharp_doc`) sanitise each section body via [`sanitize_rust_idioms_keep_sections`].
    sanitize_rust_idioms_inner(text, target, true)
}

/// Same as [`sanitize_rust_idioms`] but never drops rustdoc section headings.
///
/// Used by emitters that have already split the doc into sections and need to
/// sanitise each body fragment independently (e.g. C# XML doc emission with
/// per-section `<param>` / `<returns>` / `<exception>` tags).
pub fn sanitize_rust_idioms_keep_sections(text: &str, target: DocTarget) -> String {
    sanitize_rust_idioms_inner(text, target, false)
}

fn sanitize_rust_idioms_inner(text: &str, target: DocTarget, drop_csharp_sections: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_rust_fence = false;
    let mut in_other_fence = false;
    // For C# XML doc: once a `# Examples` / `# Arguments` / etc. heading is
    // encountered, drop the entire remainder of the comment. Rustdoc section
    // headings cannot be safely embedded inside `<summary>` and the per-section
    // content (code fences, intra-doc links, generics) is the leading cause
    // of CS1002/CS1519 leakage. The plain `<summary>` path collapses to the
    // top-level prose only.
    let mut csharp_section_dropped = false;

    for line in text.lines() {
        if csharp_section_dropped {
            continue;
        }
        let trimmed = line.trim_start();
        if drop_csharp_sections
            && matches!(target, DocTarget::CSharpDoc)
            && !in_rust_fence
            && !in_other_fence
            && is_rustdoc_section_heading(trimmed)
        {
            csharp_section_dropped = true;
            continue;
        }

        // Detect code fence boundaries.
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_rust_fence {
                // Closing fence of a rust block.
                in_rust_fence = false;
                match target {
                    DocTarget::TsDoc | DocTarget::JsDoc | DocTarget::CSharpDoc | DocTarget::PhpDoc | DocTarget::JavaDoc => {
                        // Entire rust block dropped — don't emit closing fence.
                    }
                }
                continue;
            }
            if in_other_fence {
                // Closing fence of a non-rust block.
                in_other_fence = false;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            // Opening fence — determine language.
            let lang = rest.split(',').next().unwrap_or("").trim();
            let is_rust = lang.is_empty() || lang == "rust" || lang.starts_with("rust,");
            if is_rust {
                in_rust_fence = true;
                match target {
                    DocTarget::TsDoc | DocTarget::JsDoc | DocTarget::CSharpDoc | DocTarget::PhpDoc | DocTarget::JavaDoc => {
                        // Drop the entire rust fence block — skip opening line.
                        // Rust code examples are not portable to any of the target languages.
                    }
                }
                continue;
            }
            // Non-rust fence: pass through verbatim.
            in_other_fence = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Inside a rust fence.
        if in_rust_fence {
            match target {
                DocTarget::TsDoc | DocTarget::JsDoc | DocTarget::CSharpDoc | DocTarget::PhpDoc | DocTarget::JavaDoc => {
                    // Drop content of rust fences — all targets filter out Rust code examples.
                }
            }
            continue;
        }

        // Inside a non-rust fence: pass through verbatim.
        if in_other_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Check if this line is a bare `#[...]` attribute line.
        let stripped_indent = line.trim_start();
        if stripped_indent.starts_with("#[") && stripped_indent.ends_with(']') {
            // Attribute-only line — drop entirely.
            continue;
        }

        // Normal prose line: apply token-level transformations.
        let sanitized = apply_prose_transforms(line, target);
        out.push_str(&sanitized);
        out.push('\n');
    }

    // Trim trailing newline added by the loop (preserve internal newlines).
    if out.ends_with('\n') && !text.ends_with('\n') {
        out.pop();
    }

    // For C# XML doc, escape any remaining `<`, `>`, `&` so the result is
    // safe to embed inside `<summary>...</summary>`. By this point the
    // Rust-idiom substitutions have replaced `Vec<T>` / `Option<T>` /
    // `HashMap<K, V>` / `Result<T, E>` with their idiomatic forms, but
    // unrecognised generic constructs (e.g. trait-object references) may
    // still contain raw angle brackets that would break C# XML parsing.
    if matches!(target, DocTarget::CSharpDoc) {
        out = xml_escape_for_csharp(&out);
    }

    out
}

/// Return `true` if `line` (already left-trimmed) is a Rustdoc section heading
/// such as `# Examples`, `# Arguments`, `# Returns`, `# Errors`, `# Panics`,
/// or `# Safety`. Case-insensitive on the heading name.
fn is_rustdoc_section_heading(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("# ") else {
        return false;
    };
    let head = rest.trim().to_ascii_lowercase();
    matches!(
        head.as_str(),
        "arguments" | "args" | "returns" | "errors" | "panics" | "safety" | "example" | "examples"
    )
}

/// XML-escape `<`, `>`, `&` for safe embedding inside a C# `<summary>` element.
///
/// `<` / `>` may legitimately appear in prose after Rust-idiom substitution
/// when the substitutions produce C#-friendly forms (e.g. `Dictionary<K, V>`).
/// Those are still XML-significant characters and must be entity-escaped for
/// XML parsers (Roslyn, doxygen) to accept the resulting `<summary>` block.
fn xml_escape_for_csharp(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Apply prose-level Rust-idiom transformations to a single line.
///
/// Some transformations span or precede backtick boundaries and must be applied
/// to the full line before tokenisation:
///
/// 1. Intra-doc links (`` [`...`] ``) — they wrap a backtick pair.
/// 2. `::` path separator — even inside backtick spans it should become `.`
///    for all foreign-language targets, since the target language uses `.` for
///    member access and package paths in code examples too.
///
/// All remaining transformations are applied only to literal (non-code) segments
/// after tokenisation.
fn apply_prose_transforms(line: &str, target: DocTarget) -> String {
    // Step 1: replace intra-doc links before tokenisation (they span backtick pairs).
    let line = replace_intradoc_links(line, target);

    // Step 2: replace :: everywhere (including inside backtick spans).
    // All targets use `.` as the member/package separator, so this is always safe.
    let line = replace_path_separator(&line);

    // Step 3: strip .unwrap() and .expect() everywhere, including inside backtick spans,
    // since these Rust error-handling idioms are meaningless in all target languages.
    let line = strip_unwrap_expect(&line);

    // Step 4: tokenise and apply remaining transforms only to literal segments.
    let segments = tokenize_backtick_spans(&line);
    let mut result = String::with_capacity(line.len());
    for (is_code, span) in segments {
        if is_code {
            result.push('`');
            result.push_str(span);
            result.push('`');
        } else {
            result.push_str(&transform_prose_segment(span, target));
        }
    }
    result
}

/// Split a line into alternating literal/code segments.
///
/// Returns `Vec<(is_code, &str)>` where `is_code` is true for the content
/// between a matched backtick pair. Unmatched backticks are treated as
/// literal characters (passed through as literal segments).
fn tokenize_backtick_spans(line: &str) -> Vec<(bool, &str)> {
    let mut segments = Vec::new();
    let bytes = line.as_bytes();
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'`' {
            // Emit preceding literal segment.
            if i > start {
                segments.push((false, &line[start..i]));
            }
            // Find the closing backtick.
            let code_start = i + 1;
            let close = bytes[code_start..].iter().position(|&b| b == b'`');
            if let Some(offset) = close {
                let code_end = code_start + offset;
                segments.push((true, &line[code_start..code_end]));
                i = code_end + 1;
                start = i;
            } else {
                // No closing backtick — treat as literal from here.
                segments.push((false, &line[i..]));
                start = line.len();
                i = line.len();
            }
        } else {
            i += 1;
        }
    }
    if start < line.len() {
        segments.push((false, &line[start..]));
    }
    segments
}

/// Apply all prose-level Rust substitutions to a literal text segment.
///
/// Intra-doc links have already been replaced by `apply_prose_transforms`
/// before tokenisation; this function handles the remaining transformations.
fn transform_prose_segment(text: &str, target: DocTarget) -> String {
    let mut s = text.to_string();

    // 1. Strip #[derive(...)] and other inline attribute-style references.
    s = strip_inline_attributes(&s);

    // 2. Strip pub fn, crate::, &self, &mut self.
    s = s.replace("pub fn ", "");
    s = s.replace("crate::", "");
    s = s.replace("&mut self", "");
    s = s.replace("&self", "");

    // 3. Strip lifetime and bound markers.
    s = strip_lifetime_and_bounds(&s);

    // 4. Type substitutions (order matters — most specific first).
    s = replace_type_wrappers(&s, target);

    // 5. Some(x) -> the value (x).
    s = replace_some_calls(&s);

    // 5b. Bare "Some <lowercase>" in prose -> drop "Some ".
    s = replace_some_keyword_in_prose(&s);

    // 6. None -> null / undefined (word boundary, uppercase only).
    s = replace_none_keyword(&s, target);

    // Note: :: -> . and .unwrap()/.expect() stripping are applied to the full
    // line before tokenisation in apply_prose_transforms and therefore do not
    // need to be repeated here.

    s
}

/// Advance byte position `i` in `s` past one full UTF-8 character, push that
/// character to `out`, and return the new byte position.
///
/// All the byte-crawling helpers below look for ASCII special characters only.
/// When none matches, they must advance by one full character (not one byte)
/// to avoid splitting multi-byte UTF-8 sequences.
#[inline]
fn advance_char(s: &str, out: &mut String, i: usize) -> usize {
    // Safety: `i` must be a valid char boundary; callers guarantee this
    // because all branch points look for ASCII bytes which are always
    // single-byte char boundaries.
    let ch = s[i..].chars().next().expect("valid UTF-8 position");
    out.push(ch);
    i + ch.len_utf8()
}

/// Replace `` [`Type::method()`] `` and `` [`Foo`] `` intra-doc links with
/// backtick-wrapped identifiers, converting `::` to `.`.
fn replace_intradoc_links(s: &str, _target: DocTarget) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for [`
        if i + 1 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'`' {
            // Find closing `]
            let search_start = i + 2;
            let mut found = false;
            let mut j = search_start;
            while j + 1 < bytes.len() {
                if bytes[j] == b'`' && bytes[j + 1] == b']' {
                    let inner = &s[search_start..j];
                    // Convert :: to . in the inner part.
                    let converted = inner.replace("::", ".");
                    out.push('`');
                    out.push_str(&converted);
                    out.push('`');
                    i = j + 2;
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                i = advance_char(s, &mut out, i);
            }
        } else {
            i = advance_char(s, &mut out, i);
        }
    }
    out
}

/// Strip inline `#[...]` attribute references (not on their own line — those
/// are handled as full-line drops in the main loop).
fn strip_inline_attributes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Skip until matching ']', handling nesting.
            let mut depth = 0usize;
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'[' {
                    depth += 1;
                } else if bytes[j] == b']' {
                    depth -= 1;
                    if depth == 0 {
                        i = j + 1;
                        break;
                    }
                }
                j += 1;
            }
            if depth != 0 {
                // Unmatched bracket: emit literally.
                i = advance_char(s, &mut out, i);
            }
        } else {
            i = advance_char(s, &mut out, i);
        }
    }
    out
}

/// Strip `'static`, `Send + Sync`, `Send`, `Sync` from prose text.
fn strip_lifetime_and_bounds(s: &str) -> String {
    // Order matters: match compound forms before simple forms.
    let mut out = s.to_string();
    // Strip `Send + Sync` (with optional spaces around `+`).
    out = regex_replace_all(&out, r"Send\s*\+\s*Sync", "");
    out = regex_replace_all(&out, r"Sync\s*\+\s*Send", "");
    // Strip standalone Send/Sync only at word boundaries.
    out = regex_replace_word_boundary(&out, "Send", "");
    out = regex_replace_word_boundary(&out, "Sync", "");
    // Strip 'static lifetime markers.
    out = regex_replace_all(&out, r"'\s*static\b", "");
    out
}

/// Replace occurrences of `pattern` (treated as a simple substring pattern
/// with `\s*` only, no full regex) with `replacement` in `s`.
///
/// This is a lightweight regex-free replacement for simple patterns that
/// only need literal text or `\s*` between tokens.
fn regex_replace_all(s: &str, pattern: &str, replacement: &str) -> String {
    // Inline tiny pattern compiler for the three patterns we actually use.
    match pattern {
        r"Send\s*\+\s*Sync" => replace_with_optional_spaces(s, "Send", "+", "Sync", replacement),
        r"Sync\s*\+\s*Send" => replace_with_optional_spaces(s, "Sync", "+", "Send", replacement),
        r"'\s*static\b" => replace_static_lifetime(s, replacement),
        _ => s.replace(pattern, replacement),
    }
}

/// Replace `word_boundary(keyword)` occurrences in `s` with `replacement`.
fn regex_replace_word_boundary(s: &str, keyword: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let klen = keyword.len();
    let bytes = s.as_bytes();
    let kbytes = keyword.as_bytes();
    if klen == 0 || klen > bytes.len() {
        return s.to_string();
    }
    let mut i = 0;
    while i + klen <= bytes.len() {
        if &bytes[i..i + klen] == kbytes {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok =
                i + klen >= bytes.len() || !bytes[i + klen].is_ascii_alphanumeric() && bytes[i + klen] != b'_';
            if before_ok && after_ok {
                out.push_str(replacement);
                i += klen;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    if i < bytes.len() {
        out.push_str(&s[i..]);
    }
    out
}

/// Replace `A <spaces> op <spaces> B` triplets with `replacement`.
fn replace_with_optional_spaces(s: &str, a: &str, op: &str, b: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();
    let total = chars.len();

    while i < total {
        // Try to match `a` at position i.
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let op_chars: Vec<char> = op.chars().collect();

        if chars[i..].starts_with(&a_chars) {
            let mut j = i + a_chars.len();
            // Skip spaces.
            while j < total && chars[j] == ' ' {
                j += 1;
            }
            // Match op.
            if j + op_chars.len() <= total && chars[j..].starts_with(&op_chars) {
                let mut k = j + op_chars.len();
                // Skip spaces.
                while k < total && chars[k] == ' ' {
                    k += 1;
                }
                // Match b.
                if k + b_chars.len() <= total && chars[k..].starts_with(&b_chars) {
                    out.push_str(replacement);
                    i = k + b_chars.len();
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Replace `'static` lifetime markers (with optional spaces after `'`).
fn replace_static_lifetime(s: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            // Peek ahead skipping spaces.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let keyword = b"static";
            if bytes[j..].starts_with(keyword) {
                let end = j + keyword.len();
                // Must be followed by non-identifier char or end.
                let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_';
                if after_ok {
                    out.push_str(replacement);
                    i = end;
                    continue;
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Replace Rust generic type wrappers in prose.
fn replace_type_wrappers(s: &str, target: DocTarget) -> String {
    // Order: most specific patterns first.
    let mut out = s.to_string();

    // Vec<u8> — must come before Vec<T>.
    let vec_u8_replacement = match target {
        DocTarget::PhpDoc => "string",
        DocTarget::JavaDoc => "byte[]",
        DocTarget::TsDoc | DocTarget::JsDoc => "Uint8Array",
        DocTarget::CSharpDoc => "byte[]",
    };
    out = replace_generic1(&out, "Vec", "u8", vec_u8_replacement);

    // HashMap<K, V> — must come before Vec<T> to avoid order-dependency issues.
    let map_replacement_fn = |k: &str, v: &str| match target {
        DocTarget::PhpDoc => format!("array<{k}, {v}>"),
        DocTarget::JavaDoc => format!("Map<{k}, {v}>"),
        DocTarget::TsDoc | DocTarget::JsDoc => format!("Record<{k}, {v}>"),
        DocTarget::CSharpDoc => format!("Dictionary<{k}, {v}>"),
    };
    out = replace_generic2(&out, "HashMap", &map_replacement_fn);

    // Vec<T> — generic.
    out = replace_generic1_passthrough(&out, "Vec", |inner| format!("{inner}[]"));

    // Option<T>.
    let option_replacement_fn = |inner: &str| match target {
        DocTarget::PhpDoc => format!("{inner}?"),
        DocTarget::JavaDoc => format!("{inner} | null"),
        DocTarget::TsDoc | DocTarget::JsDoc => format!("{inner} | undefined"),
        DocTarget::CSharpDoc => format!("{inner}?"),
    };
    out = replace_generic1_passthrough(&out, "Option", option_replacement_fn);

    // Result<T, E> — drop the error type, keep the success type.
    // C# has no Result type; the binding throws exceptions, so just the success type
    // is meaningful in prose. We do this for C# only; other targets historically left
    // `Result<T, E>` unchanged (their tests assert nothing about it).
    if matches!(target, DocTarget::CSharpDoc) {
        out = replace_generic2(&out, "Result", &|t: &str, _e: &str| t.to_string());
    }

    // Smart pointer wrappers: strip to inner type.
    for wrapper in &["Arc", "Box", "Mutex", "RwLock", "Rc", "Cell", "RefCell"] {
        out = replace_generic1_passthrough(&out, wrapper, |inner| inner.to_string());
    }

    out
}

/// Replace `Name<SingleArg>` where SingleArg is an exact literal (e.g. `Vec<u8>`).
fn replace_generic1(s: &str, name: &str, arg: &str, replacement: &str) -> String {
    let pattern = format!("{name}<{arg}>");
    s.replace(&pattern, replacement)
}

/// Replace `Name<T>` → `f(T)` for an arbitrary inner type expression.
///
/// Handles nested generics by counting angle-bracket depth.
fn replace_generic1_passthrough<F>(s: &str, name: &str, f: F) -> String
where
    F: Fn(&str) -> String,
{
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let prefix = format!("{name}<");
    let pbytes = prefix.as_bytes();
    let bytes = s.as_bytes();

    while i < bytes.len() {
        if bytes[i..].starts_with(pbytes) {
            // Check that the char before is not alphanumeric (word boundary).
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let inner_start = i + pbytes.len();
                // Find the matching '>'.
                let mut depth = 1usize;
                let mut j = inner_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'<' => depth += 1,
                        b'>' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let inner = &s[inner_start..j];
                    out.push_str(&f(inner));
                    i = j + 1;
                    continue;
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Replace `Name<K, V>` → `f(K, V)` for two-argument generics (e.g. `HashMap`).
fn replace_generic2<F>(s: &str, name: &str, f: &F) -> String
where
    F: Fn(&str, &str) -> String,
{
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let prefix = format!("{name}<");
    let pbytes = prefix.as_bytes();
    let bytes = s.as_bytes();

    while i < bytes.len() {
        if bytes[i..].starts_with(pbytes) {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let inner_start = i + pbytes.len();
                // Find the matching '>' respecting nesting.
                let mut depth = 1usize;
                let mut j = inner_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'<' => depth += 1,
                        b'>' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let inner = &s[inner_start..j];
                    // Split on the first ',' at depth 0.
                    let split = split_on_comma_at_top_level(inner);
                    if let Some((k, v)) = split {
                        out.push_str(&f(k.trim(), v.trim()));
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Split `s` on the first comma that is at angle-bracket depth 0.
fn split_on_comma_at_top_level(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (idx, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some((&s[..idx], &s[idx + 1..])),
            _ => {}
        }
    }
    None
}

/// Replace `Some(x)` in prose with `the value (x)`.
fn replace_some_calls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let prefix = b"Some(";
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i..].starts_with(prefix) {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            if before_ok {
                let arg_start = i + prefix.len();
                // Find matching ')' respecting nesting.
                let mut depth = 1usize;
                let mut j = arg_start;
                while j < bytes.len() {
                    match bytes[j] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if depth == 0 && j < bytes.len() {
                    let arg = &s[arg_start..j];
                    out.push_str("the value (");
                    out.push_str(arg);
                    out.push(')');
                    i = j + 1;
                    continue;
                }
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Drop bare `Some ` when it appears as a Rust-idiom modifier in prose
/// ("(Some values)", "Some keys leave the previous", etc.). The `Some(...)`
/// call form is handled separately by [`replace_some_calls`].
///
/// Match shape: word-boundary `Some` + single ASCII space + ASCII-lowercase
/// letter. The "Some " prefix is dropped; the following word is preserved.
/// `SomeType`, `Some.method()`, `Some(x)`, and sentence-initial `Some `
/// followed by an uppercase noun stay untouched.
fn replace_some_keyword_in_prose(s: &str) -> String {
    let keyword = b"Some ";
    let klen = keyword.len();
    let bytes = s.as_bytes();
    if klen >= bytes.len() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i + klen < bytes.len() {
        if &bytes[i..i + klen] == keyword {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok = bytes[i + klen].is_ascii_lowercase();
            if before_ok && after_ok {
                i += klen;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    if i < bytes.len() {
        out.push_str(&s[i..]);
    }
    out
}

/// Replace `None` (at word boundaries, uppercase) with the target-appropriate nil.
fn replace_none_keyword(s: &str, target: DocTarget) -> String {
    let replacement = match target {
        DocTarget::PhpDoc | DocTarget::JavaDoc | DocTarget::CSharpDoc => "null",
        DocTarget::TsDoc | DocTarget::JsDoc => "undefined",
    };
    let keyword = b"None";
    let klen = keyword.len();
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    if klen > bytes.len() {
        return s.to_string();
    }
    let mut i = 0;

    while i + klen <= bytes.len() {
        if &bytes[i..i + klen] == keyword {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok =
                i + klen >= bytes.len() || !bytes[i + klen].is_ascii_alphanumeric() && bytes[i + klen] != b'_';
            if before_ok && after_ok {
                out.push_str(replacement);
                i += klen;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    if i < bytes.len() {
        out.push_str(&s[i..]);
    }
    out
}

/// Replace standalone `::` between identifiers with `.`.
fn replace_path_separator(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b':' {
            // Only replace if surrounded by identifier characters or end/start of string.
            let before_ok = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let after_ok = i + 2 < bytes.len() && (bytes[i + 2].is_ascii_alphanumeric() || bytes[i + 2] == b'_');
            if before_ok || after_ok {
                out.push('.');
                i += 2;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

/// Strip `.unwrap()` and `.expect("...")` calls from prose.
fn strip_unwrap_expect(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Match .unwrap().
        if bytes[i..].starts_with(b".unwrap()") {
            i += b".unwrap()".len();
            continue;
        }
        // Match .expect(...).
        if bytes[i..].starts_with(b".expect(") {
            let arg_start = i + b".expect(".len();
            let mut depth = 1usize;
            let mut j = arg_start;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth == 0 {
                i = j + 1;
                continue;
            }
        }
        i = advance_char(s, &mut out, i);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_phpdoc() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "Simple documentation", "    ", "TestException");
        assert!(out.contains("/**"));
        assert!(out.contains("Simple documentation"));
        assert!(out.contains("*/"));
    }

    #[test]
    fn test_phpdoc_escaping() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "Handle */ sequences", "", "TestException");
        assert!(out.contains("Handle * / sequences"));
    }

    #[test]
    fn test_emit_csharp_doc() {
        let mut out = String::new();
        emit_csharp_doc(&mut out, "C# documentation", "    ", "TestException");
        assert!(out.contains("<summary>"));
        assert!(out.contains("C# documentation"));
        assert!(out.contains("</summary>"));
    }

    #[test]
    fn test_csharp_xml_escaping() {
        let mut out = String::new();
        emit_csharp_doc(&mut out, "foo < bar & baz > qux", "", "TestException");
        assert!(out.contains("foo &lt; bar &amp; baz &gt; qux"));
    }

    #[test]
    fn test_emit_elixir_doc() {
        let mut out = String::new();
        emit_elixir_doc(&mut out, "Elixir documentation");
        assert!(out.contains("@doc \"\"\""));
        assert!(out.contains("Elixir documentation"));
        assert!(out.contains("\"\"\""));
    }

    #[test]
    fn test_elixir_heredoc_escaping() {
        let mut out = String::new();
        emit_elixir_doc(&mut out, "Handle \"\"\" sequences");
        assert!(out.contains("Handle \"\" \" sequences"));
    }

    #[test]
    fn test_emit_roxygen() {
        let mut out = String::new();
        emit_roxygen(&mut out, "R documentation");
        assert!(out.contains("#' R documentation"));
    }

    #[test]
    fn test_emit_swift_doc() {
        let mut out = String::new();
        emit_swift_doc(&mut out, "Swift documentation", "    ");
        assert!(out.contains("/// Swift documentation"));
    }

    #[test]
    fn test_emit_javadoc() {
        let mut out = String::new();
        emit_javadoc(&mut out, "Java documentation", "    ");
        assert!(out.contains("/**"));
        assert!(out.contains("Java documentation"));
        assert!(out.contains("*/"));
    }

    #[test]
    fn test_emit_kdoc() {
        let mut out = String::new();
        emit_kdoc(&mut out, "Kotlin documentation", "    ");
        assert!(out.contains("/**"));
        assert!(out.contains("Kotlin documentation"));
        assert!(out.contains("*/"));
    }

    #[test]
    fn test_emit_dartdoc() {
        let mut out = String::new();
        emit_dartdoc(&mut out, "Dart documentation", "    ");
        assert!(out.contains("/// Dart documentation"));
    }

    #[test]
    fn test_emit_gleam_doc() {
        let mut out = String::new();
        emit_gleam_doc(&mut out, "Gleam documentation", "    ");
        assert!(out.contains("/// Gleam documentation"));
    }

    #[test]
    fn test_emit_zig_doc() {
        let mut out = String::new();
        emit_zig_doc(&mut out, "Zig documentation", "    ");
        assert!(out.contains("/// Zig documentation"));
    }

    #[test]
    fn test_empty_doc_skipped() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "", "", "TestException");
        emit_csharp_doc(&mut out, "", "", "TestException");
        emit_elixir_doc(&mut out, "");
        emit_roxygen(&mut out, "");
        emit_kdoc(&mut out, "", "");
        emit_dartdoc(&mut out, "", "");
        emit_gleam_doc(&mut out, "", "");
        emit_zig_doc(&mut out, "", "");
        assert!(out.is_empty());
    }

    #[test]
    fn test_doc_first_paragraph_joined_single_line() {
        assert_eq!(doc_first_paragraph_joined("Simple doc."), "Simple doc.");
    }

    #[test]
    fn test_doc_first_paragraph_joined_wrapped_sentence() {
        // Simulates a docstring like convert's: "Convert HTML to Markdown,\nreturning a result."
        let doc = "Convert HTML to Markdown,\nreturning a result.";
        assert_eq!(
            doc_first_paragraph_joined(doc),
            "Convert HTML to Markdown, returning a result."
        );
    }

    #[test]
    fn test_doc_first_paragraph_joined_stops_at_blank_line() {
        let doc = "First paragraph.\nStill first.\n\nSecond paragraph.";
        assert_eq!(doc_first_paragraph_joined(doc), "First paragraph. Still first.");
    }

    #[test]
    fn test_doc_first_paragraph_joined_empty() {
        assert_eq!(doc_first_paragraph_joined(""), "");
    }

    #[test]
    fn test_parse_rustdoc_sections_basic() {
        let doc = "Extracts text from a file.\n\n# Arguments\n\n* `path` - The file path.\n\n# Returns\n\nThe extracted text.\n\n# Errors\n\nReturns `KreuzbergError` on failure.";
        let sections = parse_rustdoc_sections(doc);
        assert_eq!(sections.summary, "Extracts text from a file.");
        assert_eq!(sections.arguments.as_deref(), Some("* `path` - The file path."));
        assert_eq!(sections.returns.as_deref(), Some("The extracted text."));
        assert_eq!(sections.errors.as_deref(), Some("Returns `KreuzbergError` on failure."));
        assert!(sections.panics.is_none());
    }

    #[test]
    fn test_parse_rustdoc_sections_example_with_fence() {
        let doc = "Run the thing.\n\n# Example\n\n```rust\nlet x = run();\n```";
        let sections = parse_rustdoc_sections(doc);
        assert_eq!(sections.summary, "Run the thing.");
        assert!(sections.example.as_ref().unwrap().contains("```rust"));
        assert!(sections.example.as_ref().unwrap().contains("let x = run();"));
    }

    #[test]
    fn test_parse_rustdoc_sections_pound_inside_fence_is_not_a_heading() {
        // Even though we get rustdoc-hidden lines pre-stripped, a literal
        // `# foo` inside a non-rust fence (e.g. shell example) must not
        // start a new section.
        let doc = "Summary.\n\n# Example\n\n```bash\n# install deps\nrun --foo\n```";
        let sections = parse_rustdoc_sections(doc);
        assert_eq!(sections.summary, "Summary.");
        assert!(sections.example.as_ref().unwrap().contains("# install deps"));
    }

    #[test]
    fn test_parse_arguments_bullets_dash_separator() {
        let body = "* `path` - The file path.\n* `config` - Optional configuration.";
        let pairs = parse_arguments_bullets(body);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("path".to_string(), "The file path.".to_string()));
        assert_eq!(pairs[1], ("config".to_string(), "Optional configuration.".to_string()));
    }

    #[test]
    fn test_parse_arguments_bullets_continuation_line() {
        let body = "* `path` - The file path,\n  resolved relative to cwd.\n* `mode` - Open mode.";
        let pairs = parse_arguments_bullets(body);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].1, "The file path, resolved relative to cwd.");
    }

    #[test]
    fn test_replace_fence_lang_rust_to_typescript() {
        let body = "```rust\nlet x = run();\n```";
        let out = replace_fence_lang(body, "typescript");
        assert!(out.starts_with("```typescript"));
        assert!(out.contains("let x = run();"));
    }

    #[test]
    fn test_replace_fence_lang_preserves_attrs() {
        let body = "```rust,no_run\nlet x = run();\n```";
        let out = replace_fence_lang(body, "typescript");
        assert!(out.starts_with("```typescript,no_run"));
    }

    #[test]
    fn test_replace_fence_lang_no_fence_unchanged() {
        let body = "Plain prose with `inline code`.";
        let out = replace_fence_lang(body, "typescript");
        assert_eq!(out, "Plain prose with `inline code`.");
    }

    fn fixture_sections() -> RustdocSections {
        let doc = "Extracts text from a file.\n\n# Arguments\n\n* `path` - The file path.\n* `config` - Optional configuration.\n\n# Returns\n\nThe extracted text and metadata.\n\n# Errors\n\nReturns an error when the file is unreadable.\n\n# Example\n\n```rust\nlet result = extract(\"file.pdf\")?;\n```";
        parse_rustdoc_sections(doc)
    }

    #[test]
    fn test_render_jsdoc_sections() {
        let sections = fixture_sections();
        let out = render_jsdoc_sections(&sections);
        assert!(out.starts_with("Extracts text from a file."));
        assert!(out.contains("@param path - The file path."));
        assert!(out.contains("@param config - Optional configuration."));
        assert!(out.contains("@returns The extracted text and metadata."));
        assert!(out.contains("@throws Returns an error when the file is unreadable."));
        // fixture example is ```rust — stripped when target is TypeScript
        assert!(!out.contains("@example"), "Rust example must not appear in TSDoc");
        assert!(!out.contains("```typescript"));
        assert!(!out.contains("```rust"));
    }

    #[test]
    fn test_render_jsdoc_sections_preserves_typescript_example() {
        let doc = "Do something.\n\n# Example\n\n```typescript\nconst x = doSomething();\n```";
        let sections = parse_rustdoc_sections(doc);
        let out = render_jsdoc_sections(&sections);
        assert!(out.contains("@example"), "TypeScript example must be preserved");
        assert!(out.contains("```typescript"));
    }

    #[test]
    fn test_render_javadoc_sections() {
        let sections = fixture_sections();
        let out = render_javadoc_sections(&sections, "KreuzbergRsException");
        assert!(out.contains("@param path The file path."));
        assert!(out.contains("@return The extracted text and metadata."));
        assert!(out.contains("@throws KreuzbergRsException Returns an error when the file is unreadable."));
        // Java rendering omits the example block (handled separately by emit_javadoc which
        // wraps code in `<pre>{@code}</pre>`); we just confirm summary survives.
        assert!(out.starts_with("Extracts text from a file."));
    }

    #[test]
    fn test_render_csharp_xml_sections() {
        let sections = fixture_sections();
        let out = render_csharp_xml_sections(&sections, "KreuzbergException");
        assert!(out.contains("<summary>\nExtracts text from a file.\n</summary>"));
        assert!(out.contains("<param name=\"path\">The file path.</param>"));
        assert!(out.contains("<returns>The extracted text and metadata.</returns>"));
        assert!(out.contains("<exception cref=\"KreuzbergException\">"));
        assert!(out.contains("<example><code language=\"csharp\">"));
        assert!(out.contains("let result = extract"));
    }

    #[test]
    fn test_render_phpdoc_sections() {
        let sections = fixture_sections();
        let out = render_phpdoc_sections(&sections, "KreuzbergException");
        assert!(out.contains("@param mixed $path The file path."));
        assert!(out.contains("@return The extracted text and metadata."));
        assert!(out.contains("@throws KreuzbergException"));
        // fixture example is ```rust — stripped when target is PHP
        assert!(!out.contains("```php"), "Rust example must not appear in PHPDoc");
        assert!(!out.contains("```rust"));
    }

    #[test]
    fn test_render_phpdoc_sections_preserves_php_example() {
        let doc = "Do something.\n\n# Example\n\n```php\n$x = doSomething();\n```";
        let sections = parse_rustdoc_sections(doc);
        let out = render_phpdoc_sections(&sections, "MyException");
        assert!(out.contains("```php"), "PHP example must be preserved");
    }

    #[test]
    fn test_render_doxygen_sections() {
        let sections = fixture_sections();
        let out = render_doxygen_sections(&sections);
        assert!(out.contains("\\param path The file path."));
        assert!(out.contains("\\return The extracted text and metadata."));
        assert!(out.contains("\\code"));
        assert!(out.contains("\\endcode"));
    }

    #[test]
    fn test_emit_yard_doc_simple() {
        let mut out = String::new();
        emit_yard_doc(&mut out, "Simple Ruby documentation", "    ");
        assert!(out.contains("# Simple Ruby documentation"));
    }

    #[test]
    fn test_emit_yard_doc_empty() {
        let mut out = String::new();
        emit_yard_doc(&mut out, "", "    ");
        assert!(out.is_empty());
    }

    #[test]
    fn test_emit_yard_doc_with_sections() {
        let mut out = String::new();
        let doc = "Extracts text from a file.\n\n# Arguments\n\n* `path` - The file path.\n\n# Returns\n\nThe extracted text.\n\n# Errors\n\nReturns error on failure.";
        emit_yard_doc(&mut out, doc, "  ");
        assert!(out.contains("# Extracts text from a file."));
        assert!(out.contains("# @param path The file path."));
        assert!(out.contains("# @return The extracted text."));
        assert!(out.contains("# @raise Returns error on failure."));
    }

    #[test]
    fn test_emit_c_doxygen_simple_prose() {
        let mut out = String::new();
        emit_c_doxygen(&mut out, "Free a string.", "");
        assert!(out.contains("/// Free a string."), "got: {out}");
    }

    #[test]
    fn test_emit_c_doxygen_with_sections() {
        let mut out = String::new();
        let doc = "Extract content from a file.\n\n# Arguments\n\n* `path` - Path to the file.\n* `mode` - Read mode.\n\n# Returns\n\nA newly allocated string the caller owns.\n\n# Errors\n\nReturns null when the file is unreadable.";
        emit_c_doxygen(&mut out, doc, "");
        assert!(out.contains("/// Extract content from a file."));
        assert!(out.contains("/// \\param path Path to the file."));
        assert!(out.contains("/// \\param mode Read mode."));
        assert!(out.contains("/// \\return A newly allocated string the caller owns."));
        assert!(out.contains("/// \\note Returns null when the file is unreadable."));
    }

    #[test]
    fn test_emit_c_doxygen_safety_section_maps_to_note() {
        let mut out = String::new();
        let doc = "Free a buffer.\n\n# Safety\n\nPointer must have been returned by this library.";
        emit_c_doxygen(&mut out, doc, "");
        assert!(out.contains("/// \\note SAFETY: Pointer must have been returned by this library."));
    }

    #[test]
    fn test_emit_c_doxygen_example_renders_code_fence() {
        let mut out = String::new();
        let doc = "Demo.\n\n# Example\n\n```rust\nlet x = run();\n```";
        emit_c_doxygen(&mut out, doc, "");
        assert!(out.contains("/// \\code"));
        assert!(out.contains("/// \\endcode"));
        assert!(out.contains("let x = run();"));
    }

    #[test]
    fn test_emit_c_doxygen_strips_markdown_links() {
        let mut out = String::new();
        let doc = "See [the docs](https://example.com/x) for details.";
        emit_c_doxygen(&mut out, doc, "");
        assert!(
            out.contains("the docs (https://example.com/x)"),
            "expected flattened link, got: {out}"
        );
        assert!(!out.contains("](https://"));
    }

    #[test]
    fn test_emit_c_doxygen_word_wraps_long_lines() {
        let mut out = String::new();
        let long = "a ".repeat(80);
        emit_c_doxygen(&mut out, long.trim(), "");
        for line in out.lines() {
            // Each emitted prefix is "/// " (4 chars); the body after that
            // should be ≤ 100 chars per `DOXYGEN_WRAP_WIDTH`.
            let body = line.trim_start_matches("/// ");
            assert!(body.len() <= 100, "line too long ({}): {line}", body.len());
        }
    }

    #[test]
    fn test_emit_c_doxygen_empty_input_is_noop() {
        let mut out = String::new();
        emit_c_doxygen(&mut out, "", "");
        emit_c_doxygen(&mut out, "   \n\t  ", "");
        assert!(out.is_empty());
    }

    #[test]
    fn test_emit_c_doxygen_indent_applied() {
        let mut out = String::new();
        emit_c_doxygen(&mut out, "Hello.", "    ");
        assert!(out.starts_with("    /// Hello."));
    }

    #[test]
    fn test_render_yard_sections() {
        let sections = fixture_sections();
        let out = render_yard_sections(&sections);
        assert!(out.contains("@param path The file path."));
        assert!(out.contains("@return The extracted text and metadata."));
        assert!(out.contains("@raise Returns an error when the file is unreadable."));
        // fixture example is ```rust — stripped when target is Ruby
        assert!(!out.contains("@example"), "Rust example must not appear in YARD");
        assert!(!out.contains("```ruby"));
        assert!(!out.contains("```rust"));
    }

    #[test]
    fn test_render_yard_sections_preserves_ruby_example() {
        let doc = "Do something.\n\n# Example\n\n```ruby\nputs :hi\n```";
        let sections = parse_rustdoc_sections(doc);
        let out = render_yard_sections(&sections);
        assert!(out.contains("@example"), "Ruby example must be preserved");
        assert!(out.contains("```ruby"));
    }

    // --- M1: example_for_target unit tests ---

    #[test]
    fn example_for_target_rust_fenced_suppressed_for_php() {
        let example = "```rust\nlet x = 1;\n```";
        assert_eq!(
            example_for_target(example, "php"),
            None,
            "rust-fenced example must be omitted for PHP target"
        );
    }

    #[test]
    fn example_for_target_bare_fence_defaults_to_rust_suppressed_for_ruby() {
        let example = "```\nlet x = 1;\n```";
        assert_eq!(
            example_for_target(example, "ruby"),
            None,
            "bare fence is treated as Rust and must be omitted for Ruby target"
        );
    }

    #[test]
    fn example_for_target_php_example_preserved_for_php() {
        let example = "```php\n$x = 1;\n```";
        let result = example_for_target(example, "php");
        assert!(result.is_some(), "PHP example must be preserved for PHP target");
        assert!(result.unwrap().contains("```php"));
    }

    #[test]
    fn example_for_target_ruby_example_preserved_for_ruby() {
        let example = "```ruby\nputs :hi\n```";
        let result = example_for_target(example, "ruby");
        assert!(result.is_some(), "Ruby example must be preserved for Ruby target");
        assert!(result.unwrap().contains("```ruby"));
    }

    #[test]
    fn render_phpdoc_sections_with_rust_example_emits_no_at_example_block() {
        let doc = "Convert HTML.\n\n# Arguments\n\n* `html` - The HTML input.\n\n# Example\n\n```rust\nlet result = convert(html, None)?;\n```";
        let sections = parse_rustdoc_sections(doc);
        let out = render_phpdoc_sections(&sections, "HtmlToMarkdownException");
        assert!(!out.contains("```php"), "no PHP @example block for Rust source");
        assert!(!out.contains("```rust"), "raw Rust must not leak into PHPDoc");
        assert!(out.contains("@param"), "other sections must still be emitted");
    }

    // --- KDoc ktfmt-canonical format tests ---

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_short_single_line() {
        let mut out = String::new();
        emit_kdoc_ktfmt_canonical(&mut out, "Simple doc.", "");
        assert_eq!(
            out, "/** Simple doc. */\n",
            "short single-line comment should collapse to canonical format"
        );
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_short_with_indent() {
        let mut out = String::new();
        emit_kdoc_ktfmt_canonical(&mut out, "Text node (most frequent - 100+ per document)", "    ");
        assert_eq!(out, "    /** Text node (most frequent - 100+ per document) */\n");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_long_comment_uses_multiline() {
        let mut out = String::new();
        let long_text = "This is a very long documentation comment that exceeds the 100-character line width limit and should therefore be emitted in multi-line format";
        emit_kdoc_ktfmt_canonical(&mut out, long_text, "");
        assert!(out.contains("/**\n"), "long comment should start with newline");
        assert!(out.contains(" * "), "long comment should use multi-line format");
        assert!(out.contains(" */\n"), "long comment should end with newline");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_multiline_comment() {
        let mut out = String::new();
        let doc = "First line.\n\nSecond paragraph.";
        emit_kdoc_ktfmt_canonical(&mut out, doc, "");
        assert!(out.contains("/**\n"), "multi-paragraph should use multi-line format");
        assert!(out.contains(" * First line."), "first paragraph preserved");
        assert!(out.contains(" *\n"), "blank line preserved");
        assert!(out.contains(" * Second paragraph."), "second paragraph preserved");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_empty_doc() {
        let mut out = String::new();
        emit_kdoc_ktfmt_canonical(&mut out, "", "");
        assert!(out.is_empty(), "empty doc should produce no output");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_fits_within_100_chars() {
        let mut out = String::new();
        // Construct exactly at the boundary: indent(0) + "/** " + content + " */" = 100 chars
        // "/** " = 4 chars, " */" = 3 chars, so content can be 93 chars
        let content = "a".repeat(93);
        emit_kdoc_ktfmt_canonical(&mut out, &content, "");
        let line = out.lines().next().unwrap();
        assert_eq!(
            line.len(),
            100,
            "should fit exactly at 100 chars and use single-line format"
        );
        assert!(out.starts_with("/**"), "should use single-line format");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_exceeds_100_chars() {
        let mut out = String::new();
        // Exceed 100 chars: content of 94 chars with "/** " + " */" = 101 chars
        let content = "a".repeat(94);
        emit_kdoc_ktfmt_canonical(&mut out, &content, "");
        assert!(
            out.contains("/**\n"),
            "should use multi-line format when exceeding 100 chars"
        );
        assert!(out.contains(" * "), "multi-line format with ` * ` prefix");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_respects_indent() {
        let mut out = String::new();
        // With 4-char indent, max content is 89 chars (4 + 4 + 89 + 3 = 100)
        let content = "a".repeat(89);
        emit_kdoc_ktfmt_canonical(&mut out, &content, "    ");
        let line = out.lines().next().unwrap();
        assert_eq!(line.len(), 100, "should respect indent in 100-char calculation");
        assert!(line.starts_with("    /** "), "should include indent");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_real_world_enum_variant() {
        let mut out = String::new();
        emit_kdoc_ktfmt_canonical(&mut out, "Text node (most frequent - 100+ per document)", "    ");
        // This is from NodeType enum; should collapse to single-line
        assert!(out.starts_with("    /** "), "should preserve 4-space indent");
        assert!(out.contains(" */\n"), "should end with newline");
        // Verify it's single-line format
        let line_count = out.lines().count();
        assert_eq!(line_count, 1, "should be single-line format");
    }

    #[test]
    fn test_emit_kdoc_ktfmt_canonical_real_world_data_class_field() {
        let mut out = String::new();
        let doc = "Heading style to use in Markdown output (ATX `#` or Setext underline).";
        emit_kdoc_ktfmt_canonical(&mut out, doc, "    ");
        // This is from ConversionOptions data class; should collapse to single-line
        let line_count = out.lines().count();
        assert_eq!(line_count, 1, "should be single-line format");
        assert!(out.starts_with("    /** "), "should have correct indent");
    }

    // --- sanitize_rust_idioms tests ---

    #[test]
    fn sanitize_intradoc_link_with_path_separator_java() {
        let input = "See [`ConversionOptions::builder()`] for details.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("`ConversionOptions.builder()`"), "got: {out}");
        assert!(!out.contains("[`"), "brackets must be removed, got: {out}");
    }

    #[test]
    fn sanitize_intradoc_link_simple_type_php() {
        let input = "Returns a [`ConversionResult`].";
        let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
        assert!(out.contains("`ConversionResult`"), "got: {out}");
        assert!(!out.contains("[`"), "got: {out}");
    }

    #[test]
    fn sanitize_none_to_null_javadoc() {
        let input = "Returns None when no value is found.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("null"), "got: {out}");
        assert!(!out.contains("None"), "got: {out}");
    }

    #[test]
    fn sanitize_none_to_undefined_tsdoc() {
        let input = "Returns None if absent.";
        let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
        assert!(out.contains("undefined"), "got: {out}");
        assert!(!out.contains("None"), "got: {out}");
    }

    #[test]
    fn sanitize_some_x_to_the_value_x() {
        let input = "Pass Some(value) to enable.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("the value (value)"), "got: {out}");
        assert!(!out.contains("Some("), "got: {out}");
    }

    #[test]
    fn sanitize_bare_some_followed_by_lowercase_noun_is_dropped() {
        // Real leak from html-to-markdown PreprocessingOptionsUpdate.java:16.
        let input =
            "Only specified fields (Some values) will override existing options; None values leave the previous";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(
            out.contains("(values)"),
            "bare `Some ` before lowercase noun must be stripped; got: {out}"
        );
        assert!(
            out.contains("null values"),
            "bare `None ` must also be replaced; got: {out}"
        );
        assert!(!out.contains("Some "), "Some prefix must not survive; got: {out}");
    }

    #[test]
    fn sanitize_bare_some_does_not_touch_identifiers_or_uppercase_followers() {
        // SomeType, Some.method(), Some(x), and "Some Title" (proper noun) all preserved.
        let cases = [
            "SomeType lives on.",
            "Some.method() returns Self.",
            "Some Title",
            "Some(x) is a value.",
        ];
        for case in cases {
            let out = sanitize_rust_idioms(case, DocTarget::JavaDoc);
            // For the Some(x) case, replace_some_calls (run earlier) converts to "the value (x)"
            // so "Some" itself is gone — that's expected; everything else preserves "Some".
            if case.starts_with("Some(") {
                assert!(out.contains("the value (x)"), "got: {out}");
            } else {
                assert!(out.contains("Some"), "Some must survive in {case:?}; got: {out}");
            }
        }
    }

    #[test]
    fn sanitize_option_t_to_nullable_php() {
        let input = "The result is Option<String>.";
        let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
        assert!(out.contains("String?"), "got: {out}");
        assert!(!out.contains("Option<"), "got: {out}");
    }

    #[test]
    fn sanitize_option_t_to_or_null_java() {
        let input = "The result is Option<String>.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("String | null"), "got: {out}");
    }

    #[test]
    fn sanitize_option_t_to_or_undefined_tsdoc() {
        let input = "The result is Option<String>.";
        let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
        assert!(out.contains("String | undefined"), "got: {out}");
    }

    #[test]
    fn sanitize_vec_u8_per_target() {
        assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::PhpDoc).contains("string"));
        assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::JavaDoc).contains("byte[]"));
        assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::TsDoc).contains("Uint8Array"));
        assert!(sanitize_rust_idioms("Takes Vec<u8>.", DocTarget::JsDoc).contains("Uint8Array"));
    }

    #[test]
    fn sanitize_vec_t_to_array() {
        let input = "Returns Vec<String>.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("String[]"), "got: {out}");
        assert!(!out.contains("Vec<"), "got: {out}");
    }

    #[test]
    fn sanitize_hashmap_per_target() {
        let input = "Uses HashMap<String, u32>.";
        assert!(sanitize_rust_idioms(input, DocTarget::PhpDoc).contains("array<String, u32>"));
        assert!(sanitize_rust_idioms(input, DocTarget::JavaDoc).contains("Map<String, u32>"));
        assert!(sanitize_rust_idioms(input, DocTarget::TsDoc).contains("Record<String, u32>"));
    }

    #[test]
    fn sanitize_arc_wrapper_stripped() {
        let input = "Holds Arc<Config>.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("Config"), "got: {out}");
        assert!(!out.contains("Arc<"), "got: {out}");
    }

    #[test]
    fn sanitize_box_mutex_rwlock_rc_cell_refcell_stripped() {
        for wrapper in &["Box", "Mutex", "RwLock", "Rc", "Cell", "RefCell"] {
            let input = format!("Contains {wrapper}<Inner>.");
            let out = sanitize_rust_idioms(&input, DocTarget::JavaDoc);
            assert!(out.contains("Inner"), "wrapper {wrapper} not stripped, got: {out}");
            assert!(
                !out.contains(&format!("{wrapper}<")),
                "wrapper {wrapper} still present, got: {out}"
            );
        }
    }

    #[test]
    fn sanitize_send_sync_stripped() {
        let input = "The type is Send + Sync.";
        let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
        assert!(!out.contains("Send"), "got: {out}");
        assert!(!out.contains("Sync"), "got: {out}");
    }

    #[test]
    fn sanitize_static_lifetime_stripped() {
        let input = "Requires 'static lifetime.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(!out.contains("'static"), "got: {out}");
    }

    #[test]
    fn sanitize_pub_fn_stripped() {
        let input = "Calls pub fn convert().";
        let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
        assert!(!out.contains("pub fn"), "got: {out}");
        assert!(out.contains("convert()"), "got: {out}");
    }

    #[test]
    fn sanitize_crate_prefix_stripped() {
        let input = "See crate::error::ConversionError.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(!out.contains("crate::"), "got: {out}");
        assert!(out.contains("error.ConversionError"), "got: {out}");
    }

    #[test]
    fn sanitize_unwrap_expect_stripped() {
        let input = "Call result.unwrap() or result.expect(\"msg\").";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(!out.contains(".unwrap()"), "got: {out}");
        assert!(!out.contains(".expect("), "got: {out}");
    }

    #[test]
    fn sanitize_no_mutation_inside_backticks() {
        // None inside backtick span must not be replaced.
        let input = "Use `None` as the argument.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("`None`"), "backtick span must be preserved, got: {out}");
    }

    #[test]
    fn sanitize_rust_fence_dropped_for_tsdoc() {
        let input = "Intro.\n\n```rust\nlet x = 1;\n```\n\nTrailer.";
        let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
        assert!(
            !out.contains("let x = 1;"),
            "rust fence content must be dropped, got: {out}"
        );
        assert!(!out.contains("```rust"), "got: {out}");
        assert!(out.contains("Trailer."), "text after fence must survive, got: {out}");
    }

    #[test]
    fn sanitize_rust_fence_dropped_for_java() {
        let input = "Intro.\n\n```rust\nlet x = 1;\n```\n\nTrailer.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        // Rust fences are now dropped entirely for Java (Rust code is not portable).
        assert!(
            !out.contains("let x = 1;"),
            "fence content must be dropped for Java, got: {out}"
        );
        assert!(!out.contains("```"), "fence markers must be dropped, got: {out}");
        assert!(out.contains("Intro."), "prose before fence kept: {out}");
        assert!(out.contains("Trailer."), "prose after fence kept: {out}");
    }

    #[test]
    fn sanitize_non_rust_fence_passed_through() {
        let input = "Example:\n\n```typescript\nconst x = 1;\n```";
        let out = sanitize_rust_idioms(input, DocTarget::TsDoc);
        assert!(out.contains("```typescript"), "non-rust fence must survive, got: {out}");
        assert!(out.contains("const x = 1;"), "got: {out}");
    }

    #[test]
    fn sanitize_backtick_code_span_not_mutated_option() {
        // Option<T> inside backtick span must not be replaced.
        let input = "The type is `Option<String>`.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        // The backtick-protected span should be preserved verbatim.
        assert!(
            out.contains("`Option<String>`"),
            "code span must be preserved, got: {out}"
        );
    }

    #[test]
    fn sanitize_idempotent() {
        // Running twice should produce the same result as running once.
        let input = "Returns None when Vec<String> is empty.";
        let once = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        let twice = sanitize_rust_idioms(&once, DocTarget::JavaDoc);
        assert_eq!(once, twice, "sanitize_rust_idioms should be idempotent");
    }

    #[test]
    fn sanitize_multiline_prose() {
        let input = "Convert HTML to Markdown.\n\nReturns None on failure.\nUse Option<String> for the result.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("null"), "None must be replaced on line 2, got: {out}");
        assert!(
            out.contains("String | null"),
            "Option<String> must be replaced on line 3, got: {out}"
        );
    }

    #[test]
    fn sanitize_attribute_line_dropped() {
        let input = "#[derive(Debug, Clone)]\nSome documentation.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(!out.contains("#[derive("), "attribute line must be dropped, got: {out}");
        // Prose survives, though bare "Some " before a lowercase noun is stripped
        // by `replace_some_keyword_in_prose`, so accept either form.
        assert!(out.contains("documentation."), "prose must survive, got: {out}");
    }

    #[test]
    fn sanitize_path_separator_in_prose() {
        let input = "See std::collections::HashMap for details.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("std.collections.HashMap"), ":: must become ., got: {out}");
    }

    #[test]
    fn sanitize_none_not_replaced_inside_identifier() {
        // "NoneType" must not be replaced.
        let input = "Unlike NoneType in Python.";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(out.contains("NoneType"), "NoneType must not be replaced, got: {out}");
    }

    // --- CSharpDoc target tests ---

    #[test]
    fn sanitize_csharp_drops_rust_section_headings_and_example_body() {
        // The GraphQLErrorException case: `# Examples` heading followed by a
        // ```ignore code fence containing `Self::error_code`, `Result<T, E>`,
        // intra-doc links — all of which previously leaked into `<summary>`.
        let input = "Convert error to HTTP status code\n\n\
            Maps GraphQL error types to status codes.\n\n\
            # Examples\n\n\
            ```ignore\n\
            use spikard_graphql::error::GraphQLError;\n\
            let error = GraphQLError::AuthenticationError(\"Invalid token\".to_string());\n\
            assert_eq!(error.status_code(), 401);\n\
            ```\n";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        assert!(
            out.contains("Convert error to HTTP status code"),
            "summary preserved: {out}"
        );
        assert!(out.contains("Maps GraphQL error types"), "prose preserved: {out}");
        assert!(!out.contains("# Examples"), "heading dropped: {out}");
        assert!(!out.contains("```"), "code fence dropped: {out}");
        assert!(!out.contains("Self::error_code"), "Self::method dropped: {out}");
        assert!(
            !out.contains("GraphQLError::AuthenticationError"),
            "rust path dropped: {out}"
        );
    }

    #[test]
    fn sanitize_csharp_intradoc_link_with_path_separator() {
        let input = "See [`Self::error_code`] for the variant codes.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        assert!(out.contains("`Self.error_code`"), "intra-doc link normalised: {out}");
        assert!(!out.contains("[`"), "square brackets removed: {out}");
        assert!(!out.contains("::"), ":: replaced with .: {out}");
    }

    #[test]
    fn sanitize_csharp_result_type_keeps_success_drops_error() {
        let input = "Returns Result<String, ConversionError> on failure.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        assert!(out.contains("String"), "success type kept: {out}");
        assert!(!out.contains("Result<"), "Result wrapper dropped: {out}");
        assert!(!out.contains("ConversionError"), "error type dropped: {out}");
    }

    #[test]
    fn sanitize_csharp_option_becomes_nullable() {
        let input = "Returns Option<String>.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        // After XML-escaping, the `?` survives but any surviving `<`/`>` get escaped.
        assert!(out.contains("String?"), "Option<T> -> T?: {out}");
        assert!(!out.contains("Option<"), "Option dropped: {out}");
    }

    #[test]
    fn sanitize_csharp_vec_u8_becomes_byte_array() {
        let input = "Accepts Vec<u8>.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        // `byte[]` survives — the `[` is not XML-significant.
        assert!(out.contains("byte[]"), "Vec<u8> -> byte[]: {out}");
    }

    #[test]
    fn sanitize_csharp_hashmap_becomes_dictionary() {
        let input = "Holds HashMap<String, u32>.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        // The `<` / `>` produced by Dictionary<K, V> must be XML-escaped.
        assert!(
            out.contains("Dictionary&lt;String, u32&gt;"),
            "HashMap -> Dictionary with XML-escaped brackets: {out}"
        );
    }

    #[test]
    fn sanitize_csharp_none_to_null() {
        let input = "Returns None on miss.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        assert!(out.contains("null"), "None -> null: {out}");
        assert!(!out.contains("None"), "None replaced: {out}");
    }

    #[test]
    fn sanitize_csharp_escapes_raw_angle_brackets_and_amp() {
        // Unrecognised `<...>` constructs (e.g. trait objects, generic params on
        // unknown names) must still be XML-escaped so the result is valid inside
        // `<summary>`.
        let input = "Accepts Box<dyn Trait> and combines a & b.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        // Box<T> wrapper is stripped to inner type, leaving `dyn Trait`.
        assert!(out.contains("dyn Trait"), "Box<T> stripped: {out}");
        assert!(out.contains("&amp;"), "ampersand escaped: {out}");
    }

    #[test]
    fn sanitize_csharp_drops_rust_code_fence_entirely() {
        let input = "Intro.\n\n```rust\nlet x: Vec<u8> = vec![];\n```\n\nTrailer.";
        let out = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        assert!(!out.contains("let x"), "code fence body dropped: {out}");
        assert!(!out.contains("```"), "fence markers dropped: {out}");
        assert!(out.contains("Intro."), "prose before fence kept: {out}");
        assert!(out.contains("Trailer."), "prose after fence kept: {out}");
    }

    #[test]
    fn sanitize_csharp_keep_sections_does_not_drop_headings() {
        // The sections-preserving variant leaves heading lines alone so callers
        // that have already extracted sections can sanitise each body fragment.
        let input = "Summary.\n\n# Arguments\n\n* `name` - the value.";
        let out = sanitize_rust_idioms_keep_sections(input, DocTarget::CSharpDoc);
        assert!(out.contains("# Arguments"), "heading preserved: {out}");
        assert!(out.contains("name"), "body preserved: {out}");
    }

    #[test]
    fn sanitize_csharp_idempotent() {
        let input = "Returns Option<String> or None.";
        let once = sanitize_rust_idioms(input, DocTarget::CSharpDoc);
        let twice = sanitize_rust_idioms(&once, DocTarget::CSharpDoc);
        assert_eq!(once, twice, "CSharpDoc sanitisation must be idempotent");
    }

    #[test]
    fn sanitize_phpdoc_drops_unmarked_rust_code_fences() {
        // Regression test: unmarked code fences (```\n...\n```) in Rust docstrings
        // are treated as Rust code and should be dropped for PHP target.
        let input = "Detect language name from a file extension.\n\nReturns `None` for unrecognized extensions.\n\n```\nuse tree_sitter_language_pack::detect_language_from_extension;\nassert_eq!(detect_language_from_extension(\"py\"), Some(\"python\"));\nassert_eq!(detect_language_from_extension(\"RS\"), Some(\"rust\"));\nassert_eq!(detect_language_from_extension(\"xyz\"), None);\n```";
        let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
        assert!(!out.contains("use tree_sitter_language_pack"), "Rust use stmt dropped: {out}");
        assert!(!out.contains("assert_eq!"), "Rust code dropped: {out}");
        assert!(!out.contains("```"), "fence markers dropped: {out}");
        assert!(out.contains("Detect language name"), "prose before fence kept: {out}");
        assert!(out.contains("unrecognized extensions"), "prose kept: {out}");
    }

    #[test]
    fn sanitize_javadoc_drops_unmarked_rust_code_fences() {
        // Regression test: unmarked code fences in Rust docstrings should be dropped
        // for Java target as well.
        let input = "Process a file.\n\n```\nlet result = process(\"def hello(): pass\", &config).unwrap();\n```";
        let out = sanitize_rust_idioms(input, DocTarget::JavaDoc);
        assert!(!out.contains("unwrap"), "Rust unwrap dropped: {out}");
        assert!(!out.contains("```"), "fence markers dropped: {out}");
        assert!(out.contains("Process a file"), "prose kept: {out}");
    }

    #[test]
    fn sanitize_phpdoc_drops_explicit_rust_fences() {
        // Explicit ```rust fences should also be dropped for PHP.
        let input = "Summary.\n\n```rust\nuse std::path::PathBuf;\nlet p = PathBuf::from(\"/tmp\");\n```";
        let out = sanitize_rust_idioms(input, DocTarget::PhpDoc);
        assert!(!out.contains("use std::"), "Rust code dropped: {out}");
        assert!(!out.contains("PathBuf"), "Rust types dropped: {out}");
        assert!(!out.contains("```"), "fence markers dropped: {out}");
        assert!(out.contains("Summary"), "prose kept: {out}");
    }
}
