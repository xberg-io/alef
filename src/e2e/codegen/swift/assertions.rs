use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::accessors::{
    materialise_vec_temporaries, swift_array_contains_expr, swift_array_count_expr, swift_count_target,
    swift_stringy_aggregator_contains_assert, swift_traversal_contains_assert,
};
use super::values::{escape_swift, json_to_swift, swift_numeric_literal_cast};

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
    enum_fields: &HashSet<String>,
    is_streaming: bool,
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
                            String::new()
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            format!("        XCTAssertEqual(chunks.count, {n})\n")
                        } else {
                            String::new()
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!("        XCTAssertEqual({expr}, \"{escaped}\")\n")
                        } else if let Some(b) = assertion.value.as_ref().and_then(|v| v.as_bool()) {
                            format!("        XCTAssertEqual({expr}, {b})\n")
                        } else {
                            String::new()
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
                            String::new()
                        }
                    }
                    "contains" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_swift(s);
                            format!(
                                "        XCTAssertTrue({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\")\n"
                            )
                        } else {
                            String::new()
                        }
                    }
                    _ => format!(
                        "        // streaming field '{f}': assertion type '{}' not rendered\n",
                        assertion.assertion_type
                    ),
                };
                if !line.is_empty() {
                    out.push_str(&line);
                }
            }
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "        // skipped: field '{f}' not available on result type");
            return;
        }
    }

    // Skip assertions that traverse a tagged-union variant boundary.
    // In Swift, FormatMetadata and similar enum-backed opaque types are exposed as
    // plain classes by swift-bridge — variant accessor methods (e.g., `.excel()`)
    // are not generated, so such assertions cannot be expressed.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && field_resolver.tagged_union_split(f).is_some() {
            let _ = writeln!(
                out,
                "        // skipped: field '{f}' crosses a tagged-union variant boundary (not expressible in Swift)"
            );
            return;
        }
    }

    // Determine if this field is an enum type.
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

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
    let field_is_array = assertion.field.as_deref().is_some_and(|f| {
        !f.is_empty()
            && (field_resolver.is_array(f)
                || field_resolver.is_array(field_resolver.resolve(f))
                || field_resolver.is_collection_root(f)
                || field_resolver.is_collection_root(field_resolver.resolve(f)))
    });

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
    let (vec_setup, field_expr, is_map_subscript) = materialise_vec_temporaries(&field_expr_raw, &local_suffix);
    // The `contains` / `not_contains` traversal branch builds its own
    // accessor from `field_resolver.accessor(array_part, ...)`, ignoring
    // `field_expr`. Emitting the vec_setup there would produce dead
    // `let _vec_… = …` lines, so skip it for those traversal cases.
    let field_uses_traversal = assertion.field.as_deref().is_some_and(|f| f.contains("[]."));
    let traversal_skips_field_expr = field_uses_traversal
        && matches!(
            assertion.assertion_type.as_str(),
            "contains" | "not_contains" | "not_empty" | "is_empty"
        );
    if !traversal_skips_field_expr {
        for line in &vec_setup {
            let _ = writeln!(out, "        {line}");
        }
    }

    // In Swift, optional chaining with `?.` makes the result optional even if the
    // called method's return type isn't marked optional. For example:
    // `result.markdown()?.content()` returns `Optional<RustString>` because
    // `markdown()` is optional and the `?.` operator wraps the result.
    // Detect this by checking if the accessor contains `?.`.
    let accessor_is_optional = field_expr.contains("?.");
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
                    if field_is_enum {
                        // Enum fields: `to_string()` (snake_case) returns RustString;
                        // `.toString()` converts it to a Swift String.
                        // `string_expr` already incorporates this call chain.
                        let trim_expr =
                            format!("{string_expr}.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)");
                        let _ = writeln!(out, "        XCTAssertEqual({trim_expr}, {swift_val})");
                    } else {
                        // For optional strings (String?), use ?? to coalesce before trimming.
                        // `.toString()` converts RustString → Swift String before calling
                        // `.trimmingCharacters`, which requires a concrete String type.
                        // string_expr already incorporates field_is_optional via ?.toString() ?? "".
                        let trim_expr =
                            format!("{string_expr}.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)");
                        let _ = writeln!(out, "        XCTAssertEqual({trim_expr}, {swift_val})");
                    }
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
                    // []. traversal: field like "links[].url" → contains(where:) closure.
                    let traversal_handled = if let Some(f) = assertion.field.as_deref() {
                        if let Some(dot) = f.find("[].") {
                            let array_part = &f[..dot];
                            let elem_part = &f[dot + 3..];
                            let line = swift_traversal_contains_assert(
                                array_part,
                                elem_part,
                                f,
                                &swift_val,
                                result_var,
                                false,
                                &format!("expected to contain: \\({swift_val})"),
                                enum_fields,
                                field_resolver,
                            );
                            let _ = writeln!(out, "{line}");
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !traversal_handled {
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
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                // []. traversal: field like "links[].link_type" → contains(where:) per value.
                if let Some(f) = assertion.field.as_deref() {
                    if let Some(dot) = f.find("[].") {
                        let array_part = &f[..dot];
                        let elem_part = &f[dot + 3..];
                        for val in values {
                            let swift_val = json_to_swift(val);
                            let line = swift_traversal_contains_assert(
                                array_part,
                                elem_part,
                                f,
                                &swift_val,
                                result_var,
                                false,
                                &format!("expected to contain: \\({swift_val})"),
                                enum_fields,
                                field_resolver,
                            );
                            let _ = writeln!(out, "{line}");
                        }
                        // handled — skip remaining branches
                    } else {
                        // For array fields (RustVec<RustString>), check membership via map+contains.
                        let field_is_array = field_resolver.is_array(field_resolver.resolve(f));
                        if field_is_array {
                            let (contains_expr, is_optional) = swift_array_contains_expr(
                                assertion.field.as_deref(),
                                result_var,
                                field_resolver,
                                result_field_accessor,
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
            if let Some(expected) = &assertion.value {
                let swift_val = json_to_swift(expected);
                // []. traversal: "links[].url" → XCTAssertFalse(array.contains(where:))
                let traversal_handled = if let Some(f) = assertion.field.as_deref() {
                    if let Some(dot) = f.find("[].") {
                        let array_part = &f[..dot];
                        let elem_part = &f[dot + 3..];
                        let line = swift_traversal_contains_assert(
                            array_part,
                            elem_part,
                            f,
                            &swift_val,
                            result_var,
                            true,
                            &format!("expected NOT to contain: \\({swift_val})"),
                            enum_fields,
                            field_resolver,
                        );
                        let _ = writeln!(out, "{line}");
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !traversal_handled {
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({string_expr}.contains({swift_val}), \"expected NOT to contain: \\({swift_val})\")"
                    );
                }
            }
        }
        "not_empty" => {
            // For optional fields (Optional<T>), check that the value is non-nil.
            // For array fields (RustVec<T>), check .isEmpty on the vec directly.
            // For result_is_simple (e.g. Data, String), use .isEmpty directly on
            // the result — avoids calling .toString() on non-RustString types.
            // For string fields, convert to Swift String and check .isEmpty.
            // []. traversal: "links[].url" → contains(where: { !elem.isEmpty })
            let traversal_not_empty_handled = if let Some(f) = assertion.field.as_deref() {
                if let Some(dot) = f.find("[].") {
                    let array_part = &f[..dot];
                    let elem_part = &f[dot + 3..];
                    let array_accessor = field_resolver.accessor(array_part, "swift", result_var);
                    let resolved_full = field_resolver.resolve(f);
                    let resolved_elem_part = resolved_full
                        .find("[].")
                        .map(|d| &resolved_full[d + 3..])
                        .unwrap_or(elem_part);
                    let elem_accessor = field_resolver.accessor(resolved_elem_part, "swift", "$0");
                    let elem_is_enum = enum_fields.contains(f) || enum_fields.contains(resolved_full);
                    let elem_is_optional = field_resolver.is_optional(resolved_elem_part)
                        || field_resolver.is_optional(field_resolver.resolve(resolved_elem_part));
                    let elem_str = if elem_is_enum {
                        format!("{elem_accessor}.to_string().toString()")
                    } else if elem_is_optional {
                        format!("({elem_accessor}?.toString() ?? \"\")")
                    } else {
                        format!("{elem_accessor}.toString()")
                    };
                    let _ = writeln!(
                        out,
                        "        XCTAssertTrue({array_accessor}.contains(where: {{ !{elem_str}.isEmpty }}), \"expected non-empty value\")"
                    );
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !traversal_not_empty_handled {
                if bare_result_is_option {
                    let _ = writeln!(out, "        XCTAssertNotNil({result_var}, \"expected non-nil value\")");
                } else if field_is_optional {
                    let _ = writeln!(out, "        XCTAssertNotNil({field_expr}, \"expected non-nil value\")");
                } else if field_is_array {
                    let _ = writeln!(
                        out,
                        "        XCTAssertFalse({field_expr}.isEmpty, \"expected non-empty value\")"
                    );
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
                    if let Some(count_target) =
                        swift_count_target(&field_expr, field_resolver, assertion.field.as_deref())
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
                        let _ = writeln!(
                            out,
                            "        // skipped: field is a scalar String without meaningful .count"
                        );
                    }
                }
            }
        }
        "is_empty" => {
            if bare_result_is_option {
                let _ = writeln!(out, "        XCTAssertNil({result_var}, \"expected nil value\")");
            } else if field_is_optional {
                let _ = writeln!(out, "        XCTAssertNil({field_expr}, \"expected nil value\")");
            } else if field_is_array {
                let _ = writeln!(
                    out,
                    "        XCTAssertTrue({field_expr}.isEmpty, \"expected empty value\")"
                );
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
                    let _ = writeln!(
                        out,
                        "        // skipped: field is a scalar String without meaningful .count"
                    );
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
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // Use string_expr.count: for RustString fields string_expr already has
                    // .toString() appended, giving a Swift String whose .count is character count.
                    let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({string_expr}.count, {n})");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "        XCTAssertLessThanOrEqual({string_expr}.count, {n})");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // For fields nested inside an optional parent (e.g. document.nodes where
                    // document is Optional), the accessor generates `result.document().nodes()`
                    // which doesn't compile in Swift without optional chaining.
                    if let Some(count_expr) =
                        swift_array_count_expr(assertion.field.as_deref(), result_var, field_resolver)
                    {
                        let _ = writeln!(out, "        XCTAssertGreaterThanOrEqual({count_expr}, {n})");
                    } else {
                        // swift_array_count_expr returns None when the field is a scalar String
                        // marked (incorrectly) as an array in fields_array. Such fields don't
                        // support .count and would produce invalid code.
                        if let Some(f) = &assertion.field {
                            let _ = writeln!(
                                out,
                                "        // skipped: field '{f}' is a scalar String without meaningful .count"
                            );
                        }
                    }
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    if let Some(count_expr) =
                        swift_array_count_expr(assertion.field.as_deref(), result_var, field_resolver)
                    {
                        let _ = writeln!(out, "        XCTAssertEqual({count_expr}, {n})");
                    } else {
                        // swift_array_count_expr returns None when the field is a scalar String
                        // marked (incorrectly) as an array in fields_array. Such fields don't
                        // support .count and would produce invalid code.
                        if let Some(f) = &assertion.field {
                            let _ = writeln!(
                                out,
                                "        // skipped: field '{f}' is a scalar String without meaningful .count"
                            );
                        }
                    }
                }
            }
        }
        "is_true" => {
            let assert_expr = if accessor_is_optional {
                format!("({field_expr} ?? false)")
            } else {
                field_expr.clone()
            };
            let _ = writeln!(out, "        XCTAssertTrue({assert_expr})");
        }
        "is_false" => {
            let assert_expr = if accessor_is_optional {
                format!("({field_expr} ?? true)")
            } else {
                field_expr.clone()
            };
            let _ = writeln!(out, "        XCTAssertFalse({assert_expr})");
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
            // Already handled by the call succeeding without exception.
        }
        "error" => {
            // Handled at the test method level.
        }
        "method_result" => {
            let _ = writeln!(out, "        // method_result assertions not yet implemented for Swift");
        }
        other => {
            panic!("Swift e2e generator: unsupported assertion type: {other}");
        }
    }
}
