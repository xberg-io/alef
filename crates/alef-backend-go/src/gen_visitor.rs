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
/// - A static C helper in the CGo preamble constructs the VTable by referencing all
///   exported Go trampolines — this is valid because CGo compiles the preamble together
///   with the Go file that carries the `//export` declarations.
/// - `ConvertWithVisitor` registers the visitor, builds the VTable via the static C
///   helper, then calls `{prefix}_{bridge_snake}_new` + `{prefix}_options_set_{field}`
///   + `{prefix}_convert` to perform conversion.
///
/// # VTable ABI
///
/// Each function pointer in the VTable has the signature:
///   `(user_data: void*, ctx: char* /* JSON */, ...extras..., out_result: char**) -> int32_t`
///
/// `user_data` is the first argument; `ctx` is a JSON-encoded `NodeContext`; `out_result`
/// receives a heap-allocated C string when the visitor returns a Custom/Error result.
///
/// This differs from the legacy `VisitorCallbacks` pattern (FunctionParam bind_via), where
/// `user_data` was a FIELD on the struct and context was a typed `*NodeContext` pointer.
use alef_core::hash::{self, CommentStyle};
use serde_json;

/// Derive the cbindgen-generated C type name for a Rust FFI type.
///
/// cbindgen prepends the uppercased `ffi_prefix` to the Rust struct name verbatim.
/// Example: prefix="htm", Rust name="HtmHtmlVisitorVTable" → "HTMHtmHtmlVisitorVTable".
///
/// Note: the Rust struct name already includes the pascal-case prefix segment
/// (e.g. `Htm`), so only the uppercase prefix is prepended here.
pub(crate) fn ffi_c_type_name(ffi_prefix: &str, rust_basename: &str) -> String {
    let prefix_upper = ffi_prefix.to_uppercase();
    format!("{prefix_upper}{rust_basename}")
}

/// A single visitor callback specification.
struct CallbackSpec {
    /// Field name in the C VTable struct (snake_case).
    c_field: &'static str,
    /// Exported Go function name (e.g. `goVisitText`).
    export_name: &'static str,
    /// Go interface method name (PascalCase).
    go_method: &'static str,
    /// Doc comment for the Go interface method.
    doc: &'static str,
    /// Extra C parameters after `(user_data, ctx)` and before `(out_result)`.
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
        doc: "VisitTableRow visits table rows. Cells are passed as a JSON-encoded slice of strings.",
        extra: &[ExtraParam {
            c_name: "cells",
            c_type: "*C.char",
            go_name: "cells",
            go_iface_type: "[]string",
            decode: "decodeCellsJSON(cells)",
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
                c_type: "C.uintptr_t",
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

/// Generate the complete visitor.go file content for the options-field VTable ABI.
///
/// # Parameters
///
/// - `pkg_name`: Go package name (e.g. `"htmltomarkdown"`).
/// - `ffi_prefix`: C function prefix (e.g. `"htm"`).
/// - `ffi_header`: C header filename (e.g. `"html_to_markdown.h"`).
/// - `ffi_crate_dir`: path from go output dir to the FFI crate dir.
/// - `to_root`: relative path from go output dir to the repo root.
/// - `vtable_trait_name`: Rust trait name used to derive the VTable struct name
///   (e.g. `"HtmlVisitor"` → `"HtmHtmlVisitorVTable"`).
/// - `options_field`: field name on `ConversionOptions` that holds the bridge
///   (e.g. `"visitor"`).
pub fn gen_visitor_file(
    pkg_name: &str,
    ffi_prefix: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    to_root: &str,
    vtable_trait_name: &str,
    options_field: &str,
) -> String {
    let mut out = String::with_capacity(32_768);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));

    // Derive C type names.
    // VTable: {PREFIX_UPPER}{PascalPrefix}{TraitName}VTable  e.g. HTMHtmHtmlVisitorVTable
    // Bridge: {PREFIX_UPPER}{PascalPrefix}{TraitName}Bridge  e.g. HTMHtmHtmlVisitorBridge
    let pascal_prefix = {
        let mut chars = ffi_prefix.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    };
    let prefix_upper = ffi_prefix.to_uppercase();
    let vtable_rust_name = format!("{pascal_prefix}{vtable_trait_name}VTable");
    let bridge_rust_name = format!("{pascal_prefix}{vtable_trait_name}Bridge");
    let vtable_c_type = ffi_c_type_name(ffi_prefix, &vtable_rust_name);
    let bridge_c_type = ffi_c_type_name(ffi_prefix, &bridge_rust_name);
    let conversion_options_type = format!("{prefix_upper}ConversionOptions");

    // Derive bridge_snake from bridge_rust_name for fn names.
    // e.g. "HtmHtmlVisitorBridge" → "htm_html_visitor_bridge"
    let bridge_snake = to_snake_case(&bridge_rust_name);
    let fn_bridge_new = format!("{ffi_prefix}_{bridge_snake}_new");
    let fn_bridge_free = format!("{ffi_prefix}_{bridge_snake}_free");
    let fn_options_set_visitor = format!("{ffi_prefix}_options_set_{options_field}");
    let fn_options_free = format!("{ffi_prefix}_conversion_options_free");
    let fn_options_from_json = format!("{ffi_prefix}_conversion_options_from_json");
    let fn_convert = format!("{ffi_prefix}_convert");
    let fn_result_free = format!("{ffi_prefix}_conversion_result_free");

    // -------------------------------------------------------------------------
    // CGo preamble
    // -------------------------------------------------------------------------
    let callbacks: Vec<_> = CALLBACKS
        .iter()
        .map(|spec| {
            minijinja::Value::from_serialize(serde_json::json!({
                "export_name": spec.export_name,
                "c_sig": c_signature(spec),
                "c_field": spec.c_field,
            }))
        })
        .collect();

    out.push_str(&crate::template_env::render(
        "visitor_preamble.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            to_root => to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_header => ffi_header,
            callbacks => callbacks,
            vtable_c_type => vtable_c_type.clone(),
        },
    ));

