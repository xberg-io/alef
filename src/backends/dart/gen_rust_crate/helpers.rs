//! Helper functions for Dart bridge code generation.
//!
//! Provides utilities for documentation emission.

use crate::core::config::Language;
use crate::docs::clean_doc;

/// Emit cleaned Dartdoc documentation for a Rust-side bridge function.
///
/// Cleans Rust-specific doc strings and formats as Dartdoc (/// ...).
/// This documentation is picked up by FRB and propagates to the Dart-side generated code.
pub(crate) fn emit_cleaned_dartdoc(out: &mut String, doc: &str, indent: &str) {
    if doc.is_empty() {
        return;
    }
    let cleaned = clean_doc(doc, Language::Dart);
    crate::codegen::doc_emission::emit_dartdoc(out, &cleaned, indent);
}
