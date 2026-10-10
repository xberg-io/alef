#[derive(Default)]
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

/// Rustdoc section names that `docs::doc_cleaning::convert_doc_headings_to_bold` converts to an
/// inline bold label (`**Note:**`) rather than a Markdown heading. This is the single list both
/// pipelines consult, so they cannot drift apart again.
///
/// Ordering matters in `parse_rustdoc_sections`: the recognised-section match runs FIRST, so
/// `errors`/`returns`/`panics`/`safety` are captured into their own [`RustdocSections`] field
/// and never reach the drop branch below. Only `note`/`notes` -- which have no field -- do.
/// Such a name must still end the current section rather than be folded into it (the default
/// for unrecognised headings), or its prose resurfaces raw inside e.g. the `example` field
/// alongside the bold label the docs pipeline already emitted for the same heading. That is
/// alef #192: a `# Note` after `# Examples` rendered twice -- once bolded, once verbatim
/// inside the Example block. ~keep
pub const BOLD_LABEL_ONLY_SECTION_NAMES: &[&str] = &["errors", "returns", "panics", "safety", "notes", "note"];

/// Parse a rustdoc string into [`RustdocSections`].
///
/// Recognises level-1 ATX headings whose name matches one of the standard
/// rustdoc section names (`Arguments`, `Returns`, `Errors`, `Panics`,
/// `Safety`, `Example`, `Examples`). Anything before the first heading
/// becomes `summary`. Unrecognised headings are folded into the
/// preceding section verbatim, so unconventional rustdoc isn't lost --
/// except for [`BOLD_LABEL_ONLY_SECTION_NAMES`], which end the current
/// section and are dropped rather than folded in, since another pipeline
/// already owns and emits that content. ~keep
///
/// The input is expected to already have rustdoc-hidden lines stripped
/// and intra-doc-link syntax rewritten by
/// `crate::extract::extractor::helpers::normalize_rustdoc`.
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
    // While true, incoming lines belong to a section owned elsewhere (see
    // `BOLD_LABEL_ONLY_SECTION_NAMES`) and must not be folded into `buf`.
    let mut dropping = false;
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
            if !dropping {
                buf.push_str(line);
                buf.push('\n');
            }
            continue;
        }
        if !in_fence && let Some(rest) = trimmed.strip_prefix("# ") {
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
                dropping = false;
                continue;
            }
            if BOLD_LABEL_ONLY_SECTION_NAMES.contains(&head.as_str()) {
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
                current = None;
                dropping = true;
                continue;
            }
        }
        if !dropping {
            buf.push_str(line);
            buf.push('\n');
        }
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
        } else if !trimmed.is_empty()
            && let Some(last) = out.last_mut()
        {
            if !last.1.is_empty() {
                last.1.push(' ');
            }
            last.1.push_str(trimmed);
        }
    }
    out
}

/// Return `true` if `tag` (the first comma-separated token after the opening
/// ` ``` ` of a code fence) identifies a Rust code block.
///
/// This covers:
/// - bare tag (empty string) — rustdoc treats unlabelled fences as Rust by default
/// - `"rust"` — explicit Rust
/// - `"rust,<attrs>"` — Rust with trailing comma-separated attributes
/// - rustdoc test-attribute-only fences: `no_run`, `ignore`, `should_panic`,
///   `compile_fail` — these are only meaningful to rustdoc and always indicate
///   Rust code even when `rust` itself is omitted
/// - `"edition2018"`, `"edition2021"`, etc. — edition-gated Rust examples
pub(super) fn is_rust_fence_tag(tag: &str) -> bool {
    tag.is_empty()
        || tag == "rust"
        || tag.starts_with("rust,")
        || NAMED_RUSTDOC_FENCE_ATTRIBUTES.contains(&tag)
        || tag.starts_with("edition")
}

