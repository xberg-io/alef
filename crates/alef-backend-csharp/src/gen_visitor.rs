/// Generate C# visitor support: IVisitor interface, NodeContext/VisitResult records,
/// VisitorCallbacks (P/Invoke delegate struct), and ConvertWithVisitor method.
///
/// # P/Invoke delegate callback strategy
///
/// C# uses `[UnmanagedFunctionPointer]` delegate types to create `IntPtr` function pointers
/// that can be passed through the `HTMHtmVisitorCallbacks` C struct.
///
/// - `NodeContext`: a `record` with fields from `HTMHtmNodeContext`.
/// - `VisitResult`: a discriminated union using a record class hierarchy.
/// - `IVisitor`: an interface with default no-op implementations for all 40 callbacks.
/// - `VisitorCallbacks`: an internal class that allocates `GCHandle`s for all delegate
///   instances and writes them into a marshalled struct layout matching the C struct.
/// - `ConvertWithVisitor`: static method on the wrapper class that creates the delegate
///   struct, calls `htm_visitor_create`, `htm_convert_with_visitor`, deserialises JSON.
use alef_core::hash::{self, CommentStyle};
use heck::ToSnakeCase;
use std::fmt::Write;

// ---------------------------------------------------------------------------
// Callback specification table
// ---------------------------------------------------------------------------

pub struct CallbackSpec {
    /// Field name in `HTMHtmVisitorCallbacks`.
    pub c_field: &'static str,
    /// C# interface method name (PascalCase).
    pub cs_method: &'static str,
    /// XML doc summary.
    pub doc: &'static str,
    /// Extra parameters beyond `NodeContext` in the C# interface.
    pub extra: &'static [ExtraParam],
    /// If true, add `bool isHeader` (only visit_table_row).
    pub has_is_header: bool,
}

pub struct ExtraParam {
    /// C# parameter name in the interface.
    pub cs_name: &'static str,
    /// C# type in the interface method signature.
    pub cs_type: &'static str,
    /// P/Invoke types for each raw C parameter (one or more per Java param).
    pub pinvoke_types: &'static [&'static str],
    /// C# expression to decode the raw P/Invoke args (vars named `raw<CsName>N`).
    pub decode: &'static str,
}

