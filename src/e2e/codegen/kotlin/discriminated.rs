//! Kotlin discriminated-union (sealed class) assertion helpers.
//!
//! Mirrors `csharp/discriminated.rs` but emits Kotlin `is` pattern matching
//! against `FormatMetadata` subclasses.  Sealed class subclasses expose the
//! payload as a single `metadata` property (see `FormatMetadata.Excel(val metadata: ExcelMetadata)`),
//! so the inner field is accessed as `variant.metadata.<innerCamelCase>`.

use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

use crate::e2e::escape::escape_kotlin;
use crate::e2e::fixture::Assertion;

use super::values::json_to_kotlin;

/// Detect if a field path navigates a discriminated union variant.
/// Pattern: `metadata.format.<variant_name>(.<inner_field>)?`
/// Returns: Some((variant_pascal, inner_field_snake)) if matched.
pub(super) fn parse_discriminated_union_access(field: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = field.split('.').collect();
    if !(parts.len() == 3 || parts.len() == 4) {
        return None;
    }
    if parts[0] != "metadata" || parts[1] != "format" {
        return None;
    }
    let variant_name = parts[2];
    let known_variants = [
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
        "code",
    ];
    if !known_variants.contains(&variant_name) {
        return None;
    }
    let variant_pascal = variant_name.to_upper_camel_case();
    let inner_field = if parts.len() == 4 {
        parts[3].to_string()
    } else {
        String::new()
    };
    Some((variant_pascal, inner_field))
}

/// Render an assertion against a sealed-class variant's inner field.
///
/// `variant_var` is the bound name from `is FormatMetadata.<Variant> -> { … }`
/// (e.g. `format_excel`).  Sealed-class subclasses expose their payload as
/// `.metadata`, so the field expression is `variant_var.metadata.<innerCamelCase>`.
pub(super) fn render_discriminated_union_assertion(
    out: &mut String,
    assertion: &Assertion,
    variant_var: &str,
    inner_field: &str,
) {
    if inner_field.is_empty() {
        return;
    }

    let field_camel = inner_field.to_lower_camel_case();
    // The variant payload field (`variant.metadata.<inner>`) is frequently Optional in the
    // alef-generated Kotlin types (e.g. `ExcelMetadata.sheetCount: Int?`).  The fixture
    // assertion only fires when the variant matched, so a null inner field would itself be
    // a test failure — assert non-null with `!!.` before the comparison so kotlinc accepts
    // arithmetic and ordering operators (`>=`, `>`, etc.) on the receiver.
    let field_expr = format!("{variant_var}.metadata.{field_camel}!!");

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let kt_val = json_to_kotlin(expected);
                if expected.is_string() {
                    let _ = writeln!(
                        out,
                        "                assertEquals({kt_val}, {field_expr}.trim(), \"expected: {}\")",
                        escape_kotlin(expected.as_str().unwrap_or(""))
                    );
                } else if expected.as_bool() == Some(true) {
                    let _ = writeln!(out, "                assertTrue({field_expr}, \"expected true\")");
                } else if expected.as_bool() == Some(false) {
                    let _ = writeln!(out, "                assertFalse({field_expr}, \"expected false\")");
                } else {
                    let _ = writeln!(
                        out,
                        "                assertEquals({kt_val}, {field_expr}, \"expected: {kt_val}\")"
                    );
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} >= {kt_val}, \"expected >= {kt_val}\")"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} <= {kt_val}, \"expected <= {kt_val}\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} > {kt_val}, \"expected > {kt_val}\")"
                );
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let kt_val = json_to_kotlin(val);
                let _ = writeln!(
                    out,
                    "                assertTrue({field_expr} < {kt_val}, \"expected < {kt_val}\")"
                );
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                if let Some(s) = expected.as_str() {
                    let lower = s.to_lowercase();
                    let _ = writeln!(
                        out,
                        "                assertTrue({field_expr}.orEmpty().toString().lowercase().contains(\"{}\".lowercase()), \"expected to contain: {}\")",
                        escape_kotlin(&lower),
                        escape_kotlin(s)
                    );
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    if let Some(s) = val.as_str() {
                        let lower = s.to_lowercase();
                        let _ = writeln!(
                            out,
                            "                assertTrue({field_expr}.orEmpty().toString().lowercase().contains(\"{}\".lowercase()), \"expected to contain: {}\")",
                            escape_kotlin(&lower),
                            escape_kotlin(s)
                        );
                    }
                }
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr}.toString().isNotEmpty(), \"expected non-empty value\")"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "                assertTrue({field_expr}.toString().isEmpty(), \"expected empty value\")"
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "                // skipped: assertion type '{}' not yet supported for discriminated union fields",
                assertion.assertion_type
            );
        }
    }
}
