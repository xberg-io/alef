//! Helper functions for Dart bridge code generation.
//!
//! Provides utilities for documentation emission.

use crate::core::config::Language;
use crate::core::ir::{FieldDef, TypeRef};
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

/// Widen an opaque-wrapper `cfg` so the wrapper resolves on the Android x86_64
/// emulator triple.
///
/// Core re-exports such as `<crate>::BackendTrait` can carry a
/// `not(all(target_os = "android", target_arch = "x86_64"))` clause: the real
/// implementation module is gated out on that triple (an upstream linkage
/// constraint), so the re-export must be too. The core crate compensates by
/// emitting a crate-root **stub** of the same type under the complementary
/// `cfg`, so the crate-root type always resolves there.
///
/// The Dart opaque wrapper only needs that crate-root path to resolve — it
/// wraps whatever the crate-root type is (real or stub). But FRB's
/// `frb_generated.rs` references the wrapper (`crate::LlmBackend`)
/// unconditionally, so the wrapper must exist on every target the binding
/// builds for, including Android x86_64. Inheriting the re-export's
/// android-x86_64 exclusion verbatim makes the wrapper vanish there while
/// `frb_generated.rs` still references it — an `E0433`/`E0412`.
///
/// This strips the android-x86_64 exclusion clause from the wrapper `cfg` so it
/// tracks where the crate-root type resolves (real-or-stub) rather than where
/// the real impl module lives. Whitespace-insensitive; leaves any other clause
/// untouched.
pub(crate) fn widen_opaque_wrapper_cfg(cfg: &str) -> String {
    const ANDROID_X86_64_EXCLUSION: &str = r#"not(all(target_os = "android", target_arch = "x86_64"))"#;

    if !cfg.contains(ANDROID_X86_64_EXCLUSION) {
        return cfg.to_string();
    }

    let with_leading = format!(", {ANDROID_X86_64_EXCLUSION}");
    let with_trailing = format!("{ANDROID_X86_64_EXCLUSION}, ");
    if cfg.contains(&with_leading) {
        cfg.replace(&with_leading, "")
    } else if cfg.contains(&with_trailing) {
        cfg.replace(&with_trailing, "")
    } else {
        cfg.replace(ANDROID_X86_64_EXCLUSION, "")
    }
}

/// The mirrored error type a fallible bridge function returns, when its declared error is one
/// the crate mirrors for Dart. `error_type` is the name the extractor recorded, possibly a path.
///
/// flutter_rust_bridge throws a returned error value to Dart as its generated class, so
/// returning the mirror keeps the variant and its fields. `None` means the error is not
/// mirrored and the bridge reports its text as a `String`. ~keep
pub(crate) fn typed_error_name(error_type: &str, typed_errors: &std::collections::HashSet<String>) -> Option<String> {
    let name = error_type.rsplit("::").next().unwrap_or(error_type).trim();
    typed_errors.contains(name).then(|| name.to_string())
}

/// The `Result` error type and the `map_err` argument for a fallible bridge function.
pub(crate) fn error_return(typed: Option<&str>) -> (String, String) {
    match typed {
        Some(name) => (name.to_string(), format!("{name}::from")),
        None => ("String".to_string(), "|e| e.to_string()".to_string()),
    }
}

/// The same field without its option, when the mirror keeps the `Option` wrapper
/// (`frb_rust_type` with `optional`). A field that is already `Optional(_)` keeps its own path.
pub(super) fn non_optional_inner(field: &FieldDef) -> Option<FieldDef> {
    (field.optional && !matches!(field.ty, TypeRef::Optional(_))).then(|| FieldDef {
        optional: false,
        ..field.clone()
    })
}

/// Map `binding` through its option with `conv` (written against `v`). An identity conversion
/// returns the binding unchanged. `None` stays `None`; `Some("")` stays `Some("")`.
pub(super) fn map_through_option(binding: &str, conv: &str) -> String {
    if conv == "v" {
        binding.to_string()
    } else {
        format!("{binding}.map(|v| {conv})")
    }
}

#[cfg(test)]
mod tests {
    use super::widen_opaque_wrapper_cfg;

    #[test]
    fn strips_android_x86_64_exclusion_when_trailing_clause() {
        let cfg = r#"all(feature = "ner-llm", not(target_arch = "wasm32"), not(all(target_os = "android", target_arch = "x86_64")))"#;
        let widened = widen_opaque_wrapper_cfg(cfg);
        assert_eq!(
            widened, r#"all(feature = "ner-llm", not(target_arch = "wasm32"))"#,
            "android-x86_64 exclusion must be removed from the wrapper cfg"
        );
    }

    #[test]
    fn strips_android_x86_64_exclusion_when_leading_clause() {
        let cfg = r#"all(not(all(target_os = "android", target_arch = "x86_64")), feature = "ner-llm")"#;
        let widened = widen_opaque_wrapper_cfg(cfg);
        assert_eq!(widened, r#"all(feature = "ner-llm")"#);
    }

    #[test]
    fn leaves_unrelated_cfg_untouched() {
        let cfg = r#"all(feature = "ner-llm", not(target_arch = "wasm32"))"#;
        assert_eq!(widen_opaque_wrapper_cfg(cfg), cfg);
    }

    #[test]
    fn empty_cfg_is_passthrough() {
        assert_eq!(widen_opaque_wrapper_cfg(""), "");
    }
}
