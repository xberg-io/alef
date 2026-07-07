use crate::codegen::keywords::swift_ident;
use crate::e2e::field_access::FieldResolver;
use heck::ToLowerCamelCase;
use std::collections::HashMap;

/// Build a Swift accessor path for the given fixture field, inserting `()` on
/// every segment and `?` after every optional non-leaf segment.
///
/// This is the core helper for count/contains helpers that need to reconstruct
/// the path with correct optional chaining from the raw fixture field name.
///
/// Rewrite a Swift accessor expression to capture any `RustVec` temporaries
/// in a local before subscripting them. Returns `(setup_lines, rewritten_expr)`.
///
/// swift-bridge's `Vec_<T>$get` returns a raw pointer into the Vec's storage
/// wrapped in a `T.SelfRef`. If the Vec was a temporary, ARC may release it
/// before the ref is dereferenced, leaving the pointer dangling and reads
/// returning empty/garbage. Hoisting the Vec into a `let` binding ties the
/// Vec's lifetime to the enclosing function scope, so the ref stays valid.
///
/// Only the first `()[...]` occurrence per expression is materialised — that
/// covers all current fixture access patterns (single-level subscripts on a
/// result field). Nested subscripts are rare and would need a more elaborate
/// pass; if they appear, this returns conservative output (just the first
/// hoist) which is still correct.
/// Returns `(setup_lines, rewritten_expr, is_map_subscript)`. `is_map_subscript` is
/// true when the subscript key was a string literal, indicating the parent
/// accessor returns a JSON-encoded Map (RustString) and the rewritten expression
/// already evaluates to `String?` so callers should NOT append `.toString()`.
pub(super) fn materialise_vec_temporaries(expr: &str, name_suffix: &str) -> (Vec<String>, String, bool) {
    let Some(idx) = expr.find("()[") else {
        return (Vec::new(), expr.to_string(), false);
    };
    let after_open = idx + 3; // position after `()[`
    let Some(close_rel) = expr[after_open..].find(']') else {
        return (Vec::new(), expr.to_string(), false);
    };
    let subscript_end = after_open + close_rel; // index of `]`
    let prefix = &expr[..idx + 2]; // includes `()`
    let subscript = &expr[idx + 2..=subscript_end]; // `[N]`
    let tail = &expr[subscript_end + 1..]; // everything after `]`
    let method_dot = expr[..idx].rfind('.').unwrap_or(0);
    let method = &expr[method_dot + 1..idx];
    let local = format!("_vec_{}_{}", method, name_suffix);

    // String-key subscript (e.g. `["title"]`) signals a Map-like access. swift-bridge
    // serialises non-leaf Maps (e.g. `HashMap<String, String>`) as JSON-encoded
    // RustString rather than exposing a Swift dictionary. Decode the RustString to
    // `[String: String]` before subscripting so `_vec_X["title"]` works.
    let inner = subscript.trim_start_matches('[').trim_end_matches(']');
    let is_string_key = inner.starts_with('"') && inner.ends_with('"');
    let setup = if is_string_key {
        format!(
            "let {local} = (try? JSONSerialization.jsonObject(with: ({prefix}.toString() ?? \"{{}}\").data(using: .utf8)!) as? [String: String]) ?? [:]"
        )
    } else {
        format!("let {local} = {prefix}")
    };

    let rewritten = format!("{local}{subscript}{tail}");
    (vec![setup], rewritten, is_string_key)
}

