//! Generate Java visitor support: interface, NodeContext record, VisitResult sealed interface,
//! VisitorBridge (upcall stubs), and IR-driven convert-with-visitor method fragments.
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
//! - convert-with-visitor methods drive the full lifecycle with caller-provided
//!   IR/config-derived method shape and FFI body fragments.

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
    crate::backends::java::template_env::render(
        "native_lib_visitor_handles.jinja",
        minijinja::context! {
            prefix => prefix,
            prefix_upper => pu,
        },
    )
}

/// Explicit inputs for rendering a convert-with-visitor method.
///
/// Returns the method source as a string (without surrounding class braces).
pub struct ConvertWithVisitorMethod<'a> {
    pub return_type: &'a str,
    pub method_name: &'a str,
    pub params: &'a str,
    pub exception_class: &'a str,
    pub prefix: &'a str,
    pub visitor_arg: &'a str,
    pub marshal_body: &'a str,
    pub invoke_body: &'a str,
}

/// Generate a convert-with-visitor method from IR/config-derived inputs.
///
/// Returns the method source as a string (without surrounding class braces).
pub fn gen_convert_with_visitor_method(method: &ConvertWithVisitorMethod<'_>) -> String {
    assert!(
        !method.return_type.is_empty(),
        "convert-with-visitor return type must be derived before rendering"
    );
    assert!(
        !method.method_name.is_empty(),
        "convert-with-visitor method name must be derived before rendering"
    );
    assert!(
        !method.params.is_empty(),
        "convert-with-visitor parameters must be derived before rendering"
    );
    assert!(
        !method.exception_class.is_empty(),
        "convert-with-visitor exception class must be derived before rendering"
    );
    assert!(
        !method.prefix.is_empty(),
        "convert-with-visitor native prefix must be derived before rendering"
    );
    assert!(
        !method.visitor_arg.is_empty(),
        "convert-with-visitor visitor argument must be derived before rendering"
    );
    assert!(
        !method.marshal_body.is_empty(),
        "convert-with-visitor marshal body must be derived before rendering"
    );
    assert!(
        !method.invoke_body.is_empty(),
        "convert-with-visitor invoke body must be derived before rendering"
    );

    let pu = method.prefix.to_uppercase();
    crate::backends::java::template_env::render(
        "convert_with_visitor.jinja",
        minijinja::context! {
            return_type => method.return_type,
            method_name => method.method_name,
            params => method.params,
            exception_class => method.exception_class,
            prefix_upper => pu,
            visitor_arg => method.visitor_arg,
            marshal_body => method.marshal_body,
            invoke_body => method.invoke_body,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_visitor_files_returns_four_files() {
        let files = gen_visitor_files("dev.sample_crate", "Demo");
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
    fn gen_convert_with_visitor_method_uses_explicit_method_shape() {
        let out = gen_convert_with_visitor_method(&ConvertWithVisitorMethod {
            return_type: "RenderResult",
            method_name: "renderWithVisitor",
            params: "String source, RenderOptions options, Visitor visitor",
            exception_class: "RendererException",
            prefix: "rnd",
            visitor_arg: "visitor",
            marshal_body: "var cSource = arena.allocateFrom(source);",
            invoke_body: "return invokeRender(cSource, visitorHandle);",
        });
        assert!(out.contains("renderWithVisitor"), "must define explicit method name");
        assert!(out.contains("RendererException"), "must use explicit exception type");
        assert!(out.contains("RenderResult"), "must use explicit IR-derived return type");
        assert!(
            out.contains("NativeLib.RND_VISITOR_CREATE"),
            "must call correct native handle"
        );
        assert!(
            !out.contains("ConversionResult") && !out.contains("ConversionOptions"),
            "must not synthesize legacy conversion DTO names"
        );
        assert!(
            !out.contains("CONVERSION_OPTIONS") && !out.contains("CONVERT_WITH_VISITOR"),
            "must not synthesize legacy conversion native handle names"
        );
    }
}
