use crate::core::ir::TypeRef;

/// Generate a human-readable description for an enum variant from its name.
///
/// Handles both PascalCase (`SingleColumn` → "Single column") and
/// SCREAMING_CASE (`CODE_BLOCK` → "Code block element") variant names.
pub(crate) fn generate_enum_variant_description(variant_name: &str) -> String {
    if variant_name.is_empty() {
        return String::new();
    }

    match variant_name {
        "TEXT" => return "Text format".to_string(),
        "MARKDOWN" => return "Markdown format".to_string(),
        "HTML" | "Html" => return "Preserve as HTML `<mark>` tags".to_string(),
        "JSON" => return "JSON format".to_string(),
        "CSV" => return "CSV format".to_string(),
        "XML" => return "XML format".to_string(),
        "PDF" => return "PDF format".to_string(),
        "PLAIN" => return "Plain text format".to_string(),
        _ => {}
    }

    let is_screaming = variant_name
        .chars()
        .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit());

    let words: Vec<String> = if is_screaming {
        variant_name
            .split('_')
            .filter(|s| !s.is_empty())
            .map(|w| w.to_lowercase())
            .collect()
    } else {
        let mut parts = Vec::new();
        let mut current = String::new();
        let chars: Vec<char> = variant_name.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if c.is_uppercase() && !current.is_empty() {
                let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
                if next_is_lower && current.len() > 1 && current.chars().all(|ch| ch.is_uppercase()) {
                    let last = current.pop().expect("current is non-empty");
                    if !current.is_empty() {
                        parts.push(current);
                    }
                    current = String::new();
                    current.push(last);
                } else {
                    parts.push(current);
                    current = String::new();
                }
            }
            current.push(c);
        }
        if !current.is_empty() {
            parts.push(current);
        }
        parts.into_iter().map(|w| w.to_lowercase()).collect()
    };

    if words.is_empty() {
        return String::new();
    }

    let joined = words.join(" ");
    let suffix = determine_enum_variant_suffix(&joined, is_screaming);

    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => {
            let capitalized = first.to_uppercase().to_string() + chars.as_str();
            if suffix.is_empty() {
                capitalized
            } else {
                format!("{capitalized} {suffix}")
            }
        }
        None => String::new(),
    }
}

/// Determine an appropriate suffix for an enum variant description.
pub(crate) fn determine_enum_variant_suffix(readable: &str, is_screaming: bool) -> &'static str {
    let format_words = [
        "text", "markdown", "html", "json", "csv", "xml", "pdf", "yaml", "toml", "docx", "xlsx", "pptx", "rtf",
        "latex", "rst", "asciidoc", "epub",
    ];
    for w in &format_words {
        if readable == *w {
            return "format";
        }
    }

    let element_words = [
        "heading",
        "paragraph",
        "blockquote",
        "table",
        "figure",
        "caption",
        "footnote",
        "header",
        "footer",
        "section",
        "title",
        "subtitle",
        "image",
    ];
    for w in &element_words {
        if readable == *w {
            return "element";
        }
    }

    let no_suffix_endings = [
        "format", "mode", "type", "level", "style", "strategy", "method", "state", "status", "error", "element",
        "block", "list", "model",
    ];
    for ending in &no_suffix_endings {
        if readable.ends_with(ending) {
            return "";
        }
    }

    if is_screaming && (readable.contains("list") || readable.contains("block") || readable.contains("item")) {
        return "";
    }

    ""
}