/// Returns `(accessor_expr, has_optional)` where `has_optional` is true when
/// at least one `?.` was inserted.
///
/// Note: Once we emit a `?` to unwrap an Optional, Swift's type system treats
/// the result as non-Optional for the remainder of the chain, even if the Rust
/// IR type annotation says the next field is Optional. We track whether the chain
/// is already in an "unwrapped" state via `already_unwrapped` — after the first
/// `?`, subsequent optional fields should NOT emit another `?` because the Swift
/// expression is already concrete.
pub(super) fn swift_build_accessor(field: &str, result_var: &str, field_resolver: &FieldResolver) -> (String, bool) {
    let resolved = field_resolver.resolve(field);
    let parts: Vec<&str> = resolved.split('.').collect();

    // Track the current IR type as we walk segments so each segment can be
    // emitted with property syntax (first-class Codable struct) or method-call
    // syntax (typealias-to-`RustBridge.X`). Mirrors the per-segment dispatch in
    // `render_swift_with_first_class_map`.
    let mut current_type: Option<String> = field_resolver.swift_root_type().cloned();
    // Once a chain crosses a `[N]` subscript, we are operating on a RustVec
    // element, which is always the OPAQUE `RustBridge.T` (swift-bridge does not
    // convert RustVec elements into the first-class Codable struct). Pin
    // opaque method-call syntax after the first index step.
    let mut via_rust_vec = false;
    // Once a chain crosses an opaque (typealias-to-`RustBridge.X`) segment, every
    // subsequent accessor must also be opaque (method-call syntax). Calling a
    // method on `RustBridge.X` returns the OPAQUE wrapper of the next type, even
    // when that next type is independently eligible for first-class emission.
    // See `field_access::render_swift_with_first_class_map` for the matching
    // invariant. Without this, `metrics.total_lines` on an opaque parent emits
    // `.metrics().totalLines` instead of `.metrics().totalLines()`.
    let mut via_opaque = false;

    let mut out = result_var.to_string();
    let mut has_optional = false;
    // Once we emit a `?` to unwrap an Optional, subsequent segments should NOT
    // emit additional `?` operators. In Swift, `.summary()?.strategy()` unwraps
    // to a concrete `SummaryResult`, so `.strategy()` is called on the unwrapped
    // value and does not need another `?` even if the full path `summary.strategy`
    // is marked optional in the fixture config.
    let mut already_unwrapped = false;
    let mut path_so_far = String::new();
    let total = parts.len();
    for (i, part) in parts.iter().enumerate() {
        let is_leaf = i == total - 1;
        // Handle array index subscripts within a segment, e.g. `data[0]`.
        // `data[0]` must become `.data()[0]` (opaque) or `.data[0]` (first-class).
        // Split at the first `[` if present.
        let (field_name, subscript): (&str, Option<&str>) = if let Some(bracket_pos) = part.find('[') {
            (&part[..bracket_pos], Some(&part[bracket_pos..]))
        } else {
            (part, None)
        };

        if !path_so_far.is_empty() {
            path_so_far.push('.');
        }
        // Build the base path (without subscript) for the optional check. When the
        // segment is e.g. `tool_calls[0]`, we want to check `is_optional` against
        // "choices[0].message.tool_calls" not "choices[0].message.tool_calls[0]".
        let base_path = {
            let mut p = path_so_far.clone();
            p.push_str(field_name);
            p
        };
        // Now push the full part (with subscript if any) so path_so_far is correct
        // for subsequent segment checks.
        path_so_far.push_str(part);

        // First-class struct fields → property access (no `()`); typealias-to-
        // opaque fields → method-call access (`()`). Once we've indexed through
        // a RustVec, every subsequent segment is on an opaque element.
        // When current_type is None (opaque parent that doesn't appear in field_types),
        // treat it as opaque and use method-call syntax.
        let is_first_class = current_type
            .as_ref()
            .is_some_and(|t| field_resolver.swift_is_first_class(Some(t)));
        let property_syntax = !via_rust_vec && !via_opaque && is_first_class;
        if !property_syntax {
            via_opaque = true;
        }
        out.push('.');
        // Swift bindings (both first-class `public let` props and swift-bridge
        // method names) always use lowerCamelCase — never raw snake_case from IR.
        out.push_str(&field_name.to_lower_camel_case());
        if let Some(sub) = subscript {
            // When the getter for this subscripted field is itself optional
            // (e.g. tool_calls returns Optional<RustVec<T>>), insert `?` before
            // the subscript so Swift unwraps the Optional before indexing.
            // Only emit `?` if we haven't already unwrapped in this chain.
            let field_is_optional = field_resolver.is_optional(&base_path);
            let access = if property_syntax { "" } else { "()" };
            if field_is_optional && !already_unwrapped {
                out.push_str(&format!("{access}?"));
                has_optional = true;
                already_unwrapped = true;
            } else {
                out.push_str(access);
            }
            out.push_str(sub);
            // Do NOT append a trailing `?` after the subscript index: in Swift,
            // `optionalVec?[N]` via `Collection.subscript` returns the element
            // type `T` directly. The parent `has_optional` flag is still set
            // when `field_is_optional` is true, which causes the enclosing
            // expression to be wrapped in `(... ?? fallback)` correctly.
            // Indexing into a Vec<Named> yields a Named element. Only pin opaque
            // syntax when the array itself was opaque (method-call); when the
            // owner is first-class, the array is a Swift `[T]` whose elements
            // are first-class T (property access).
            current_type = field_resolver.swift_advance(current_type.as_deref(), field_name);
            if !property_syntax {
                via_rust_vec = true;
            }
        } else {
            if !property_syntax {
                out.push_str("()");
            }
            // Insert `?` after the accessor for non-leaf optional fields so the
            // next member access becomes `?.`. Only emit `?` if we haven't already
            // unwrapped in this chain with a previous optional chaining operator.
            if !is_leaf && field_resolver.is_optional(&base_path) && !already_unwrapped {
                out.push('?');
                has_optional = true;
                already_unwrapped = true;
            }
            current_type = field_resolver.swift_advance(current_type.as_deref(), field_name);
        }
    }
    (out, has_optional)
}

