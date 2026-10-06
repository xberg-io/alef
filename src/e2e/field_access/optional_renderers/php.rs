//! PHP accessor renderers: `->getFoo()` vs `->foo` selected from the owner type's getter map.

use super::super::renderers::quoted_key_literal;
use super::super::types::{PathSegment, PhpGetterMap};
use super::{push_key_field_name, push_key_index_suffix};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

pub(in crate::e2e::field_access) fn render_php(segments: &[PathSegment], result_var: &str) -> String {
    let mut out = result_var.to_string();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                out.push_str("->");
                // PHP properties are camelCase (per #[php(prop, name = "...")]),
                // so convert snake_case field names to camelCase.
                out.push_str(&f.to_lower_camel_case());
            }
            PathSegment::ArrayField { name, index } => {
                if !name.is_empty() {
                    out.push_str("->");
                    out.push_str(&name.to_lower_camel_case());
                }
                out.push_str(&format!("[{index}]"));
            }
            PathSegment::MapAccess { field, key } => {
                out.push_str("->");
                out.push_str(&field.to_lower_camel_case());
                out.push_str(&format!("[{}]", quoted_key_literal(key)));
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("count({current})");
            }
        }
    }
    out
}

/// PHP accessor that distinguishes between scalar fields (property access: `->camelCase`)
/// and non-scalar fields (getter-method access: `->getCamelCase()`).
///
/// ext-php-rs 0.15.x exposes scalar fields via `#[php(prop)]` as PHP properties, but
/// non-scalar fields (Named structs, `Vec<Named>`, `Map`, etc.) require a `#[php(getter)]`
/// method because `get_method_props` is unimplemented in ext-php-rs-derive 0.11.7.
/// The generated getter method name is `get{CamelCase}` (stripping the `get_` prefix and
/// converting the camelCase remainder to a PHP property name), so e2e assertions must call
/// `->getCamelCase()` for those fields.
///
/// `getter_map` carries the per-`(owner_type, field_name)` classification along with the
/// chain-resolution metadata required to walk multi-segment paths through the IR's nested
/// type graph. Each path segment is classified using the *current* owner type, then the
/// owner cursor advances to the field's referenced `Named` type (if any) for the next
/// segment. When `root_type` is unset the renderer falls back to the legacy bare-name
/// union, which is unsafe but preserves backwards compatibility for callers that have
/// not wired type resolution.
pub(in crate::e2e::field_access) fn render_php_with_getters(
    segments: &[PathSegment],
    result_var: &str,
    getter_map: &PhpGetterMap,
    optional_fields: &HashSet<String>,
) -> String {
    render_php_with_getters_from_owner(
        segments,
        result_var,
        getter_map,
        optional_fields,
        getter_map.root_type.clone(),
    )
}

/// [`render_php_with_getters`], but for a path that is already relative to a bound collection
/// element (the closure/loop variable a wildcard fixture path expands to) rather than to the
/// call's result variable -- `owner_type` is the IR type of THAT element, resolved by
/// [`php_element_owner_type`], not `getter_map.root_type`. Mirrors
/// `render_python_element_with_optionals` for the getter-vs-property classification: a result
/// envelope and its collection elements can classify independently (a scalar `#[php(prop)]` on
/// one, a `#[php(getter)]`-only field of the same name on the other), so starting the element
/// cursor at the result root answers the wrong owner's question.
pub(in crate::e2e::field_access) fn render_php_element_with_getters(
    segments: &[PathSegment],
    element_var: &str,
    getter_map: &PhpGetterMap,
    optional_fields: &HashSet<String>,
    owner_type: Option<String>,
) -> String {
    render_php_with_getters_from_owner(segments, element_var, getter_map, optional_fields, owner_type)
}

