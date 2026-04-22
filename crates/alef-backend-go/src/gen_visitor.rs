/// Generate Go visitor interface, CGo callback trampolines, and `ConvertWithVisitor`.
///
/// # CGo callback strategy
///
/// CGo does not allow passing Go function values as C function pointers. The generated
/// code uses a handle-table pattern:
///
/// - A package-level `sync.Map` keyed by `uintptr` stores active `Visitor` values.
/// - A counter provides unique IDs; the ID is cast to `unsafe.Pointer` and passed as
///   `user_data` to every C callback.
/// - Package-level `//export goVisit*` functions look up the visitor by ID and call
///   the appropriate method.
/// - A static C helper in the CGo preamble constructs `HTMHtmVisitorCallbacks` by
///   referencing all exported Go trampolines — this is valid because CGo compiles the
///   preamble together with the Go file that carries the `//export` declarations.
/// - `ConvertWithVisitor` registers the visitor, calls the C helper to build the
///   callbacks struct, then calls `htm_visitor_create` / `htm_convert_with_visitor` /
///   `htm_visitor_free`, and unregisters the visitor on return.
use std::fmt::Write;

/// A single visitor callback specification.
struct CallbackSpec {
    /// Field name in `HTMHtmVisitorCallbacks` (snake_case).
    c_field: &'static str,
    /// Exported Go function name (e.g. `goVisitText`).
    export_name: &'static str,
    /// Go interface method name (PascalCase).
    go_method: &'static str,
    /// Doc comment for the Go interface method.
    doc: &'static str,
    /// Extra C parameters after `(ctx, user_data)` and before `(out_custom, out_len)`.
    /// Each entry: (c_param_name, c_type, go_var_name, go_type_in_interface, decode_expr).
    /// `decode_expr` is the Go expression to convert the C parameter to the Go interface type.
    extra: &'static [ExtraParam],
    /// If true, add an `isHeader C.int32_t` parameter (only for visit_table_row).
    has_is_header: bool,
}

struct ExtraParam {
    c_name: &'static str,
    c_type: &'static str,
    go_name: &'static str,
    /// Type as it appears in the Go interface method signature.
    go_iface_type: &'static str,
    /// Expression to convert the C value to `go_iface_type`.
    decode: &'static str,
}

