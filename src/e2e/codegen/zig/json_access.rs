use crate::e2e::field_access::{FieldResolver, push_owner_segment};

#[derive(Clone, Copy)]
pub(super) enum Unwrap {
    Panic,
    Error,
}

/// Error returned by a wildcard element check when the element lacks the field. ~keep
const WILDCARD_MISSING_FIELD_ERROR: &str = "error.WildcardElementFieldMissing";

pub(super) fn json_path_expr_with(
    result_var: &str,
    anchor_prefix: &str,
    field_path: &str,
    unwrap: Unwrap,
    field_resolver: &FieldResolver,
) -> String {
    if anchor_prefix.is_empty()
        && let Some(expression) = super::enum_collection::accessor(result_var, field_path, field_resolver)
    {
        return expression;
    }
    let segments: Vec<&str> = field_path.split('.').collect();
    let mut expr = result_var.to_string();
    let mut owner_path = anchor_prefix.to_string();
    for seg in &segments {
        let bare = seg.split('[').next().unwrap_or(seg);
        // Skip a segment naming a variant of an internally-tagged enum: serde flattens such a
        // variant's fields beside the discriminator, so no JSON key for the variant name
        // exists to look up. Answered from the consumer's IR — see
        // `FieldResolver::is_internally_tagged_variant_segment`. ~keep
        if field_resolver.is_internally_tagged_variant_segment(&owner_path, bare) {
            push_owner_segment(&mut owner_path, bare);
            continue;
        }
        // Handle array accessor notation:
        //   "links[]"     → bare trailing wildcard with no element field; falls back to
        //                   element 0. A wildcard with an element field ("links[].url")
        //                   never reaches here — the caller lowers it to a loop instead.
        //   "results[0]"  → access the array, then specific index N. ~keep
        if let Some(key) = seg.strip_suffix("[]") {
            expr = format!(
                "{}.array.items[0]",
                json_get(&expr, &owner_path, key, unwrap, field_resolver)
            );
        } else if let Some(bracket_pos) = seg.find('[') {
            if let Some(end_pos) = seg.find(']')
                && end_pos > bracket_pos + 1
                && end_pos == seg.len() - 1
            {
                let key = &seg[..bracket_pos];
                let idx = &seg[bracket_pos + 1..end_pos];
                if idx.chars().all(|c| c.is_ascii_digit()) {
                    expr = format!(
                        "{}.array.items[{idx}]",
                        json_get(&expr, &owner_path, key, unwrap, field_resolver)
                    );
                    push_owner_segment(&mut owner_path, bare);
                    continue;
                }
                // Non-numeric bracket: HashMap<String, _> key access. FRB / serde
                // serialize maps as JSON objects, so `field[key]` resolves to
                // `.object.get("field").?.object.get("key").?`. Used by nested fixture objects.
                // `metadata.document.open_graph[title]` alias pattern where
                // `open_graph` is a `HashMap<String, String>`.
                expr = json_get(
                    &json_get(&expr, &owner_path, key, unwrap, field_resolver),
                    &owner_path,
                    idx,
                    unwrap,
                    field_resolver,
                );
                push_owner_segment(&mut owner_path, bare);
                continue;
            }
            expr = json_get(&expr, &owner_path, seg, unwrap, field_resolver);
        } else {
            expr = json_get(&expr, &owner_path, seg, unwrap, field_resolver);
        }
        push_owner_segment(&mut owner_path, bare);
    }
    expr
}

/// One `.object.get("key")` step, unwrapped according to `unwrap`.
///
/// `key` may be a JSON key serde omits entirely for some values even though the
/// underlying Rust field is always present (`#[serde(skip_serializing_if = "...")]` on a
/// required `Vec<T>`/`HashMap<K, V>`, or on an `Option<T>` alongside `Option::is_none`) —
/// see `FieldDef::serde_skip_serializing_if` and `FieldResolver::is_wire_optional_key`.
/// `.?`/`orelse return err` both assume the key is present, which panics/fails the whole
/// test on exactly the values that legitimately triggered the skip. Substituting a null value
/// for a missing wire-optional key is safe regardless of the caller's `unwrap` mode: it
/// matches how this same template already treats a *present* `null` value (the
/// `is_empty`/`not_empty`/`is_true`/`is_false` branches all special-case `.null`), so a
/// wire-optional field renders the same assertion outcome whether serde wrote `null` or
/// omitted the key outright. The fallback must be `std.json.Value{ .null = {} }`, not the bare
/// `.null` enum literal: chained straight into further `.object.get(...)` with no declaration
/// in between, a bare literal has no result type for Zig's peer resolution — an uncaught
/// non-compiling regression in 0.62.7. ~keep
fn json_get(expr: &str, owner_path: &str, key: &str, unwrap: Unwrap, field_resolver: &FieldResolver) -> String {
    let field_path = if owner_path.is_empty() {
        key.to_string()
    } else {
        format!("{owner_path}.{key}")
    };
    // The anchored answer wins whenever the IR resolves this step: a bare-name hit must not
    // mark a required field absent just because an unrelated type skips a field of the same
    // name (alef#899). Unresolvable steps keep the historic bare-name guard. ~keep
    let wire_optional = field_resolver
        .is_wire_optional_at(&field_path)
        .unwrap_or_else(|| field_resolver.is_wire_optional_key(key));
    if wire_optional {
        return format!("({expr}.object.get(\"{key}\") orelse std.json.Value{{ .null = {{}} }})");
    }
    match unwrap {
        Unwrap::Panic => format!("{expr}.object.get(\"{key}\").?"),
        Unwrap::Error => format!("({expr}.object.get(\"{key}\") orelse return {WILDCARD_MISSING_FIELD_ERROR})"),
    }
}
