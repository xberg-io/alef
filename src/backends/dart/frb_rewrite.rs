//! Post-processing rewriter for flutter_rust_bridge-generated `lib.dart`.
//!
//! flutter_rust_bridge emits sealed-class tagged-union variants with positional
//! parameter names (`field0`, `field1`, ...) when the underlying Rust variant
//! is a tuple variant:
//!
//! ```dart
//! const factory FormatMetadata.pdf({required PdfMetadata field0}) =
//!     FormatMetadata_Pdf;
//! ```
//!
//! These positional names are awkward for callers and inconsistent with the
//! kotlin/swift/etc. binding surface, which derives payload-informed names
//! (`metadata`, `value`, `value0`, ...) using the shared algorithm defined in
//! `alef-backend-kotlin::gen_bindings::shared::kotlin_field_name_with_type`.
//!
//! [`rewrite_frb_sealed_variants`] post-processes the frb-generated source and
//! rewrites variant parameter names to match the payload-derived convention.
//! Other code in the file is left untouched.
//!
//! Algorithm (per variant declaration line(s)):
//! 1. Match the canonical frb sealed-variant signature:
//!    `const factory <Enum>.<variantCamel>({required <PayloadType> field<N>, ...}) = <Enum>_<VariantPascal>;`
//! 2. Recover the `VariantPascal` token from the trailing assignment so that
//!    the variant name is unambiguous (the dotted form is lowerCamel, which
//!    cannot be reliably inverted back to PascalCase for multi-word variants).
//! 3. For each `field<N>` parameter, derive its new name from the payload type
//!    using the payload-derived helper (see [`payload_param_name`]).

use regex::Regex;
use std::sync::OnceLock;

/// Rewrite all flutter_rust_bridge sealed-class variant parameter names in
/// `source` from positional (`field0`, `field1`, ...) to payload-derived names.
///
/// Returns the rewritten source. Lines that do not match the variant signature
/// are returned verbatim, so this function is safe to apply unconditionally to
/// any frb-generated `lib.dart`.
pub fn rewrite_frb_sealed_variants(source: &str) -> String {
    let variant_re = variant_regex();

    variant_re
        .replace_all(source, |caps: &regex::Captures<'_>| {
            let prefix = &caps["prefix"];
            let params = &caps["params"];
            let suffix = &caps["suffix"];
            let variant_pascal = &caps["variant"];

            let rewritten_params = rewrite_param_list(params, variant_pascal);
            format!("{prefix}{rewritten_params}{suffix}")
        })
        .into_owned()
}

/// Match a single frb sealed-variant `const factory` line, allowing the
/// parameter block and the trailing `= <Enum>_<Variant>;` to span lines.
///
/// Capture groups:
/// - `prefix`  — `const factory <Enum>.<variantCamel>({`
/// - `params`  — the parameter list between `{` and `}` (no braces)
/// - `suffix`  — `}) = <Enum>_<Variant>;`
/// - `variant` — `<Variant>` (PascalCase) extracted from the assignment
fn variant_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The frb output can wrap before the `=` or before the `_Variant`
        // identifier. Use `(?s)` so `.` matches newlines inside the param block
        // and the inter-token whitespace.
        Regex::new(
            r"(?s)(?P<prefix>const\s+factory\s+[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\s*\(\s*\{)(?P<params>[^{}]*)(?P<suffix>\}\s*\)\s*=\s*[A-Za-z_][A-Za-z0-9_]*_(?P<variant>[A-Za-z][A-Za-z0-9]*)\s*;)",
        )
        .expect("variant regex must compile")
    })
}

/// Rewrite the comma-separated parameter list inside the variant constructor.
///
/// Each parameter has the shape `required <Type> field<N>`. The `<Type>` can be
/// `List<T>`, `Map<K, V>`, nullable types, etc. — we capture everything between
/// the leading `required` keyword and the trailing positional `field<N>` ident.
fn rewrite_param_list(params: &str, variant_pascal: &str) -> String {
    let param_re = param_regex();

    // Collect each `required <type> field<N>` match so we know `total_fields`
    // before deriving names (the payload-derived algorithm differs for
    // single-field vs multi-field variants).
    let matches: Vec<regex::Captures<'_>> = param_re.captures_iter(params).collect();
    let total_fields = matches
        .iter()
        .filter(|m| {
            let name = m.name("name").map(|m| m.as_str()).unwrap_or("");
            is_positional_field(name)
        })
        .count();

    if total_fields == 0 {
        return params.to_string();
    }

    let mut out = String::with_capacity(params.len());
    let mut cursor = 0usize;

    for caps in &matches {
        let whole = caps.get(0).expect("regex match must have group 0");
        let name_match = caps.name("name").expect("name capture is required");
        let raw_name = name_match.as_str();

        // Preserve everything between the previous match end and the start of
        // this `field<N>` ident untouched (whitespace, commas, the `required`
        // keyword, and the type).
        out.push_str(&params[cursor..name_match.start()]);

        if let Some(field_idx) = field_index(raw_name) {
            let type_name = caps.name("type").map(|m| m.as_str()).unwrap_or("").trim();
            let new_name = payload_param_name(type_name, variant_pascal, field_idx, total_fields);
            out.push_str(&new_name);
        } else {
            // Already a named parameter; leave it alone.
            out.push_str(raw_name);
        }

        cursor = name_match.end();
        // Continue from the end of the ident; the rest of `whole` (if any) is
        // accounted for by the next iteration's prefix slice.
        let _ = whole; // silence unused warning when no debug
    }

    out.push_str(&params[cursor..]);
    out
}

