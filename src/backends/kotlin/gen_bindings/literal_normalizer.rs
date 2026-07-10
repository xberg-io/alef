//! Post-processing fixes for Kotlin generated code.
//!
//! Handles issues that cannot be fixed at codegen time due to constraints.
//! This module applies regex-based fixes to generated .kt files.

use regex::Regex;

/// Fix integer-like float literals in Kotlin code.
///
/// Kotlin requires float literals to have a decimal point or exponent.
/// Rust's f64 Display drops trailing zeros (e.g., "32.0" becomes "32"),
/// resulting in invalid Kotlin code like `val field: Double = 32`.
///
/// This post-processor finds patterns like `: Double = <digit>` and
/// converts them to `: Double = <digit>.0`.
pub fn fix_float_literals(content: &str) -> String {
    let double_pattern = Regex::new(r"(: Double = )(\d+)([^0-9.])").expect("invalid regex");
    let content = double_pattern.replace_all(content, "${1}${2}.0${3}").into_owned();

    let double_eol_pattern = Regex::new(r"(: Double = )(\d+)$").expect("invalid regex");
    let content = double_eol_pattern.replace_all(&content, "${1}${2}.0").into_owned();

    let float_pattern = Regex::new(r"(: Float = )(\d+)(f)").expect("invalid regex");
    float_pattern.replace_all(&content, "${1}${2}.0${3}").into_owned()
}
