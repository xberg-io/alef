use std::fmt::Write;

pub(super) fn string_value_expression(field: &str, is_pointer: bool, is_data_interface: bool) -> String {
    if is_data_interface {
        format!("jsonString(t, {field})")
    } else if is_pointer {
        format!("string(*{field})")
    } else {
        format!("string({field})")
    }
}

pub(super) fn contains_value_expression(
    field: &str,
    is_pointer: bool,
    is_array: bool,
    is_data_interface: bool,
) -> String {
    if is_data_interface || is_array {
        format!("jsonString(t, {field})")
    } else {
        string_value_expression(field, is_pointer, false)
    }
}

pub(super) fn render_guarded_scalar_comparison(
    out: &mut String,
    guard: Option<&str>,
    field_expr: &str,
    operator: &str,
    comparison_value: &str,
    expected_message: &str,
) -> bool {
    let Some(guard) = guard else {
        return false;
    };
    let _ = writeln!(out, "\tif {guard} != nil {{");
    let _ = writeln!(out, "\t\tif {field_expr} {operator} {comparison_value} {{");
    let _ = writeln!(
        out,
        "\t\t\tt.Errorf(\"expected {expected_message}, got %v\", {field_expr})"
    );
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t}}");
    true
}

/// Grouping for `render_count_assertion`'s boolean options, kept out of the parameter list
/// itself so the function stays under the 6-parameter lint ceiling.
pub(super) struct CountAssertionShape {
    pub(super) is_slice: bool,
    pub(super) exact: bool,
    /// Whether a sibling `not_empty` assertion on the same field already fails the test
    /// when the field is nil -- see `AssertionRenderContext::presence_checked_fields`.
    pub(super) has_sibling_presence_check: bool,
}

pub(super) fn render_count_assertion(
    out: &mut String,
    field: &str,
    count: u64,
    nullable_guard: Option<&str>,
    shape: CountAssertionShape,
) {
    let CountAssertionShape {
        is_slice,
        exact,
        has_sibling_presence_check,
    } = shape;
    let (method, message) = if exact {
        ("Equal", format!("expected exactly {count} elements"))
    } else {
        ("GreaterOrEqual", format!("expected at least {count} elements"))
    };
    let is_length = field.starts_with("len(");
    let length = if is_length {
        field.to_string()
    } else if nullable_guard.is_some() && !is_slice {
        format!("len(*{field})")
    } else {
        format!("len({field})")
    };
    match nullable_guard {
        // `is_length` means `field` is already a `len(...)`/pseudo `.length`/`.count`/`.size`
        // expression measuring a derived scalar off `guard` (e.g. a string's length behind an
        // optional pointer) -- `guard` here is NOT the collection this assertion is about, it
        // is an unrelated optional value the measurement happens to be taken through. Nil is a
        // legitimate "not populated" state with no presence claim attached, the same semantics
        // `render_guarded_scalar_comparison` already applies to optional scalars like
        // `QualityScore`. Only a *named* collection field (`is_length == false`, e.g.
        // `elements`, `chunks`, `detected_languages`) is the thing a count assertion exists to
        // guarantee produced something, so only that shape gets the failing `else` below. ~keep
        Some(guard) if is_length => {
            let _ = writeln!(out, "\tif {guard} != nil {{");
            let _ = writeln!(out, "\t\tassert.{method}(t, {length}, {count}, \"{message}\")");
            let _ = writeln!(out, "\t}}");
        }
        // A sibling `not_empty` assertion on the same field already fails the test when
        // `guard` is nil (`render_not_empty`), so an `else` here would only duplicate
        // that failure -- stay guard-only. ~keep
        Some(guard) if has_sibling_presence_check => {
            let _ = writeln!(out, "\tif {guard} != nil {{");
            let _ = writeln!(out, "\t\tassert.{method}(t, {length}, {count}, \"{message}\")");
            let _ = writeln!(out, "\t}}");
        }
        // No sibling presence assertion covers this field, and it is a real named collection
        // (not a pseudo-length measurement): without an `else`, a nil guard would make the
        // count assertion silently not run and the test would pass on exactly the regression
        // it exists to catch. Fail instead. ~keep
        Some(guard) => {
            let _ = writeln!(out, "\tif {guard} != nil {{");
            let _ = writeln!(out, "\t\tassert.{method}(t, {length}, {count}, \"{message}\")");
            let _ = writeln!(out, "\t}} else {{");
            let _ = writeln!(out, "\t\tt.Errorf(\"{message}, got nil\")");
            let _ = writeln!(out, "\t}}");
        }
        None => {
            let _ = writeln!(out, "\tassert.{method}(t, {length}, {count}, \"{message}\")");
        }
    }
}

pub(super) fn render_length_assertion(
    out: &mut String,
    field: &str,
    length: u64,
    nullable_guard: Option<&str>,
    is_pointer: bool,
    minimum: bool,
) {
    let (method, relation) = if minimum {
        ("GreaterOrEqual", ">=")
    } else {
        ("LessOrEqual", "<=")
    };
    let expression = if field.starts_with("len(") {
        field.to_string()
    } else if is_pointer {
        format!("len(*{field})")
    } else {
        format!("len({field})")
    };
    if let Some(guard) = nullable_guard {
        let _ = writeln!(out, "\tif {guard} != nil {{");
        let _ = writeln!(
            out,
            "\t\tassert.{method}(t, {expression}, {length}, \"expected length {relation} {length}\")"
        );
        let _ = writeln!(out, "\t}}");
    } else {
        let _ = writeln!(
            out,
            "\tassert.{method}(t, {expression}, {length}, \"expected length {relation} {length}\")"
        );
    }
}
