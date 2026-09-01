use std::collections::HashMap;

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// Whether `path`'s FINAL segment is an indexed array element (`foo[0]`) rather than the
/// collection itself.
///
/// ~keep Mirrors `FieldResolver::rust_unwrap_binding`'s `leaf_is_indexed_element`: `is_optional`/
/// `is_array`/`target_field_is_pointer` all resolve a field NAME after bracket-stripping (via
/// `parse_path`/`segment_name`), so `detected_languages` and `detected_languages[0]` answer
/// IDENTICALLY -- both report whatever `Option<Vec<String>>` says about the COLLECTION. That is
/// correct for the bare collection but wrong for an element already reached through an index: a
/// trailing `[0]` has already consumed the container's own optionality (a valid index only exists
/// once the `Option` and the slice are non-empty), so the element itself is a concrete `string`,
/// never nilable. Without this guard `resolve_assertion_field_shape` pushed the collection's
/// `is_optional` onto the element, producing `result.DetectedLanguages[0] == nil` against a plain
/// Go `string` -- `invalid operation: mismatched types string and untyped nil`.
fn leaf_is_indexed_element(path: &str) -> bool {
    let Some(open) = path.rfind('[') else {
        return false;
    };
    path.ends_with(']')
        && open + 1 < path.len() - 1
        && path[open + 1..path.len() - 1].bytes().all(|b| b.is_ascii_digit())
}

pub(super) struct AssertionFieldShape {
    pub is_optional: bool,
    pub is_pointer: bool,
    pub is_nullable: bool,
    pub is_array_for_len: bool,
    pub is_slice: bool,
    pub is_data_interface: bool,
}

pub(super) fn resolve_assertion_field_shape(
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    optional_locals: &HashMap<String, String>,
) -> AssertionFieldShape {
    let Some(field) = assertion.field.as_deref() else {
        return AssertionFieldShape {
            is_optional: false,
            is_pointer: false,
            is_nullable: false,
            is_array_for_len: false,
            is_slice: false,
            is_data_interface: false,
        };
    };
    let resolved = field_resolver.resolve(field);
    let check_path = resolved
        .strip_suffix(".length")
        .or_else(|| resolved.strip_suffix(".count"))
        .or_else(|| resolved.strip_suffix(".size"))
        .unwrap_or(resolved);
    let uses_plain_local = optional_locals.contains_key(field);
    let leaf_is_indexed_element = leaf_is_indexed_element(check_path);
    let is_optional = !leaf_is_indexed_element && field_resolver.is_optional(check_path) && !uses_plain_local;
    let is_array_for_len = !leaf_is_indexed_element && field_resolver.is_array(check_path);
    let is_slice = !leaf_is_indexed_element && field_resolver.is_array(resolved);
    // ~keep `target_field_is_pointer` is the authoritative answer: it reads the same
    // `go_struct_field_type` the Go binding backend itself emits from (see
    // `ir_result_fields::pointer_at_path`). `None` means that walk could not resolve `check_path`
    // at all -- NOT "the field is pointer-shaped" -- so guessing `is_optional && !is_array_for_len`
    // here treated a resolution failure as a successful "yes": an `Option<Vec<T>>` result field
    // Go always flattens to a plain nilable slice (never a pointer) got dereferenced as
    // `len(*result.Chunks)`, a Go compile error (`cannot indirect ... variable of type []T`).
    // Defaulting to `false` never adds a spurious `*`; the worst case is a missing deref the IR
    // could not prove was needed, which is a comparison against the wrong Go type and fails to
    // build immediately, rather than silently asserting nothing.
    let is_pointer = !leaf_is_indexed_element
        && !uses_plain_local
        && field_resolver.target_field_is_pointer(check_path).unwrap_or(false);

    AssertionFieldShape {
        is_optional,
        is_pointer,
        is_nullable: is_optional || is_pointer,
        is_array_for_len,
        is_slice,
        is_data_interface: field_resolver.target_field_is_data_interface(check_path),
    }
}