const CALLBACKS: &[CallbackSpec] = &[
    CallbackSpec {
        c_field: "visit_text",
        export_name: "goVisitText",
        go_method: "VisitText",
        doc: "VisitText is called for text nodes.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_element_start",
        export_name: "goVisitElementStart",
        go_method: "VisitElementStart",
        doc: "VisitElementStart is called before entering any element.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_element_end",
        export_name: "goVisitElementEnd",
        go_method: "VisitElementEnd",
        doc: "VisitElementEnd is called after exiting any element; receives the default markdown output.",
        extra: &[ExtraParam {
            c_name: "output",
            c_type: "*C.char",
            go_name: "output",
            go_iface_type: "string",
            decode: "C.GoString(output)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_link",
        export_name: "goVisitLink",
        go_method: "VisitLink",
        doc: "VisitLink visits anchor links. title is nil when the attribute is absent.",
        extra: &[
            ExtraParam {
                c_name: "href",
                c_type: "*C.char",
                go_name: "href",
                go_iface_type: "string",
                decode: "C.GoString(href)",
            },
            ExtraParam {
                c_name: "text",
                c_type: "*C.char",
                go_name: "text",
                go_iface_type: "string",
                decode: "C.GoString(text)",
            },
            ExtraParam {
                c_name: "title",
                c_type: "*C.char",
                go_name: "title",
                go_iface_type: "*string",
                decode: "optGoString(title)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_image",
        export_name: "goVisitImage",
        go_method: "VisitImage",
        doc: "VisitImage visits images. title is nil when absent.",
        extra: &[
            ExtraParam {
                c_name: "src",
                c_type: "*C.char",
                go_name: "src",
                go_iface_type: "string",
                decode: "C.GoString(src)",
            },
            ExtraParam {
                c_name: "alt",
                c_type: "*C.char",
                go_name: "alt",
                go_iface_type: "string",
                decode: "C.GoString(alt)",
            },
            ExtraParam {
                c_name: "title",
                c_type: "*C.char",
                go_name: "title",
                go_iface_type: "*string",
                decode: "optGoString(title)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_heading",
        export_name: "goVisitHeading",
        go_method: "VisitHeading",
        doc: "VisitHeading visits heading elements h1-h6. id is nil when absent.",
        extra: &[
            ExtraParam {
                c_name: "level",
                c_type: "C.uint32_t",
                go_name: "level",
                go_iface_type: "uint32",
                decode: "uint32(level)",
            },
            ExtraParam {
                c_name: "text",
                c_type: "*C.char",
                go_name: "text",
                go_iface_type: "string",
                decode: "C.GoString(text)",
            },
            ExtraParam {
                c_name: "id",
                c_type: "*C.char",
                go_name: "id",
                go_iface_type: "*string",
                decode: "optGoString(id)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_code_block",
        export_name: "goVisitCodeBlock",
        go_method: "VisitCodeBlock",
        doc: "VisitCodeBlock visits code blocks. lang is nil when absent.",
        extra: &[
            ExtraParam {
                c_name: "lang",
                c_type: "*C.char",
                go_name: "lang",
                go_iface_type: "*string",
                decode: "optGoString(lang)",
            },
            ExtraParam {
                c_name: "code",
                c_type: "*C.char",
                go_name: "code",
                go_iface_type: "string",
                decode: "C.GoString(code)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_code_inline",
        export_name: "goVisitCodeInline",
        go_method: "VisitCodeInline",
        doc: "VisitCodeInline visits inline code elements.",
        extra: &[ExtraParam {
            c_name: "code",
            c_type: "*C.char",
            go_name: "code",
            go_iface_type: "string",
            decode: "C.GoString(code)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_item",
        export_name: "goVisitListItem",
        go_method: "VisitListItem",
        doc: "VisitListItem visits list items.",
        extra: &[
            ExtraParam {
                c_name: "ordered",
                c_type: "C.int32_t",
                go_name: "ordered",
                go_iface_type: "bool",
                decode: "ordered != 0",
            },
            ExtraParam {
                c_name: "marker",
                c_type: "*C.char",
                go_name: "marker",
                go_iface_type: "string",
                decode: "C.GoString(marker)",
            },
            ExtraParam {
                c_name: "text",
                c_type: "*C.char",
                go_name: "text",
                go_iface_type: "string",
                decode: "C.GoString(text)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_start",
        export_name: "goVisitListStart",
        go_method: "VisitListStart",
        doc: "VisitListStart is called before processing a list.",
        extra: &[ExtraParam {
            c_name: "ordered",
            c_type: "C.int32_t",
            go_name: "ordered",
            go_iface_type: "bool",
            decode: "ordered != 0",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_list_end",
        export_name: "goVisitListEnd",
        go_method: "VisitListEnd",
        doc: "VisitListEnd is called after processing a list.",
        extra: &[
            ExtraParam {
                c_name: "ordered",
                c_type: "C.int32_t",
                go_name: "ordered",
                go_iface_type: "bool",
                decode: "ordered != 0",
            },
            ExtraParam {
                c_name: "output",
                c_type: "*C.char",
                go_name: "output",
                go_iface_type: "string",
                decode: "C.GoString(output)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_table_start",
        export_name: "goVisitTableStart",
        go_method: "VisitTableStart",
        doc: "VisitTableStart is called before processing a table.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_table_row",
        export_name: "goVisitTableRow",
        go_method: "VisitTableRow",
        doc: "VisitTableRow visits table rows. Cells are passed as a slice of strings.",
        extra: &[ExtraParam {
            c_name: "cells",
            c_type: "**C.char",
            go_name: "cells",
            go_iface_type: "[]string",
            decode: "decodeCells(cells, cellCount)",
        }],
        has_is_header: true,
    },
    CallbackSpec {
        c_field: "visit_table_end",
        export_name: "goVisitTableEnd",
        go_method: "VisitTableEnd",
        doc: "VisitTableEnd is called after processing a table.",
        extra: &[ExtraParam {
            c_name: "output",
            c_type: "*C.char",
            go_name: "output",
            go_iface_type: "string",
            decode: "C.GoString(output)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_blockquote",
        export_name: "goVisitBlockquote",
        go_method: "VisitBlockquote",
        doc: "VisitBlockquote visits blockquote elements.",
        extra: &[
            ExtraParam {
                c_name: "content",
                c_type: "*C.char",
                go_name: "content",
                go_iface_type: "string",
                decode: "C.GoString(content)",
            },
            ExtraParam {
                c_name: "depth",
                c_type: "C.size_t",
                go_name: "depth",
                go_iface_type: "uint",
                decode: "uint(depth)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_strong",
        export_name: "goVisitStrong",
        go_method: "VisitStrong",
        doc: "VisitStrong visits strong/bold elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_emphasis",
        export_name: "goVisitEmphasis",
        go_method: "VisitEmphasis",
        doc: "VisitEmphasis visits emphasis/italic elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_strikethrough",
        export_name: "goVisitStrikethrough",
        go_method: "VisitStrikethrough",
        doc: "VisitStrikethrough visits strikethrough elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_underline",
        export_name: "goVisitUnderline",
        go_method: "VisitUnderline",
        doc: "VisitUnderline visits underline elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_subscript",
        export_name: "goVisitSubscript",
        go_method: "VisitSubscript",
        doc: "VisitSubscript visits subscript elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_superscript",
        export_name: "goVisitSuperscript",
        go_method: "VisitSuperscript",
        doc: "VisitSuperscript visits superscript elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_mark",
        export_name: "goVisitMark",
        go_method: "VisitMark",
        doc: "VisitMark visits mark/highlight elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_line_break",
        export_name: "goVisitLineBreak",
        go_method: "VisitLineBreak",
        doc: "VisitLineBreak visits line break elements.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_horizontal_rule",
        export_name: "goVisitHorizontalRule",
        go_method: "VisitHorizontalRule",
        doc: "VisitHorizontalRule visits horizontal rule elements.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_custom_element",
        export_name: "goVisitCustomElement",
        go_method: "VisitCustomElement",
        doc: "VisitCustomElement visits custom/unknown elements.",
        extra: &[
            ExtraParam {
                c_name: "tagName",
                c_type: "*C.char",
                go_name: "tagName",
                go_iface_type: "string",
                decode: "C.GoString(tagName)",
            },
            ExtraParam {
                c_name: "html",
                c_type: "*C.char",
                go_name: "html",
                go_iface_type: "string",
                decode: "C.GoString(html)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_list_start",
        export_name: "goVisitDefinitionListStart",
        go_method: "VisitDefinitionListStart",
        doc: "VisitDefinitionListStart visits definition list elements.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_term",
        export_name: "goVisitDefinitionTerm",
        go_method: "VisitDefinitionTerm",
        doc: "VisitDefinitionTerm visits definition term elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_description",
        export_name: "goVisitDefinitionDescription",
        go_method: "VisitDefinitionDescription",
        doc: "VisitDefinitionDescription visits definition description elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_definition_list_end",
        export_name: "goVisitDefinitionListEnd",
        go_method: "VisitDefinitionListEnd",
        doc: "VisitDefinitionListEnd is called after processing a definition list.",
        extra: &[ExtraParam {
            c_name: "output",
            c_type: "*C.char",
            go_name: "output",
            go_iface_type: "string",
            decode: "C.GoString(output)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_form",
        export_name: "goVisitForm",
        go_method: "VisitForm",
        doc: "VisitForm visits form elements. action and method may be nil.",
        extra: &[
            ExtraParam {
                c_name: "action",
                c_type: "*C.char",
                go_name: "action",
                go_iface_type: "*string",
                decode: "optGoString(action)",
            },
            ExtraParam {
                c_name: "method",
                c_type: "*C.char",
                go_name: "method",
                go_iface_type: "*string",
                decode: "optGoString(method)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_input",
        export_name: "goVisitInput",
        go_method: "VisitInput",
        doc: "VisitInput visits input elements. name and value may be nil.",
        extra: &[
            ExtraParam {
                c_name: "inputType",
                c_type: "*C.char",
                go_name: "inputType",
                go_iface_type: "string",
                decode: "C.GoString(inputType)",
            },
            ExtraParam {
                c_name: "name",
                c_type: "*C.char",
                go_name: "name",
                go_iface_type: "*string",
                decode: "optGoString(name)",
            },
            ExtraParam {
                c_name: "value",
                c_type: "*C.char",
                go_name: "value",
                go_iface_type: "*string",
                decode: "optGoString(value)",
            },
        ],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_button",
        export_name: "goVisitButton",
        go_method: "VisitButton",
        doc: "VisitButton visits button elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_audio",
        export_name: "goVisitAudio",
        go_method: "VisitAudio",
        doc: "VisitAudio visits audio elements. src may be nil.",
        extra: &[ExtraParam {
            c_name: "src",
            c_type: "*C.char",
            go_name: "src",
            go_iface_type: "*string",
            decode: "optGoString(src)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_video",
        export_name: "goVisitVideo",
        go_method: "VisitVideo",
        doc: "VisitVideo visits video elements. src may be nil.",
        extra: &[ExtraParam {
            c_name: "src",
            c_type: "*C.char",
            go_name: "src",
            go_iface_type: "*string",
            decode: "optGoString(src)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_iframe",
        export_name: "goVisitIframe",
        go_method: "VisitIframe",
        doc: "VisitIframe visits iframe elements. src may be nil.",
        extra: &[ExtraParam {
            c_name: "src",
            c_type: "*C.char",
            go_name: "src",
            go_iface_type: "*string",
            decode: "optGoString(src)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_details",
        export_name: "goVisitDetails",
        go_method: "VisitDetails",
        doc: "VisitDetails visits details elements.",
        extra: &[ExtraParam {
            c_name: "open",
            c_type: "C.int32_t",
            go_name: "open",
            go_iface_type: "bool",
            decode: "open != 0",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_summary",
        export_name: "goVisitSummary",
        go_method: "VisitSummary",
        doc: "VisitSummary visits summary elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figure_start",
        export_name: "goVisitFigureStart",
        go_method: "VisitFigureStart",
        doc: "VisitFigureStart is called before processing a figure element.",
        extra: &[],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figcaption",
        export_name: "goVisitFigcaption",
        go_method: "VisitFigcaption",
        doc: "VisitFigcaption visits figcaption elements.",
        extra: &[ExtraParam {
            c_name: "text",
            c_type: "*C.char",
            go_name: "text",
            go_iface_type: "string",
            decode: "C.GoString(text)",
        }],
        has_is_header: false,
    },
    CallbackSpec {
        c_field: "visit_figure_end",
        export_name: "goVisitFigureEnd",
        go_method: "VisitFigureEnd",
        doc: "VisitFigureEnd is called after processing a figure element.",
        extra: &[ExtraParam {
            c_name: "output",
            c_type: "*C.char",
            go_name: "output",
            go_iface_type: "string",
            decode: "C.GoString(output)",
        }],
        has_is_header: false,
    },
];

/// Generate the complete visitor.go file content.
///
/// `pkg_name`: Go package name (e.g. `"htmltomarkdown"`).
/// `ffi_prefix`: C function prefix (e.g. `"htm"`).
/// `ffi_header`: C header filename (e.g. `"html_to_markdown.h"`).
/// `ffi_crate_dir`: path from go output dir to the FFI crate dir.
/// `to_root`: relative path from go output dir to the repo root.
pub fn gen_visitor_file(
    pkg_name: &str,
    ffi_prefix: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    to_root: &str,
) -> String {
    let mut out = String::with_capacity(32_768);

    writeln!(out, "// Code generated by alef. DO NOT EDIT.").ok();
    writeln!(out).ok();
    writeln!(out, "package {pkg_name}").ok();
    writeln!(out).ok();

    // -------------------------------------------------------------------------
    // CGo preamble
    // -------------------------------------------------------------------------
    writeln!(out, "/*").ok();
    writeln!(out, "#cgo CFLAGS: -I${{SRCDIR}}/{to_root}{ffi_crate_dir}/include").ok();
    writeln!(out, "#include \"{ffi_header}\"").ok();
    writeln!(out, "#include <stdlib.h>").ok();
    writeln!(out, "#include <string.h>").ok();
    writeln!(out).ok();

    // Forward-declare all exported Go trampolines so the static helper below can
    // reference them.  CGo will resolve these at link time.
    for spec in CALLBACKS {
        let c_sig = c_signature(spec, ffi_prefix);
        writeln!(out, "extern int32_t {}({});", spec.export_name, c_sig).ok();
    }

    writeln!(out).ok();

    // Static C helper that constructs HTMHtmVisitorCallbacks with all Go trampolines.
    writeln!(
        out,
        "static HTMHtmVisitorCallbacks makeVisitorCallbacks(void* user_data) {{"
    )
    .ok();
    writeln!(out, "    HTMHtmVisitorCallbacks cbs;").ok();
    writeln!(out, "    memset(&cbs, 0, sizeof(cbs));").ok();
    writeln!(out, "    cbs.user_data = user_data;").ok();
    for spec in CALLBACKS {
        writeln!(out, "    cbs.{} = {};", spec.c_field, spec.export_name).ok();
    }
    writeln!(out, "    return cbs;").ok();
    writeln!(out, "}}").ok();

    writeln!(out, "*/").ok();
    writeln!(out, "import \"C\"").ok();
    writeln!(out).ok();

    writeln!(out, "import (").ok();
    writeln!(out, "\t\"encoding/json\"").ok();
    writeln!(out, "\t\"fmt\"").ok();
    writeln!(out, "\t\"sync\"").ok();
    writeln!(out, "\t\"sync/atomic\"").ok();
    writeln!(out, "\t\"unsafe\"").ok();
    writeln!(out, ")").ok();
    writeln!(out).ok();

    // -------------------------------------------------------------------------
    // NodeContext
    // -------------------------------------------------------------------------
    writeln!(
        out,
        "// NodeContext carries context information passed to every visitor callback."
    )
    .ok();
    writeln!(out, "type NodeContext struct {{").ok();
    writeln!(out, "\t// NodeType is a coarse-grained node type tag.").ok();
    writeln!(out, "\tNodeType int32").ok();
    writeln!(out, "\t// TagName is the HTML element tag name (e.g. \"div\").").ok();
    writeln!(out, "\tTagName string").ok();
    writeln!(out, "\t// Depth is the DOM depth (0 = root).").ok();
    writeln!(out, "\tDepth uint").ok();
    writeln!(out, "\t// IndexInParent is the 0-based sibling index.").ok();
    writeln!(out, "\tIndexInParent uint").ok();
    writeln!(
        out,
        "\t// ParentTag is the parent element tag name, or nil at the root."
    )
    .ok();
    writeln!(out, "\tParentTag *string").ok();
    writeln!(out, "\t// IsInline is true when this element is treated as inline.").ok();
    writeln!(out, "\tIsInline bool").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // -------------------------------------------------------------------------
    // VisitResult
    // -------------------------------------------------------------------------
    writeln!(out, "// VisitResult controls conversion flow from a visitor callback.").ok();
    writeln!(out, "type VisitResult struct {{").ok();
    writeln!(
        out,
        "\t// Code is the numeric visit-result code (0=Continue, 1=Skip, 2=PreserveHtml, 3=Custom, 4=Error)."
    )
    .ok();
    writeln!(out, "\tCode int32").ok();
    writeln!(out, "\t// Custom is non-nil only for Custom (3) and Error (4) codes.").ok();
    writeln!(out, "\tCustom *string").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "// VisitResultContinue returns a Continue VisitResult (proceed with default conversion)."
    )
    .ok();
    writeln!(
        out,
        "func VisitResultContinue() VisitResult {{ return VisitResult{{Code: 0}} }}"
    )
    .ok();
    writeln!(out).ok();
    writeln!(
        out,
        "// VisitResultSkip returns a Skip VisitResult (omit this element from output)."
    )
    .ok();
    writeln!(
        out,
        "func VisitResultSkip() VisitResult {{ return VisitResult{{Code: 1}} }}"
    )
    .ok();
    writeln!(out).ok();
    writeln!(
        out,
        "// VisitResultPreserveHtml returns a PreserveHtml VisitResult (keep original HTML verbatim)."
    )
    .ok();
    writeln!(
        out,
        "func VisitResultPreserveHtml() VisitResult {{ return VisitResult{{Code: 2}} }}"
    )
    .ok();
    writeln!(out).ok();
    writeln!(
        out,
        "// VisitResultCustom returns a Custom VisitResult with the given Markdown replacement."
    )
    .ok();
    writeln!(out, "func VisitResultCustom(markdown string) VisitResult {{").ok();
    writeln!(out, "\treturn VisitResult{{Code: 3, Custom: &markdown}}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "// VisitResultError returns an Error VisitResult that aborts conversion."
    )
    .ok();
    writeln!(out, "func VisitResultError(msg string) VisitResult {{").ok();
    writeln!(out, "\treturn VisitResult{{Code: 4, Custom: &msg}}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // -------------------------------------------------------------------------
    // Visitor interface
    // -------------------------------------------------------------------------
    writeln!(
        out,
        "// Visitor is the interface implemented by types that observe the HTML-to-Markdown"
    )
    .ok();
    writeln!(
        out,
        "// conversion pipeline.  Embed BaseVisitor to get no-op defaults for all methods."
    )
    .ok();
    writeln!(out, "type Visitor interface {{").ok();
    for spec in CALLBACKS {
        let param_str = iface_param_str(spec);
        writeln!(out, "\t// {}", spec.doc).ok();
        writeln!(out, "\t{}({param_str}) VisitResult", spec.go_method).ok();
    }
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // -------------------------------------------------------------------------
    // BaseVisitor — no-op defaults
    // -------------------------------------------------------------------------
    writeln!(
        out,
        "// BaseVisitor provides default no-op implementations for all Visitor methods."
    )
    .ok();
    writeln!(
        out,
        "// Embed it in your struct and override only the methods you need."
    )
    .ok();
    writeln!(out, "type BaseVisitor struct{{}}").ok();
    writeln!(out).ok();
    for spec in CALLBACKS {
        let param_str = iface_param_str(spec);
        // Build blank identifiers to suppress "declared but not used" errors.
        let blank_ids: Vec<String> = iface_param_names(spec)
            .into_iter()
            .map(|n| format!("\t_ = {n}"))
            .collect();
        writeln!(out, "func (BaseVisitor) {}({param_str}) VisitResult {{", spec.go_method).ok();
        for b in &blank_ids {
            writeln!(out, "{b}").ok();
        }
        writeln!(out, "\treturn VisitResultContinue()").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
    }

    // -------------------------------------------------------------------------
    // Visitor registry
    // -------------------------------------------------------------------------
    writeln!(
        out,
        "// visitorRegistry maps visitor handle IDs to active Visitor instances."
    )
    .ok();
    writeln!(
        out,
        "// CGo does not allow passing Go function values as C function pointers;"
    )
    .ok();
    writeln!(
        out,
        "// we use a numeric ID (stored in user_data) to look up the Visitor at callback time."
    )
    .ok();
    writeln!(out, "var (").ok();
    writeln!(out, "\tvisitorRegistry sync.Map").ok();
    writeln!(out, "\tvisitorIDCounter atomic.Uint64").ok();
    writeln!(out, ")").ok();
    writeln!(out).ok();
    writeln!(out, "func registerVisitor(v Visitor) uintptr {{").ok();
    writeln!(out, "\tid := uintptr(visitorIDCounter.Add(1))").ok();
    writeln!(out, "\tvisitorRegistry.Store(id, v)").ok();
    writeln!(out, "\treturn id").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(out, "func unregisterVisitor(id uintptr) {{").ok();
    writeln!(out, "\tvisitorRegistry.Delete(id)").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(out, "func lookupVisitor(id uintptr) (Visitor, bool) {{").ok();
    writeln!(out, "\tv, ok := visitorRegistry.Load(id)").ok();
    writeln!(out, "\tif !ok {{").ok();
    writeln!(out, "\t\treturn nil, false").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn v.(Visitor), true").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // -------------------------------------------------------------------------
    // Shared helpers
    // -------------------------------------------------------------------------
    writeln!(out, "func decodeNodeContext(c *C.HTMHtmNodeContext) NodeContext {{").ok();
    writeln!(out, "\tctx := NodeContext{{").ok();
    writeln!(out, "\t\tNodeType:      int32(c.node_type),").ok();
    writeln!(out, "\t\tTagName:       C.GoString(c.tag_name),").ok();
    writeln!(out, "\t\tDepth:         uint(c.depth),").ok();
    writeln!(out, "\t\tIndexInParent: uint(c.index_in_parent),").ok();
    writeln!(out, "\t\tIsInline:      c.is_inline != 0,").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tif c.parent_tag != nil {{").ok();
    writeln!(out, "\t\ts := C.GoString(c.parent_tag)").ok();
    writeln!(out, "\t\tctx.ParentTag = &s").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn ctx").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "func encodeVisitResult(r VisitResult, outCustom **C.char, outLen *C.size_t) C.int32_t {{"
    )
    .ok();
    writeln!(out, "\tif (r.Code == 3 || r.Code == 4) && r.Custom != nil {{").ok();
    writeln!(out, "\t\tcs := C.CString(*r.Custom)").ok();
    writeln!(out, "\t\t*outCustom = cs").ok();
    writeln!(out, "\t\t*outLen = C.size_t(len(*r.Custom))").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn C.int32_t(r.Code)").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(out, "func optGoString(p *C.char) *string {{").ok();
    writeln!(out, "\tif p == nil {{").ok();
    writeln!(out, "\t\treturn nil").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\ts := C.GoString(p)").ok();
    writeln!(out, "\treturn &s").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    writeln!(out, "func decodeCells(cells **C.char, count C.size_t) []string {{").ok();
    writeln!(out, "\tn := int(count)").ok();
    writeln!(out, "\tif n == 0 {{").ok();
    writeln!(out, "\t\treturn nil").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tptrs := (*[1 << 28]*C.char)(unsafe.Pointer(cells))[:n:n]").ok();
    writeln!(out, "\tsl := make([]string, n)").ok();
    writeln!(out, "\tfor i, p := range ptrs {{").ok();
    writeln!(out, "\t\tsl[i] = C.GoString(p)").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn sl").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // -------------------------------------------------------------------------
    // //export trampolines
    // -------------------------------------------------------------------------
    for spec in CALLBACKS {
        gen_trampoline(&mut out, spec);
    }

    // -------------------------------------------------------------------------
    // ConvertWithVisitor
    // -------------------------------------------------------------------------
    gen_convert_with_visitor(&mut out, ffi_prefix);

    out
}

/// Build the C parameter list string for the extern declaration of an exported Go function.
fn c_signature(spec: &CallbackSpec, _ffi_prefix: &str) -> String {
    // CGO's //export generates non-const parameter types in the prolog header,
    // so extern declarations must match — no const qualifiers.
    let mut parts = vec!["HTMHtmNodeContext* ctx".to_string(), "void* user_data".to_string()];
    for ep in spec.extra {
        let ctype = match ep.c_type {
            "*C.char" => "char*",
            "C.int32_t" => "int32_t",
            "C.uint32_t" => "uint32_t",
            "C.size_t" => "size_t",
            "**C.char" => "char**",
            _ => "void*",
        };
        parts.push(format!("{ctype} {}", ep.c_name));
    }
    if spec.has_is_header {
        parts.push("int32_t isHeader".to_string());
    }
    parts.push("char** out_custom".to_string());
    parts.push("size_t* out_len".to_string());
    parts.join(", ")
}

/// Build the Go interface method parameter string.
fn iface_param_str(spec: &CallbackSpec) -> String {
    let mut params = vec!["ctx NodeContext".to_string()];
    for ep in spec.extra {
        params.push(format!("{} {}", ep.go_name, ep.go_iface_type));
    }
    if spec.has_is_header {
        params.push("isHeader bool".to_string());
    }
    params.join(", ")
}

/// Return just the parameter names for a Go interface method.
fn iface_param_names(spec: &CallbackSpec) -> Vec<String> {
    let mut names = vec!["ctx".to_string()];
    for ep in spec.extra {
        names.push(ep.go_name.to_string());
    }
    if spec.has_is_header {
        names.push("isHeader".to_string());
    }
    names
}

/// Generate one `//export goVisit*` C callback trampoline.
fn gen_trampoline(out: &mut String, spec: &CallbackSpec) {
    // Build Go function parameter list (CGo types).
    let mut go_params = vec![
        "ctx *C.HTMHtmNodeContext".to_string(),
        "userData unsafe.Pointer".to_string(),
    ];
    for ep in spec.extra {
        go_params.push(format!("{} {}", ep.c_name, ep.c_type));
    }
    if spec.has_is_header {
        go_params.push("isHeader C.int32_t".to_string());
    }
    go_params.push("outCustom **C.char".to_string());
    go_params.push("outLen *C.size_t".to_string());

    writeln!(out, "//export {}", spec.export_name).ok();
    writeln!(out, "func {}({}) C.int32_t {{", spec.export_name, go_params.join(", ")).ok();
    writeln!(out, "\tid := uintptr(uintptr(userData))").ok();
    writeln!(out, "\tv, ok := lookupVisitor(id)").ok();
    writeln!(out, "\tif !ok {{").ok();
    writeln!(out, "\t\treturn 0").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tnodeCtx := decodeNodeContext(ctx)").ok();

    // Decode each extra parameter.
    for ep in spec.extra {
        writeln!(out, "\tgo{} := {}", capitalize(ep.go_name), ep.decode).ok();
    }
    if spec.has_is_header {
        writeln!(out, "\tgoIsHeader := isHeader != 0").ok();
    }

    // Build call args.
    let mut call_args = vec!["nodeCtx".to_string()];
    for ep in spec.extra {
        call_args.push(format!("go{}", capitalize(ep.go_name)));
    }
    if spec.has_is_header {
        call_args.push("goIsHeader".to_string());
    }

    writeln!(out, "\tr := v.{}({})", spec.go_method, call_args.join(", ")).ok();
    writeln!(out, "\treturn encodeVisitResult(r, outCustom, outLen)").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
}

/// Capitalize the first character of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Generate the `ConvertWithVisitor` function.
fn gen_convert_with_visitor(out: &mut String, ffi_prefix: &str) {
    writeln!(
        out,
        "// ConvertWithVisitor converts HTML to Markdown, invoking visitor callbacks during"
    )
    .ok();
    writeln!(
        out,
        "// the conversion pipeline.  Pass nil for options to use defaults."
    )
    .ok();
    writeln!(
        out,
        "// Pass a struct embedding BaseVisitor and overriding only the methods you need."
    )
    .ok();
    writeln!(
        out,
        "func ConvertWithVisitor(html string, options *ConversionOptions, visitor Visitor) (*ConversionResult, error) {{"
    )
    .ok();
    writeln!(out, "\tcHtml := C.CString(html)").ok();
    writeln!(out, "\tdefer C.free(unsafe.Pointer(cHtml))").ok();
    writeln!(out).ok();
    writeln!(out, "\tvar cOptions *C.HTMConversionOptions").ok();
    writeln!(out, "\tif options != nil {{").ok();
    writeln!(
        out,
        "\t\tjsonBytes, err := json.Marshal(options)\n\t\tif err != nil {{\n\t\t\treturn nil, fmt.Errorf(\"failed to marshal conversion options: %w\", err)\n\t\t}}\n\t\ttmpStr := C.CString(string(jsonBytes))\n\t\tcOptions = C.{ffi_prefix}_conversion_options_from_json(tmpStr)\n\t\tC.free(unsafe.Pointer(tmpStr))\n\t\tdefer C.{ffi_prefix}_conversion_options_free(cOptions)"
    )
    .ok();
    writeln!(out, "\t}}").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "\t// Register visitor and build the C callback struct via the static C helper."
    )
    .ok();
    writeln!(out, "\tid := registerVisitor(visitor)").ok();
    writeln!(out, "\tdefer unregisterVisitor(id)").ok();
    writeln!(out, "\tcbs := C.makeVisitorCallbacks(unsafe.Pointer(id))").ok();
    writeln!(out).ok();
    writeln!(out, "\tvisitorHandle := C.{ffi_prefix}_visitor_create(&cbs)").ok();
    writeln!(out, "\tif visitorHandle == nil {{").ok();
    writeln!(out, "\t\treturn nil, fmt.Errorf(\"failed to create visitor handle\")").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tdefer C.{ffi_prefix}_visitor_free(visitorHandle)").ok();
    writeln!(out).ok();
    writeln!(
        out,
        "\tptr := C.{ffi_prefix}_convert_with_visitor(cHtml, cOptions, visitorHandle)"
    )
    .ok();
    writeln!(out, "\tif err := lastError(); err != nil {{").ok();
    writeln!(out, "\t\tif ptr != nil {{").ok();
    writeln!(out, "\t\t\tC.{ffi_prefix}_free_string(ptr)").ok();
    writeln!(out, "\t\t}}").ok();
    writeln!(out, "\t\treturn nil, err").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tif ptr == nil {{").ok();
    writeln!(out, "\t\treturn nil, fmt.Errorf(\"conversion returned nil\")").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tdefer C.{ffi_prefix}_free_string(ptr)").ok();
    writeln!(out, "\tvar result ConversionResult").ok();
    writeln!(
        out,
        "\tif err := json.Unmarshal([]byte(C.GoString(ptr)), &result); err != nil {{"
    )
    .ok();
    writeln!(
        out,
        "\t\treturn nil, fmt.Errorf(\"failed to unmarshal conversion result: %w\", err)"
    )
    .ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn &result, nil").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
}