/// Generate a `[String]` (or `[String]?`) expression for a `RustVec<RustString>`
/// field so that `contains` membership checks work against plain Swift Strings.
///
/// We use `.map { $0.asStr().toString() }` because:
/// 1. Iterating a `RustVec<RustString>` yields `RustStringRef` (not `RustString`), which
///    only has `asStr()` but not `toString()` directly. swift-bridge auto-renames the
///    Rust `as_str` method to lowerCamelCase `asStr` on the Swift side.
/// 2. The accessor may end with an `Optional<RustVec<RustString>>` (e.g. `sheet_names()` is
///    `Option<Vec<String>>` in Rust, which becomes `Optional<RustVec<RustString>>` in Swift).
/// 3. Optional chaining from parent `?.` already produces `Optional<RustVec<T>>`.
///
/// The returned tuple's bool indicates whether the result is `Optional<[String]>`
/// (callers coalesce with `?? []`) or already a concrete `[String]`. Emitting
/// `?? []` against a non-optional value compiles with a Swift warning but is
/// surfaced as an error in strict CI configurations, so we only emit `?.map`
/// + `?? []` when the accessor is genuinely optional.
///
/// Generate a `XCTAssert{True|False}(array.contains(where: { elem_str.contains(val) }), msg)` line
/// for field paths that traverse a collection with `[].` notation (e.g. `links[].url`).
///
/// `array_part` — left side of `[].` (e.g. `"links"`)
/// `element_part` — right side (e.g. `"url"` or `"link_type"`)
/// `full_field` — original assertion.field (used for enum lookup against the full path)
#[allow(clippy::too_many_arguments)]
pub(super) fn swift_traversal_contains_assert(
    array_part: &str,
    element_part: &str,
    full_field: &str,
    val_expr: &str,
    result_var: &str,
    negate: bool,
    msg: &str,
    enum_fields: &std::collections::HashSet<String>,
    field_resolver: &FieldResolver,
) -> String {
    let array_accessor = field_resolver.accessor(array_part, "swift", result_var);
    let resolved_full = field_resolver.resolve(full_field);
    let resolved_elem_part = resolved_full
        .find("[].")
        .map(|d| &resolved_full[d + 3..])
        .unwrap_or(element_part);
    let elem_accessor = field_resolver.accessor(resolved_elem_part, "swift", "$0");
    let elem_is_enum = enum_fields.contains(full_field) || enum_fields.contains(resolved_full);
    let elem_is_optional = field_resolver.is_optional(resolved_elem_part)
        || field_resolver.is_optional(field_resolver.resolve(resolved_elem_part));
    let elem_str = if elem_is_enum {
        // Enum-typed fields are bridged as `String` (RustString in Swift).
        // A single `.toString()` converts RustString → Swift String.
        format!("{elem_accessor}.toString()")
    } else if elem_is_optional {
        format!("({elem_accessor}?.toString() ?? \"\")")
    } else {
        format!("{elem_accessor}.toString()")
    };
    let assert_fn = if negate { "XCTAssertFalse" } else { "XCTAssertTrue" };
    format!("        {assert_fn}({array_accessor}.contains(where: {{ {elem_str}.contains({val_expr}) }}), \"{msg}\")")
}