/// Generate a human-readable description for an error variant from its PascalCase name.
///
/// Splits PascalCase into words and forms a sentence like "IO errors" or "Parsing errors".
pub(crate) fn generate_error_variant_description(variant_name: &str) -> String {
    let mut words = Vec::new();
    let mut current = String::new();
    for c in variant_name.chars() {
        if c.is_uppercase() && !current.is_empty() {
            words.push(current);
            current = String::new();
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }

    if words.is_empty() {
        return String::new();
    }

    let readable = words.join(" ").to_lowercase();
    let mut chars = readable.chars();
    match chars.next() {
        Some(first) => {
            let capitalized = first.to_uppercase().to_string() + chars.as_str();
            format!("{capitalized} errors")
        }
        None => String::new(),
    }
}

/// Generate a human-readable field description from its name and type
/// when no explicit doc comment exists on a struct field.
pub(crate) fn generate_field_description(field_name: &str, type_ref: &TypeRef) -> String {
    match field_name {
        "content" => return "The extracted text content".to_string(),
        "mime_type" => return "The detected MIME type".to_string(),
        "metadata" => return "Document metadata".to_string(),
        "tables" => return "Tables extracted from the document".to_string(),
        "images" => return "Images extracted from the document".to_string(),
        "pages" => return "Per-page content".to_string(),
        "chunks" => return "Text chunks for chunking/embedding".to_string(),
        "elements" => return "Semantic document elements".to_string(),
        "name" => return "The name".to_string(),
        "path" => return "File path".to_string(),
        "description" => return "Human-readable description".to_string(),
        "version" => return "Version string".to_string(),
        "id" => return "Unique identifier".to_string(),
        "enabled" => return "Whether this feature is enabled".to_string(),
        "size" => return "Size in bytes".to_string(),
        "count" => return "Number of items".to_string(),
        _ => {}
    }

    if let Some(rest) = field_name.strip_suffix("_count") {
        let readable = rest.replace('_', " ");
        let pluralized = if readable.ends_with('s') {
            readable
        } else {
            format!("{readable}s")
        };
        return format!("Number of {pluralized}");
    }
    if let Some(rest) = field_name.strip_prefix("is_") {
        let readable = rest.replace('_', " ");
        return format!("Whether {readable}");
    }
    if let Some(rest) = field_name.strip_prefix("has_") {
        let readable = rest.replace('_', " ");
        return format!("Whether {readable}");
    }
    if let Some(rest) = field_name.strip_prefix("max_") {
        let readable = rest.replace('_', " ");
        return format!("Maximum {readable}");
    }
    if let Some(rest) = field_name.strip_prefix("min_") {
        let readable = rest.replace('_', " ");
        return format!("Minimum {readable}");
    }

    if let TypeRef::Named(type_name) = type_ref {
        let readable_type = type_name.chars().enumerate().fold(String::new(), |mut acc, (i, c)| {
            if c.is_uppercase() && i > 0 {
                acc.push(' ');
                acc.push(c.to_ascii_lowercase());
            } else if i == 0 {
                acc.push(c.to_ascii_lowercase());
            } else {
                acc.push(c);
            }
            acc
        });
        let readable_name = snake_to_readable(field_name);
        return format!("{readable_name} ({readable_type})");
    }

    snake_to_readable(field_name)
}

/// Convert a `snake_case` identifier to `Readable text` (capitalize first letter).
pub(crate) fn snake_to_readable(name: &str) -> String {
    let readable = name.replace('_', " ");
    let mut chars = readable.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Generate a human-readable parameter description from its name and type
/// when no explicit doc comment or `# Arguments` entry exists.
pub(crate) fn generate_param_description(name: &str, ty: &TypeRef) -> String {
    let article = match name {
        "config" | "configuration" => return "The configuration options".to_string(),
        "options" | "opts" => return "The options to use".to_string(),
        "path" | "file_path" => return "Path to the file".to_string(),
        "content" | "contents" => return "The content to process".to_string(),
        "input" => return "The input data".to_string(),
        "output" => return "The output destination".to_string(),
        "url" => return "The URL to fetch".to_string(),
        "timeout" => return "Timeout duration".to_string(),
        "callback" | "cb" => return "Callback function".to_string(),
        _ => "The",
    };

    let type_hint = match ty {
        TypeRef::Named(type_name) => type_name.chars().enumerate().fold(String::new(), |mut acc, (i, c)| {
            if c.is_uppercase() && i > 0 {
                acc.push(' ');
                acc.push(c.to_ascii_lowercase());
            } else if i == 0 {
                acc.push(c.to_ascii_lowercase());
            } else {
                acc.push(c);
            }
            acc
        }),
        _ => name.replace('_', " "),
    };

    format!("{article} {type_hint}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::TypeRef;

    #[test]
    fn test_generate_field_description_known_names() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("content", &ty), "The extracted text content");
        assert_eq!(generate_field_description("mime_type", &ty), "The detected MIME type");
        assert_eq!(generate_field_description("metadata", &ty), "Document metadata");
        assert_eq!(
            generate_field_description("tables", &ty),
            "Tables extracted from the document"
        );
        assert_eq!(
            generate_field_description("images", &ty),
            "Images extracted from the document"
        );
        assert_eq!(generate_field_description("pages", &ty), "Per-page content");
        assert_eq!(
            generate_field_description("chunks", &ty),
            "Text chunks for chunking/embedding"
        );
        assert_eq!(
            generate_field_description("elements", &ty),
            "Semantic document elements"
        );
        assert_eq!(generate_field_description("name", &ty), "The name");
        assert_eq!(generate_field_description("path", &ty), "File path");
        assert_eq!(
            generate_field_description("description", &ty),
            "Human-readable description"
        );
        assert_eq!(generate_field_description("version", &ty), "Version string");
        assert_eq!(generate_field_description("id", &ty), "Unique identifier");
        assert_eq!(
            generate_field_description("enabled", &ty),
            "Whether this feature is enabled"
        );
        assert_eq!(generate_field_description("size", &ty), "Size in bytes");
        assert_eq!(generate_field_description("count", &ty), "Number of items");
    }

    #[test]
    fn test_generate_field_description_prefix_patterns() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("row_count", &ty), "Number of rows");
        assert_eq!(generate_field_description("is_valid", &ty), "Whether valid");
        assert_eq!(generate_field_description("has_errors", &ty), "Whether errors");
        assert_eq!(generate_field_description("max_retries", &ty), "Maximum retries");
        assert_eq!(generate_field_description("min_confidence", &ty), "Minimum confidence");
        assert_eq!(generate_field_description("is_ocr_enabled", &ty), "Whether ocr enabled");
    }

    #[test]
    fn test_generate_field_description_named_type() {
        let ty = TypeRef::Named("ExtractionConfig".to_string());
        assert_eq!(generate_field_description("config", &ty), "Config (extraction config)");
    }

    #[test]
    fn test_generate_field_description_fallback_snake_case() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("column_types", &ty), "Column types");
        assert_eq!(generate_field_description("output_format", &ty), "Output format");
    }

    #[test]
    fn test_snake_to_readable() {
        assert_eq!(snake_to_readable("row_count"), "Row count");
        assert_eq!(snake_to_readable("column_types"), "Column types");
        assert_eq!(snake_to_readable("x"), "X");
        assert_eq!(snake_to_readable(""), "");
    }

    #[test]
    fn test_generate_enum_variant_description_well_known() {
        assert_eq!(generate_enum_variant_description("TEXT"), "Text format");
        assert_eq!(generate_enum_variant_description("MARKDOWN"), "Markdown format");
        assert_eq!(
            generate_enum_variant_description("HTML"),
            "Preserve as HTML `<mark>` tags"
        );
        assert_eq!(generate_enum_variant_description("JSON"), "JSON format");
        assert_eq!(generate_enum_variant_description("PDF"), "PDF format");
        assert_eq!(generate_enum_variant_description("PLAIN"), "Plain text format");
    }

    #[test]
    fn test_generate_enum_variant_description_screaming_case() {
        assert_eq!(generate_enum_variant_description("CODE_BLOCK"), "Code block");
        assert_eq!(generate_enum_variant_description("ORDERED_LIST"), "Ordered list");
        assert_eq!(generate_enum_variant_description("BULLET_LIST"), "Bullet list");
        assert_eq!(generate_enum_variant_description("HEADING"), "Heading element");
    }

    #[test]
    fn test_generate_enum_variant_description_pascal_case() {
        assert_eq!(generate_enum_variant_description("SingleColumn"), "Single column");
        assert_eq!(generate_enum_variant_description("AutoOsd"), "Auto osd");
    }

    #[test]
    fn test_generate_enum_variant_description_empty() {
        assert_eq!(generate_enum_variant_description(""), "");
    }

    #[test]
    fn test_determine_enum_variant_suffix_format_words() {
        for word in ["text", "markdown", "html", "json", "csv", "xml", "pdf", "yaml"] {
            assert_eq!(
                determine_enum_variant_suffix(word, false),
                "format",
                "expected 'format' suffix for '{word}'"
            );
        }
    }

    #[test]
    fn test_determine_enum_variant_suffix_element_words() {
        for word in [
            "heading",
            "paragraph",
            "blockquote",
            "table",
            "figure",
            "caption",
            "footnote",
            "header",
            "footer",
            "section",
            "title",
            "image",
        ] {
            assert_eq!(
                determine_enum_variant_suffix(word, false),
                "element",
                "expected 'element' suffix for '{word}'"
            );
        }
    }

    #[test]
    fn test_determine_enum_variant_suffix_no_suffix_when_ending_matches_category_word() {
        let no_suffix_cases = [
            "extraction mode",
            "output format",
            "heading style",
            "retry strategy",
            "connection state",
            "error status",
            "dom element",
            "code block",
            "ordered list",
            "language model",
        ];
        for word in no_suffix_cases {
            assert_eq!(
                determine_enum_variant_suffix(word, false),
                "",
                "expected empty suffix for '{word}'"
            );
        }
    }

    #[test]
    fn test_determine_enum_variant_suffix_screaming_with_list_block_item() {
        assert_eq!(determine_enum_variant_suffix("bullet list", true), "");
        assert_eq!(determine_enum_variant_suffix("code block", true), "");
        assert_eq!(determine_enum_variant_suffix("list item", true), "");
    }

    #[test]
    fn test_determine_enum_variant_suffix_unknown_word_returns_empty() {
        assert_eq!(determine_enum_variant_suffix("single column", false), "");
        assert_eq!(determine_enum_variant_suffix("auto osd", false), "");
        assert_eq!(determine_enum_variant_suffix("left", false), "");
    }

    #[test]
    fn test_generate_field_description_count_suffix_already_plural() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("errors_count", &ty), "Number of errors");
    }

    #[test]
    fn test_generate_field_description_count_suffix_singular_words() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("page_count", &ty), "Number of pages");
        assert_eq!(generate_field_description("word_count", &ty), "Number of words");
    }

    #[test]
    fn test_generate_field_description_is_prefix_multi_word() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("is_read_only", &ty), "Whether read only");
        assert_eq!(generate_field_description("is_active", &ty), "Whether active");
    }

    #[test]
    fn test_generate_field_description_has_prefix() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("has_metadata", &ty), "Whether metadata");
        assert_eq!(
            generate_field_description("has_ocr_support", &ty),
            "Whether ocr support"
        );
    }

    #[test]
    fn test_generate_field_description_at_suffix_falls_back_to_snake_readable() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("created_at", &ty), "Created at");
        assert_eq!(generate_field_description("updated_at", &ty), "Updated at");
    }

    #[test]
    fn test_generate_field_description_max_compound_name() {
        let ty = TypeRef::String;
        assert_eq!(generate_field_description("max_retries", &ty), "Maximum retries");
        assert_eq!(generate_field_description("max_size", &ty), "Maximum size");
    }

    #[test]
    fn test_generate_field_description_primitive_type_uses_name_fallback() {
        use crate::core::ir::PrimitiveType;
        assert_eq!(
            generate_field_description("confidence", &TypeRef::Primitive(PrimitiveType::F64)),
            "Confidence"
        );
    }
}
