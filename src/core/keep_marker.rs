//! Removal of alef's internal `~keep` comment marker from rendered template output.
//!
//! `~keep` is a marker for `poly`'s uncomment pass: a comment carrying it is spared when
//! `poly fmt` strips unmarked comments. It is meaningful in a *source* tree that `poly`
//! reads. It is meaningless — and user-visible noise — in the generated code alef hands a
//! consumer, because alef rewrites those files in full on every run.
//!
//! Template authors write `~keep` inside `.jinja` sources for the ordinary reason: to stop
//! alef's own `poly fmt` from deleting the rationale in the template file. Nothing about
//! that intent says the token should survive into the consumer's tree, and before this pass
//! existed it did — 56 tokens reached one consumer's `crates/*-ffi/src/lib.rs` alone. Rather
//! than police individual templates, [`strip_keep_markers`] runs on every built-in template
//! render, so a template gaining a new marker cannot reintroduce the leak.
//!
//! Deliberately NOT applied to consumer-supplied templates (`docs::render`,
//! `readme::template`, `extensions::template`, [`crate::core::template_env::TemplateEnv`]):
//! a consumer writing `~keep` in their own template is asking for it in their own output,
//! and their `poly` run is the one that will read it. ~keep
//!
//! Also deliberately out of reach of this pass: the markers alef emits **on purpose**, all of
//! them from Rust string literals and none of them through a `.jinja` render, so scoping the
//! strip to template rendering is what keeps them intact:
//!
//! - `scaffold::CLIPPY_WORKSPACE_LINTS_RATIONALE` — binding-crate `Cargo.toml`; that manifest
//!   is rewritten in full every run, but the consumer's uncomment pass runs *between* runs.
//! - `scaffold::languages::{ruby, dart, swift, zig}` create-only test seeds and
//!   `backends::kotlin_android::gen_seed_test` — written once and never regenerated over, so
//!   the marker is the only thing keeping the rationale alive in the consumer's tree.
//! - `scaffold::languages::zig`'s `build.zig` test-target block comment.
//!
//! Stripping any of those would delete protection real files depend on. ~keep

/// The marker token itself.
const MARKER: &str = "~keep";

/// Punctuation that belongs to the marker rather than to the sentence around it: `~keep:`
/// and `~keep,` introduce the rationale that follows them, so the punctuation is part of the
/// token being removed. Removing only `~keep` welds the orphaned punctuation onto the end of
/// the preceding sentence, leaving a dangling `.:`. ~keep
const MARKER_PUNCTUATION: [char; 3] = [':', ',', ';'];

/// Delimiter pairs a marker may be wrapped in. `(~keep)` and `` `~keep` `` own their
/// brackets; dropping only the token leaves an empty `()` or an empty code span. ~keep
const MARKER_DELIMITERS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('`', '`')];

/// Strip every `~keep` marker from `rendered`, leaving the surrounding text verbatim.
///
/// Line-oriented on purpose: the whitespace rules below reason about "start of text" and
/// "end of text", and applying them to a whole multi-line document would let a marker at the
/// end of one line consume the newline and the next line's indentation. Splitting first
/// bounds every rewrite to the line the marker sits on. ~keep
///
/// A line that consists of nothing but a comment introducer and a marker keeps its
/// introducer (`// ~keep` becomes `//`). Deleting the line would be a larger and less
/// reversible edit than the leak this pass exists to fix. ~keep
pub(crate) fn strip_keep_markers(rendered: &str) -> String {
    if !rendered.contains(MARKER) {
        return rendered.to_string();
    }
    rendered.split('\n').map(strip_line).collect::<Vec<_>>().join("\n")
}

