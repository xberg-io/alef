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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawOutput0)!",
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
                decode: "Marshal.PtrToStringAnsi(rawHref0)!",
            },
            ExtraParam {
                cs_name: "text",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringAnsi(rawText0)!",
            },
            ExtraParam {
                cs_name: "title",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawTitle0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawTitle0)",
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
                decode: "Marshal.PtrToStringAnsi(rawSrc0)!",
            },
            ExtraParam {
                cs_name: "alt",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringAnsi(rawAlt0)!",
            },
            ExtraParam {
                cs_name: "title",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawTitle0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawTitle0)",
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
                decode: "Marshal.PtrToStringAnsi(rawText0)!",
            },
            ExtraParam {
                cs_name: "id",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawId0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawId0)",
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
                decode: "rawLang0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawLang0)",
            },
            ExtraParam {
                cs_name: "code",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringAnsi(rawCode0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawCode0)!",
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
                decode: "Marshal.PtrToStringAnsi(rawMarker0)!",
            },
            ExtraParam {
                cs_name: "text",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
                decode: "Marshal.PtrToStringAnsi(rawOutput0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawOutput0)!",
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
                decode: "Marshal.PtrToStringAnsi(rawContent0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
                decode: "Marshal.PtrToStringAnsi(rawTagName0)!",
            },
            ExtraParam {
                cs_name: "html",
                cs_type: "string",
                pinvoke_types: &["IntPtr"],
                decode: "Marshal.PtrToStringAnsi(rawHtml0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawOutput0)!",
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
                decode: "rawAction0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawAction0)",
            },
            ExtraParam {
                cs_name: "method",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawMethod0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawMethod0)",
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
                decode: "Marshal.PtrToStringAnsi(rawInputType0)!",
            },
            ExtraParam {
                cs_name: "name",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawName0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawName0)",
            },
            ExtraParam {
                cs_name: "value",
                cs_type: "string?",
                pinvoke_types: &["IntPtr"],
                decode: "rawValue0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawValue0)",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "rawSrc0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawSrc0)",
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
            decode: "rawSrc0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawSrc0)",
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
            decode: "rawSrc0 == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(rawSrc0)",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawText0)!",
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
            decode: "Marshal.PtrToStringAnsi(rawOutput0)!",
        }],
        has_is_header: false,
    },
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns `(filename, content)` pairs for all visitor-related C# files.
pub fn gen_visitor_files(namespace: &str) -> Vec<(String, String)> {
    vec![
        ("NodeContext.cs".to_string(), gen_node_context(namespace)),
        ("VisitResult.cs".to_string(), gen_visit_result(namespace)),
        ("IVisitor.cs".to_string(), gen_ivisitor(namespace)),
        ("VisitorCallbacks.cs".to_string(), gen_visitor_callbacks(namespace)),
    ]
}

/// Generate the P/Invoke declarations needed in NativeMethods.cs for visitor FFI.
///
/// When `has_options_field_bridge` is `true` (i.e. `bind_via = "options_field"` is
/// configured), the three legacy symbols `{prefix}_visitor_create`,
/// `{prefix}_visitor_free`, and `{prefix}_convert_with_visitor` are NOT emitted — they
/// no longer exist in the FFI surface after the options-field refactor.  Only the
/// `{prefix}_visitor_handle_free` entry-point (emitted separately by
/// `gen_native_methods`) is needed in that mode.
pub fn gen_native_methods_visitor(
    namespace: &str,
    lib_name: &str,
    prefix: &str,
    has_options_field_bridge: bool,
) -> String {
    let _ = namespace;
    let _ = lib_name;

    if has_options_field_bridge {
        // In options-field mode the visitor handle is attached via
        // `{prefix}_options_set_visitor` (emitted by the options-field bridge loop).
        // The three legacy symbols are absent from the FFI surface.
        return String::new();
    }

    let mut out = String::with_capacity(512);
    writeln!(out).ok();
    writeln!(out, "    // Visitor FFI").ok();
    writeln!(
        out,
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{prefix}_visitor_create\")]"
    )
    .ok();
    writeln!(
        out,
        "    internal static extern IntPtr VisitorCreate(IntPtr callbacks);"
    )
    .ok();
    writeln!(out).ok();
    writeln!(
        out,
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{prefix}_visitor_free\")]"
    )
    .ok();
    writeln!(out, "    internal static extern void VisitorFree(IntPtr visitor);").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{prefix}_convert_with_visitor\")]"
    )
    .ok();
    writeln!(
        out,
        "    internal static extern IntPtr ConvertWithVisitor([MarshalAs(UnmanagedType.LPStr)] string html, IntPtr options, IntPtr visitor);"
    )
    .ok();
    out
}

