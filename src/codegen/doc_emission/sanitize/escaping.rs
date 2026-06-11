/// Escape JSDoc block-close sequences (`*/`) by replacing with `* /`.
///
/// JSDoc comments use `/** ... */` blocks. If rustdoc content contains a backtick
/// code span like `` `/* ... */` ``, the `*/` inside the backticks lands verbatim
/// in the emitted JSDoc and prematurely closes the comment block, breaking downstream
/// tools like oxfmt. This function replaces `*/` with `* /` (backslash breaks the
/// terminator matching) while preserving visual rendering in docs.
pub(super) fn escape_jsdoc_block_close(s: &str) -> String {
    s.replace("*/", "* /")
}

/// XML-escape `<`, `>`, `&` for safe embedding inside a C# `<summary>` element.
///
/// `<` / `>` may legitimately appear in prose after Rust-idiom substitution
/// when the substitutions produce C#-friendly forms (e.g. `Dictionary<K, V>`).
/// Those are still XML-significant characters and must be entity-escaped for
/// XML parsers (Roslyn, doxygen) to accept the resulting `<summary>` block.
pub(super) fn xml_escape_for_csharp(s: &str) -> String {
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