fn strip_line(line: &str) -> String {
    let mut output = line.to_string();
    let mut search_from = 0;
    while let Some(found) = output[search_from..].find(MARKER) {
        let mut start = search_from + found;
        let mut end = start + MARKER.len();

        if let Some(punctuation) = output[end..].chars().next()
            && MARKER_PUNCTUATION.contains(&punctuation)
        {
            end += punctuation.len_utf8();
        }

        for (open, close) in MARKER_DELIMITERS {
            if output[..start].ends_with(open) && output[end..].starts_with(close) {
                start -= open.len_utf8();
                end += close.len_utf8();
                break;
            }
        }

        let space_before = start == 0 || output[..start].ends_with(char::is_whitespace);
        let space_after = end == output.len() || output[end..].starts_with(char::is_whitespace);

        // Exactly one separator must survive. A removal that runs to the end of the line has
        // nothing left to separate, so the run in front of it goes too; anywhere else the run
        // behind it goes and the one in front keeps the surviving words apart. When prose
        // punctuation abuts the marker on the right (`(see the note ~keep)`), taking the run in
        // front is what avoids leaving a space before that punctuation. ~keep
        if space_before && space_after {
            if end == output.len() {
                start -= trailing_whitespace_len(&output[..start]);
            } else {
                end += leading_whitespace_len(&output[end..]);
            }
        } else if space_before {
            start -= trailing_whitespace_len(&output[..start]);
        }

        output.replace_range(start..end, "");
        search_from = start;
    }
    output
}

fn trailing_whitespace_len(text: &str) -> usize {
    text.len() - text.trim_end().len()
}

fn leading_whitespace_len(text: &str) -> usize {
    text.len() - text.trim_start().len()
}

#[cfg(test)]
mod tests {
    use super::strip_keep_markers;

    #[test]
    fn strip_keep_markers_leaves_text_without_a_marker_byte_identical() {
        let source = "// A comment with no marker.\nfn main() {}\n";
        assert_eq!(strip_keep_markers(source), source);
    }

    #[test]
    fn strip_keep_markers_removes_a_trailing_marker_and_keeps_the_whole_comment() {
        let rendered = "/// Backed by the FFI's `AlefHandle` (`u64`), not a pointer. ~keep\npub const A: u8 = 0;\n";
        let stripped = strip_keep_markers(rendered);
        assert_eq!(
            stripped,
            "/// Backed by the FFI's `AlefHandle` (`u64`), not a pointer.\npub const A: u8 = 0;\n"
        );
    }

    #[test]
    fn strip_keep_markers_removes_a_mid_comment_marker_and_keeps_the_prose_on_both_sides() {
        let rendered = "// SAFETY: ~keep The Swift API requires conformers to synchronize state.\n";
        assert_eq!(
            strip_keep_markers(rendered),
            "// SAFETY: The Swift API requires conformers to synchronize state.\n"
        );
    }

    #[test]
    fn strip_keep_markers_removes_a_parenthesized_marker_without_leaving_empty_parentheses() {
        let rendered = "/// never smuggled into human-facing text. (~keep)\n";
        assert_eq!(
            strip_keep_markers(rendered),
            "/// never smuggled into human-facing text.\n"
        );
    }

    #[test]
    fn strip_keep_markers_strips_every_marker_variant_leaving_readable_prose() {
        let cases: &[(&str, &str)] = &[
            ("Recovered from a parse error. ~keep", "Recovered from a parse error."),
            (
                "~keep Explains why the fallback is safe.",
                "Explains why the fallback is safe.",
            ),
            (
                "The pool ~keep is rebuilt per request.",
                "The pool is rebuilt per request.",
            ),
            (
                "Drained after every page. ~keep: that drain observes one thread.",
                "Drained after every page. that drain observes one thread.",
            ),
            ("See the note below. ~keep:", "See the note below."),
            ("~keep: mirrors the bench tolerance.", "mirrors the bench tolerance."),
            (
                "Mirrors `image.rs`. ~keep, same rationale as above.",
                "Mirrors `image.rs`. same rationale as above.",
            ),
            (
                "Guarded by a flag. ~keep; the flag is compile-time.",
                "Guarded by a flag. the flag is compile-time.",
            ),
            (
                "Never smuggled into human-facing text. (~keep)",
                "Never smuggled into human-facing text.",
            ),
            (
                "Applies to both paths (see the note ~keep).",
                "Applies to both paths (see the note).",
            ),
            (
                "Tracked separately [~keep] for the async path.",
                "Tracked separately for the async path.",
            ),
            (
                "See the `~keep` note in `prepare_session`.",
                "See the note in `prepare_session`.",
            ),
            ("Two markers ~keep on one line. ~keep", "Two markers on one line."),
            ("~keep", ""),
            ("~keep:", ""),
            ("(~keep)", ""),
        ];
        for (raw, expected) in cases {
            let stripped = strip_keep_markers(raw);
            assert_eq!(&stripped, expected, "stripping {raw:?}");
            assert!(!stripped.contains("~keep"), "marker survived in {stripped:?}");
            assert!(!stripped.contains(".:"), "dangling punctuation in {stripped:?}");
        }
    }