/// Generate the `ConvertWithVisitor` method to inject into the wrapper class.
pub fn gen_convert_with_visitor_method(exception_name: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(2048);
    writeln!(out, "    /// <summary>").ok();
    writeln!(
        out,
        "    /// Convert HTML to Markdown, invoking visitor callbacks during processing."
    )
    .ok();
    writeln!(out, "    /// </summary>").ok();
    writeln!(
        out,
        "    public static ConversionResult? ConvertWithVisitor(string html, ConversionOptions? options, IVisitor visitor)"
    )
    .ok();
    writeln!(out, "    {{").ok();
    writeln!(out, "        ArgumentNullException.ThrowIfNull(html);").ok();
    writeln!(out, "        ArgumentNullException.ThrowIfNull(visitor);").ok();
    writeln!(out).ok();
    writeln!(out, "        using var callbacks = new VisitorCallbacks(visitor);").ok();
    writeln!(out).ok();
    writeln!(out, "        var optionsHandle = IntPtr.Zero;").ok();
    writeln!(out, "        if (options != null)").ok();
    writeln!(out, "        {{").ok();
    writeln!(
        out,
        "            var optionsJson = JsonSerializer.Serialize(options, JsonOptions);"
    )
    .ok();
    writeln!(
        out,
        "            optionsHandle = NativeMethods.ConversionOptionsFromJson(optionsJson);"
    )
    .ok();
    writeln!(out, "        }}").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "        var visitorHandle = NativeMethods.VisitorCreate(callbacks.NativePtr);"
    )
    .ok();
    writeln!(out, "        if (visitorHandle == IntPtr.Zero)").ok();
    writeln!(out, "        {{").ok();
    writeln!(
        out,
        "            if (optionsHandle != IntPtr.Zero) NativeMethods.ConversionOptionsFree(optionsHandle);"
    )
    .ok();
    writeln!(out, "            throw GetLastError();").ok();
    writeln!(out, "        }}").ok();
    writeln!(out).ok();
    writeln!(out, "        try").ok();
    writeln!(out, "        {{").ok();
    writeln!(
        out,
        "            var resultPtr = NativeMethods.ConvertWithVisitor(html, optionsHandle, visitorHandle);"
    )
    .ok();
    writeln!(
        out,
        "            if (optionsHandle != IntPtr.Zero) NativeMethods.ConversionOptionsFree(optionsHandle);"
    )
    .ok();
    writeln!(out, "            if (resultPtr == IntPtr.Zero)").ok();
    writeln!(out, "            {{").ok();
    writeln!(out, "                var err = GetLastError();").ok();
    writeln!(out, "                if (err.Code != 0) throw err;").ok();
    writeln!(out, "                return null;").ok();
    writeln!(out, "            }}").ok();
    writeln!(out, "            var json = Marshal.PtrToStringAnsi(resultPtr);").ok();
    writeln!(out, "            NativeMethods.FreeString(resultPtr);").ok();
    writeln!(
        out,
        "            return JsonSerializer.Deserialize<ConversionResult>(json!, JsonOptions);"
    )
    .ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        finally").ok();
    writeln!(out, "        {{").ok();
    writeln!(out, "            NativeMethods.VisitorFree(visitorHandle);").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    let _ = exception_name;
    let _ = prefix;
    out
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
    writeln!(out, "    int NodeType,").ok();
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
    writeln!(out, "}}").ok();
    out
}

