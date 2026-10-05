//! Composite-literal pieces of Go DTO rendering that do not depend on the surrounding struct.
//!
//! Split out of `setup.rs` to keep that file under this crate's file-length limit. ~keep

use crate::e2e::escape::go_string_literal;

use super::super::json_values::json_to_go;
use super::{GoFieldSite, GoValueContext, go_named_field_expression, qualified_go_type};

/// Lower a fixture array into a Go slice literal of the element type the binding declares.
///
/// Go emits `Vec<T>` as `[]T` and leaves it unpointered even for an optional field
/// (`go_optional_type`), so unlike the scalar arms this never wraps the result in `ptr(..)`.
/// A multi-line result is relative to its own first column, like every other nested literal:
/// the enclosing literal shifts its continuation lines one tab deeper. `Ok(None)` omits the whole field -- a value that is not an array, or an element with no
/// expression of the declared element type -- because dropping one element would change the
/// request the snippet documents. ~keep
pub(super) fn go_slice_expression(
    element: &crate::core::ir::TypeRef,
    value: &serde_json::Value,
    context: GoValueContext<'_>,
    site: GoFieldSite<'_>,
) -> anyhow::Result<Option<String>> {
    let Some(items) = value.as_array() else {
        return Ok(None);
    };
    let element_type = match element {
        crate::core::ir::TypeRef::String | crate::core::ir::TypeRef::Path => "string".to_string(),
        crate::core::ir::TypeRef::Primitive(_) => {
            crate::backends::go::type_map::go_struct_field_type(element).into_owned()
        }
        crate::core::ir::TypeRef::Named(name) => qualified_go_type(context.import_alias, name),
        _ => return Ok(None),
    };
    let mut rendered = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let item_pointer = format!("{}/{index}", site.pointer);
        let item_site = GoFieldSite {
            pointer: &item_pointer,
            ..site
        };
        let expression = match element {
            crate::core::ir::TypeRef::Named(name) => go_named_field_expression(item, name, context, item_site, false)?,
            crate::core::ir::TypeRef::Primitive(_) if item.is_number() || item.is_boolean() => json_to_go(item),
            crate::core::ir::TypeRef::String | crate::core::ir::TypeRef::Path => match item.as_str() {
                Some(text) => go_string_literal(text),
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        rendered.push(expression);
    }
    if rendered.iter().all(|expression| !expression.contains('\n')) {
        return Ok(Some(format!("[]{element_type}{{{}}}", rendered.join(", "))));
    }
    let elements = rendered
        .iter()
        .map(|expression| format!("\t{},", expression.replace('\n', "\n\t")))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(format!("[]{element_type}{{\n{elements}\n}}")))
}

/// The number of spaces to pad each field name with so `gofmt` leaves the literal alone.
///
/// `gofmt` aligns values only across consecutive single-line fields: a multi-line value forms a
/// section of its own, so padding a neighbouring key out to its width is a rewrite `gofmt` would
/// undo. ~keep
pub(super) fn field_paddings(field_values: &[(String, String)]) -> Vec<usize> {
    let mut paddings = vec![0usize; field_values.len()];
    let mut run_start = 0;
    while run_start < field_values.len() {
        if field_values[run_start].1.contains('\n') {
            run_start += 1;
            continue;
        }
        let run_end = field_values[run_start..]
            .iter()
            .position(|(_, expression)| expression.contains('\n'))
            .map_or(field_values.len(), |offset| run_start + offset);
        let widest = field_values[run_start..run_end]
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or_default();
        for (padding, (name, _)) in paddings[run_start..run_end]
            .iter_mut()
            .zip(&field_values[run_start..run_end])
        {
            *padding = widest - name.len();
        }
        run_start = run_end;
    }
    paddings
}
