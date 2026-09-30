use crate::core::config::Language;
use crate::docs::doc_cleaning::{clean_doc, clean_doc_inline};

#[test]
fn canonical_markdown_labels_bare_fences_and_spaces_headings() {
    let input = "Summary.  Next sentence.\n- item\n##### Details\n```\nvalue\n```";
    let expected = "Summary. Next sentence.\n\n- item\n\n##### Details\n\n```rust\nvalue\n```";

    assert_eq!(clean_doc(input, Language::Python), expected);
}

#[test]
fn table_cell_markdown_turns_fenced_blocks_into_inline_code() {
    let input = "In TOML:\n\n```toml\n[client]\nmodel_tier = \"server\"\n```\nDone.";
    let expected = "In TOML: `[client] model_tier = \"server\"` Done.";

    assert_eq!(clean_doc_inline(input, Language::Python), expected);
}

#[test]
fn table_cell_markdown_uses_a_longer_delimiter_for_embedded_backticks() {
    let input = "```text\ncall `value` now\n```";

    assert_eq!(clean_doc_inline(input, Language::Python), "``call `value` now``");
}

#[test]
fn canonical_markdown_preserves_wide_fences_and_ignores_shorter_inner_runs() {
    let input = "````\n```not a closing fence\nvalue\n````";
    let expected = "````rust\n```not a closing fence\nvalue\n````";

    assert_eq!(clean_doc(input, Language::Python), expected);
}