fn gen_ivisitor(namespace: &str) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "#nullable enable").ok();
    writeln!(out).ok();
    writeln!(out, "using System;").ok();
    writeln!(out).ok();
    writeln!(out, "namespace {namespace};").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "/// <summary>Visitor interface for the HTML-to-Markdown conversion pipeline.</summary>"
    )
    .ok();
    writeln!(out, "public interface IVisitor").ok();
    writeln!(out, "{{").ok();
    for spec in CALLBACKS {
        let params = iface_param_str(spec);
        writeln!(out, "    /// <summary>{}</summary>", spec.doc).ok();
        writeln!(
            out,
            "    VisitResult {}({}) => new VisitResult.Continue();",
            spec.cs_method, params
        )
        .ok();
    }
    writeln!(out, "}}").ok();
    out
}

/// Generate `VisitorCallbacks.cs` which holds `GCHandle`s for all 40 delegates and
/// writes them into a `Marshal.AllocHGlobal` block matching the C struct layout.
fn gen_visitor_callbacks(namespace: &str) -> String {
    let mut out = String::with_capacity(32_768);
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "#nullable enable").ok();
    writeln!(out).ok();
    writeln!(out, "using System;").ok();
    writeln!(out, "using System.Runtime.InteropServices;").ok();
    writeln!(out).ok();
    writeln!(out, "namespace {namespace};").ok();
    writeln!(out).ok();
    writeln!(out, "/// <summary>").ok();
    writeln!(out, "/// Allocates P/Invoke delegates for a IVisitor and assembles").ok();
    writeln!(out, "/// the C HTMHtmVisitorCallbacks struct in unmanaged memory.").ok();
    writeln!(out, "/// </summary>").ok();
    writeln!(out, "internal sealed class VisitorCallbacks : IDisposable").ok();
    writeln!(out, "{{").ok();
    writeln!(out, "    private readonly IVisitor _visitor;").ok();
    writeln!(
        out,
        "    private readonly IntPtr _nativeStruct; // HTMHtmVisitorCallbacks"
    )
    .ok();
    writeln!(out, "    private bool _disposed;").ok();
    writeln!(out).ok();

    // Declare delegate types and delegate fields
    for spec in CALLBACKS {
        let delegate_type = delegate_type_name(spec.cs_method);
        let pinvoke_params = delegate_pinvoke_params(spec);
        writeln!(out, "    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]").ok();
        writeln!(
            out,
            "    private delegate int {}Delegate({});",
            delegate_type, pinvoke_params
        )
        .ok();
        writeln!(
            out,
            "    private readonly {}Delegate _del{};",
            delegate_type, spec.cs_method
        )
        .ok();
    }

    writeln!(out).ok();
    writeln!(out, "    internal IntPtr NativePtr => _nativeStruct;").ok();
    writeln!(out).ok();

    // Constructor
    let num_slots = CALLBACKS.len() + 1; // user_data + callbacks
    writeln!(out, "    internal VisitorCallbacks(IVisitor visitor)").ok();
    writeln!(out, "    {{").ok();
    writeln!(out, "        _visitor = visitor;").ok();
    writeln!(out).ok();

    // Create delegates
    for spec in CALLBACKS {
        let dt = delegate_type_name(spec.cs_method);
        writeln!(
            out,
            "        _del{} = new {}Delegate(Handle{});",
            spec.cs_method, dt, spec.cs_method
        )
        .ok();
    }

    // Allocate struct: user_data + 40 IntPtr slots = 41 * IntPtr.Size
    writeln!(out).ok();
    writeln!(
        out,
        "        // HTMHtmVisitorCallbacks = user_data + {n} callback function pointers",
        n = CALLBACKS.len()
    )
    .ok();
    writeln!(
        out,
        "        _nativeStruct = Marshal.AllocHGlobal(IntPtr.Size * {num_slots});"
    )
    .ok();
    writeln!(
        out,
        "        // Slot 0: user_data = IntPtr.Zero (visitor captured via delegate closure)"
    )
    .ok();
    writeln!(out, "        Marshal.WriteIntPtr(_nativeStruct, 0, IntPtr.Zero);").ok();

    for (i, spec) in CALLBACKS.iter().enumerate() {
        let offset = (i + 1) * 8; // assuming 8-byte pointers (64-bit)
        writeln!(
            out,
            "        Marshal.WriteIntPtr(_nativeStruct, {offset}, Marshal.GetFunctionPointerForDelegate(_del{}));",
            spec.cs_method
        )
        .ok();
    }

    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // Handle methods
    for spec in CALLBACKS {
        gen_handle_method(&mut out, spec);
    }

    // DecodeNodeContext helper
    writeln!(out, "    private static NodeContext DecodeNodeContext(IntPtr ctxPtr)").ok();
    writeln!(out, "    {{").ok();
    writeln!(
        out,
        "        // HTMHtmNodeContext: int32 node_type, char* tag_name, uintptr depth,"
    )
    .ok();
    writeln!(
        out,
        "        //                    uintptr index_in_parent, char* parent_tag, int32 is_inline"
    )
    .ok();
    writeln!(out, "        int nodeType = Marshal.ReadInt32(ctxPtr, 0);").ok();
    writeln!(out, "        var tagNamePtr = Marshal.ReadIntPtr(ctxPtr, 8);").ok();
    writeln!(
        out,
        "        string tagName = Marshal.PtrToStringAnsi(tagNamePtr) ?? string.Empty;"
    )
    .ok();
    writeln!(out, "        ulong depth = (ulong)(long)Marshal.ReadInt64(ctxPtr, 16);").ok();
    writeln!(
        out,
        "        ulong indexInParent = (ulong)(long)Marshal.ReadInt64(ctxPtr, 24);"
    )
    .ok();
    writeln!(out, "        var parentTagPtr = Marshal.ReadIntPtr(ctxPtr, 32);").ok();
    writeln!(
        out,
        "        string? parentTag = parentTagPtr == IntPtr.Zero ? null : Marshal.PtrToStringAnsi(parentTagPtr);"
    )
    .ok();
    writeln!(out, "        int isInlineRaw = Marshal.ReadInt32(ctxPtr, 40);").ok();
    writeln!(
        out,
        "        return new NodeContext(nodeType, tagName, depth, indexInParent, parentTag, isInlineRaw != 0);"
    )
    .ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // DecodeCells helper
    writeln!(
        out,
        "    private static string[] DecodeCells(IntPtr cellsPtr, long count)"
    )
    .ok();
    writeln!(out, "    {{").ok();
    writeln!(out, "        var result = new string[count];").ok();
    writeln!(out, "        for (long i = 0; i < count; i++)").ok();
    writeln!(out, "        {{").ok();
    writeln!(
        out,
        "            var ptr = Marshal.ReadIntPtr(cellsPtr, (int)(i * IntPtr.Size));"
    )
    .ok();
    writeln!(
        out,
        "            result[i] = Marshal.PtrToStringAnsi(ptr) ?? string.Empty;"
    )
    .ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        return result;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // EncodeVisitResult helper
    writeln!(
        out,
        "    private static int EncodeVisitResult(VisitResult result, IntPtr outCustom, IntPtr outLen)"
    )
    .ok();
    writeln!(out, "    {{").ok();
    writeln!(out, "        return result switch").ok();
    writeln!(out, "        {{").ok();
    writeln!(out, "            VisitResult.Continue => 0,").ok();
    writeln!(out, "            VisitResult.Skip => 1,").ok();
    writeln!(out, "            VisitResult.PreserveHtml => 2,").ok();
    writeln!(
        out,
        "            VisitResult.Custom c => EncodeString(c.Markdown, outCustom, outLen, 3),"
    )
    .ok();
    writeln!(
        out,
        "            VisitResult.Error e => EncodeString(e.Message, outCustom, outLen, 4),"
    )
    .ok();
    writeln!(out, "            _ => 0").ok();
    writeln!(out, "        }};").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    writeln!(
        out,
        "    private static int EncodeString(string text, IntPtr outCustom, IntPtr outLen, int code)"
    )
    .ok();
    writeln!(out, "    {{").ok();
    writeln!(out, "        var bytes = System.Text.Encoding.UTF8.GetBytes(text);").ok();
    writeln!(out, "        var buf = Marshal.AllocHGlobal(bytes.Length + 1);").ok();
    writeln!(out, "        Marshal.Copy(bytes, 0, buf, bytes.Length);").ok();
    writeln!(out, "        Marshal.WriteByte(buf, bytes.Length, 0);").ok();
    writeln!(out, "        Marshal.WriteIntPtr(outCustom, buf);").ok();
    writeln!(out, "        Marshal.WriteInt64(outLen, (long)bytes.Length);").ok();
    writeln!(out, "        return code;").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();

    // Dispose
    writeln!(out, "    public void Dispose()").ok();
    writeln!(out, "    {{").ok();
    writeln!(out, "        if (_disposed) return;").ok();
    writeln!(out, "        _disposed = true;").ok();
    writeln!(out, "        Marshal.FreeHGlobal(_nativeStruct);").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn delegate_type_name(cs_method: &str) -> String {
    cs_method.to_string()
}