/// Returns `(map_expr, is_optional)` where `map_expr` is the `.map { … }` chain
/// that converts each element to a Swift `String`, and `is_optional` reports
/// whether the resulting expression is `Optional<[String]>` (callers should
/// coalesce with `?? []`) or already a concrete `[String]`.
///
/// When `materialized_expr` is provided (from a prior call to `materialise_vec_temporaries`),
/// use that expression instead of rebuilding the accessor. This keeps RustVec temporaries
/// bound to locals, preventing use-after-free when swift-bridge releases them.
pub(super) fn swift_array_contains_expr(
    field: Option<&str>,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_field_accessor: &HashMap<String, String>,
    materialized_expr: Option<&str>,
) -> (String, bool) {
    // swift-bridge auto-renames Rust snake_case methods to lowerCamelCase on the
    // Swift side. `RustStringRef::as_str()` is exposed as `asStr()` — emitting
    // `as_str()` produces "value of type 'XRef' has no member 'as_str'" at
    // compile time.
    let Some(f) = field else {
        return (format!("{result_var}.map {{ $0.asStr().toString() }}"), false);
    };
    // Allow per-call overrides to name a different element accessor — used when
    // the array element is an opaque struct whose "name string" accessor is
    // not `as_str` (e.g. `StructureItem` exposes `kind() -> String`). The map
    // is keyed on the fixture field name (and resolved alias as a fallback).
    let resolved_field = field_resolver.resolve(f);
    let elem_accessor_name = result_field_accessor
        .get(f)
        .or_else(|| result_field_accessor.get(resolved_field))
        .cloned()
        .unwrap_or_else(|| "as_str".to_string());
    let elem_call = swift_ident(&elem_accessor_name.to_lower_camel_case());
    // When a materialized expression is provided (from materialise_vec_temporaries),
    // use it directly instead of rebuilding. This keeps RustVec temporaries bound.
    let (accessor, has_optional) = if let Some(expr) = materialized_expr {
        (expr.to_string(), swift_build_accessor(f, result_var, field_resolver).1)
    } else {
        swift_build_accessor(f, result_var, field_resolver)
    };
    // Only chain `?.map` when the accessor is actually optional. The previous
    // unconditional `?.map` produced "cannot use optional chaining on
    // non-optional value of type 'RustVec<…>'" for plain `Vec<T>` fields.
    let field_is_optional =
        has_optional || field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f));
    if field_is_optional {
        (format!("{accessor}?.map {{ $0.{elem_call}().toString() }}"), true)
    } else {
        (format!("{accessor}.map {{ $0.{elem_call}().toString() }}"), false)
    }
}

/// Emit a `XCTAssertTrue(array.contains(where: { ... }), msg)` line that
/// aggregates every text-bearing accessor on the element type of a `Vec<T>`
/// field, mirroring python's `_alef_e2e_item_texts` helper.
///
/// Returns `None` when:
///   - `field` is missing
///   - The field's root or leaf type cannot be resolved
///   - The element type has fewer than 2 stringy fields (the existing
///     single-accessor path is good enough and emits simpler code)
///
/// When matched, emits a closure that gathers `source().toString()`,
/// `items().map { $0.asStr().toString() }`, `alias()?.toString()`, etc. into
/// a flat `[String]` and substring-matches the expected value against every
/// entry. The matcher is lenient so that fixtures asserting `"os"` against
/// the `imports` field — where `ImportInfo.source` may be the bare module
/// name (`"os"`), the entire import statement (`"import os"`), or the
/// imported items (`from os import path` → items=["path"]) — succeed
/// regardless of how the language extractor surfaces the value.
pub(super) fn swift_stringy_aggregator_contains_assert(
    field: Option<&str>,
    result_var: &str,
    field_resolver: &FieldResolver,
    swift_val: &str,
) -> Option<String> {
    use crate::e2e::field_access::StringyFieldKind;
    let field = field?;
    let resolved = field_resolver.resolve(field);
    // Only handle simple top-level array fields (no nested chains) for now.
    // Field path containing `.` or `[` is left to the existing traversal/array
    // paths.
    if resolved.contains('.') || resolved.contains('[') {
        return None;
    }
    let root_type = field_resolver.swift_root_type()?.clone();
    let elem_type = field_resolver.swift_advance(Some(&root_type), resolved)?;
    let stringy = field_resolver.swift_stringy_fields(&elem_type)?;
    if stringy.len() < 2 {
        return None;
    }
    let array_accessor = field_resolver.accessor(field, "swift", result_var);
    let mut texts_lines: Vec<String> = Vec::new();
    for sf in stringy {
        let call = swift_ident(&sf.name.to_lower_camel_case());
        match sf.kind {
            StringyFieldKind::Plain => {
                texts_lines.push(format!("                texts.append(item.{call}().toString())"));
            }
            StringyFieldKind::Optional => {
                texts_lines.push(format!(
                    "                if let v = item.{call}() {{ texts.append(v.toString()) }}"
                ));
            }
            StringyFieldKind::Vec => {
                // `item.field()` returns `RustVec<RustString>`. Mapping its
                // elements yields `RustStringRef` — a swift-bridge wrapper
                // around the borrowed RustString — which has `as_str()`
                // (snake_case, defined in `SwiftBridgeCore.swift`), NOT
                // `toString()` (only `RustString` has the latter via the
                // extension that calls `self.as_str().toString()`).
                texts_lines.push(format!(
                    "                texts.append(contentsOf: item.{call}().map {{ $0.as_str().toString() }})"
                ));
            }
        }
    }
    let texts_block = texts_lines.join("\n");
    Some(format!(
        "        XCTAssertTrue({array_accessor}.contains(where: {{ item in\n            var texts = [String]()\n{texts_block}\n            return texts.contains(where: {{ $0.contains({swift_val}) }})\n        }}), \"expected to contain: \\({swift_val})\")"
    ))
}

