//! Shared knowledge of the `FormatMetadata` enum shape, needed by every backend that
//! navigates the crate's serialized JSON directly instead of through a typed accessor.
//!
//! `FormatMetadata` is internally tagged (`#[serde(tag = "format_type", rename_all =
//! "snake_case")]`), so its wire form is FLAT: the variant's own fields sit as siblings of the
//! `format_type` discriminator, with no JSON key for the variant name itself
//! (`{"format_type":"excel","sheet_count":2,...}`). Typed-language e2e backends (python, rust,
//! elixir, ruby) map a fixture path's `.excel` segment to a real variant accessor and need no
//! help here. A backend that instead walks the parsed JSON tree by key — Zig's
//! `std.json.Value` lookup chain, Swift's `JSONSerialization` navigation of a swift-bridge
//! JSON-bridged leaf — must skip that segment entirely, or it looks up a `"excel"` key that
//! does not exist and the assertion silently reads `null`.

/// Variant names of `FormatMetadata` (snake_case, from `#[serde(rename_all = "snake_case")]`).
///
/// These appear as typed accessors in fixture paths (e.g. `format.excel.sheet_count`) but are
/// NOT JSON keys. Kept as a single hand-maintained list, shared by every JSON-navigating
/// backend, rather than duplicated per backend where the copies could drift.
pub(crate) const FORMAT_METADATA_VARIANTS: &[&str] = &[
    "pdf",
    "docx",
    "excel",
    "email",
    "pptx",
    "archive",
    "image",
    "xml",
    "text",
    "html",
    "ocr",
    "csv",
    "bibtex",
    "citation",
    "fiction_book",
    "dbf",
    "jats",
    "epub",
    "pst",
    "audio",
    "code",
];

/// Whether `segment` is a `FormatMetadata` variant-name accessor immediately following a
/// `format` field in a fixture path — the shape a JSON-navigating backend must skip rather
/// than look up as a key. `prev_segment` is the bare (bracket-stripped) name of the segment
/// immediately before `segment` in the same path.
pub(crate) fn is_format_metadata_variant_segment(prev_segment: Option<&str>, segment: &str) -> bool {
    prev_segment == Some("format") && FORMAT_METADATA_VARIANTS.contains(&segment)
}