/// rustdoc's named code-block attributes, as documented in the rustdoc book.
///
/// Excludes `rust` itself, which names the language rather than qualifying the doctest.
const NAMED_RUSTDOC_FENCE_ATTRIBUTES: &[&str] = &[
    "no_run",
    "ignore",
    "should_panic",
    "compile_fail",
    "test_harness",
    "standalone_crate",
];

/// Whether `attribute` is a rustdoc code-block attribute rather than renderer meta.
///
/// ~keep rustdoc's vocabulary is finite and documented; anything in it steers rustdoc's
/// doctest harness and means nothing to a markdown renderer, so it is safe to drop.
/// Anything outside it is deliberately kept — silently discarding a token a consumer
/// wrote is worse than carrying it through as fence meta.
fn is_rustdoc_fence_attribute(attribute: &str) -> bool {
    if attribute == "rust" || NAMED_RUSTDOC_FENCE_ATTRIBUTES.contains(&attribute) {
        return true;
    }
    // `edition2015` / `edition2018` / `edition2021` / `edition2024`.
    if let Some(year) = attribute.strip_prefix("edition") {
        return !year.is_empty() && year.chars().all(|character| character.is_ascii_digit());
    }
    // Platform-scoped ignores, e.g. `ignore-wasm32`.
    if let Some(target) = attribute.strip_prefix("ignore-") {
        return !target.is_empty();
    }
    // Expected-error codes on a `compile_fail` block, e.g. `E0308`.
    if let Some(code) = attribute.strip_prefix('E') {
        return code.len() == 4 && code.chars().all(|character| character.is_ascii_digit());
    }
    false
}

/// Convert a rustdoc fence info string's comma-separated attribute tail into markdown
/// fence meta.
///
/// ~keep A markdown info string is `<language> <meta…>`: the language is the first
/// whitespace-delimited token, and commas are not part of that grammar. Copying
/// rustdoc's `rust,no_run` through verbatim therefore yields the language `rust,no_run`,
/// which renderers and `alef snippets audit --docs` both reject as unknown. Recognised
/// rustdoc attributes are dropped; anything unrecognised is re-joined with spaces so it
/// lands in the meta slot instead of corrupting the language token.
fn rustdoc_attributes_as_fence_meta(attribute_tail: &str) -> String {
    let mut meta = String::new();
    for attribute in attribute_tail.split(',') {
        let attribute = attribute.trim();
        if attribute.is_empty() || is_rustdoc_fence_attribute(attribute) {
            continue;
        }
        meta.push(' ');
        meta.push_str(attribute);
    }
    meta
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
            return if tag.is_empty() || is_rust_fence_tag(tag) {
                "rust"
            } else {
                tag
            };
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
/// Only opening fences gain the language tag — the matching closing fence is
/// always emitted bare (` ``` `), exactly like the Markdown it is closing. A
/// fence line is an opener the first time it appears and a closer the next,
/// alternating for every fenced block found in `body`.
///
/// When no fence is present the body is returned unchanged. Used by
/// emitters that need to convert ` ```rust ` examples into
/// ` ```typescript ` / ` ```python ` / ` ```swift ` etc.
pub fn replace_fence_lang(body: &str, lang_replacement: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            let indent = &line[..line.len() - trimmed.len()];
            out.push_str(indent);
            out.push_str("```");
            if in_fence {
                // Closing fence: always bare, never re-tagged with a language.
            } else {
                let attribute_tail = rest.find(',').map(|index| &rest[index + 1..]).unwrap_or("");
                out.push_str(lang_replacement);
                out.push_str(&rustdoc_attributes_as_fence_meta(attribute_tail));
            }
            in_fence = !in_fence;
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
                out.push_str(&crate::codegen::template_env::render(
                    "doc_jsdoc_param.jinja",
                    crate::alef_context! { name => &name },
                ));
            } else {
                out.push_str(&crate::codegen::template_env::render(
                    "doc_jsdoc_param_desc.jinja",
                    crate::alef_context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_jsdoc_returns.jinja",
            crate::alef_context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_jsdoc_throws.jinja",
            crate::alef_context! { content => err.trim() },
        ));
    }
    if let Some(example) = sections.example.as_deref()
        && let Some(body) = example_for_target(example, "typescript")
    {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("@example\n");
        out.push_str(&body);
    }
    out
}