    #[test]
    fn strip_keep_markers_does_not_consume_the_newline_or_the_next_line_indentation() {
        let rendered = "        // context survives. ~keep\n        let value = 1;\n";
        let stripped = strip_keep_markers(rendered);
        assert_eq!(stripped, "        // context survives.\n        let value = 1;\n");
        assert_eq!(stripped.lines().count(), 2, "line count changed: {stripped:?}");
    }

    #[test]
    fn strip_keep_markers_handles_a_marker_on_every_line_of_a_block() {
        let rendered = "// first line. ~keep\n// second line. ~keep\n// third line. ~keep\n";
        assert_eq!(
            strip_keep_markers(rendered),
            "// first line.\n// second line.\n// third line.\n"
        );
    }

    #[test]
    fn strip_keep_markers_keeps_the_comment_introducer_when_the_marker_is_the_whole_comment() {
        assert_eq!(strip_keep_markers("    // ~keep\n"), "    //\n");
    }
}

/// End-to-end coverage of the wiring: these go through a backend's real `template_env::render`
/// on a real shipped template that carries a marker, so they fail if the strip call is dropped
/// from that backend, not merely if [`strip_keep_markers`] regresses. Each asserts the comment
/// rendered at all *before* asserting the token is gone — a strip that ate the comment would
/// otherwise pass the absence check. ~keep
#[cfg(test)]
mod rendered_template_tests {
    #[test]
    fn zig_trait_bridge_alias_renders_its_rationale_without_the_keep_marker() {
        let rendered = crate::backends::zig::template_env::render(
            "trait_bridge_alias.jinja",
            crate::alef_context! { alias => "VisitorHandle" },
        );

        assert!(
            rendered.contains("/// Backed by the FFI's `AlefHandle` (`u64`), not a pointer."),
            "the rationale comment must survive the strip verbatim, got:\n{rendered}"
        );
        assert!(
            rendered.contains("pub const VisitorHandle = u64;"),
            "the aliased declaration must still render, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("~keep"),
            "marker leaked into zig output:\n{rendered}"
        );
    }

    #[test]
    fn napi_error_converter_renders_its_rationale_without_the_parenthesized_keep_marker() {
        let rendered = crate::codegen::template_env::render(
            "error_gen/napi_error_converter.jinja",
            crate::alef_context! { rust_path => "sample_crate::SampleError", fn_name => "sample_error_to_napi" },
        );

        assert!(
            rendered.contains("never smuggled into human-facing text."),
            "the rationale comment must survive the strip verbatim, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("text. ()") && !rendered.contains("text. )"),
            "the marker's own parentheses must go with it, got:\n{rendered}"
        );
        assert!(
            rendered.contains("fn sample_error_to_napi(e: sample_crate::SampleError) -> napi::Error {"),
            "the converter signature must still render, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("~keep"),
            "marker leaked into napi output:\n{rendered}"
        );
    }

    #[test]
    fn csharp_opaque_handle_header_renders_its_rationale_without_the_keep_marker() {
        let rendered = crate::backends::csharp::template_env::render(
            "opaque_handle_header.jinja",
            crate::alef_context! {
                namespace => "Sample.Bindings",
                class_name => "Document",
                free_method => "sample_document_free",
            },
        );

        assert!(
            rendered
                .contains("billion concurrently live handles of this type, and 32-bit .NET targets are legacy-only."),
            "the rationale comment must survive the strip verbatim, got:\n{rendered}"
        );
        assert!(
            rendered.contains("internal sealed class DocumentSafeHandle : SafeHandle"),
            "the class declaration on the line after the comment must still render, got:\n{rendered}"
        );
        assert!(!rendered.contains("~keep"), "marker leaked into C# output:\n{rendered}");
    }
}