    // NOTE: NodeType is defined in binding.go as `type NodeType string`.
    // Do NOT re-declare it here — that would cause a redeclaration compile error.
    out.push_str(&crate::template_env::render(
        "visitor_node_context_and_result.jinja",
        minijinja::Value::default(),
    ));
    out.push('\n');

    // -------------------------------------------------------------------------
    // Visitor interface
    // -------------------------------------------------------------------------
    out.push_str("// Visitor is the interface implemented by types that observe the HTML-to-Markdown\n");
    out.push_str("// conversion pipeline.  Embed BaseVisitor to get no-op defaults for all methods.\n");
    out.push_str("type Visitor interface {\n");
    for spec in CALLBACKS {
        let param_str = iface_param_str(spec);
        out.push_str(&crate::template_env::render(
            "visitor_interface_method.jinja",
            minijinja::context! {
                doc => spec.doc,
                method => spec.go_method,
                params => param_str,
            },
        ));
    }
    out.push_str("}\n");
    out.push('\n');

    // -------------------------------------------------------------------------
    // BaseVisitor — no-op defaults
    // -------------------------------------------------------------------------
    out.push_str("// BaseVisitor provides default no-op implementations for all Visitor methods.\n");
    out.push_str("// Embed it in your struct and override only the methods you need.\n");
    out.push_str("type BaseVisitor struct{}\n");
    out.push('\n');
    for spec in CALLBACKS {
        let param_str = iface_param_str(spec);
        // Build blank identifiers to suppress "declared but not used" errors.
        let blank_ids: Vec<String> = iface_param_names(spec).into_iter().collect();
        out.push_str(&crate::template_env::render(
            "base_visitor_method.jinja",
            minijinja::context! {
                doc => spec.go_method,
                method_name => spec.go_method,
                params => param_str,
                blank_ids => blank_ids,
            },
        ));
        out.push('\n');
    }

