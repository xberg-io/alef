//! Individual Java file generators: NodeContext, VisitResult, Visitor, VisitorBridge.

use alef_core::hash::{self, CommentStyle};

use super::callbacks::CALLBACKS;
use super::helpers::{callback_descriptor, callback_method_type, gen_handle_method, iface_param_str, stub_var_name};

/// Number of callbacks per generated `registerStubsN` Java method.
/// Used by both the stub-call list (constructor body) and the stub-method emitter.
const CHUNK_SIZE: usize = 5;

pub(super) fn gen_node_context(package: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::template_env::render(
        "node_context.jinja",
        minijinja::context! {
            header => header,
            package => package,
        },
    )
}

pub(super) fn gen_visit_result(package: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::template_env::render(
        "visit_result.jinja",
        minijinja::context! {
            header => header,
            package => package,
        },
    )
}

pub(super) fn gen_visitor_interface(package: &str, _class_name: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    let callbacks: Vec<_> = CALLBACKS
        .iter()
        .map(|spec| {
            minijinja::context! {
                doc => spec.doc,
                java_method => spec.java_method,
                params => iface_param_str(spec),
            }
        })
        .collect();
    crate::template_env::render(
        "visitor_interface.jinja",
        minijinja::context! {
            header => header,
            package => package,
            callbacks => callbacks,
        },
    )
}

/// Wrap arbitrary Java file content with package declaration and imports using the visitor_files template.
/// This demonstrates the usage of visitor_files.jinja template for generic file wrapping.
#[allow(dead_code)]
fn wrap_java_file(package: &str, imports: Vec<String>, content: String) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);
    crate::template_env::render(
        "visitor_files.jinja",
        minijinja::context! {
            header => header,
            package => package,
            imports => imports,
            content => content,
        },
    )
}