fn iface_param_str(spec: &CallbackSpec) -> String {
    let mut params = vec!["NodeContext context".to_string()];
    for ep in spec.extra {
        params.push(format!("{} {}", ep.cs_type, ep.cs_name));
    }
    if spec.has_is_header {
        params.push("bool isHeader".to_string());
    }
    params.join(", ")
}

/// Build the P/Invoke delegate parameter list (all raw C types).
fn delegate_pinvoke_params(spec: &CallbackSpec) -> String {
    let mut params = vec!["IntPtr ctx".to_string(), "IntPtr userData".to_string()];
    for ep in spec.extra {
        for (idx, ptype) in ep.pinvoke_types.iter().enumerate() {
            params.push(format!("{ptype} {}", raw_var_name(ep.cs_name, idx)));
        }
    }
    if spec.has_is_header {
        params.push("int isHeader".to_string());
    }
    params.push("IntPtr outCustom".to_string());
    params.push("IntPtr outLen".to_string());
    params.join(", ")
}

/// Generate one `Handle*` method inside `VisitorCallbacks`.
fn gen_handle_method(out: &mut String, spec: &CallbackSpec) {
    let params = delegate_pinvoke_params(spec);
    writeln!(out, "    private int Handle{}({})", spec.cs_method, params).ok();
    writeln!(out, "    {{").ok();
    writeln!(out, "        try").ok();
    writeln!(out, "        {{").ok();
    writeln!(out, "            var context = DecodeNodeContext(ctx);").ok();

    // Decode extra params
    for ep in spec.extra {
        let mut decode = ep.decode.to_string();
        for (idx, _) in ep.pinvoke_types.iter().enumerate() {
            let placeholder = format!("raw{}{}", capitalize(ep.cs_name), idx);
            let var = raw_var_name(ep.cs_name, idx);
            decode = decode.replace(&placeholder, &var);
        }
        writeln!(out, "            var {} = {};", ep.cs_name, decode).ok();
    }
    if spec.has_is_header {
        writeln!(out, "            var goIsHeader = isHeader != 0;").ok();
    }

    let mut call_args = vec!["context".to_string()];
    for ep in spec.extra {
        call_args.push(ep.cs_name.to_string());
    }
    if spec.has_is_header {
        call_args.push("goIsHeader".to_string());
    }

    writeln!(
        out,
        "            var result = _visitor.{}({});",
        spec.cs_method,
        call_args.join(", ")
    )
    .ok();
    writeln!(out, "            return EncodeVisitResult(result, outCustom, outLen);").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        catch").ok();
    writeln!(out, "        {{").ok();
    writeln!(out, "            return 0;").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
}

fn raw_var_name(cs_name: &str, idx: usize) -> String {
    format!("raw{}{idx}", capitalize(cs_name))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
