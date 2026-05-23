//! Helper functions for Kotlin code generation.
//!
//! Provides utilities for documentation emission and type/name formatting.

use crate::core::config::Language;
use crate::docs::clean_doc;

/// Emit cleaned KDoc documentation for a declaration in ktfmt-canonical format.
///
/// Cleans Rust-specific doc strings and formats as KDoc (/** ... */), collapsing
/// short comments to single-line format when they fit within ktfmt's 100-character
/// line width limit. This ensures generated code requires no post-processing by ktfmt.
pub(crate) fn emit_cleaned_kdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let cleaned = clean_doc(doc, Language::Kotlin);
    crate::codegen::doc_emission::emit_kdoc_ktfmt_canonical(out, &cleaned, indent);
}
