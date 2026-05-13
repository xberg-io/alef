//! Helper functions for Gleam code generation.
//!
//! Provides utilities for documentation emission.

use alef_core::config::Language;
use alef_docs::clean_doc;

/// Emit cleaned Gleam documentation for a declaration.
///
/// Cleans Rust-specific doc strings and formats as Gleam doc comments (/// ...).
pub(crate) fn emit_cleaned_gleam_doc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let cleaned = clean_doc(doc, Language::Gleam);
    alef_codegen::doc_emission::emit_gleam_doc(out, &cleaned, indent);
}