/// Generate a `.count` expression for an array field that may be nested inside optional parents.
///
/// Swift-bridge exposes all Rust fields as methods with `()`. When ancestor segments are
/// optional, we use `?.` chaining. The final count is coalesced with `?? 0` when there
/// are optional ancestors so the XCTAssert macro receives a non-optional `Int`.
///
/// Also check if the field itself (the leaf) is optional, which happens when the field
/// returns Optional<RustVec<T>> (e.g., `links()` may return Optional).
///
/// When `materialized_expr` is provided (from a prior call to `materialise_vec_temporaries`),
/// use that expression instead of rebuilding the accessor. This keeps RustVec temporaries
/// bound to locals, preventing use-after-free when swift-bridge releases them.
///
/// Returns `None` when the field is actually a scalar String (not a collection) that was
/// incorrectly marked as an array in the e2e config. In this case, count assertions
/// should be skipped.
pub(super) fn swift_array_count_expr(
    field: Option<&str>,
    result_var: &str,
    field_resolver: &FieldResolver,
    materialized_expr: Option<&str>,
) -> Option<String> {
    let Some(f) = field else {
        return Some(format!("{result_var}.count"));
    };
    // When a materialized expression is provided (from materialise_vec_temporaries),
    // use it directly instead of rebuilding. This keeps RustVec temporaries bound.
    let accessor = if let Some(expr) = materialized_expr {
        expr.to_string()
    } else {
        swift_build_accessor(f, result_var, field_resolver).0
    };
    let mut has_optional = swift_build_accessor(f, result_var, field_resolver).1;
    // Also check if the leaf field itself is optional.
    if field_resolver.is_optional(f) {
        has_optional = true;
    }
    // For opaque method-call accessors (e.g., `result.elements()`), check if the field
    // is a non-Vec type. If so, it would wrap with `.toString()` to convert RustString to Swift String.
    // But if the field is actually a scalar string (not a collection), we cannot meaningfully
    // call .count on it, so return None to signal that this assertion should be skipped.
    let count_target = swift_count_target(&accessor, field_resolver, Some(f))?;
    Some(if has_optional {
        // In Swift, accessing .count on an optional with ?. returns Optional<Int>,
        // so we coalesce with ?? 0 to get a concrete Int for XCTAssert.
        if count_target.contains("?.") {
            format!("({count_target}.count ?? 0)")
        } else {
            // If no ?. but field is optional, the field_expr itself is Optional<RustVec<T>>
            // so we need ?. to call count.
            format!("({count_target}?.count ?? 0)")
        }
    } else {
        format!("{count_target}.count")
    })
}

/// Return the count-able target expression for `field_expr`.
///
/// For opaque method-call accessors (ending in `()` or `()?`), the returned
/// value depends on the field's IR kind:
///
/// - `Vec<T>` ⇒ `RustVec<T>`, which exposes `.count` directly. No wrap.
/// - `String` ⇒ `RustString`, which does NOT expose `.count`. Since wrapping
///   with `.toString()` loses the collection semantics, return None to signal
///   that count assertions cannot be generated for scalar string fields.
///
/// First-class property accessors (no trailing parens) return Swift values
/// that already support `.count` directly.
///
/// The discriminator is the field's resolved leaf type, looked up against the
/// `SwiftFirstClassMap`'s vec field set when available. If the field is
/// unknown (None), fall back to checking whether the field would be wrapped
/// with `.toString()` — indicating a scalar String field unsuitable for counting.
pub(super) fn swift_count_target(
    field_expr: &str,
    field_resolver: &FieldResolver,
    field: Option<&str>,
) -> Option<String> {
    let is_method_call = field_expr.trim_end().ends_with(')');
    if !is_method_call {
        return Some(field_expr.to_string());
    }
    if let Some(f) = field
        && field_resolver.leaf_is_vec_via_swift_map(field_resolver.resolve(f))
    {
        return Some(field_expr.to_string());
    }
    // A non-Vec method-call accessor is a scalar String (RustString) leaf. Converting
    // it to a Swift `String` via `.toString()` yields a value that DOES expose a
    // meaningful `.count` (character length), so wrap with `.toString()` and let the
    // caller append `.count` for length assertions (e.g. `count_min`, `is_empty`).
    Some(format!("{field_expr}.toString()"))
}
