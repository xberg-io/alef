//! Language-native documentation comment emission.
//! Provides standardized functions for emitting doc comments in different languages.

/// Emit PHPDoc-style comments (/** ... */)
/// Used for PHP classes, methods, and properties.
///
/// Translates rustdoc sections (`# Arguments` → `@param`,
/// `# Returns` → `@return`, `# Errors` → `@throws`,
/// `# Example` → ` ```php ` fence) via [`render_phpdoc_sections`].
pub fn emit_phpdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let sections = parse_rustdoc_sections(doc);
    let any_section = sections.arguments.is_some()
        || sections.returns.is_some()
        || sections.errors.is_some()
        || sections.example.is_some();
    let body = if any_section {
        render_phpdoc_sections(&sections, "KreuzbergException")
    } else {
        doc.to_string()
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
pub fn emit_csharp_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let sections = parse_rustdoc_sections(doc);
    let any_section = sections.arguments.is_some()
        || sections.returns.is_some()
        || sections.errors.is_some()
        || sections.example.is_some();
    if !any_section {
        // Backwards-compatible path: plain `<summary>` for prose-only docs.
        out.push_str(indent);
        out.push_str("/// <summary>\n");
        for line in doc.lines() {
            out.push_str(indent);
            out.push_str("/// ");
            out.push_str(&escape_csharp_doc_line(line));
            out.push('\n');
        }
        out.push_str(indent);
        out.push_str("/// </summary>\n");
        return;
    }
    let rendered = render_csharp_xml_sections(&sections, "KreuzbergException");
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

/// Escape C# XML doc line: handle XML special characters.
fn escape_csharp_doc_line(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
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
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("@example\n");
        out.push_str(&replace_fence_lang(example.trim(), "typescript"));
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
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&replace_fence_lang(example.trim(), "php"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_phpdoc() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "Simple documentation", "    ");
        assert!(out.contains("/**"));
        assert!(out.contains("Simple documentation"));
        assert!(out.contains("*/"));
    }

    #[test]
    fn test_phpdoc_escaping() {
        let mut out = String::new();
        emit_phpdoc(&mut out, "Handle */ sequences", "");
        assert!(out.contains("Handle * / sequences"));
    }

    #[test]
    fn test_emit_csharp_doc() {
        let mut out = String::new();
        emit_csharp_doc(&mut out, "C# documentation", "    ");
        assert!(out.contains("<summary>"));
        assert!(out.contains("C# documentation"));
        assert!(out.contains("</summary>"));
    }

    #[test]
    fn test_csharp_xml_escaping() {
        let mut out = String::new();
        emit_csharp_doc(&mut out, "foo < bar & baz > qux", "");
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
        emit_phpdoc(&mut out, "", "");
        emit_csharp_doc(&mut out, "", "");
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
        assert!(out.contains("@example"));
        assert!(out.contains("```typescript"));
        assert!(!out.contains("```rust"));
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
        assert!(out.contains("```php"));
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
}
