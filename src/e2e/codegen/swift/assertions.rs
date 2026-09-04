use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::codegen::payload_union_skip::{UnionLoweringTarget, payload_union_skip_line};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use super::accessors::{
    materialise_vec_temporaries, swift_array_contains_expr, swift_array_count_expr, swift_array_is_empty_expr,
    swift_array_not_empty_predicate, swift_count_target, swift_stringy_aggregator_contains_assert,
};
use super::values::{escape_swift, json_to_swift, swift_numeric_literal_cast};
use super::wildcard_assertion::render_wildcard_assertion;

/// ~keep The token a skip marker names when the assertion has no field path at all (a bare-result
/// assertion). Every registered wording quotes a token, and a marker that quotes nothing matches
/// no shape — which is how `// skipped: field is a scalar String without meaningful .count` stayed
/// invisible to both funnels and to a grep census.
pub(super) const BARE_RESULT_TOKEN: &str = "<bare result>";

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_is_array: bool,
    result_is_option: bool,
    result_element_is_string: bool,
    result_field_accessor: &HashMap<String, String>,
    is_streaming: bool,
    returns_void: bool,
) {
    // When the bare result is `Optional<T>` (no field path) the opaque class
    // exposed by swift-bridge has no `.toString()` method, so the usual
    // `.toString().isEmpty` pattern produces compile errors. Detect the
    // "bare result" case and prefer `XCTAssertNil` / `XCTAssertNotNil`.
    let bare_result_is_option = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    // Streaming virtual fields resolve against the `chunks` collected-array variable.
    // Intercept before is_valid_for_result so they are never skipped.
    // Also intercept `usage.*` deep-paths in streaming tests: `AsyncThrowingStream` does
    // not have a `usage()` method, so we must route them through the chunks accessor.
    if let Some(f) = &assertion.field {
        let is_streaming_usage_path =
            is_streaming && (f == "usage" || (f.starts_with("usage.") || f.starts_with("usage[")));
        // Only route through the streaming-virtual `chunks` accessor when this is
        // actually a streaming fixture. Non-streaming fixtures (e.g. `process()`
        // with `chunkMaxSize`) expose `chunks` as a real `ProcessResult` field, so
        // emit `result.chunks()` via the regular field-accessor path below.
        if is_streaming
            && !f.is_empty()
            && (crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) || is_streaming_usage_path)
        {
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "swift", "chunks")
            {
                let line = match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThanOrEqual(chunks.count, {n})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertEqual(chunks.count, {n})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!("        XCTAssertEqual({expr}, \"{escaped}\")\n")
                        } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                            format!("        XCTAssertEqual({expr}, {b})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "not_empty" => {
                        format!("        XCTAssertFalse({expr}.isEmpty, \"expected non-empty\")\n")
                    }
                    "is_empty" => {
                        format!("        XCTAssertTrue({expr}.isEmpty, \"expected empty\")\n")
                    }
                    "is_true" => {
                        format!("        XCTAssertTrue({expr})\n")
                    }
                    "is_false" => {
                        format!("        XCTAssertFalse({expr})\n")
                    }
                    "greater_than" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertGreaterThan(chunks.count, {n})\n")
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!(
                                "        XCTAssertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\")\n"
                            )
                        } else {
                            streaming_assertion_value_skip_line("        ", "//", f, &assertion.assertion_type) + "\n"
                        }
                    }
                    _ => format!(
                        "{}\n",
                        streaming_assertion_type_skip_line("        ", "//", f, &assertion.assertion_type)
                    ),
                };
                out.push_str(&line);
            } else {
                // ~keep The accessor returns `None` for reachable inputs — a `stream.has_*_event`
                // predicate never resolves here, since `accessor` supplies no item type — and this
                // branch used to be absent, so the assertion vanished with no line for
                // `fail_on_unavailable_field_markers` to see. alef's streaming adapter owns the
                // gap, so it is counted, never fatal.
                let _ = writeln!(
                    out,
                    "        // skipped: {}",
                    FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
                );
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
        return;
    }

    if let Some(line) = super::leaf_shape::json_bridged_traversal_skip(field_resolver, assertion.field.as_deref())
        .or_else(|| super::leaf_shape::non_countable_leaf_count_skip(field_resolver, assertion.field.as_deref()))
    {
        out.push_str(&line);
        return;
    }

    // Skip assertions that traverse a tagged-union variant boundary.
    // In Swift, FormatMetadata and similar enum-backed opaque types are exposed as
    // plain classes by swift-bridge — variant accessor methods (e.g., `.excel()`)
    // are not generated, so such assertions cannot be expressed.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && field_resolver.tagged_union_split(f).is_some()
    {
        let _ = writeln!(
            out,
            "        // skipped: {}",
            FieldSkip::CrossesTaggedUnionBoundaryInSwift.message(f)
        );
        return;
    }

    // A `foo[].bar` fixture path names EVERY element of `foo`, not element 0. The shared
    // accessor has no wildcard concept: `parse_path` lowers `foo[]` to
    // `PathSegment::ArrayField { index: 0 }`, so any arm reaching the generic accessor emits
    // `result.foo()[0].bar()` — an assertion about one element wearing the fixture's "some
    // element" wording. Route every wildcard path here first so an assertion type this
    // backend cannot traverse leaves a visible skip instead of a silent index-0 assertion,
    // matching the pre-dispatch every other backend already performs. ~keep
    if let Some(field) = assertion.field.as_deref()
        && let Some(dot) = field.find("[].")
    {
        render_wildcard_assertion(out, assertion, field, dot, result_var, field_resolver);
        return;
    }

    // A payload-carrying union is the union shape `gen_bindings::enums::emit_enum` renders with
    // associated values instead of a `: String` raw-value enum. It has no `.rawValue`, and every
    // string arm below reads one lowered expression, so withholding the accessor alone would
    // route it into `XCTAssertEqual(result.kind, "key_value")` — a type mismatch that does not
    // compile. Refuse, after the wildcard gate so a union-typed element keeps its own path. ~keep
    if let Some(line) = payload_union_skip_line(
        "        ",
        "//",
        field_resolver,
        assertion.field.as_deref(),
        UnionLoweringTarget::Swift,
    ) {
        let _ = writeln!(out, "{line}");
        return;
    }

    // Determine if this field is an enum type. `field_resolver.is_enum` consults the
    // hand-maintained `fields_enum`/`enum_fields` config first and only then the IR-derived
    // classification (`with_ir_enum_map`), so an explicit config entry still wins; a config-only
    // check emitted `XCTAssertEqual(result.kind().toString(), "key_value")` against a field whose
    // Swift type is the generated enum `DataNodeKind`, not compile-comparable to a `String`. ~keep
    let field_is_enum = assertion
        .field
        .as_deref()
        .filter(|f| !f.is_empty())
        .is_some_and(|f| field_resolver.is_enum(f));

    // Determine if this field is a display-as-text content union (e.g. `AssistantContent`).
    // Such fields are emitted as Swift enums (not `String`) and expose a `.text()` method
    // that concatenates the plain-text representation. The assertion must call `.text()` to
    // compare against the fixture's expected string, mirroring the Kotlin/Go/Java backends.
    let field_is_display_as_text = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_display_as_text(f));

    let field_is_optional = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty() && (field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f)))
    });
    // ~keep A field the swift-bridge scan positively recorded as JSON-bridged is a scalar
    // `RustString` at the Swift surface no matter what the IR says its logical shape is --
    // `is_array`/`is_collection_root` answer from the IR alone and know nothing about the
    // bridging, so without this guard a genuinely JSON-bridged `Vec<T>` field read as
    // array-shaped here and routed `not_empty`/`is_empty` into a bare `.isEmpty` call, which
    // does not exist on `RustString` ("value of type 'RustString' has no member 'isEmpty'").
    let field_is_array = assertion.field.as_deref().is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        !f.is_empty()
            && !field_resolver.leaf_is_json_bridged_via_swift_map(resolved)
            && (field_resolver.is_array(f)
                || field_resolver.is_array(resolved)
                || field_resolver.is_collection_root(f)
                || field_resolver.is_collection_root(resolved))
    });
    // ~keep The refusal that keeps the guard above from turning a discriminating assertion into
    // one that cannot fail. See `leaf_shape::unspellable_collection_emptiness_skip`.
    let collection_emptiness_skip =
        super::leaf_shape::unspellable_collection_emptiness_skip(field_resolver, assertion.field.as_deref());

    let field_expr_raw = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "swift", result_var),
            _ => result_var.to_string(),
        }
    };

    // swift-bridge `RustVec<T>` exposes its elements as `T.SelfRef`, which holds
    // a raw pointer into the parent Vec's storage. When the Vec is a temporary
    // (e.g. `result.json_ld()` called inline), Swift ARC may release it before
    // the ref is used, leaving the ref's pointer dangling. Materialise the
    // temporary into a local so it survives the full expression chain.
    //
    // The local name is suffixed with the assertion type plus a hash of the
    // assertion's discriminating fields so multiple assertions on the same
    // collection don't redeclare the same name.
    let local_suffix = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        assertion.field.hash(&mut hasher);
        assertion
            .value
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .hash(&mut hasher);
        format!(
            "{}_{:x}",
            assertion.assertion_type.replace(['-', '.'], "_"),
            hasher.finish() & 0xffff_ffff,
        )
    };
    // `None` is a mixed map-then-vec chain `json_bridged_traversal_skip` above missed (no IR
    // data on this resolver) — see `materialise_vec_temporaries`'s own doc. ~keep
    let Some((vec_setup, field_expr, is_map_subscript)) = materialise_vec_temporaries(&field_expr_raw, &local_suffix)
    else {
        let field_label = assertion
            .field
            .as_deref()
            .filter(|f| !f.is_empty())
            .unwrap_or(BARE_RESULT_TOKEN);
        out.push_str(&super::leaf_shape::mixed_map_then_vec_traversal_skip(field_label));
        return;
    };
    // Wildcard paths never reach here — they return via `render_wildcard_assertion` above —
    // so `field_expr` is always the expression the arms below assert on and its setup lines
    // are never dead. The previous suppression list named `is_empty`, which had no traversal
    // branch to suppress for: it dropped the `let _vec_… = …` binding while still emitting an
    // expression referencing that local, so `is_empty` on a wildcard path emitted Swift
    // naming an undeclared variable. ~keep
    for line in &vec_setup {
        let _ = writeln!(out, "        {line}");
    }

    // In Swift, optional chaining with `?.` makes the result optional even if the
    // called method's return type isn't marked optional. For example:
    // `result.markdown()?.content()` returns `Optional<RustString>` because
    // `markdown()` is optional and the `?.` operator wraps the result.
    // Detect this by checking if the accessor contains `?.`.
    let accessor_is_optional = field_expr.contains("?.");
    let leaf_getter_is_optional =
        super::leaf_shape::leaf_getter_is_optional(field_resolver, assertion.field.as_deref());
    // First-class Codable Swift struct property access leaves no trailing `()`
    // on the leaf segment — e.g. `result.text` (Swift `String`) vs
    // `result.text()` (RustBridge.RustString). When the leaf is property
    // access, we already have a Swift `String` (or `String?`) and must NOT
    // re-wrap with `.toString()`. Detect this by looking at the final segment
    // after the last `.` — property access ends in a bare identifier (no
    // trailing `()` or `()?`).
    let leaf_is_property_access = {
        let trimmed = field_expr.trim_end_matches('?');
        // Skip subscripts: `name?[0]` should still see `name` as the field.
        let last_segment = trimmed.rsplit_once('.').map(|(_, s)| s).unwrap_or(trimmed);
        let last_segment = last_segment.split('[').next().unwrap_or(last_segment);
        !last_segment.ends_with(')') && !last_segment.is_empty()
    };

    // Bare-result Option<T> case: the call returns `Optional<String>` (or
    // similar) so the field_expr is `result` typed as `String?`. String
    // assertions like `XCTAssertEqual(result.trimmingCharacters(...), …)` will
    // not compile against an optional — coalesce to `""` so the macro sees a
    // concrete Swift `String`.
    let bare_result_is_simple_option =
        result_is_simple && result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();

    // For enum fields, need to handle the string representation differently in Swift.
    // Swift enums don't have `.rawValue` unless they're explicitly RawRepresentable.
    // Check if this is an enum type and handle accordingly.
    // For optional fields (Optional<RustString>), use optional chaining before toString().
    // For other fields: swift-bridge returns all Rust `String` fields as `RustString`.
    // We add .toString() here so string assertions (contains, hasPrefix, etc.) work.
    // Non-string opaque fields (DocumentStructure, etc.) should not appear in string
    // assertions — the fixture schema controls which assertions apply to which fields.
    let string_expr = if field_is_display_as_text {
        // Display-as-text content union (e.g. `AssistantContent`): the leaf is a Swift
        // enum exposing `.text()` returning a non-optional `String`. For optional content
        // (`AssistantContent?`) or an optional ancestor chain, unwrap with `?.text()` and
        // coalesce to "" so XCTAssert receives a concrete Swift `String`.
        if field_is_optional || accessor_is_optional {
            format!("({field_expr}?.text() ?? \"\")")
        } else {
            format!("{field_expr}.text()")
        }
    } else if is_map_subscript {
        // The field_expr already evaluates to `String?` (from a JSON-decoded
        // `[String: String]` subscript). No `.toString()` chain needed —
        // coalesce the optional to "" and use the Swift String directly.
        format!("({field_expr} ?? \"\")")
    } else if leaf_is_property_access {
        // First-class Codable struct field access: leaf is already a Swift
        // `String` (or `String?`/enum type) — never a `RustString` requiring
        // `.toString()`. For optional leaves, coalesce to "" so XCTAssert
        // receives a non-optional Swift `String`.
        if field_is_enum && (field_is_optional || accessor_is_optional) {
            // Optional first-class Codable enum (e.g. `FinishReason?` where
            // `FinishReason: String, Codable`). `.rawValue` gives the serde
            // wire value (e.g. "tool_calls") so assertions match fixture JSON.
            format!("(({field_expr})?.rawValue ?? \"\")")
        } else if field_is_enum {
            format!("{field_expr}.rawValue")
        } else if field_is_optional || accessor_is_optional || bare_result_is_simple_option {
            format!("({field_expr} ?? \"\")")
        } else {
            field_expr.to_string()
        }
    } else if field_is_enum && leaf_getter_is_optional {
        format!("({field_expr}?.toString() ?? \"\")")
    } else if field_is_enum && accessor_is_optional {
        // Enum-typed leaf reached through an ancestor optional chain. The chain's `?`
        // already propagated, so `field_expr` is `Optional<RustString>` even though
        // the leaf accessor itself is non-Optional. Use `.toString()` (no extra `?`)
        // to avoid Swift's "cannot use optional chaining on non-optional value" error.
        format!("({field_expr}.toString() ?? \"\")")
    } else if field_is_enum && field_is_optional {
        // Enum-typed field that is itself Optional<RustString> (e.g. `finish_reason()`
        // returning `Optional<RustString>` at the binding surface) — unwrap with `?`.
        format!("({field_expr}?.toString() ?? \"\")")
    } else if field_is_enum {
        // Enum-typed fields are now bridged as `String` (RustString in Swift) rather than
        // as opaque enum handles. The getter on the Rust side calls `to_string()` internally
        // and returns a `String` across the FFI. In Swift this arrives as `RustString`, so
        // `.toString()` converts it to a Swift `String` — one call, not two.
        format!("{field_expr}.toString()")
    } else if leaf_getter_is_optional {
        format!("({field_expr}?.toString() ?? \"\")")
    } else if accessor_is_optional {
        // Ancestor optional chain already propagated `?` (e.g. `result.summary()?.strategy()`),
        // so the whole `field_expr` is Optional<RustString> regardless of whether the leaf
        // field itself is also marked optional. Adding another `?` before `.toString()` here
        // would emit `result.summary()?.strategy()?.toString()` which Swift rejects:
        // "cannot use optional chaining on non-optional value of type 'RustString'".
        // The earlier `?` from the accessor's chain already unwraps; use `.toString()` here.
        format!("({field_expr}.toString() ?? \"\")")
    } else if field_is_optional {
        // Leaf field itself is Optional<RustString> with no ancestor chain — need
        // ?.toString() to unwrap before stringifying.
        format!("({field_expr}?.toString() ?? \"\")")
    } else {
        format!("{field_expr}.toString()")
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                if expected.is_string() {
                    let _ = writeln!(out, "        XCTAssertEqual({string_expr}, {swift_val})");
                } else {
                    // For numeric fields, cast the expected value to match the field's type (e.g., UInt).
                    let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                    let _ = writeln!(out, "        XCTAssertEqual({field_expr}, {cast_swift_val})");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                // When the root result IS the array (result_is_simple + result_is_array) and
                // there is no field path, check array membership via map+contains.
                let no_field = assertion.field.as_deref().is_none_or(|f| f.is_empty());
                if result_is_simple && result_is_array && no_field {
                    if result_element_is_string {
                        // The Swift binding exposes the result as a native
                        // `[String]` (e.g. `manifestLanguages() -> [String]`),
                        // not the opaque `RustVec<RustString>`. Iterating
                        // elements yields plain Swift `String`, which has no
                        // `asStr()` — emit a direct `.contains(...)` instead.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({result_var}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    } else {
                        // RustVec<RustString> iteration yields RustStringRef (no `toString()`);
                        // use `.asStr().toString()` to convert each element to a Swift String.
                        // swift-bridge renames `as_str` → `asStr` automatically.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({result_var}.map {{ $0.asStr().toString() }}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                } else {
                    // For array fields (RustVec<RustString>), check membership via map+contains.
                    let field_is_array = assertion
                        .field
                        .as_deref()
                        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));
                    if field_is_array {
                        // First try the "stringy aggregator" path: when the array element
                        // is an opaque DTO with several text-bearing accessors (e.g.
                        // ImportInfo with source/items/alias, or StructureItem with
                        // kind/name/signature/...), emit a `contains(where: { ... })`
                        // closure that walks every accessor and does substring matching,
                        // mirroring python's `_alef_e2e_item_texts`. This avoids the
                        // brittle "primary accessor" guess (e.g. ImportInfo → source
                        // misses imports whose name lives in `items`).
                        let aggregator = swift_stringy_aggregator_contains_assert(
                            assertion.field.as_deref(),
                            result_var,
                            field_resolver,
                            &swift_val,
                        );
                        if let Some(line) = aggregator {
                            let _ = writeln!(out, "{line}");
                        } else {
                            let (contains_expr, is_optional) = swift_array_contains_expr(
                                assertion.field.as_deref(),
                                result_var,
                                field_resolver,
                                result_field_accessor,
                                Some(&field_expr),
                            );
                            let wrapped = if is_optional {
                                format!("({contains_expr} ?? [])")
                            } else {
                                contains_expr
                            };
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({wrapped}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    } else if field_is_enum {
                        // Enum fields: use `toString().toString()` (via string_expr) to get the
                        // serde variant name as a Swift String, then check substring containment.
                        // Swift's `String.contains("")` returns false; guard with `.isEmpty` so
                        // fixtures that assert containment of an empty string still pass.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({swift_val}.isEmpty || {string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    } else {
                        // Same `isEmpty` guard as the enum branch — every string trivially
                        // "contains" the empty string, but Swift's `String.contains` does not.
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({swift_val}.isEmpty || {string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                if let Some(f) = assertion.field.as_deref() {
                    // For array fields (RustVec<RustString>), check membership via map+contains.
                    let field_is_array = field_resolver.is_array(field_resolver.resolve(f));
                    if field_is_array {
                        let (contains_expr, is_optional) = swift_array_contains_expr(
                            assertion.field.as_deref(),
                            result_var,
                            field_resolver,
                            result_field_accessor,
                            Some(&field_expr),
                        );
                        let wrapped = if is_optional {
                            format!("({contains_expr} ?? [])")
                        } else {
                            contains_expr
                        };
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({wrapped}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    } else if field_is_enum {
                        // Enum fields: use `toString().toString()` (via string_expr) to get the
                        // serde variant name as a Swift String, then check substring containment.
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    } else {
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let _ = writeln!(
                                out,
                                "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                            );
                        }
                    }
                } else {
                    // No field — fall back to existing string_expr path.
                    for val in values {
                        let swift_val = json_to_swift(val);
                        let _ = writeln!(
                            out,
                            "        XCTAssertTrue({string_expr}.contains({swift_val}), \"expected to contain: \\({swift_val})\")"
                        );
                    }
                }
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({string_expr}.contains({swift_val}), \"expected NOT to contain: \\({swift_val})\")"
                );
            }
        }
        "not_empty" => {
            // For optional fields (Optional<T>), check that the value is non-nil.
            // For array fields (RustVec<T>), check .isEmpty on the vec directly.
            // For result_is_simple (e.g. Data, String), use .isEmpty directly on
            // the result — avoids calling .toString() on non-RustString types.
            // For string fields, convert to Swift String and check .isEmpty.
            if bare_result_is_option {
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({string_expr}.isEmpty, \"expected non-empty value\")"
                );
            } else if let Some(line) = &collection_emptiness_skip {
                out.push_str(line);
            } else if field_is_array && field_is_optional {
                out.push_str(&crate::e2e::template_env::render(
                    "swift/not_empty_assertion.swift.jinja",
                    minijinja::context! { predicate => format!("{field_expr}?.isEmpty == false") },
                ));
            } else if field_is_optional {
                out.push_str(&crate::e2e::template_env::render(
                    "swift/not_empty_assertion.swift.jinja",
                    minijinja::context! { predicate => format!("{field_expr} != nil") },
                ));
            } else if field_is_array {
                let predicate = swift_array_not_empty_predicate(&field_expr, accessor_is_optional);
                out.push_str(&crate::e2e::template_env::render(
                    "swift/not_empty_assertion.swift.jinja",
                    minijinja::context! { predicate => predicate },
                ));
            } else if result_is_simple {
                // result_is_simple: result is a primitive (Data, String, etc.) — use .isEmpty directly.
                let _ = writeln!(
                    out,
                    "        XCTAssertFalse({result_var}.isEmpty, \"expected non-empty value\")"
                );
            } else {
                // First-class Swift struct fields are properties typed as native Swift
                // `String` / `[T]` / `Data` etc — all of which expose `.count` (and
                // `String`/`Array` also expose `.isEmpty`). Use `.count > 0` so the same
                // path works whether the field is a String or an Array.
                //
                // When the accessor contains a `?.` optional chain, `.count` returns an
                // Optional which Swift cannot compare directly to `0`; coalesce via `?? 0`
                // so the assertion typechecks.
                //
                // For opaque method-call accessors (`result.id()`), the returned type is
                // `RustString`, which lacks `.count`. Convert to Swift `String` first via
                // `.toString()`. Array fields short-circuit above via `field_is_array`, so
                // method-call accessors landing here are guaranteed to be the scalar /
                // string flavour; vec accessors return `RustVec` (whose `.count` is fine).
                if let Some(count_target) = swift_count_target(&field_expr, field_resolver, assertion.field.as_deref())
                {
                    let len_expr = if accessor_is_optional {
                        format!("({count_target}.count ?? 0)")
                    } else {
                        format!("{count_target}.count")
                    };
                    let _ = writeln!(
                        out,
                        "        XCTAssertGreaterThan({len_expr}, 0, \"expected non-empty value\")"
                    );
                } else {
                    out.push_str(&super::leaf_shape::non_countable_leaf_skip_line(
                        assertion.field.as_deref(),
                    ));
                }
            }
        }
        "is_empty" => {
            if bare_result_is_option {
                let _ = writeln!(out, "        XCTAssertNil({result_var}, \"expected nil value\")");
            } else if let Some(line) = &collection_emptiness_skip {
                out.push_str(line);
            } else if field_is_optional {
                let _ = writeln!(out, "        XCTAssertNil({field_expr}, \"expected nil value\")");
            } else if field_is_array {
                let is_empty_expr = swift_array_is_empty_expr(&field_expr, accessor_is_optional);
                let _ = writeln!(out, "        XCTAssertTrue({is_empty_expr}, \"expected empty value\")");
            } else {
                // Symmetric with not_empty: use .count == 0 on first-class Swift types.
                // Wrap opaque method-call accessors (`result.id()`) with `.toString()` so
                // `.count` lands on Swift `String`, not `RustString` (which lacks `.count`).
                if let Some(count_target) = swift_count_target(&field_expr, field_resolver, assertion.field.as_deref())
                {
                    let len_expr = if accessor_is_optional {
                        format!("({count_target}.count ?? 0)")
                    } else {
                        format!("{count_target}.count")
                    };
                    let _ = writeln!(out, "        XCTAssertEqual({len_expr}, 0, \"expected empty value\")");
                } else {
                    out.push_str(&super::leaf_shape::non_countable_leaf_skip_line(
                        assertion.field.as_deref(),
                    ));
                }
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let swift_val = json_to_swift(v);
                        format!("{string_expr}.contains({swift_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({joined}, \"expected to contain at least one of the specified values\")"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(out, "        XCTAssertGreaterThan({compare_expr}, {cast_swift_val})");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(out, "        XCTAssertLessThan({compare_expr}, {cast_swift_val})");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                // For optional numeric fields (or when the accessor chain is optional),
                // coalesce to 0 before comparing so the expression is non-optional.
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(
                    out,
                    "        XCTAssertGreaterThanOrEqual({compare_expr}, {cast_swift_val})"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let swift_val = json_to_swift(val);
                let field_is_optional = accessor_is_optional
                    || assertion.field.as_deref().is_some_and(|f| {
                        field_resolver.is_optional(f) || field_resolver.is_optional(field_resolver.resolve(f))
                    });
                let compare_expr = if field_is_optional {
                    let cast_val = swift_numeric_literal_cast(&field_expr, "0");
                    format!("({field_expr} ?? {cast_val})")
                } else {
                    field_expr.clone()
                };
                let cast_swift_val = swift_numeric_literal_cast(&field_expr, &swift_val);
                let _ = writeln!(
                    out,
                    "        XCTAssertLessThanOrEqual({compare_expr}, {cast_swift_val})"
                );
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasPrefix({swift_val}), \"expected to start with: \\({swift_val})\")"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({string_expr}.hasSuffix({swift_val}), \"expected to end with: \\({swift_val})\")"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                // Use string_expr.count: for RustString fields string_expr already has
                // .toString() appended, giving a Swift String whose .count is character count.
                let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({string_expr}.count, {n})");
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "        XCTAssertLessThanOrEqual({string_expr}.count, {n})");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                // For fields nested inside an optional parent (e.g. document.nodes where
                // document is Optional), the accessor generates `result.document().nodes()`
                // which doesn't compile in Swift without optional chaining.
                if let Some(count_expr) = swift_array_count_expr(
                    assertion.field.as_deref(),
                    result_var,
                    field_resolver,
                    Some(&field_expr),
                ) {
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({count_expr}, {n})");
                } else if let Some(count_expr) = super::leaf_shape::swift_json_bridged_count_expr(
                    field_resolver,
                    assertion.field.as_deref(),
                    &field_expr,
                ) {
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({count_expr}, {n})");
                } else {
                    out.push_str(&super::leaf_shape::non_countable_leaf_skip_line(
                        assertion.field.as_deref(),
                    ));
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                if let Some(count_expr) = swift_array_count_expr(
                    assertion.field.as_deref(),
                    result_var,
                    field_resolver,
                    Some(&field_expr),
                ) {
                    let _ = writeln!(out, "        XCTAssertEqual({count_expr}, {n})");
                } else if let Some(count_expr) = super::leaf_shape::swift_json_bridged_count_expr(
                    field_resolver,
                    assertion.field.as_deref(),
                    &field_expr,
                ) {
                    let _ = writeln!(out, "        XCTAssertEqual({count_expr}, {n})");
                } else {
                    out.push_str(&super::leaf_shape::non_countable_leaf_skip_line(
                        assertion.field.as_deref(),
                    ));
                }
            }
        }
        "is_true" | "is_false" => {
            // `accessor_is_optional` only catches an intermediate `?.` in the chain -- a
            // field that is ITSELF the optional leaf (e.g. `data` in `data.kind`, with no
            // further segment to safe-navigate past) leaves `field_expr` as `result.data()`
            // with no `?.` anywhere, so that check alone misses it. Consult the resolver
            // directly for the leaf's own optionality too. ~keep
            let leaf_is_optional = assertion
                .field
                .as_deref()
                .is_some_and(|f| field_resolver.is_optional(field_resolver.resolve(f)));
            if accessor_is_optional || leaf_is_optional {
                // `T?`: "is_true"/"is_false" mean "present"/"absent" -- `?? false` only
                // type-checks when T is `Bool` and for any other T (e.g. `DataNode?`) it is a
                // compile error. `!= nil` is the interpretation that holds for any T,
                // matching the Rust `.is_some()` convention for this assertion type.
                if assertion.assertion_type == "is_true" {
                    let _ = writeln!(out, "        XCTAssertNotNil({field_expr})");
                } else {
                    let _ = writeln!(out, "        XCTAssertNil({field_expr})");
                }
            } else if assertion.assertion_type == "is_true" {
                let _ = writeln!(out, "        XCTAssertTrue({field_expr})");
            } else {
                let _ = writeln!(out, "        XCTAssertFalse({field_expr})");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                let _ = writeln!(
                    out,
                    "        XCTAssertNotNil({string_expr}.range(of: {swift_val}, options: .regularExpression), \"expected value to match regex: \\({swift_val})\")"
                );
            }
        }
        "not_error" => {
            super::not_error_assertion::render_not_error_assertion(out, returns_void);
        }
        "error" => {
            // ~keep Handled at the test method level, via `render_error_catch_block`
            // in `test_method.rs` (plain success catch or a declared-value check).
        }
        "method_result" => {
            let _ = writeln!(out, "        // method_result assertions not yet implemented for Swift");
        }
        other => {
            panic!("Swift e2e generator: unsupported assertion type: {other}");
        }
    }
}

#[cfg(test)]
#[path = "assertions/skip_marker_tests.rs"]
mod skip_marker_tests;
