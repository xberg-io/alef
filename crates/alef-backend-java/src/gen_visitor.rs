/// Generate Java visitor support: interface, NodeContext record, VisitResult sealed interface,
/// VisitorBridge (upcall stubs), and convertWithVisitor method.
///
/// # Panama FFM upcall strategy
///
/// Java cannot expose method references as raw C function pointers. The generated
/// code uses Java 21+ Foreign Function & Memory API (Panama) upcall stubs:
///
/// - `NodeContext`: a `record` carrying the fields from `HTMHtmNodeContext`.
/// - `VisitResult`: a `sealed interface` with `Continue`, `Skip`, `PreserveHtml`,
///   `Custom`, and `Error` implementations.
/// - `Visitor`: an `interface` with default no-op methods for all 40 callbacks.
/// - `VisitorBridge`: a package-private class that allocates one `MemorySegment`
///   upcall stub per callback inside a `Arena.ofConfined()` scope, then writes
///   all stubs into a flat `MemorySegment` matching `HTMHtmVisitorCallbacks`.
/// - `convertWithVisitor`: static method on the wrapper class that drives the full
///   lifecycle: marshal options → create `VisitorBridge` → `htm_visitor_create` →
///   `htm_convert_with_visitor` → deserialise JSON result → `htm_visitor_free`.
use std::fmt::Write;

// ---------------------------------------------------------------------------
// Callback specification table (mirrors crates/alef-backend-go/src/gen_visitor.rs)
// ---------------------------------------------------------------------------

pub struct CallbackSpec {
    /// Field name in `HTMHtmVisitorCallbacks` (snake_case). Used for documentation.
    pub c_field: &'static str,
    /// Java interface method name (camelCase).
    pub java_method: &'static str,
    /// Javadoc line.
    pub doc: &'static str,
    /// Extra parameters beyond `NodeContext` in the Java interface.
    pub extra: &'static [ExtraParam],
    /// If true, add `boolean isHeader` (only visit_table_row).
    pub has_is_header: bool,
}

pub struct ExtraParam {
    /// Java parameter name in the interface.
    pub java_name: &'static str,
    /// Java type in the interface method signature.
    pub java_type: &'static str,
    /// Panama `ValueLayout` constants for each C-level argument that maps to this Java param.
    /// One Java param can correspond to multiple C args (e.g. cells = ptr + count).
    pub c_layouts: &'static [&'static str],
    /// Java expression to build the interface-typed value from the raw C parameters.
    /// Raw variables are named `raw_<java_name>_<idx>` where idx counts within c_layouts.
    pub decode: &'static str,
}

