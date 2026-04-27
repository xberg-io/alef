//! Helper functions for Kotlin code generation.
//!
//! Provides utilities for documentation emission and type/name formatting.

use alef_core::config::Language;
use alef_docs::clean_doc;

/// Emit cleaned KDoc documentation for a declaration.
///
/// Cleans Rust-specific doc strings and formats as KDoc (/** ... */).
pub(crate) fn emit_cleaned_kdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let cleaned = clean_doc(doc, Language::Kotlin);
    alef_codegen::doc_emission::emit_kdoc(out, &cleaned, indent);
}