pub const CALLBACKS: &[CallbackSpec] = &[
    CallbackSpec {
        c_field: "visit_text",
        cs_method: "VisitText",
        doc: "Called for text nodes.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_element_start",
        cs_method: "VisitElementStart",
        doc: "Called before entering any element.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_element_end",
        cs_method: "VisitElementEnd",
        doc: "Called after exiting any element; receives the default markdown output.",
        extra: &[ExtraParam {
            cs_name: "output",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawOutput0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_link",
        cs_method: "VisitLink",
        doc: "Called for anchor links. title is null when the attribute is absent.",
        extra: &[
            ExtraParam {
                cs_name: "href",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawHref0)!",
            },
            ExtraParam {
                cs_name: "text",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawText0)!",
            },
            ExtraParam {
                cs_name: "title",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawTitle0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawTitle0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_image",
        cs_method: "VisitImage",
        doc: "Called for images. title is null when absent.",
        extra: &[
            ExtraParam {
                cs_name: "src",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawSrc0)!",
            },
            ExtraParam {
                cs_name: "alt",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawAlt0)!",
            },
            ExtraParam {
                cs_name: "title",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawTitle0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawTitle0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_heading",
        cs_method: "VisitHeading",
        doc: "Called for heading elements h1-h6. id is null when absent.",
        extra: &[
            ExtraParam {
                cs_name: "level",
                cs_type: "uint",
                pinvoke_types: &["uint"],
                decode: "rawLevel0",
            },
            ExtraParam {
                cs_name: "text",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawText0)!",
            },
            ExtraParam {
                cs_name: "id",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawId0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawId0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_code_block",
        cs_method: "VisitCodeBlock",
        doc: "Called for code blocks. lang is null when absent.",
        extra: &[
            ExtraParam {
                cs_name: "lang",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawLang0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawLang0)",
            },
            ExtraParam {
                cs_name: "code",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawCode0)!",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_code_inline",
        cs_method: "VisitCodeInline",
        doc: "Called for inline code elements.",
        extra: &[ExtraParam {
            cs_name: "code",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawCode0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_item",
        cs_method: "VisitListItem",
        doc: "Called for list items.",
        extra: &[
            ExtraParam {
                cs_name: "ordered",
                cs_type: "bool",
                pinvoke_types: &["int"],
                decode: "rawOrdered0 != 0",
            },
            ExtraParam {
                cs_name: "marker",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawMarker0)!",
            },
            ExtraParam {
                cs_name: "text",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawText0)!",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_start",
        cs_method: "VisitListStart",
        doc: "Called before processing a list.",
        extra: &[ExtraParam {
            cs_name: "ordered",
            cs_type: "bool",
            pinvoke_types: &["int"],
            decode: "rawOrdered0 != 0",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_end",
        cs_method: "VisitListEnd",
        doc: "Called after processing a list.",
        extra: &[
            ExtraParam {
                cs_name: "ordered",
                cs_type: "bool",
                pinvoke_types: &["int"],
                decode: "rawOrdered0 != 0",
            },
            ExtraParam {
                cs_name: "output",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawOutput0)!",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_table_start",
        cs_method: "VisitTableStart",
        doc: "Called before processing a table.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_table_row",
        cs_method: "VisitTableRow",
        doc: "Called for table rows. cells contains the cell text values.",
        extra: &[ExtraParam {
            cs_name: "cells",
            cs_type: "string[]",
            pinvoke_types: &["IntPtr", "UIntPtr"],
            decode: "DecodeCells(rawCells0, (long)(ulong)rawCells1)",
        }],
        has_is_header: true,
    },
    CallbackSpec {
        c_field: "visit_table_end",
        cs_method: "VisitTableEnd",
        doc: "Called after processing a table.",
        extra: &[ExtraParam {
            cs_name: "output",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawOutput0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_blockquote",
        cs_method: "VisitBlockquote",
        doc: "Called for blockquote elements.",
        extra: &[
            ExtraParam {
                cs_name: "content",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawContent0)!",
            },
            ExtraParam {
                cs_name: "depth",
                cs_type: "ulong",
                pinvoke_types: &["UIntPtr"],
                decode: "(ulong)rawDepth0",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_strong",
        cs_method: "VisitStrong",
        doc: "Called for strong/bold elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_emphasis",
        cs_method: "VisitEmphasis",
        doc: "Called for emphasis/italic elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_strikethrough",
        cs_method: "VisitStrikethrough",
        doc: "Called for strikethrough elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_underline",
        cs_method: "VisitUnderline",
        doc: "Called for underline elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_subscript",
        cs_method: "VisitSubscript",
        doc: "Called for subscript elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_superscript",
        cs_method: "VisitSuperscript",
        doc: "Called for superscript elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_mark",
        cs_method: "VisitMark",
        doc: "Called for mark/highlight elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_line_break",
        cs_method: "VisitLineBreak",
        doc: "Called for line break elements.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_horizontal_rule",
        cs_method: "VisitHorizontalRule",
        doc: "Called for horizontal rule elements.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_custom_element",
        cs_method: "VisitCustomElement",
        doc: "Called for custom or unknown elements.",
        extra: &[
            ExtraParam {
                cs_name: "tagName",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawTagName0)!",
            },
            ExtraParam {
                cs_name: "html",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawHtml0)!",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_list_start",
        cs_method: "VisitDefinitionListStart",
        doc: "Called before a definition list.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_term",
        cs_method: "VisitDefinitionTerm",
        doc: "Called for definition term elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_description",
        cs_method: "VisitDefinitionDescription",
        doc: "Called for definition description elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_list_end",
        cs_method: "VisitDefinitionListEnd",
        doc: "Called after a definition list.",
        extra: &[ExtraParam {
            cs_name: "output",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawOutput0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_form",
        cs_method: "VisitForm",
        doc: "Called for form elements. action and method may be null.",
        extra: &[
            ExtraParam {
                cs_name: "action",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawAction0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawAction0)",
            },
            ExtraParam {
                cs_name: "method",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawMethod0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawMethod0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_input",
        cs_method: "VisitInput",
        doc: "Called for input elements. name and value may be null.",
        extra: &[
            ExtraParam {
                cs_name: "inputType",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringUTF8(rawInputType0)!",
            },
            ExtraParam {
                cs_name: "name",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawName0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawName0)",
            },
            ExtraParam {
                cs_name: "value",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawValue0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawValue0)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_button",
        cs_method: "VisitButton",
        doc: "Called for button elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_audio",
        cs_method: "VisitAudio",
        doc: "Called for audio elements. src may be null.",
        extra: &[ExtraParam {
            cs_name: "src",
            cs_type: "string?",
            pinvoke_types: &["IntPtr"],
            decode: "rawSrc0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawSrc0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_video",
        cs_method: "VisitVideo",
        doc: "Called for video elements. src may be null.",
        extra: &[ExtraParam {
            cs_name: "src",
            cs_type: "string?",
            pinvoke_types: &["IntPtr"],
            decode: "rawSrc0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawSrc0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_iframe",
        cs_method: "VisitIframe",
        doc: "Called for iframe elements. src may be null.",
        extra: &[ExtraParam {
            cs_name: "src",
            cs_type: "string?",
            pinvoke_types: &["IntPtr"],
            decode: "rawSrc0 == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(rawSrc0)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_details",
        cs_method: "VisitDetails",
        doc: "Called for details elements.",
        extra: &[ExtraParam {
            cs_name: "open",
            cs_type: "bool",
            pinvoke_types: &["int"],
            decode: "rawOpen0 != 0",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_summary",
        cs_method: "VisitSummary",
        doc: "Called for summary elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figure_start",
        cs_method: "VisitFigureStart",
        doc: "Called before a figure element.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figcaption",
        cs_method: "VisitFigcaption",
        doc: "Called for figcaption elements.",
        extra: &[ExtraParam {
            cs_name: "text",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawText0)!",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figure_end",
        cs_method: "VisitFigureEnd",
        doc: "Called after a figure element.",
        extra: &[ExtraParam {
            cs_name: "output",
            cs_type: "string",
            pinvoke_types: &["IntPtr"],
            decode: "Marshal.PtrToStringUTF8(rawOutput0)!",
        }],
        has_is_header: false,
    },
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns `(filename, content)` pairs for all visitor-related C# files.
///
/// IVisitor.cs and VisitorCallbacks.cs are superseded by IVisitor and VisitorCallbacks
/// in TraitBridges.cs which use the HtmlVisitorBridge approach. They are intentionally
/// excluded here; stale committed copies are removed by delete_superseded_visitor_files.
pub fn gen_visitor_files(namespace: &str) -> Vec<(String, String)> {
    vec![
        ("NodeContext.cs".to_string(), gen_node_context(namespace)),
        ("VisitResult.cs".to_string(), gen_visit_result(namespace)),
    ]
}

/// Generate the P/Invoke declarations needed in NativeMethods.cs for visitor FFI.
///
/// Parameters:
/// - `namespace`: C# namespace (unused, kept for compatibility)
/// - `lib_name`: Native library name (unused, kept for compatibility)
/// - `prefix`: C FFI function name prefix (e.g., "htm")
/// - `trait_name`: Name of the visitor trait (e.g., "HtmlVisitor") for bridge function names
/// - `options_field`: Field name in options to set visitor on (e.g., "visitor")
pub fn gen_native_methods_visitor(
    namespace: &str,
    lib_name: &str,
    prefix: &str,
    trait_name: &str,
    options_field: &str,
) -> String {
    let mut out = String::with_capacity(512);
    writeln!(out).ok();
    writeln!(out, "    // Visitor FFI (HtmlVisitorBridge)").ok();

    // Generate function names:
    // htm_htm_html_visitor_bridge_new, htm_htm_html_visitor_bridge_free, htm_options_set_visitor
    let trait_snake = trait_name.to_snake_case();
    let bridge_snake = format!("{prefix}_{trait_snake}_bridge");
    let fn_bridge_new = format!("{prefix}_{bridge_snake}_new");
    let fn_bridge_free = format!("{prefix}_{bridge_snake}_free");
    let fn_options_set = format!("{prefix}_options_set_{options_field}");

    writeln!(
        out,
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{fn_bridge_new}\")]"
    )
    .ok();
    writeln!(
        out,
        "    internal static extern IntPtr HtmlVisitorBridgeNew(IntPtr vtable, IntPtr userData);"
    )
    .ok();
    writeln!(out).ok();

    writeln!(
        out,
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{fn_bridge_free}\")]"
    )
    .ok();
    writeln!(out, "    internal static extern void HtmlVisitorBridgeFree(IntPtr bridge);").ok();
    writeln!(out).ok();

    writeln!(
        out,
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{fn_options_set}\")]"
    )
    .ok();
    writeln!(
        out,
        "    internal static extern int ConversionOptionsSetVisitor(IntPtr options, IntPtr visitor);"
    )
    .ok();

    let _ = namespace;
    let _ = lib_name;
    out
}

/// DEPRECATED: gen_convert_with_visitor_method is no longer used.
/// The visitor logic is now integrated into the main Convert() method in gen_wrapper_function,
/// which creates the HtmlVisitorBridge and uses htm_options_set_visitor instead.
#[allow(dead_code)]
pub fn gen_convert_with_visitor_method(exception_name: &str, prefix: &str) -> String {
    let _ = exception_name;
    let _ = prefix;
    String::new()
}

// ---------------------------------------------------------------------------
// Individual file generators
// ---------------------------------------------------------------------------

fn gen_node_context(namespace: &str) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "#nullable enable").ok();
    writeln!(out).ok();
    writeln!(out, "using System;").ok();
    writeln!(out).ok();
    writeln!(out, "namespace {namespace};").ok();
    writeln!(out).ok();
    writeln!(out, "/// <summary>Context passed to every visitor callback.</summary>").ok();
    writeln!(out, "public record NodeContext(").ok();
    writeln!(out, "    /// <summary>Coarse-grained node type tag.</summary>").ok();
    writeln!(out, "    NodeType NodeType,").ok();
    writeln!(out, "    /// <summary>HTML element tag name (e.g. \"div\").</summary>").ok();
    writeln!(out, "    string TagName,").ok();
    writeln!(out, "    /// <summary>DOM depth (0 = root).</summary>").ok();
    writeln!(out, "    ulong Depth,").ok();
    writeln!(out, "    /// <summary>0-based sibling index.</summary>").ok();
    writeln!(out, "    ulong IndexInParent,").ok();
    writeln!(
        out,
        "    /// <summary>Parent element tag name, or null at the root.</summary>"
    )
    .ok();
    writeln!(out, "    string? ParentTag,").ok();
    writeln!(
        out,
        "    /// <summary>True when this element is treated as inline.</summary>"
    )
    .ok();
    writeln!(out, "    bool IsInline").ok();
    writeln!(out, ");").ok();
    out
}

fn gen_visit_result(namespace: &str) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "#nullable enable").ok();
    writeln!(out).ok();
    writeln!(out, "using System;").ok();
    writeln!(out).ok();
    writeln!(out, "namespace {namespace};").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "/// <summary>Controls how the visitor affects the conversion pipeline.</summary>"
    )
    .ok();
    writeln!(out, "public abstract record VisitResult").ok();
    writeln!(out, "{{").ok();
    writeln!(out, "    private VisitResult() {{}}").ok();
    writeln!(out).ok();
    writeln!(out, "    /// <summary>Proceed with default conversion.</summary>").ok();
    writeln!(out, "    public sealed record Continue : VisitResult;").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "    /// <summary>Omit this element from output entirely.</summary>"
    )
    .ok();
    writeln!(out, "    public sealed record Skip : VisitResult;").ok();
    writeln!(out).ok();
    writeln!(out, "    /// <summary>Keep original HTML verbatim.</summary>").ok();
    writeln!(out, "    public sealed record PreserveHtml : VisitResult;").ok();
    writeln!(out).ok();
    writeln!(out, "    /// <summary>Replace with custom Markdown.</summary>").ok();
    writeln!(out, "    public sealed record Custom(string Markdown) : VisitResult;").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "    /// <summary>Abort conversion with an error message.</summary>"
    )
    .ok();
    writeln!(out, "    public sealed record Error(string Message) : VisitResult;").ok();
    writeln!(out).ok();
    writeln!(out, "    internal string ToFfiJson() => this switch {{").ok();
    writeln!(out, "        VisitResult.Continue => \"\\\"Continue\\\"\",").ok();
    writeln!(out, "        VisitResult.Skip => \"\\\"Skip\\\"\",").ok();
    writeln!(out, "        VisitResult.PreserveHtml => \"\\\"PreserveHtml\\\"\",").ok();
    writeln!(out, "        VisitResult.Custom c => \"{{\\\"Custom\\\":\" + System.Text.Json.JsonSerializer.Serialize(c.Markdown) + \"}}\",").ok();
    writeln!(out, "        VisitResult.Error e => \"{{\\\"Error\\\":\" + System.Text.Json.JsonSerializer.Serialize(e.Message) + \"}}\",").ok();
    writeln!(out, "        _ => \"\\\"Continue\\\"\"").ok();
    writeln!(out, "    }};").ok();
    writeln!(out, "}}").ok();
    out
}

// gen_ivisitor and gen_visitor_callbacks were removed: IVisitor and VisitorCallbacks
// are now handwritten in TraitBridges.cs (HtmlVisitorBridge pattern). Generating them
// here produced dead code that conflicted with the handwritten implementations.
