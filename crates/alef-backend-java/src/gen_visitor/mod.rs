//! Generate Java visitor support: interface, NodeContext record, VisitResult sealed interface,
//! VisitorBridge (upcall stubs), and convertWithVisitor method.
//!
//! # Panama FFM upcall strategy
//!
//! Java cannot expose method references as raw C function pointers. The generated
//! code uses Java 21+ Foreign Function & Memory API (Panama) upcall stubs:
//!
//! - `NodeContext`: a `record` carrying the fields from `HTMHtmNodeContext`.
//! - `VisitResult`: a `sealed interface` with `Continue`, `Skip`, `PreserveHtml`,
//!   `Custom`, and `Error` implementations.
//! - `Visitor`: an `interface` with default no-op methods for all 40 callbacks.
//! - `VisitorBridge`: a package-private class that allocates one `MemorySegment`
//!   upcall stub per callback inside a `Arena.ofConfined()` scope, then writes
//!   all stubs into a flat `MemorySegment` matching `HTMHtmVisitorCallbacks`.
//! - `convertWithVisitor`: static method on the wrapper class that drives the full
//!   lifecycle: marshal options → create `VisitorBridge` → `htm_visitor_create` →
//!   `htm_convert_with_visitor` → deserialise JSON result → `htm_visitor_free`.

mod callbacks;
mod files;
mod helpers;

pub use callbacks::{CALLBACKS, CallbackSpec, ExtraParam};

// ---------------------------------------------------------------------------
// Public API: generate visitor-related Java source files
// ---------------------------------------------------------------------------

/// Returns `(filename, content)` pairs for all visitor-related Java files.
///
/// Callers push these into the `files` vector in `generate_bindings`.
pub fn gen_visitor_files(package: &str, class_name: &str) -> Vec<(String, String)> {
    vec![
        ("NodeContext.java".to_string(), files::gen_node_context(package)),
        ("VisitResult.java".to_string(), files::gen_visit_result(package)),
        (
            "Visitor.java".to_string(),
            files::gen_visitor_interface(package, class_name),
        ),
        (
            "VisitorBridge.java".to_string(),
            files::gen_visitor_bridge(package, class_name),
        ),
    ]
}

/// Generate NativeLib method handle declarations for visitor FFI functions.
///
/// These lines are injected into the `NativeLib` class body after the normal handles.
pub fn gen_native_lib_visitor_handles(prefix: &str) -> String {
    let pu = prefix.to_uppercase();
    crate::template_env::render(
        "native_lib_visitor_handles.jinja",
        minijinja::context! {
            prefix => prefix,
            prefix_upper => pu,
        },
    )
}

/// Generate the `convertWithVisitor` method body to inject into the main wrapper class.
///
/// Returns the method source as a string (without surrounding class braces).
pub fn gen_convert_with_visitor_method(class_name: &str, prefix: &str) -> String {
    let pu = prefix.to_uppercase();
    let exc = format!("{class_name}Exception");
    crate::template_env::render(
        "convert_with_visitor.jinja",
        minijinja::context! {
            exception_class => exc,
            prefix_upper => pu,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_visitor_files_returns_four_files() {
        let files = gen_visitor_files("dev.kreuzberg", "Demo");
        assert_eq!(files.len(), 4, "must return 4 files");
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"NodeContext.java"), "must include NodeContext.java");
        assert!(names.contains(&"VisitResult.java"), "must include VisitResult.java");
        assert!(names.contains(&"Visitor.java"), "must include Visitor.java");
        assert!(names.contains(&"VisitorBridge.java"), "must include VisitorBridge.java");
        assert!(
            !names.contains(&"VisitContext.java"),
            "must NOT include VisitContext.java"
        );
        assert!(
            !names.contains(&"TestVisitor.java"),
            "must NOT include TestVisitor.java"
        );
        assert!(
            !names.contains(&"TestVisitorAdapter.java"),
            "must NOT include TestVisitorAdapter.java"
        );
    }

    #[test]
    fn gen_native_lib_visitor_handles_includes_all_three_handles() {
        let out = gen_native_lib_visitor_handles("htm");
        assert!(out.contains("HTM_VISITOR_CREATE"), "must have visitor create handle");
        assert!(out.contains("HTM_VISITOR_FREE"), "must have visitor free handle");
        assert!(
            out.contains("HTM_OPTIONS_SET_VISITOR_HANDLE"),
            "must have options set visitor handle"
        );
    }

    #[test]
    fn gen_convert_with_visitor_method_uses_correct_prefix() {
        let out = gen_convert_with_visitor_method("Htm", "htm");
        assert!(out.contains("convertWithVisitor"), "must define convertWithVisitor");
        assert!(out.contains("HtmException"), "must use correct exception type");
        assert!(
            out.contains("NativeLib.HTM_VISITOR_CREATE"),
            "must call correct native handle"
        );
    }
}