pub const CALLBACKS: &[CallbackSpec] = &[
    CallbackSpec {
        c_field: "visit_text",
        java_method: "visitText",
        doc: "Called for text nodes.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_element_start",
        java_method: "visitElementStart",
        doc: "Called before entering any element.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_element_end",
        java_method: "visitElementEnd",
        doc: "Called after exiting any element; receives the default markdown output.",
        extra: &[ExtraParam {
            java_name: "output",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_output_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_link",
        java_method: "visitLink",
        doc: "Called for anchor links. title is null when the attribute is absent.",
        extra: &[
            ExtraParam {
                java_name: "href",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_href_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "text",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "title",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_title_0.equals(MemorySegment.NULL) ? null : raw_title_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_image",
        java_method: "visitImage",
        doc: "Called for images. title is null when absent.",
        extra: &[
            ExtraParam {
                java_name: "src",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_src_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "alt",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_alt_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "title",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_title_0.equals(MemorySegment.NULL) ? null : raw_title_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_heading",
        java_method: "visitHeading",
        doc: "Called for heading elements h1-h6. id is null when absent.",
        extra: &[
            ExtraParam {
                java_name: "level",
                java_type: "int",
                c_layouts: &["ValueLayout.JAVA_INT"],
                decode: "(int) raw_level_0",
            },
            ExtraParam {
                java_name: "text",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "id",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_id_0.equals(MemorySegment.NULL) ? null : raw_id_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_code_block",
        java_method: "visitCodeBlock",
        doc: "Called for code blocks. lang is null when absent.",
        extra: &[
            ExtraParam {
                java_name: "lang",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_lang_0.equals(MemorySegment.NULL) ? null : raw_lang_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "code",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_code_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_code_inline",
        java_method: "visitCodeInline",
        doc: "Called for inline code elements.",
        extra: &[ExtraParam {
            java_name: "code",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_code_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_item",
        java_method: "visitListItem",
        doc: "Called for list items.",
        extra: &[
            ExtraParam {
                java_name: "ordered",
                java_type: "boolean",
                c_layouts: &["ValueLayout.JAVA_INT"],
                decode: "((int) raw_ordered_0) != 0",
            },
            ExtraParam {
                java_name: "marker",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_marker_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "text",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_start",
        java_method: "visitListStart",
        doc: "Called before processing a list.",
        extra: &[ExtraParam {
            java_name: "ordered",
            java_type: "boolean",
            c_layouts: &["ValueLayout.JAVA_INT"],
            decode: "((int) raw_ordered_0) != 0",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_end",
        java_method: "visitListEnd",
        doc: "Called after processing a list.",
        extra: &[
            ExtraParam {
                java_name: "ordered",
                java_type: "boolean",
                c_layouts: &["ValueLayout.JAVA_INT"],
                decode: "((int) raw_ordered_0) != 0",
            },
            ExtraParam {
                java_name: "output",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_output_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_table_start",
        java_method: "visitTableStart",
        doc: "Called before processing a table.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_table_row",
        java_method: "visitTableRow",
        doc: "Called for table rows. cells contains the cell text values.",
        extra: &[ExtraParam {
            java_name: "cells",
            java_type: "java.util.List<String>",
            c_layouts: &["ValueLayout.ADDRESS", "ValueLayout.JAVA_LONG"],
            decode: "decodeCells(raw_cells_0, (long) raw_cells_1)",
        }],
        has_is_header: true,
    },
    CallbackSpec {
        c_field: "visit_table_end",
        java_method: "visitTableEnd",
        doc: "Called after processing a table.",
        extra: &[ExtraParam {
            java_name: "output",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_output_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_blockquote",
        java_method: "visitBlockquote",
        doc: "Called for blockquote elements.",
        extra: &[
            ExtraParam {
                java_name: "content",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_content_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "depth",
                java_type: "long",
                c_layouts: &["ValueLayout.JAVA_LONG"],
                decode: "(long) raw_depth_0",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_strong",
        java_method: "visitStrong",
        doc: "Called for strong/bold elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_emphasis",
        java_method: "visitEmphasis",
        doc: "Called for emphasis/italic elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_strikethrough",
        java_method: "visitStrikethrough",
        doc: "Called for strikethrough elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_underline",
        java_method: "visitUnderline",
        doc: "Called for underline elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_subscript",
        java_method: "visitSubscript",
        doc: "Called for subscript elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_superscript",
        java_method: "visitSuperscript",
        doc: "Called for superscript elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_mark",
        java_method: "visitMark",
        doc: "Called for mark/highlight elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_line_break",
        java_method: "visitLineBreak",
        doc: "Called for line break elements.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_horizontal_rule",
        java_method: "visitHorizontalRule",
        doc: "Called for horizontal rule elements.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_custom_element",
        java_method: "visitCustomElement",
        doc: "Called for custom or unknown elements.",
        extra: &[
            ExtraParam {
                java_name: "tagName",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_tagName_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "html",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_html_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_list_start",
        java_method: "visitDefinitionListStart",
        doc: "Called before a definition list.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_term",
        java_method: "visitDefinitionTerm",
        doc: "Called for definition term elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_description",
        java_method: "visitDefinitionDescription",
        doc: "Called for definition description elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_list_end",
        java_method: "visitDefinitionListEnd",
        doc: "Called after a definition list.",
        extra: &[ExtraParam {
            java_name: "output",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_output_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_form",
        java_method: "visitForm",
        doc: "Called for form elements. action and method may be null.",
        extra: &[
            ExtraParam {
                java_name: "action",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_action_0.equals(MemorySegment.NULL) ? null : raw_action_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "method",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_method_0.equals(MemorySegment.NULL) ? null : raw_method_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_input",
        java_method: "visitInput",
        doc: "Called for input elements. name and value may be null.",
        extra: &[
            ExtraParam {
                java_name: "inputType",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_inputType_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "name",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_name_0.equals(MemorySegment.NULL) ? null : raw_name_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
            ExtraParam {
                java_name: "value",
                java_type: "String",
                c_layouts: &["ValueLayout.ADDRESS"],
                decode: "raw_value_0.equals(MemorySegment.NULL) ? null : raw_value_0.reinterpret(Long.MAX_VALUE).getString(0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_button",
        java_method: "visitButton",
        doc: "Called for button elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_audio",
        java_method: "visitAudio",
        doc: "Called for audio elements. src may be null.",
        extra: &[ExtraParam {
            java_name: "src",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_src_0.equals(MemorySegment.NULL) ? null : raw_src_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_video",
        java_method: "visitVideo",
        doc: "Called for video elements. src may be null.",
        extra: &[ExtraParam {
            java_name: "src",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_src_0.equals(MemorySegment.NULL) ? null : raw_src_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_iframe",
        java_method: "visitIframe",
        doc: "Called for iframe elements. src may be null.",
        extra: &[ExtraParam {
            java_name: "src",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_src_0.equals(MemorySegment.NULL) ? null : raw_src_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_details",
        java_method: "visitDetails",
        doc: "Called for details elements.",
        extra: &[ExtraParam {
            java_name: "open",
            java_type: "boolean",
            c_layouts: &["ValueLayout.JAVA_INT"],
            decode: "((int) raw_open_0) != 0",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_summary",
        java_method: "visitSummary",
        doc: "Called for summary elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figure_start",
        java_method: "visitFigureStart",
        doc: "Called before a figure element.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figcaption",
        java_method: "visitFigcaption",
        doc: "Called for figcaption elements.",
        extra: &[ExtraParam {
            java_name: "text",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_text_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figure_end",
        java_method: "visitFigureEnd",
        doc: "Called after a figure element.",
        extra: &[ExtraParam {
            java_name: "output",
            java_type: "String",
            c_layouts: &["ValueLayout.ADDRESS"],
            decode: "raw_output_0.reinterpret(Long.MAX_VALUE).getString(0)",
        }],
        has_is_header: false,
    },
];

// ---------------------------------------------------------------------------
// Public API: generate visitor-related Java source files
// ---------------------------------------------------------------------------

/// Returns `(filename, content)` pairs for all visitor-related Java files.
///
/// Callers push these into the `files` vector in `generate_bindings`.
pub fn gen_visitor_files(package: &str, class_name: &str) -> Vec<(String, String)> {
    vec![
        ("NodeContext.java".to_string(), gen_node_context(package)),
        ("VisitResult.java".to_string(), gen_visit_result(package)),
        ("Visitor.java".to_string(), gen_visitor_interface(package, class_name)),
        (
            "VisitorBridge.java".to_string(),
            gen_visitor_bridge(package, class_name),
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
        "                var optJson = arena.allocateFrom(createObjectMapper().writeValueAsString(options));"
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
        "                var json = resultPtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
    writeln!(out, "                NativeLib.{pu}_FREE_STRING.invoke(resultPtr);").ok();
    writeln!(
        out,
        "                return createObjectMapper().readValue(json, ConversionResult.class);"
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
    writeln!(out, "            }}").ok();
    writeln!(out, "        }} catch ({exc} e) {{").ok();
    writeln!(out, "            throw e;").ok();
    writeln!(out, "        }} catch (Throwable e) {{").ok();
    writeln!(out, "            throw new {exc}(\"FFI call failed\", e);").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();

    out
}

// ---------------------------------------------------------------------------
// Individual file generators
// ---------------------------------------------------------------------------

fn gen_node_context(package: &str) -> String {
    let mut out = String::with_capacity(1024);
    writeln!(out, "// Code generated by alef. DO NOT EDIT.").ok();
    writeln!(out, "package {package};").ok();
    writeln!(out).ok();
    writeln!(out, "/** Context passed to every visitor callback. */").ok();
    writeln!(out, "public record NodeContext(").ok();
    writeln!(out, "        /** Coarse-grained node type tag. */").ok();
    writeln!(out, "        int nodeType,").ok();
    writeln!(out, "        /** HTML element tag name (e.g. \"div\"). */").ok();
    writeln!(out, "        String tagName,").ok();
    writeln!(out, "        /** DOM depth (0 = root). */").ok();
    writeln!(out, "        long depth,").ok();
    writeln!(out, "        /** 0-based sibling index. */").ok();
    writeln!(out, "        long indexInParent,").ok();
    writeln!(out, "        /** Parent element tag name, or null at the root. */").ok();
    writeln!(out, "        String parentTag,").ok();
    writeln!(out, "        /** True when this element is treated as inline. */").ok();
    writeln!(out, "        boolean isInline").ok();
    writeln!(out, ") {{}}").ok();
    out
}

fn gen_visit_result(package: &str) -> String {
    let mut out = String::with_capacity(2048);
    writeln!(out, "// Code generated by alef. DO NOT EDIT.").ok();
    writeln!(out, "package {package};").ok();
    writeln!(out).ok();
    writeln!(out, "/** Controls how the visitor affects the conversion pipeline. */").ok();
    writeln!(out, "public sealed interface VisitResult").ok();
    writeln!(
        out,
        "        permits VisitResult.Continue, VisitResult.Skip, VisitResult.PreserveHtml,"
    )
    .ok();
    writeln!(out, "                VisitResult.Custom, VisitResult.Error {{").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Proceed with default conversion. */").ok();
    writeln!(out, "    record Continue() implements VisitResult {{}}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Omit this element from output entirely. */").ok();
    writeln!(out, "    record Skip() implements VisitResult {{}}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Keep original HTML verbatim. */").ok();
    writeln!(out, "    record PreserveHtml() implements VisitResult {{}}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Replace with custom Markdown. */").ok();
    writeln!(out, "    record Custom(String markdown) implements VisitResult {{}}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Abort conversion with an error message. */").ok();
    writeln!(out, "    record Error(String message) implements VisitResult {{}}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Convenience: continue with default conversion. */").ok();
    writeln!(
        out,
        "    static VisitResult continueDefault() {{ return new Continue(); }}"
    )
    .ok();
    writeln!(out).ok();
    writeln!(out, "    /** Convenience: skip this element. */").ok();
    writeln!(out, "    static VisitResult skip() {{ return new Skip(); }}").ok();
    writeln!(out).ok();
    writeln!(out, "    /** Convenience: preserve original HTML. */").ok();
    writeln!(
        out,
        "    static VisitResult preserveHtml() {{ return new PreserveHtml(); }}"
    )
    .ok();
    writeln!(out).ok();
    writeln!(out, "    /** Convenience: emit custom Markdown. */").ok();
    writeln!(
        out,
        "    static VisitResult custom(String markdown) {{ return new Custom(markdown); }}"
    )
    .ok();
    writeln!(out).ok();
    writeln!(out, "    /** Convenience: abort with error. */").ok();
    writeln!(
        out,
        "    static VisitResult error(String message) {{ return new Error(message); }}"
    )
    .ok();
    writeln!(out, "}}").ok();
    out
}

fn gen_visitor_interface(package: &str, _class_name: &str) -> String {
    let mut out = String::with_capacity(4096);
    writeln!(out, "// Code generated by alef. DO NOT EDIT.").ok();
    writeln!(out, "package {package};").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "/** Visitor interface for the HTML-to-Markdown conversion pipeline. */"
    )
    .ok();
    writeln!(out, "public interface Visitor {{").ok();
    for spec in CALLBACKS {
        let params = iface_param_str(spec);
        writeln!(out, "    /** {} */", spec.doc).ok();
        writeln!(
            out,
            "    default VisitResult {}({}) {{ return VisitResult.continueDefault(); }}",
            spec.java_method, params
        )
        .ok();
    }
    writeln!(out, "}}").ok();
    out
}

/// Generate `VisitorBridge.java` — builds Panama upcall stubs for all 40 callbacks
/// and exposes a `MemorySegment callbacksStruct()` pointing to the C struct.
fn gen_visitor_bridge(package: &str, _class_name: &str) -> String {
    let mut out = String::with_capacity(32_768);
    writeln!(out, "// Code generated by alef. DO NOT EDIT.").ok();
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
    writeln!(
        out,
        "    private static final MethodHandles.Lookup LOOKUP = MethodHandles.lookup();"
    )
    .ok();
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
        "    // HTMHtmVisitorCallbacks: user_data + {n} callback pointers = {total} pointer-sized slots",
        n = CALLBACKS.len(),
        total = num_fields,
    )
    .ok();
    writeln!(
        out,
        "    private static final long CALLBACKS_STRUCT_SIZE = (long) ValueLayout.ADDRESS.byteSize() * {num_fields}L;"
    )
    .ok();
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
    writeln!(out).ok();
    writeln!(out, "    VisitorBridge(Visitor visitor) {{").ok();
    writeln!(out, "        this.visitor = visitor;").ok();
    writeln!(out, "        this.arena = Arena.ofConfined();").ok();
    writeln!(out, "        this.struct = arena.allocate(CALLBACKS_STRUCT_SIZE);").ok();
    writeln!(
        out,
        "        // Slot 0: user_data = NULL (not needed; visitor captured via lambda closure)"
    )
    .ok();
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
    writeln!(
        out,
        "            throw new RuntimeException(\"Failed to create visitor upcall stubs\", e);"
    )
    .ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // Split callbacks into chunks of 10 each; each sub-method returns the updated offset.
    const CHUNK_SIZE: usize = 10;
    for (chunk_idx, chunk) in CALLBACKS.chunks(CHUNK_SIZE).enumerate() {
        let method_num = chunk_idx + 1;
        writeln!(
            out,
            "    private long registerStubs{}(final long offset) throws ReflectiveOperationException {{",
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
            writeln!(
                out,
                "                LOOKUP.bind(",
            )
            .ok();
            writeln!(
                out,
                "                    this, \"{}\",",
                handle_method_name(spec.java_method),
            )
            .ok();
            writeln!(
                out,
                "                    {}),",
                method_type
            )
            .ok();
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
    writeln!(
        out,
        "    //                    uintptr index_in_parent, char* parent_tag, int32 is_inline"
    )
    .ok();
    writeln!(
        out,
        "    private static final MemoryLayout CTX_LAYOUT = MemoryLayout.structLayout("
    )
    .ok();
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
    writeln!(
        out,
        "    private static NodeContext decodeNodeContext(MemorySegment ctxPtr) {{"
    )
    .ok();
    writeln!(out, "        var ctx = ctxPtr.reinterpret(CTX_LAYOUT.byteSize());").ok();
    writeln!(out, "        int nodeType = ctx.get(ValueLayout.JAVA_INT, 0L);").ok();
    writeln!(
        out,
        "        var tagNamePtr = ctx.get(ValueLayout.ADDRESS, CTX_OFFSET_TAG_NAME);"
    )
    .ok();
    writeln!(
        out,
        "        String tagName = tagNamePtr.reinterpret(Long.MAX_VALUE).getString(0);"
    )
    .ok();
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
    writeln!(
        out,
        "    private static int encodeVisitResult(VisitResult result, MemorySegment outCustom, MemorySegment outLen, Arena encArena) {{"
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
    writeln!(out, "                var buf = encArena.allocateFrom(c.markdown());").ok();
    writeln!(out, "                outCustom.set(ValueLayout.ADDRESS, 0L, buf);").ok();
    writeln!(
        out,
        "                outLen.set(ValueLayout.JAVA_LONG, 0L, (long) c.markdown().getBytes(java.nio.charset.StandardCharsets.UTF_8).length);"
    )
    .ok();
    writeln!(out, "                yield VISIT_RESULT_CUSTOM;").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "            case VisitResult.Error e -> {{").ok();
    writeln!(out, "                var buf = encArena.allocateFrom(e.message());").ok();
    writeln!(out, "                outCustom.set(ValueLayout.ADDRESS, 0L, buf);").ok();
    writeln!(
        out,
        "                outLen.set(ValueLayout.JAVA_LONG, 0L, (long) e.message().getBytes(java.nio.charset.StandardCharsets.UTF_8).length);"
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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Generate camelCase stub variable name: stub + capitalize(java_method).
/// e.g. visitText -> stubVisitText
fn stub_var_name(java_method: &str) -> String {
    let mut name = String::with_capacity(5 + java_method.len());
    name.push_str("stub");
    let mut chars = java_method.chars();
    if let Some(first) = chars.next() {
        for c in first.to_uppercase() {
            name.push(c);
        }
        name.push_str(chars.as_str());
    }
    name
}

fn handle_method_name(java_method: &str) -> String {
    // camelCase: "handle" + capitalize first letter of java_method
    let mut name = String::with_capacity(7 + java_method.len());
    name.push_str("handle");
    let mut chars = java_method.chars();
    if let Some(first) = chars.next() {
        for c in first.to_uppercase() {
            name.push(c);
        }
        name.push_str(chars.as_str());
    }
    name
}

fn iface_param_str(spec: &CallbackSpec) -> String {
    let mut params = vec!["final NodeContext context".to_string()];
    for ep in spec.extra {
        params.push(format!("final {} {}", ep.java_type, ep.java_name));
    }
    if spec.has_is_header {
        params.push("final boolean isHeader".to_string());
    }
    params.join(", ")
}

/// Build the `FunctionDescriptor` for one callback's upcall stub.
/// All callbacks: (ADDRESS ctx, ADDRESS userData, ..extra.., ADDRESS outCustom, ADDRESS outLen) -> JAVA_INT
/// Returns a multi-line string with 20-space continuation indent so no line exceeds 80 chars.
fn callback_descriptor(spec: &CallbackSpec) -> String {
    let mut layouts = vec![
        "ValueLayout.ADDRESS".to_string(), // ctx
        "ValueLayout.ADDRESS".to_string(), // user_data
    ];
    for ep in spec.extra {
        for layout in ep.c_layouts {
            layouts.push((*layout).to_string());
        }
    }
    if spec.has_is_header {
        layouts.push("ValueLayout.JAVA_INT".to_string());
    }
    layouts.push("ValueLayout.ADDRESS".to_string()); // out_custom
    layouts.push("ValueLayout.ADDRESS".to_string()); // out_len
    let indent = "                    ";
    let args = layouts.join(&format!(",\n{indent}"));
    format!("FunctionDescriptor.of(\n{indent}ValueLayout.JAVA_INT,\n{indent}{args})")
}

/// Build the `MethodType` for `LOOKUP.bind(this, name, type)`.
/// Returns a multi-line string with 20-space continuation indent so no line exceeds 80 chars.
fn callback_method_type(spec: &CallbackSpec) -> String {
    let mut types = vec![
        "MemorySegment.class".to_string(), // ctx
        "MemorySegment.class".to_string(), // user_data
    ];
    for ep in spec.extra {
        for layout in ep.c_layouts {
            types.push(layout_to_java_class(layout).to_string());
        }
    }
    if spec.has_is_header {
        types.push("int.class".to_string());
    }
    types.push("MemorySegment.class".to_string()); // out_custom
    types.push("MemorySegment.class".to_string()); // out_len
    let indent = "                    ";
    let args = types.join(&format!(",\n{indent}"));
    format!("MethodType.methodType(\n{indent}int.class,\n{indent}{args})")
}

fn layout_to_java_class(layout: &str) -> &'static str {
    match layout {
        "ValueLayout.ADDRESS" => "MemorySegment.class",
        "ValueLayout.JAVA_INT" => "int.class",
        "ValueLayout.JAVA_LONG" => "long.class",
        _ => "long.class",
    }
}

/// Generate one `handle_*` instance method inside `VisitorBridge`.
fn gen_handle_method(out: &mut String, spec: &CallbackSpec) {
    // Build method signature matching the MethodType passed to upcallStub.
    let mut params = vec![
        "final MemorySegment ctx".to_string(),
        "final MemorySegment userData".to_string(),
    ];
    for ep in spec.extra {
        for (c_idx, layout) in ep.c_layouts.iter().enumerate() {
            let java_ptype = match *layout {
                "ValueLayout.JAVA_INT" => "int",
                "ValueLayout.JAVA_LONG" => "long",
                _ => "MemorySegment",
            };
            params.push(format!("final {java_ptype} {}", raw_var_name(ep.java_name, c_idx)));
        }
    }
    if spec.has_is_header {
        params.push("final int isHeader".to_string());
    }
    params.push("final MemorySegment outCustom".to_string());
    params.push("final MemorySegment outLen".to_string());

    writeln!(
        out,
        "    int {}({}) {{",
        handle_method_name(spec.java_method),
        params.join(", ")
    )
    .ok();
    writeln!(out, "        try (var encArena = Arena.ofConfined()) {{").ok();
    writeln!(out, "            var context = decodeNodeContext(ctx);").ok();

    // Decode each extra param
    for ep in spec.extra {
        let mut decode = ep.decode.to_string();
        for (c_idx, _) in ep.c_layouts.iter().enumerate() {
            let placeholder = format!("raw_{}_{}", ep.java_name, c_idx);
            let var = raw_var_name(ep.java_name, c_idx);
            decode = decode.replace(&placeholder, &var);
        }
        writeln!(out, "            var {} = {};", ep.java_name, decode).ok();
    }
    if spec.has_is_header {
        writeln!(out, "            var goIsHeader = isHeader != 0;").ok();
    }

    // Build call args
    let mut call_args = vec!["context".to_string()];
    for ep in spec.extra {
        call_args.push(ep.java_name.to_string());
    }
    if spec.has_is_header {
        call_args.push("goIsHeader".to_string());
    }

    writeln!(
        out,
        "            var result = visitor.{}({});",
        spec.java_method,
        call_args.join(", ")
    )
    .ok();
    writeln!(
        out,
        "            return encodeVisitResult(result, outCustom, outLen, encArena);"
    )
    .ok();
    writeln!(out, "        }} catch (Throwable ignored) {{").ok();
    writeln!(out, "            return 0;").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
}

fn raw_var_name(java_name: &str, c_idx: usize) -> String {
    // camelCase: "raw" + capitalize first letter of java_name + "_" + index
    // e.g. raw_text_0 -> rawText0, raw_cells_1 -> rawCells1
    let mut name = String::with_capacity(4 + java_name.len() + 2);
    name.push_str("raw");
    let mut chars = java_name.chars();
    if let Some(first) = chars.next() {
        for c in first.to_uppercase() {
            name.push(c);
        }
        name.push_str(chars.as_str());
    }
    name.push_str(&c_idx.to_string());
    name
}