    // -------------------------------------------------------------------------
    // Visitor registry
    // -------------------------------------------------------------------------
    out.push_str("// visitorRegistry maps visitor handle IDs to active Visitor instances.\n");
    out.push_str("// CGo does not allow passing Go function values as C function pointers;\n");
    out.push_str("// we use a numeric ID (stored in user_data) to look up the Visitor at callback time.\n");
    out.push_str("var (\n");
    out.push_str("\tvisitorRegistry sync.Map\n");
    out.push_str("\tvisitorIDCounter atomic.Uint64\n");
    out.push_str(")\n");
    out.push('\n');
    out.push_str("func registerVisitor(v Visitor) uintptr {\n");
    out.push_str("\tid := uintptr(visitorIDCounter.Add(1))\n");
    out.push_str("\tvisitorRegistry.Store(id, v)\n");
    out.push_str("\treturn id\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("func unregisterVisitor(id uintptr) {\n");
    out.push_str("\tvisitorRegistry.Delete(id)\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("func lookupVisitor(id uintptr) (Visitor, bool) {\n");
    out.push_str("\tv, ok := visitorRegistry.Load(id)\n");
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn nil, false\n");
    out.push_str("\t}\n");
    out.push_str("\treturn v.(Visitor), true\n");
    out.push_str("}\n");
    out.push('\n');

    // -------------------------------------------------------------------------
    // Shared helpers
    // -------------------------------------------------------------------------

    // decodeNodeContext: decode from JSON string (VTable ABI passes ctx as *const c_char JSON)
    out.push_str("func decodeNodeContext(ctxJSON *C.char) NodeContext {\n");
    out.push_str("\tvar ctx NodeContext\n");
    out.push_str("\tif ctxJSON == nil {\n");
    out.push_str("\t\treturn ctx\n");
    out.push_str("\t}\n");
    out.push_str("\t_ = json.Unmarshal([]byte(C.GoString(ctxJSON)), &ctx)\n");
    out.push_str("\treturn ctx\n");
    out.push_str("}\n");
    out.push('\n');

    // encodeVisitResult: write serde-native JSON into *out_result so the Rust trait bridge
    // can deserialize it with serde_json::from_str::<VisitResult>.
    //
    // Rust serde-derived enum serialisation:
    //   Continue     → "Continue"
    //   Skip         → "Skip"
    //   PreserveHtml → "PreserveHtml"
    //   Custom(s)    → {"Custom":"<s>"}
    //   Error(s)     → {"Error":"<s>"}
    //
    // The return code still carries the numeric variant tag so callers that only
    // inspect the code (and don't read out_result) remain compatible.
    out.push_str("func encodeVisitResult(r VisitResult, outResult **C.char) C.int32_t {\n");
    out.push_str("\t// Encode the result as serde-native JSON so the Rust trait bridge's\n");
    out.push_str("\t// serde_json::from_str::<VisitResult> deserialiser can decode it correctly.\n");
    out.push_str("\tvar jsonStr string\n");
    out.push_str("\tswitch r.Code {\n");
    out.push_str("\tcase 1:\n");
    out.push_str("\t\tjsonStr = `\"Skip\"`\n");
    out.push_str("\tcase 2:\n");
    out.push_str("\t\tjsonStr = `\"PreserveHtml\"`\n");
    out.push_str("\tcase 3:\n");
    out.push_str("\t\tif r.Custom != nil {\n");
    out.push_str("\t\t\tb, err := json.Marshal(*r.Custom)\n");
    out.push_str("\t\t\tif err != nil {\n");
    out.push_str("\t\t\t\tb = []byte(`\"\"`)\n");
    out.push_str("\t\t\t}\n");
    out.push_str("\t\t\tjsonStr = `{\"Custom\":` + string(b) + `}`\n");
    out.push_str("\t\t} else {\n");
    out.push_str("\t\t\tjsonStr = `{\"Custom\":\"\"}`\n");
    out.push_str("\t\t}\n");
    out.push_str("\tcase 4:\n");
    out.push_str("\t\tif r.Custom != nil {\n");
    out.push_str("\t\t\tb, err := json.Marshal(*r.Custom)\n");
    out.push_str("\t\t\tif err != nil {\n");
    out.push_str("\t\t\t\tb = []byte(`\"\"`)\n");
    out.push_str("\t\t\t}\n");
    out.push_str("\t\t\tjsonStr = `{\"Error\":` + string(b) + `}`\n");
    out.push_str("\t\t} else {\n");
    out.push_str("\t\t\tjsonStr = `{\"Error\":\"\"}`\n");
    out.push_str("\t\t}\n");
    out.push_str("\tdefault: // 0 = Continue and any unknown code\n");
    out.push_str("\t\tjsonStr = `\"Continue\"`\n");
    out.push_str("\t}\n");
    out.push_str("\t*outResult = C.CString(jsonStr)\n");
    out.push_str("\treturn C.int32_t(r.Code)\n");
    out.push_str("}\n");
    out.push('\n');

    out.push_str("func optGoString(p *C.char) *string {\n");
    out.push_str("\tif p == nil {\n");
    out.push_str("\t\treturn nil\n");
    out.push_str("\t}\n");
    out.push_str("\ts := C.GoString(p)\n");
    out.push_str("\treturn &s\n");
    out.push_str("}\n");
    out.push('\n');

    // decodeCellsJSON: cells is a JSON-encoded []string in the VTable ABI.
    out.push_str("func decodeCellsJSON(cells *C.char) []string {\n");
    out.push_str("\tif cells == nil {\n");
    out.push_str("\t\treturn nil\n");
    out.push_str("\t}\n");
    out.push_str("\tvar result []string\n");
    out.push_str("\t_ = json.Unmarshal([]byte(C.GoString(cells)), &result)\n");
    out.push_str("\treturn result\n");
    out.push_str("}\n");
    out.push('\n');

    // -------------------------------------------------------------------------
    // //export trampolines
    // -------------------------------------------------------------------------
    for spec in CALLBACKS {
        gen_trampoline(&mut out, spec);
    }

    // -----------------------------------------------------------------------
    // Internal helper: convertWithVisitorHelper
    // -----------------------------------------------------------------------
    // This helper is called by Convert() in binding.go when options.Visitor is not nil.
    // It registers the visitor, builds the VTable, creates a bridge, attaches it to
    // options, calls the FFI convert function, and cleans up.
    out.push('\n');
    out.push_str("// convertWithVisitorHelper converts HTML with visitor support.\n");
    out.push_str("// Called by Convert() when options.Visitor is not nil.\n");
    out.push_str("// Returns the ConversionResult or an error.\n");
    out.push_str("func convertWithVisitorHelper(html string, options *ConversionOptions, visitor Visitor) (*ConversionResult, error) {\n");
    out.push_str("\tcHTML := C.CString(html)\n");
    out.push_str("\tdefer C.free(unsafe.Pointer(cHTML))\n");
    out.push('\n');

    // Build ConversionOptions C pointer.
    out.push_str(&crate::template_env::render(
        "c_options_var_decl.jinja",
        minijinja::context! {
            conversion_options_type => conversion_options_type,
        },
    ));
    out.push('\n');
    out.push_str("\tif options != nil {\n");
    out.push_str(&crate::template_env::render(
        "c_options_from_value.jinja",
        minijinja::context! {
            fn_options_from_json => fn_options_from_json,
            fn_options_free => fn_options_free,
        },
    ));
    out.push_str("\t}\n");
    out.push_str("\tif cOptions == nil {\n");
    out.push_str("\t\t// Allocate a default options struct so we can attach the visitor.\n");
    out.push_str("\t\tdefaultJSON := C.CString(\"{}\")\n");
    out.push_str(&crate::template_env::render(
        "c_options_from_json.jinja",
        minijinja::context! {
            fn_options_from_json => fn_options_from_json,
        },
    ));
    out.push_str("\t\tC.free(unsafe.Pointer(defaultJSON))\n");
    out.push_str(&crate::template_env::render(
        "c_options_defer_free.jinja",
        minijinja::context! {
            fn_options_free => fn_options_free,
        },
    ));
    out.push_str("\t}\n");
    out.push('\n');

    // Register visitor and build VTable.
    out.push_str("\t// Register visitor and build the C VTable via the static C helper.\n");
    out.push_str("\tid := registerVisitor(visitor)\n");
    out.push_str("\tdefer unregisterVisitor(id)\n");
    out.push_str("\tvtbl := C.makeVisitorVTable()\n");
    out.push('\n');

    // Create bridge from VTable + user_data.
    out.push_str("\t// Create a bridge that holds the VTable and the visitor ID as user_data.\n");
    out.push_str(&crate::template_env::render(
        "c_bridge_new.jinja",
        minijinja::context! {
            fn_bridge_new => fn_bridge_new,
        },
    ));
    out.push_str("\tif bridge == nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"failed to create visitor bridge\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_bridge_defer_free.jinja",
        minijinja::context! {
            fn_bridge_free => fn_bridge_free,
        },
    ));
    out.push('\n');

    // Attach bridge to options.
    out.push_str("\t// Attach the bridge to the options struct so convert() picks it up.\n");
    out.push_str(&crate::template_env::render(
        "c_options_set_visitor.jinja",
        minijinja::context! {
            fn_options_set_visitor => fn_options_set_visitor,
            bridge_c_type => bridge_c_type,
        },
    ));
    out.push('\n');

    // Call convert.
    out.push_str(&crate::template_env::render(
        "c_convert_call.jinja",
        minijinja::context! {
            fn_convert => fn_convert,
        },
    ));
    out.push_str("\tif ptr == nil {\n");
    out.push_str("\t\tif err := lastError(); err != nil {\n");
    out.push_str("\t\t\treturn nil, err\n");
    out.push_str("\t\t}\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"conversion returned nil\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_result_defer_free.jinja",
        minijinja::context! {
            fn_result_free => fn_result_free,
        },
    ));
    out.push('\n');

    // Deserialize ConversionResult.
    let fn_result_to_json = fn_result_free.replace("_free", "_to_json");
    out.push_str(&crate::template_env::render(
        "c_result_to_json.jinja",
        minijinja::context! {
            fn_result_to_json => fn_result_to_json,
        },
    ));
    out.push_str("\tif jsonPtr == nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"conversion result serialisation failed\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_free_string_defer.jinja",
        minijinja::context! {
            ffi_prefix => ffi_prefix,
        },
    ));
    out.push_str("\tvar result ConversionResult\n");
    out.push_str("\tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"failed to decode conversion result: %w\", err)\n");
    out.push_str("\t}\n");
    out.push_str("\treturn &result, nil\n");
    out.push_str("}\n");
    out.push('\n');

    out
}

/// Build the C parameter list string for the extern declaration of an exported Go function.
///
/// VTable ABI: `(void* user_data, char* ctx, ...extras..., int32_t isHeader?, char** out_result)`
fn c_signature(spec: &CallbackSpec) -> String {
    let mut parts = vec!["void* user_data".to_string(), "char* ctx".to_string()];
    for ep in spec.extra {
        let ctype = match ep.c_type {
            "*C.char" => "char*",
            "C.int32_t" => "int32_t",
            "C.uint32_t" => "uint32_t",
            "C.uintptr_t" => "uintptr_t",
            _ => "void*",
        };
        parts.push(format!("{ctype} {}", ep.c_name));
    }
    if spec.has_is_header {
        parts.push("int32_t isHeader".to_string());
    }
    parts.push("char** out_result".to_string());
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

/// Generate one `//export goVisit*` C callback trampoline for the VTable ABI.
///
/// VTable ABI signature: `(user_data unsafe.Pointer, ctx *C.char, ...extras..., outResult **C.char) C.int32_t`
fn gen_trampoline(out: &mut String, spec: &CallbackSpec) {
    // Build Go function parameter list (CGo types).
    // VTable ABI: user_data first, then ctx (JSON string), then extras, then out_result.
    let mut go_params = vec!["userData unsafe.Pointer".to_string(), "ctx *C.char".to_string()];
    for ep in spec.extra {
        go_params.push(format!("{} {}", ep.c_name, ep.c_type));
    }
    if spec.has_is_header {
        go_params.push("isHeader C.int32_t".to_string());
    }
    go_params.push("outResult **C.char".to_string());

    out.push_str(&crate::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => spec.export_name,
        },
    ));
    out.push_str(&crate::template_env::render(
        "trampoline_func_signature.jinja",
        minijinja::context! {
            name => spec.export_name,
            params => go_params.join(", "),
        },
    ));
    out.push_str("\tvisitorID := uintptr(uintptr(userData))\n");
    out.push_str("\tv, ok := lookupVisitor(visitorID)\n");
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 0\n");
    out.push_str("\t}\n");
    out.push_str("\tnodeCtx := decodeNodeContext(ctx)\n");

