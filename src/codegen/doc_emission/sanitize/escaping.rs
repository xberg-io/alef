/// Escape block-comment terminators (`*/`) by replacing with `* /`.
///
/// JSDoc and PHPDoc comments use `/** ... */` blocks. If rustdoc content contains a backtick
/// code span like `` `/** */` ``, the `*/` inside the backticks lands verbatim
/// in the emitted comment and prematurely closes the block, leaving the rest of the doc
/// to be parsed as code. Replacing `*/` with `* /` breaks the terminator match while
/// preserving the visual rendering.
pub fn escape_block_comment_close(s: &str) -> String {
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
