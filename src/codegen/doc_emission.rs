//! Language-native documentation comment emission.
//! Provides standardized functions for emitting doc comments in different languages.

mod emitters;
mod sanitize;
mod sections;

pub use emitters::{
    emit_c_doxygen, emit_csharp_doc, emit_dartdoc, emit_elixir_doc, emit_gleam_doc, emit_javadoc, emit_kdoc,
    emit_kdoc_ktfmt_canonical, emit_phpdoc, emit_roxygen, emit_rustdoc, emit_swift_doc, emit_yard_doc, emit_zig_doc,
    render_yard_sections,
};
pub use sanitize::{DocTarget, escape_block_comment_close, sanitize_rust_idioms, sanitize_rust_idioms_keep_sections};
pub use sections::{
    BOLD_LABEL_ONLY_SECTION_NAMES, RustdocSections, doc_first_paragraph_joined, example_for_target,
    parse_arguments_bullets, parse_rustdoc_sections, render_csharp_xml_sections, render_doxygen_sections,
    render_javadoc_sections, render_jsdoc_sections, render_phpdoc_sections, replace_fence_lang,
};

#[cfg(test)]
#[path = "doc_emission/tests.rs"]
mod tests;