    // Decode each extra parameter.
    for ep in spec.extra {
        out.push_str(&crate::template_env::render(
            "trampoline_param_decode.jinja",
            minijinja::context! {
                name => capitalize(ep.go_name),
                decode => ep.decode,
            },
        ));
    }
    if spec.has_is_header {
        out.push_str("\tgoIsHeader := isHeader != 0\n");
    }

    // Build call args.
    let mut call_args = vec!["nodeCtx".to_string()];
    for ep in spec.extra {
        call_args.push(format!("go{}", capitalize(ep.go_name)));
    }
    if spec.has_is_header {
        call_args.push("goIsHeader".to_string());
    }

    out.push_str(&crate::template_env::render(
        "trampoline_func_call.jinja",
        minijinja::context! {
            method => spec.go_method,
            args => call_args.join(", "),
        },
    ));
    out.push_str("}\n");
    out.push('\n');
}

/// Generate the `ConvertWithVisitor` function for the options-field VTable pattern.
///
/// Flow:
/// 1. Register the Go visitor in the global table; get a numeric ID as user_data.
/// 2. Build the VTable via the static C helper (all fn pointers set, no user_data field).
/// 3. Create a bridge via `{fn_bridge_new}(&vtbl, unsafe.Pointer(id))`.
/// 4. Attach the bridge to options via `{fn_options_set_visitor}(cOptions, bridge)`.
/// 5. Call `{fn_convert}(cHTML, cOptions)` to run conversion.
/// 6. Free bridge and options after conversion completes.
///
/// NOTE: This function is no longer used. The same logic is now inlined in
/// `convertWithVisitorHelper` and generated directly in `gen_visitor_file`.
#[allow(clippy::too_many_arguments, dead_code)]
fn gen_convert_with_visitor(
    out: &mut String,
    ffi_prefix: &str,
    conversion_options_type: &str,
    _vtable_c_type: &str,
    bridge_c_type: &str,
    fn_bridge_new: &str,
    fn_bridge_free: &str,
    fn_options_set_visitor: &str,
    fn_options_free: &str,
    fn_options_from_json: &str,
    fn_convert: &str,
    fn_result_free: &str,
) {
    out.push_str("// ConvertWithVisitor converts HTML to Markdown, invoking visitor callbacks during\n");
    out.push_str("// the conversion pipeline.  Pass nil for options to use defaults.\n");
    out.push_str("// Pass a struct embedding BaseVisitor and overriding only the methods you need.\n");
    out.push_str("func ConvertWithVisitor(html string, options *ConversionOptions, visitor Visitor) (*ConversionResult, error) {\n");
    out.push_str("\tcHTML := C.CString(html)\n");
    out.push_str("\tdefer C.free(unsafe.Pointer(cHTML))\n");
    out.push('\n');

    // Build ConversionOptions C pointer (nil → use defaults).
    out.push_str(&crate::template_env::render(
        "c_options_var_decl.jinja",
        minijinja::context! {
            conversion_options_type => conversion_options_type,
        },
    ));
    out.push('\n');
    out.push_str("\tif options != nil {\n");
    out.push_str(&crate::template_env::render(
        "c_options_from_value.jinja",
        minijinja::context! {
            fn_options_from_json => fn_options_from_json,
            fn_options_free => fn_options_free,
        },
    ));
    out.push_str("\t}\n");
    out.push_str("\tif cOptions == nil {\n");
    out.push_str("\t\t// Allocate a default options struct so we can attach the visitor.\n");
    out.push_str("\t\tdefaultJSON := C.CString(\"{}\")\n");
    out.push_str(&crate::template_env::render(
        "c_options_from_json.jinja",
        minijinja::context! {
            fn_options_from_json => fn_options_from_json,
        },
    ));
    out.push_str("\t\tC.free(unsafe.Pointer(defaultJSON))\n");
    out.push_str(&crate::template_env::render(
        "c_options_defer_free.jinja",
        minijinja::context! {
            fn_options_free => fn_options_free,
        },
    ));
    out.push_str("\t}\n");
    out.push('\n');

    // Register visitor and build VTable.
    out.push_str("\t// Register visitor and build the C VTable via the static C helper.\n");
    out.push_str("\tid := registerVisitor(visitor)\n");
    out.push_str("\tdefer unregisterVisitor(id)\n");
    out.push_str("\tvtbl := C.makeVisitorVTable()\n");
    out.push('\n');

    // Create bridge from VTable + user_data.
    out.push_str("\t// Create a bridge that holds the VTable and the visitor ID as user_data.\n");
    out.push_str(&crate::template_env::render(
        "c_bridge_new.jinja",
        minijinja::context! {
            fn_bridge_new => fn_bridge_new,
        },
    ));
    out.push_str("\tif bridge == nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"failed to create visitor bridge\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_bridge_defer_free.jinja",
        minijinja::context! {
            fn_bridge_free => fn_bridge_free,
        },
    ));
    out.push('\n');

    // Attach bridge to options.
    out.push_str("\t// Attach the bridge to the options struct so convert() picks it up.\n");
    out.push_str(&crate::template_env::render(
        "c_options_set_visitor.jinja",
        minijinja::context! {
            fn_options_set_visitor => fn_options_set_visitor,
            bridge_c_type => bridge_c_type,
        },
    ));
    out.push('\n');

    // Call convert.
    out.push_str(&crate::template_env::render(
        "c_convert_call.jinja",
        minijinja::context! {
            fn_convert => fn_convert,
        },
    ));
    out.push_str("\tif ptr == nil {\n");
    out.push_str("\t\tif err := lastError(); err != nil {\n");
    out.push_str("\t\t\treturn nil, err\n");
    out.push_str("\t\t}\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"conversion returned nil\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_result_defer_free.jinja",
        minijinja::context! {
            fn_result_free => fn_result_free,
        },
    ));
    out.push('\n');

    // Deserialize ConversionResult: convert the opaque result pointer to JSON first,
    // then unmarshal into a Go struct.  The pointer is a *ConversionResult struct (not a
    // string), so we must call the to_json helper before treating it as text.
    let fn_result_to_json = fn_result_free.replace("_free", "_to_json");
    out.push_str(&crate::template_env::render(
        "c_result_to_json.jinja",
        minijinja::context! {
            fn_result_to_json => fn_result_to_json,
        },
    ));
    out.push_str("\tif jsonPtr == nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"conversion result serialisation failed\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_free_string_defer.jinja",
        minijinja::context! {
            ffi_prefix => ffi_prefix,
        },
    ));
    out.push_str("\tvar result ConversionResult\n");
    out.push_str("\tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"failed to decode conversion result: %w\", err)\n");
    out.push_str("\t}\n");
    out.push_str("\treturn &result, nil\n");
    out.push_str("}\n");
    out.push('\n');
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Convert PascalCase to snake_case (e.g. "HtmHtmlVisitorBridge" → "htm_html_visitor_bridge").
fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}