/// Render `RustdocSections` as a JavaDoc comment body.
///
/// - `# Arguments` → `@param name desc` (one per param)
/// - `# Returns`   → `@return desc`
/// - `# Errors`    → `@throws SampleCrateRsException desc`
/// - `# Example`   → `<pre>{@code ...}</pre>` block.
///
/// `throws_class` is the FQN/simple name of the exception class to use in
/// the `@throws` tag (e.g. `"SampleCrateRsException"`).
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
            // Java checkstyle requires @param names to match the Java camelCase parameter name,
            let java_name = heck::ToLowerCamelCase::to_lower_camel_case(name.as_str());
            if desc.is_empty() {
                out.push_str(&crate::codegen::template_env::render(
                    "doc_javadoc_param.jinja",
                    crate::alef_context! { name => &java_name },
                ));
            } else {
                out.push_str(&crate::codegen::template_env::render(
                    "doc_javadoc_param_desc.jinja",
                    crate::alef_context! { name => &java_name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_javadoc_return.jinja",
            crate::alef_context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_javadoc_throws.jinja",
            crate::alef_context! { throws_class => throws_class, content => err.trim() },
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
/// - errors   → `<exception cref="SampleCrateException">desc</exception>`
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
                out.push_str(&crate::codegen::template_env::render(
                    "doc_csharp_param.jinja",
                    crate::alef_context! { name => &name },
                ));
            } else {
                out.push_str(&crate::codegen::template_env::render(
                    "doc_csharp_param_desc.jinja",
                    crate::alef_context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        out.push('\n');
        out.push_str(&crate::codegen::template_env::render(
            "doc_csharp_returns.jinja",
            crate::alef_context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        out.push('\n');
        out.push_str(&crate::codegen::template_env::render(
            "doc_csharp_exception.jinja",
            crate::alef_context! {
                exception_class => exception_class,
                content => err.trim(),
            },
        ));
    }
    if let Some(example) = sections.example.as_deref() {
        out.push('\n');
        out.push_str("<example><code language=\"csharp\">\n");
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
/// - `# Errors`    → `@throws SampleCrateException desc`
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
                out.push_str(&crate::codegen::template_env::render(
                    "doc_phpdoc_param.jinja",
                    crate::alef_context! { name => &name },
                ));
            } else {
                out.push_str(&crate::codegen::template_env::render(
                    "doc_phpdoc_param_desc.jinja",
                    crate::alef_context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_phpdoc_return.jinja",
            crate::alef_context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_phpdoc_throws.jinja",
            crate::alef_context! { throws_class => throws_class, content => err.trim() },
        ));
    }
    if let Some(example) = sections.example.as_deref()
        && let Some(body) = example_for_target(example, "php")
    {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&body);
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
                out.push_str(&crate::codegen::template_env::render(
                    "doc_doxygen_param.jinja",
                    crate::alef_context! { name => &name },
                ));
            } else {
                out.push_str(&crate::codegen::template_env::render(
                    "doc_doxygen_param_desc.jinja",
                    crate::alef_context! { name => &name, desc => &desc },
                ));
            }
        }
    }
    if let Some(ret) = sections.returns.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_doxygen_return.jinja",
            crate::alef_context! { content => ret.trim() },
        ));
    }
    if let Some(err) = sections.errors.as_deref() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&crate::codegen::template_env::render(
            "doc_doxygen_errors.jinja",
            crate::alef_context! { content => err.trim() },
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
/// Convert markup conversion, returning
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