/// Generate `VisitorBridge.java` — builds Panama upcall stubs for all 40 callbacks
/// and exposes a `MemorySegment callbacksStruct()` pointing to the C struct.
pub(super) fn gen_visitor_bridge(package: &str, _class_name: &str) -> String {
    let header = hash::header(CommentStyle::DoubleSlash);

    // The struct has user_data (pointer) + 40 function pointer fields.
    let num_fields = CALLBACKS.len() + 1; // +1 for user_data
    let num_callbacks = CALLBACKS.len();

    // Build stub_calls list: which registerStubsN method to call at each step
    let num_chunks = CALLBACKS.chunks(CHUNK_SIZE).count();
    let mut stub_calls = Vec::new();
    for i in 1..=num_chunks {
        stub_calls.push(format!("registerStubs{i}(offset)"));
    }

    // Build stub_methods: the actual method implementations as a list of strings
    let mut stub_methods = Vec::new();
    for (chunk_idx, chunk) in CALLBACKS.chunks(CHUNK_SIZE).enumerate() {
        let method_num = chunk_idx + 1;
        let mut method = String::new();
        method.push_str("    private long registerStubs");
        method.push_str(&method_num.to_string());
        method.push_str("(\n            final long offset)\n            throws ReflectiveOperationException {\n");
        method.push_str("        long off = offset;\n");
        for spec in chunk {
            let descriptor = callback_descriptor(spec);
            let method_type = callback_method_type(spec);
            let stub_var = stub_var_name(spec.java_method);
            method.push_str("        // ");
            method.push_str(spec.c_field);
            method.push('\n');
            method.push_str("        var ");
            method.push_str(&stub_var);
            method.push_str(" = LINKER.upcallStub(\n");
            method.push_str("                LOOKUP.bind(\n");
            method.push_str("                    this, \"");
            method.push_str(&super::helpers::handle_method_name(spec.java_method));
            method.push_str("\",\n");
            method.push_str("                    ");
            method.push_str(&method_type);
            method.push_str("),\n");
            method.push_str("                ");
            method.push_str(&descriptor);
            method.push_str(",\n");
            method.push_str("                arena);\n");
            method.push_str("        struct.set(ValueLayout.ADDRESS, off, ");
            method.push_str(&stub_var);
            method.push_str(");\n");
            method.push_str("        off += ValueLayout.ADDRESS.byteSize();\n");
        }
        method.push_str("        return off;\n");
        method.push_str("    }\n");
        stub_methods.push(method);
    }

    // Build handle_methods: one per callback as a list of strings
    let mut handle_methods = Vec::new();
    for spec in CALLBACKS {
        let mut method = String::new();
        gen_handle_method(&mut method, spec);
        handle_methods.push(method);
    }

    crate::template_env::render(
        "visitor_bridge.jinja",
        minijinja::context! {
            header => header,
            package => package,
            num_callbacks => num_callbacks,
            num_fields => num_fields,
            stub_calls => stub_calls,
            stub_methods => stub_methods,
            handle_methods => handle_methods,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_node_context_produces_java_record() {
        let out = gen_node_context("dev.kreuzberg");
        assert!(out.contains("package dev.kreuzberg;"), "must have package decl");
        assert!(
            out.contains("public record NodeContext("),
            "must define NodeContext record"
        );
        assert!(out.contains("int nodeType"), "must have nodeType field");
        assert!(out.contains("String tagName"), "must have tagName field");
        assert!(out.contains("boolean isInline"), "must have isInline field");
    }

    #[test]
    fn gen_visit_result_produces_sealed_interface() {
        let out = gen_visit_result("dev.kreuzberg");
        assert!(
            out.contains("public sealed interface VisitResult"),
            "must define sealed VisitResult"
        );
        assert!(out.contains("record Continue()"), "must have Continue variant");
        assert!(out.contains("record Skip()"), "must have Skip variant");
        assert!(
            out.contains("record Custom(String markdown)"),
            "must have Custom variant"
        );
        assert!(out.contains("record Error(String message)"), "must have Error variant");
    }

    #[test]
    fn gen_visitor_interface_has_all_callbacks() {
        let out = gen_visitor_interface("dev.kreuzberg", "Demo");
        assert!(
            out.contains("public interface Visitor"),
            "must define Visitor interface"
        );
        assert!(out.contains("visitText"), "must include visitText");
        assert!(out.contains("visitTableRow"), "must include visitTableRow");
        assert!(out.contains("visitFigureEnd"), "must include visitFigureEnd");
    }

    #[test]
    fn gen_visitor_bridge_produces_class_with_stubs() {
        let out = gen_visitor_bridge("dev.kreuzberg", "Demo");
        assert!(out.contains("final class VisitorBridge"), "must define VisitorBridge");
        assert!(
            out.contains("MemorySegment callbacksStruct()"),
            "must have callbacksStruct method"
        );
        assert!(out.contains("Arena.ofConfined()"), "must use confined Arena");
        assert!(out.contains("LINKER.upcallStub("), "must register upcall stubs");
    }

    #[test]
    fn gen_visitor_bridge_has_encode_visit_result() {
        let out = gen_visitor_bridge("dev.kreuzberg", "Demo");
        assert!(out.contains("encodeVisitResult"), "must have encodeVisitResult helper");
        assert!(
            out.contains("VISIT_RESULT_CONTINUE"),
            "must have VISIT_RESULT_CONTINUE constant"
        );
    }

    #[test]
    fn gen_visitor_bridge_chunk_counts_consistent() {
        let src = gen_visitor_bridge("dev.test", "VisitorBridge");
        let expected = CALLBACKS.len().div_ceil(CHUNK_SIZE);
        let stub_call_count = src.matches("offset = registerStubs").count();
        let stub_method_count = src.matches("private long registerStubs").count();
        assert_eq!(
            stub_call_count, expected,
            "constructor must invoke every registerStubsN; got {} calls, expected {}",
            stub_call_count, expected
        );
        assert_eq!(
            stub_method_count, expected,
            "must emit one registerStubsN method per chunk; got {} methods, expected {}",
            stub_method_count, expected
        );
    }
}