/// Match a single `required <Type> <ident>` parameter inside a frb variant
/// constructor. The `<Type>` is a balanced-bracket-free chunk plus `?` for
/// nullable; this is sufficient because frb does not emit nested generic types
/// like `Map<List<T>, U>` for sealed variants — but to be defensive we accept
/// any characters except `{`, `}`, `,` between the keyword and the ident.
fn param_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"required\s+(?P<type>[^,{}]+?)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?:,|$)")
            .expect("param regex must compile")
    })
}

/// Return the numeric index `N` for a positional `field<N>` parameter, or
/// `None` if `name` is a named (non-positional) parameter.
fn field_index(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("field")?;
    rest.parse::<usize>().ok()
}

/// `true` when `name` is a positional `field<N>` parameter emitted by frb.
fn is_positional_field(name: &str) -> bool {
    field_index(name).is_some()
}

/// Derive a payload-informed parameter name for a single frb variant field.
///
/// Mirrors the algorithm in `alef-backend-kotlin::gen_bindings::shared::
/// kotlin_field_name_with_type` so that all "modern" backends (kotlin, swift,
/// dart, ...) converge on the same surface.
///
/// Rules:
/// - Single-field variant with a named struct payload whose name starts with
///   the variant name (`PdfMetadata` for variant `Pdf`) → strip the prefix and
///   lowerCamelCase the remainder (`Metadata` → `metadata`).
/// - Single-field variant with a primitive/stdlib payload (`String`, `int`,
///   etc.) → `value`.
/// - Multi-field variant → `value0`, `value1`, ... (uses `field_idx`).
/// - Otherwise (single-field with no inferable prefix) → `value`.
fn payload_param_name(type_name: &str, variant_pascal: &str, field_idx: usize, total_fields: usize) -> String {
    if total_fields > 1 {
        return format!("value{field_idx}");
    }

    // Strip a trailing `?` (nullable marker) and any generic args for the
    // prefix-matching check — but keep them out of the primitive lookup as
    // well, since `List<int>` should not be considered "primitive".
    let stripped_type = type_name.trim_end_matches('?');
    let base_type = stripped_type
        .split_once('<')
        .map(|(head, _)| head)
        .unwrap_or(stripped_type)
        .trim();

    if let Some(remainder) = base_type.strip_prefix(variant_pascal)
        && !remainder.is_empty()
    {
        return to_lower_camel(remainder);
    }

    if is_dart_primitive(base_type) {
        return "value".to_string();
    }

    "value".to_string()
}

