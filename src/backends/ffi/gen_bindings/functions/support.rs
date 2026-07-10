use crate::codegen::doc_emission::emit_c_doxygen;
use crate::core::ir::{FunctionDef, MethodDef, TypeRef};

/// Render a Doxygen `///` block for an FFI extern function whose rustdoc comes
/// from the source `# Arguments` / `# Returns` / `# Errors` sections. Always
/// appends the universal FFI `\note SAFETY:` clause that the alef FFI template
/// previously hard-coded, so callers don't have to repeat it.
///
/// Returns an empty string when `doc` is empty AND no safety note is needed
/// (currently we always emit the safety note for `extern "C"` boundaries).
pub(super) fn ffi_doxygen_block(doc: &str) -> String {
    let mut full_doc = String::with_capacity(doc.len() + 128);
    if !doc.is_empty() {
        full_doc.push_str(doc);
        if !doc.contains("# Safety") {
            full_doc.push_str(
                "\n\n# Safety\n\nCaller must ensure all pointer arguments are valid or null. \
                 Returned pointers must be freed with the appropriate free function.",
            );
        }
    } else {
        full_doc.push_str(
            "# Safety\n\nCaller must ensure all pointer arguments are valid or null. \
             Returned pointers must be freed with the appropriate free function.",
        );
    }
    let mut out = String::new();
    emit_c_doxygen(&mut out, &full_doc, "");
    out
}

/// Returns true when a sanitized function can be auto-recovered via JSON-roundtrip:
/// every sanitized param is a `Vec<String>` with `original_type` set.
pub(super) fn sanitized_recoverable(func: &FunctionDef) -> bool {
    let params_ok = func.params.iter().all(|p| {
        if !p.sanitized {
            return true;
        }
        p.original_type.is_some() && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    });
    if !params_ok {
        return false;
    }
    let any_param_sanitized = func.params.iter().any(|p| p.sanitized);
    !func.sanitized || any_param_sanitized
}

/// Method-level analogue of [`sanitized_recoverable`].
pub(super) fn method_sanitized_recoverable(method: &MethodDef) -> bool {
    let params_ok = method.params.iter().all(|p| {
        if !p.sanitized {
            return true;
        }
        p.original_type.is_some() && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    });
    if !params_ok {
        return false;
    }
    let any_param_sanitized = method.params.iter().any(|p| p.sanitized);
    !method.sanitized || any_param_sanitized
}
