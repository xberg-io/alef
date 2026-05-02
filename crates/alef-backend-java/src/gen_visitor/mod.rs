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

use std::fmt::Write;

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
    let mut out = String::with_capacity(512);
    let pu = prefix.to_uppercase();

    writeln!(out).ok();
    writeln!(out, "    // Visitor FFI handles").ok();
    writeln!(
        out,
        "    static final MethodHandle {pu}_VISITOR_CREATE = LINKER.downcallHandle("
    )
    .ok();
    writeln!(out, "        LIB.find(\"{prefix}_visitor_create\").orElseThrow(),").ok();
    writeln!(
        out,
        "        FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)"
    )
    .ok();
    writeln!(out, "    );").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "    static final MethodHandle {pu}_VISITOR_FREE = LINKER.downcallHandle("
    )
    .ok();
    writeln!(out, "        LIB.find(\"{prefix}_visitor_free\").orElseThrow(),").ok();
    writeln!(out, "        FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)").ok();
    writeln!(out, "    );").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "    static final MethodHandle {pu}_CONVERT_WITH_VISITOR = LINKER.downcallHandle("
    )
    .ok();
    writeln!(
        out,
        "        LIB.find(\"{prefix}_convert_with_visitor\").orElseThrow(),"
    )
    .ok();
    writeln!(
        out,
        "        FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)"
    )
    .ok();
    writeln!(out, "    );").ok();

    out
}

/// Generate the `convertWithVisitor` method body to inject into the main wrapper class.
///
/// Returns the method source as a string (without surrounding class braces).
pub fn gen_convert_with_visitor_method(class_name: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(2048);
    let pu = prefix.to_uppercase();
    let exc = format!("{class_name}Exception");

    writeln!(
        out,
        "    public static ConversionResult convertWithVisitor(String html, ConversionOptions options, Visitor visitor) throws {exc} {{"
    )
    .ok();
    writeln!(out, "        try (var arena = Arena.ofConfined();").ok();
    writeln!(out, "             var bridge = new VisitorBridge(visitor)) {{").ok();
    writeln!(out, "            var cHtml = arena.allocateFrom(html);").ok();
    writeln!(out).ok();
    writeln!(out, "            MemorySegment optionsPtr = MemorySegment.NULL;").ok();
    writeln!(out, "            if (options != null) {{").ok();
    writeln!(
        out,
        "                var optJson = arena.allocateFrom(MAPPER.writeValueAsString(options));"
    )
    .ok();
    writeln!(
        out,
        "                optionsPtr = (MemorySegment) NativeLib.{pu}_CONVERSION_OPTIONS_FROM_JSON.invoke(optJson);"
    )
    .ok();
    writeln!(out, "            }}").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "            var visitorHandle = (MemorySegment) NativeLib.{pu}_VISITOR_CREATE.invoke(bridge.callbacksStruct());"
    )
    .ok();
    writeln!(out, "            if (visitorHandle.equals(MemorySegment.NULL)) {{").ok();
    writeln!(
        out,
        "                throw new {exc}(\"Failed to create visitor handle\", null);"
    )
    .ok();
    writeln!(out, "            }}").ok();
    writeln!(out).ok();
    writeln!(out, "            try {{").ok();
    writeln!(
        out,
        "                var resultPtr = (MemorySegment) NativeLib.{pu}_CONVERT_WITH_VISITOR.invoke(cHtml, optionsPtr, visitorHandle);"
    )
    .ok();
    writeln!(out, "                if (!optionsPtr.equals(MemorySegment.NULL)) {{").ok();
    writeln!(
        out,
        "                    NativeLib.{pu}_CONVERSION_OPTIONS_FREE.invoke(optionsPtr);"
    )
    .ok();
    writeln!(out, "                }}").ok();
    writeln!(out, "                if (resultPtr.equals(MemorySegment.NULL)) {{").ok();
    writeln!(out, "                    checkLastError();").ok();
    writeln!(out, "                    return null;").ok();
    writeln!(out, "                }}").ok();
    writeln!(
        out,
        "                var markdown = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
    writeln!(out, "                NativeLib.{pu}_FREE_STRING.invoke(resultPtr);").ok();
    writeln!(
        out,
        "                return new ConversionResult(java.util.Optional.of(markdown), java.util.Optional.empty(), null, null, null, null);"
    )
    .ok();
    writeln!(out, "            }} catch (Throwable e) {{").ok();
    writeln!(out, "                throw new {exc}(\"FFI call failed\", e);").ok();
    writeln!(out, "            }} finally {{").ok();
    writeln!(
        out,
        "                NativeLib.{pu}_VISITOR_FREE.invoke(visitorHandle);"
    )
    .ok();
    writeln!(out, "                bridge.rethrowVisitorError();").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }} catch ({exc} e) {{").ok();
    writeln!(out, "            throw e;").ok();
    writeln!(out, "        }} catch (Throwable e) {{").ok();
    writeln!(out, "            throw new {exc}(\"FFI call failed\", e);").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();

    out
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
            out.contains("HTM_CONVERT_WITH_VISITOR"),
            "must have convert with visitor handle"
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