/// Lower the first character of a PascalCase identifier; preserve the rest.
fn to_lower_camel(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Dart primitive / stdlib types that should map to a generic `value` name.
fn is_dart_primitive(type_name: &str) -> bool {
    matches!(
        type_name,
        "String"
            | "int"
            | "double"
            | "bool"
            | "num"
            | "void"
            | "dynamic"
            | "Object"
            | "Uint8List"
            | "List"
            | "Map"
            | "Set"
            | "BigInt"
            | "DateTime"
            | "Duration"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_struct_payload_uses_payload_derived_name() {
        let input = r#"sealed class FormatMetadata with _$FormatMetadata {
  const FormatMetadata._();

  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
  const factory FormatMetadata.docx({required DocxMetadata field0}) =
      FormatMetadata_Docx;
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required PdfMetadata metadata"),
            "PdfMetadata payload should be named `metadata`, got:\n{out}"
        );
        assert!(
            out.contains("required DocxMetadata metadata"),
            "DocxMetadata payload should be named `metadata`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
    }

    #[test]
    fn primitive_payload_uses_value_name() {
        let input = r#"  const factory OutputFormat.custom({required String field0}) =
      OutputFormat_Custom;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required String value"),
            "String payload should be named `value`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
    }

    #[test]
    fn multi_field_tuple_uses_value0_value1() {
        let input = r#"  const factory Point.xy({required int field0, required int field1}) =
      Point_Xy;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required int value0"),
            "first tuple field should be `value0`, got:\n{out}"
        );
        assert!(
            out.contains("required int value1"),
            "second tuple field should be `value1`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
        assert!(!out.contains("field1"), "no `field1` should remain, got:\n{out}");
    }

    #[test]
    fn named_struct_field_is_preserved() {
        // frb already emits the struct field name when the variant carries a
        // named struct payload (struct variant, not tuple variant). The
        // rewriter must not touch these.
        let input = r#"  const factory Shape.rect({required double width, required double height}) =
      Shape_Rect;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required double width"),
            "named field `width` must be preserved, got:\n{out}"
        );
        assert!(
            out.contains("required double height"),
            "named field `height` must be preserved, got:\n{out}"
        );
    }

    #[test]
    fn non_variant_lines_are_untouched() {
        // Regular function declarations, top-level classes, comments, etc.
        // must round-trip unchanged.
        let input = r#"// This file is automatically generated.
import 'package:freezed_annotation/freezed_annotation.dart';

Future<int> extractBytes({required List<int> content}) =>
    RustLib.instance.api.crateExtractBytes(content: content);

class Foo {
  final int field0;
  Foo({required this.field0});
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert_eq!(out, input, "non-variant code must round-trip unchanged");
    }

    #[test]
    fn fallback_when_prefix_does_not_match_uses_value() {
        // Variant `Image` with payload `Bitmap` does not share a prefix; the
        // helper should fall back to `value` rather than producing something
        // nonsensical.
        let input = r#"  const factory Drawable.image({required Bitmap field0}) =
      Drawable_Image;
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(
            out.contains("required Bitmap value"),
            "unrelated payload type should fall back to `value`, got:\n{out}"
        );
    }

    #[test]
    fn nullable_payload_strips_question_mark_for_inference() {
        let input = r#"  const factory Either.left({required LeftValue? field0}) =
      Either_Left;
"#;
        let out = rewrite_frb_sealed_variants(input);
        // `LeftValue?` starts with variant prefix `Left` so remainder is `Value`.
        assert!(
            out.contains("required LeftValue? value"),
            "nullable payload with prefix-matching type should produce `value`, got:\n{out}"
        );
    }

    #[test]
    fn realistic_kreuzberg_format_metadata_block() {
        // Mirror the actual frb output shape from the kreuzberg fixture.
        let input = r#"sealed class FormatMetadata with _$FormatMetadata {
  const FormatMetadata._();

  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
  const factory FormatMetadata.docx({required DocxMetadata field0}) =
      FormatMetadata_Docx;
  const factory FormatMetadata.excel({required ExcelMetadata field0}) =
      FormatMetadata_Excel;
  const factory FormatMetadata.code({required String field0}) =
      FormatMetadata_Code;
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        assert!(out.contains("required PdfMetadata metadata"));
        assert!(out.contains("required DocxMetadata metadata"));
        assert!(out.contains("required ExcelMetadata metadata"));
        assert!(out.contains("required String value"));
        assert!(
            !out.contains("field0"),
            "all `field0` occurrences must be rewritten, got:\n{out}"
        );
        // Structural lines preserved.
        assert!(out.contains("sealed class FormatMetadata"));
        assert!(out.contains("FormatMetadata_Pdf"));
    }

    #[test]
    fn idempotent_when_run_twice() {
        let input = r#"  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
"#;
        let once = rewrite_frb_sealed_variants(input);
        let twice = rewrite_frb_sealed_variants(&once);
        assert_eq!(once, twice, "rewriter must be idempotent");
    }

    #[test]
    fn multiple_distinct_sealed_class_variants_all_rewritten() {
        // Two distinct sealed classes in the same source file — the regex must
        // handle multiple non-overlapping matches without interference.
        let input = r#"sealed class FormatMetadata with _$FormatMetadata {
  const FormatMetadata._();

  const factory FormatMetadata.pdf({required PdfMetadata field0}) =
      FormatMetadata_Pdf;
  const factory FormatMetadata.docx({required DocxMetadata field0}) =
      FormatMetadata_Docx;
}

sealed class OutputFormat with _$OutputFormat {
  const OutputFormat._();

  const factory OutputFormat.custom({required String field0}) =
      OutputFormat_Custom;
  const factory OutputFormat.json({required JsonConfig field0}) =
      OutputFormat_Json;
}
"#;
        let out = rewrite_frb_sealed_variants(input);
        // FormatMetadata variants
        assert!(
            out.contains("required PdfMetadata metadata"),
            "PdfMetadata should become metadata, got:\n{out}"
        );
        assert!(
            out.contains("required DocxMetadata metadata"),
            "DocxMetadata should become metadata, got:\n{out}"
        );
        // OutputFormat variants
        assert!(
            out.contains("required String value"),
            "String should become value, got:\n{out}"
        );
        // `JsonConfig` shares the `Json` prefix with variant `Json`, so the remainder
        // `Config` is lower-cased to `config`.
        assert!(
            out.contains("required JsonConfig config"),
            "JsonConfig payload (Json prefix → Config remainder) should become `config`, got:\n{out}"
        );
        assert!(!out.contains("field0"), "no `field0` should remain, got:\n{out}");
    }
}