/// The IR type that owns the ELEMENTS of `array_segments` -- e.g. `"StructureItem"` for a
/// `structure: Vec<StructureItem>` field -- walking `getter_map.field_types` from
/// `getter_map.root_type` through every segment of the array field's own path, exactly the way
/// `render_php_with_getters`'s cursor advances. Mirrors `python_element_owner_type`; PHP has no
/// map-value-edges structure to consult because `PhpGetterMap::advance` already looks up the next
/// owner by field name alone regardless of segment kind, so a `map[key]` hop advances the same way
/// a plain field does.
pub(in crate::e2e::field_access) fn php_element_owner_type(
    array_segments: &[PathSegment],
    getter_map: &PhpGetterMap,
) -> Option<String> {
    let mut current_type = getter_map.root_type.clone();
    for segment in array_segments {
        let field_name = match segment {
            PathSegment::Field(name) | PathSegment::ArrayField { name, .. } => name.as_str(),
            PathSegment::MapAccess { field, .. } => field.as_str(),
            PathSegment::Length => continue,
        };
        current_type = getter_map.advance(current_type.as_deref(), field_name);
    }
    current_type
}

/// Appends `{arrow}{member}`, calling the `get{CamelCase}()` accessor when the owner type needs
/// one. Non-scalar fields get an ext-php-rs `get_{camelCase}` method ident, which PHP sees as
/// `->get{CamelCase}()` because the `get_` prefix is stripped when the property name is derived.
fn push_php_member(out: &mut String, arrow: &str, name: &str, getter_map: &PhpGetterMap, current_type: Option<&str>) {
    let camel = name.to_lower_camel_case();
    out.push_str(arrow);
    if getter_map.needs_getter(current_type, name) {
        out.push_str(&format!("get{}", camel.as_str()[..1].to_uppercase() + &camel[1..]));
        out.push_str("()");
    } else {
        out.push_str(&camel);
    }
}

fn render_php_with_getters_from_owner(
    segments: &[PathSegment],
    result_var: &str,
    getter_map: &PhpGetterMap,
    optional_fields: &HashSet<String>,
    owner_type: Option<String>,
) -> String {
    let mut out = result_var.to_string();
    let mut current_type: Option<String> = owner_type;
    let mut path_so_far = String::new();
    // Sticky, same convention as `render_php`: once any segment in the chain is optional,
    // every following access must null-safe-navigate off it.
    let mut prev_was_nullable = false;
    for seg in segments {
        let arrow = if prev_was_nullable { "?->" } else { "->" };
        match seg {
            PathSegment::Field(f) => {
                push_key_field_name(&mut path_so_far, seg);
                push_php_member(&mut out, arrow, f, getter_map, current_type.as_deref());
                current_type = getter_map.advance(current_type.as_deref(), f.as_str());
                prev_was_nullable = prev_was_nullable || optional_fields.contains(&path_so_far);
            }
            PathSegment::ArrayField { name, index } => {
                push_key_field_name(&mut path_so_far, seg);
                // A root-array segment (`name` empty, e.g. `[0].id`) has no getter/property to
                // resolve — `result_var` IS the array — and `current_type` does not advance
                // past it. ~keep
                if !name.is_empty() {
                    push_php_member(&mut out, arrow, name, getter_map, current_type.as_deref());
                    current_type = getter_map.advance(current_type.as_deref(), name.as_str());
                }
                out.push_str(&format!("[{index}]"));
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || optional_fields.contains(&path_so_far);
            }
            PathSegment::MapAccess { field, key } => {
                push_key_field_name(&mut path_so_far, seg);
                push_php_member(&mut out, arrow, field, getter_map, current_type.as_deref());
                out.push_str(&format!("[{}]", quoted_key_literal(key)));
                current_type = getter_map.advance(current_type.as_deref(), field.as_str());
                push_key_index_suffix(&mut path_so_far, seg);
                prev_was_nullable = prev_was_nullable || optional_fields.contains(&path_so_far);
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("count({current})");
            }
        }
    }
    out
}
