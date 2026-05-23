//! Callback specification table for the Java visitor bridge.
//!
//! Mirrors `crates/alef-backend-go/src/gen_visitor.rs`.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callbacks_table_is_non_empty() {
        assert!(!CALLBACKS.is_empty(), "CALLBACKS must have at least one entry");
    }

    #[test]
    fn all_callbacks_have_c_field_and_java_method() {
        for spec in CALLBACKS {
            assert!(!spec.c_field.is_empty(), "c_field must not be empty");
            assert!(!spec.java_method.is_empty(), "java_method must not be empty");
        }
    }

    #[test]
    fn visit_table_row_has_is_header() {
        let row = CALLBACKS
            .iter()
            .find(|s| s.c_field == "visit_table_row")
            .expect("must have visit_table_row");
        assert!(row.has_is_header, "visit_table_row must have has_is_header = true");
    }

    #[test]
    fn callbacks_with_no_extra_have_empty_extra_slice() {
        let start = CALLBACKS
            .iter()
            .find(|s| s.c_field == "visit_element_start")
            .expect("must have visit_element_start");
        assert!(start.extra.is_empty(), "visit_element_start must have no extra params");
    }

    #[test]
    fn callbacks_count_matches_expected() {
        // 40 callbacks as documented in the module-level comment
        assert_eq!(CALLBACKS.len(), 40, "expected 40 visitor callbacks");
    }
}
