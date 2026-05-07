//! Individual Java file generators: NodeContext, VisitResult, Visitor, VisitorBridge.

use alef_core::hash::{self, CommentStyle};
use std::fmt::Write;

use super::callbacks::CALLBACKS;
use super::helpers::{callback_descriptor, callback_method_type, gen_handle_method, iface_param_str, stub_var_name};

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

/// Generate `VisitorBridge.java` — builds Panama upcall stubs for all 40 callbacks
/// and exposes a `MemorySegment callbacksStruct()` pointing to the C struct.
pub(super) fn gen_visitor_bridge(package: &str, _class_name: &str) -> String {
    let mut out = String::with_capacity(32_768);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {package};").ok();
    writeln!(out).ok();
    writeln!(out, "import java.lang.foreign.Arena;").ok();
    writeln!(out, "import java.lang.foreign.FunctionDescriptor;").ok();
    writeln!(out, "import java.lang.foreign.Linker;").ok();
    writeln!(out, "import java.lang.foreign.MemoryLayout;").ok();
    writeln!(out, "import java.lang.foreign.MemorySegment;").ok();
    writeln!(out, "import java.lang.foreign.ValueLayout;").ok();
    writeln!(out, "import java.lang.invoke.MethodHandles;").ok();
    writeln!(out, "import java.lang.invoke.MethodType;").ok();
    writeln!(out, "import java.util.ArrayList;").ok();
    writeln!(out, "import java.util.List;").ok();
    writeln!(out).ok();

    writeln!(out, "/**").ok();
    writeln!(out, " * Allocates Panama FFM upcall stubs for a Visitor and assembles").ok();
    writeln!(out, " * the C HTMHtmVisitorCallbacks struct in native memory.").ok();
    writeln!(out, " */").ok();
    writeln!(out, "final class VisitorBridge implements AutoCloseable {{").ok();
    writeln!(out, "    private static final Linker LINKER = Linker.nativeLinker();").ok();
    writeln!(out, "    private static final MethodHandles.Lookup LOOKUP =").ok();
    writeln!(out, "        MethodHandles.lookup();").ok();
    writeln!(out).ok();
    // Named constants for VisitResult discriminant values
    writeln!(out, "    // VisitResult discriminant codes returned to C").ok();
    writeln!(out, "    private static final int VISIT_RESULT_CONTINUE = 0;").ok();
    writeln!(out, "    private static final int VISIT_RESULT_SKIP = 1;").ok();
    writeln!(out, "    private static final int VISIT_RESULT_PRESERVE_HTML = 2;").ok();
    writeln!(out, "    private static final int VISIT_RESULT_CUSTOM = 3;").ok();
    writeln!(out, "    private static final int VISIT_RESULT_ERROR = 4;").ok();
    writeln!(out).ok();

    // The struct has user_data (pointer) + 40 function pointer fields.
    let num_fields = CALLBACKS.len() + 1; // +1 for user_data
    writeln!(
        out,
        "    // HTMHtmVisitorCallbacks: user_data + {n} callbacks",
        n = CALLBACKS.len(),
    )
    .ok();
    writeln!(out, "    // = {total} pointer-sized slots", total = num_fields,).ok();
    writeln!(out, "    private static final long CALLBACKS_STRUCT_SIZE =").ok();
    writeln!(out, "        (long) ValueLayout.ADDRESS.byteSize() * {num_fields}L;").ok();
    writeln!(out).ok();
    // Named offset constants for HTMHtmNodeContext struct fields (avoids magic numbers)
    writeln!(out, "    // HTMHtmNodeContext field offsets").ok();
    writeln!(out, "    private static final long CTX_OFFSET_TAG_NAME = 8L;").ok();
    writeln!(out, "    private static final long CTX_OFFSET_DEPTH = 16L;").ok();
    writeln!(out, "    private static final long CTX_OFFSET_INDEX_IN_PARENT = 24L;").ok();
    writeln!(out, "    private static final long CTX_OFFSET_PARENT_TAG = 32L;").ok();
    writeln!(out, "    private static final long CTX_OFFSET_IS_INLINE = 40L;").ok();
    writeln!(out).ok();
    writeln!(out, "    private final Arena arena;").ok();
    writeln!(out, "    private final MemorySegment struct;").ok();
    writeln!(out, "    private final Visitor visitor;").ok();
    writeln!(
        out,
        "    /** Sticky exception captured during a visitor upcall; surfaced after conversion. */"
    )
    .ok();
    writeln!(out, "    private volatile Throwable visitorError;").ok();
    writeln!(out).ok();
    writeln!(out, "    VisitorBridge(Visitor visitor) {{").ok();
    writeln!(out, "        this.visitor = visitor;").ok();
    writeln!(out, "        this.arena = Arena.ofConfined();").ok();
    writeln!(out, "        this.struct = arena.allocate(CALLBACKS_STRUCT_SIZE);").ok();
    writeln!(out, "        // Slot 0: user_data = NULL").ok();
    writeln!(out, "        // (visitor captured via lambda closure)").ok();
    writeln!(out, "        struct.set(ValueLayout.ADDRESS, 0L, MemorySegment.NULL);").ok();
    writeln!(out, "        try {{").ok();
    writeln!(out, "            long offset = ValueLayout.ADDRESS.byteSize();").ok();
    // Compute number of registerStubs sub-methods needed
    let num_chunks = CALLBACKS.chunks(10).count();
    for i in 1..=num_chunks {
        if i < num_chunks {
            writeln!(out, "            offset = registerStubs{}(offset);", i).ok();
        } else {
            writeln!(out, "            registerStubs{}(offset);", i).ok();
        }
    }
    writeln!(out, "        }} catch (ReflectiveOperationException e) {{").ok();
    writeln!(out, "            arena.close();").ok();
    writeln!(out, "            throw new RuntimeException(").ok();
    writeln!(out, "                \"Failed to create visitor upcall stubs\", e);").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // Split callbacks into chunks of 5 each; each sub-method returns the updated offset.
    // Checkstyle enforces a 150-line MethodLength limit; with ~22 lines per callback,
    // chunks of 5 produce ~114 lines per method, safely under the limit.
    const CHUNK_SIZE: usize = 5;
    for (chunk_idx, chunk) in CALLBACKS.chunks(CHUNK_SIZE).enumerate() {
        let method_num = chunk_idx + 1;
        writeln!(
            out,
            "    private long registerStubs{}(\n            final long offset)\n            throws ReflectiveOperationException {{",
            method_num
        )
        .ok();
        writeln!(out, "        long off = offset;").ok();
        for spec in chunk {
            let descriptor = callback_descriptor(spec);
            let method_type = callback_method_type(spec);
            let stub_var = stub_var_name(spec.java_method);
            writeln!(out, "        // {}", spec.c_field).ok();
            writeln!(out, "        var {} = LINKER.upcallStub(", stub_var).ok();
            writeln!(out, "                LOOKUP.bind(",).ok();
            writeln!(
                out,
                "                    this, \"{}\",",
                super::helpers::handle_method_name(spec.java_method),
            )
            .ok();
            writeln!(out, "                    {}),", method_type).ok();
            writeln!(out, "                {},", descriptor).ok();
            writeln!(out, "                arena);").ok();
            writeln!(out, "        struct.set(ValueLayout.ADDRESS, off, {});", stub_var).ok();
            writeln!(out, "        off += ValueLayout.ADDRESS.byteSize();").ok();
        }
        writeln!(out, "        return off;").ok();
        writeln!(out, "    }}").ok();
        writeln!(out).ok();
    }
    writeln!(out).ok();
    writeln!(
        out,
        "    /** Returns the native HTMHtmVisitorCallbacks struct pointer. */"
    )
    .ok();
    writeln!(out, "    MemorySegment callbacksStruct() {{").ok();
    writeln!(out, "        return struct;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    void rethrowVisitorError() throws Throwable {{").ok();
    writeln!(out, "        Throwable err = visitorError;").ok();
    writeln!(out, "        if (err != null) {{").ok();
    writeln!(out, "            throw err;").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // Generate one handle_* method per callback
    for spec in CALLBACKS {
        gen_handle_method(&mut out, spec);
    }

    // decodeNodeContext helper
    writeln!(
        out,
        "    // HTMHtmNodeContext: int32 node_type, char* tag_name, uintptr depth,"
    )
    .ok();
    writeln!(out, "    //   uintptr index_in_parent, char* parent_tag,").ok();
    writeln!(out, "    //   int32 is_inline").ok();
    writeln!(out, "    private static final MemoryLayout CTX_LAYOUT =").ok();
    writeln!(out, "        MemoryLayout.structLayout(").ok();
    writeln!(out, "            ValueLayout.JAVA_INT.withName(\"node_type\"),").ok();
    writeln!(out, "            MemoryLayout.paddingLayout(4),").ok();
    writeln!(out, "            ValueLayout.ADDRESS.withName(\"tag_name\"),").ok();
    writeln!(out, "            ValueLayout.JAVA_LONG.withName(\"depth\"),").ok();
    writeln!(out, "            ValueLayout.JAVA_LONG.withName(\"index_in_parent\"),").ok();
    writeln!(out, "            ValueLayout.ADDRESS.withName(\"parent_tag\"),").ok();
    writeln!(out, "            ValueLayout.JAVA_INT.withName(\"is_inline\"),").ok();
    writeln!(out, "            MemoryLayout.paddingLayout(4)").ok();
    writeln!(out, "    );").ok();
    writeln!(out).ok();
    writeln!(out, "    private static NodeContext decodeNodeContext(").ok();
    writeln!(out, "            final MemorySegment ctxPtr) {{").ok();
    writeln!(out, "        var ctx = ctxPtr.reinterpret(").ok();
    writeln!(out, "            CTX_LAYOUT.byteSize());").ok();
    writeln!(out, "        int nodeType = ctx.get(").ok();
    writeln!(out, "            ValueLayout.JAVA_INT, 0L);").ok();
    writeln!(out, "        var tagNamePtr = ctx.get(").ok();
    writeln!(out, "            ValueLayout.ADDRESS, CTX_OFFSET_TAG_NAME);").ok();
    writeln!(out, "        String tagName = tagNamePtr").ok();
    writeln!(out, "            .reinterpret(Long.MAX_VALUE).getString(0);").ok();
    writeln!(
        out,
        "        long depth = ctx.get(ValueLayout.JAVA_LONG, CTX_OFFSET_DEPTH);"
    )
    .ok();
    writeln!(
        out,
        "        long indexInParent = ctx.get(ValueLayout.JAVA_LONG, CTX_OFFSET_INDEX_IN_PARENT);"
    )
    .ok();
    writeln!(
        out,
        "        var parentTagPtr = ctx.get(ValueLayout.ADDRESS, CTX_OFFSET_PARENT_TAG);"
    )
    .ok();
    writeln!(
        out,
        "        String parentTag = parentTagPtr.equals(MemorySegment.NULL) ? null"
    )
    .ok();
    writeln!(
        out,
        "                : parentTagPtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
    writeln!(
        out,
        "        int isInlineRaw = ctx.get(ValueLayout.JAVA_INT, CTX_OFFSET_IS_INLINE);"
    )
    .ok();
    writeln!(
        out,
        "        return new NodeContext(nodeType, tagName, depth, indexInParent, parentTag, isInlineRaw != 0);"
    )
    .ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // decodeCells helper
    writeln!(
        out,
        "    private static List<String> decodeCells(MemorySegment cellsPtr, long count) {{"
    )
    .ok();
    writeln!(out, "        var result = new ArrayList<String>((int) count);").ok();
    writeln!(out, "        for (long i = 0; i < count; i++) {{").ok();
    writeln!(
        out,
        "            var ptr = cellsPtr.get(ValueLayout.ADDRESS, i * ValueLayout.ADDRESS.byteSize());"
    )
    .ok();
    writeln!(
        out,
        "            result.add(ptr.reinterpret(Long.MAX_VALUE).getString(0));"
    )
    .ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        return result;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // encodeVisitResult helper
    // outCustom and outLen arrive as 0-byte upcall MemorySegments; we must
    // reinterpret them to the correct size before writing (Panama FFM requirement).
    // Buffers are allocated into this.arena so they are freed when the bridge closes,
    // after Rust has finished reading the pointer.
    writeln!(
        out,
        "    private int encodeVisitResult(VisitResult result, MemorySegment outCustom, MemorySegment outLen) {{"
    )
    .ok();
    writeln!(out, "        return switch (result) {{").ok();
    writeln!(
        out,
        "            case VisitResult.Continue ignored -> VISIT_RESULT_CONTINUE;"
    )
    .ok();
    writeln!(out, "            case VisitResult.Skip ignored -> VISIT_RESULT_SKIP;").ok();
    writeln!(
        out,
        "            case VisitResult.PreserveHtml ignored -> VISIT_RESULT_PRESERVE_HTML;"
    )
    .ok();
    writeln!(out, "            case VisitResult.Custom c -> {{").ok();
    writeln!(
        out,
        "                var buf = Arena.global().allocateFrom(c.markdown());"
    )
    .ok();
    writeln!(
        out,
        "                outCustom.reinterpret(ValueLayout.ADDRESS.byteSize()).set(ValueLayout.ADDRESS, 0L, buf);"
    )
    .ok();
    writeln!(
        out,
        "                long customLen = (long) c.markdown().getBytes(java.nio.charset.StandardCharsets.UTF_8).length;"
    )
    .ok();
    writeln!(out, "                outLen.reinterpret(ValueLayout.JAVA_LONG.byteSize()).set(ValueLayout.JAVA_LONG, 0L, customLen);").ok();
    writeln!(out, "                yield VISIT_RESULT_CUSTOM;").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "            case VisitResult.Error e -> {{").ok();
    writeln!(
        out,
        "                var buf = Arena.global().allocateFrom(e.message());"
    )
    .ok();
    writeln!(
        out,
        "                outCustom.reinterpret(ValueLayout.ADDRESS.byteSize()).set(ValueLayout.ADDRESS, 0L, buf);"
    )
    .ok();
    writeln!(
        out,
        "                long errorLen = (long) e.message().getBytes(java.nio.charset.StandardCharsets.UTF_8).length;"
    )
    .ok();
    writeln!(
        out,
        "                outLen.reinterpret(ValueLayout.JAVA_LONG.byteSize()).set(ValueLayout.JAVA_LONG, 0L, errorLen);"
    )
    .ok();
    writeln!(out, "                yield VISIT_RESULT_ERROR;").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "        }};").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    writeln!(out, "    @Override").ok();
    writeln!(out, "    public void close() {{").ok();
    writeln!(out, "        arena.close();").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    out
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
}
