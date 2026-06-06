use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use heck::ToLowerCamelCase;
use std::fmt::Write as FmtWrite;

use super::values::escape_dart;

/// Render `.length` / `?.length ?? 0` against a Dart field accessor.
///
/// Count-style assertions (`count_equals`, `count_min`, `min_length`, `max_length`)
/// operate on collection-typed fields. FRB v2 maps `Option<Vec<T>>` to `List<T>?`
/// (nullable) but `Vec<T>` to `List<T>` (non-null). Emitting `?.length ?? 0`
/// against a non-null receiver triggers `invalid_null_aware_operator`. Inspect
/// the IR via `FieldResolver::is_optional` and choose the safe form per field.
pub(super) fn dart_length_expr(field_accessor: &str, field: Option<&str>, field_resolver: &FieldResolver) -> String {
    let is_optional = field
        .map(|f| {
            let resolved = field_resolver.resolve(f);
            field_resolver.is_optional(f) || field_resolver.is_optional(resolved)
        })
        .unwrap_or(false);
    if is_optional {
        format!("{field_accessor}?.length ?? 0")
    } else {
        format!("{field_accessor}.length")
    }
}

pub(super) fn dart_format_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => format!("'{}'", escape_dart(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => format!("'{}'", escape_dart(&other.to_string())),
    }
}

/// Render a single fixture assertion as a Dart `package:test` `expect(...)` call.
///
/// Field paths are converted per-segment to camelCase (FRB v2 convention) using
/// [`field_to_dart_accessor`].  All 24 fixture assertion types are handled.
///
/// Assertions on fixture fields that are not in the configured `result_fields` set
/// are emitted as a `// skipped:` comment instead — the Dart binding may model a
/// different result shape than the fixture asserts on (e.g. flat `ScrapeResult` vs.
/// nested `result.browser.*`), and emitting unresolvable getters would break the
/// whole file at compile time.
pub(super) fn render_assertion_dart(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    result_is_simple: bool,
    field_resolver: &FieldResolver,
    enum_fields: &std::collections::HashSet<String>,
) {
    // Skip assertions on fields that don't exist on the dart result type. This must run
    // BEFORE the array-traversal and standard accessor paths since both emit code that
    // references the field — an unknown field path produces an `isn't defined` error.
    if !result_is_simple {
        if let Some(f) = assertion.field.as_deref() {
            // Use the head segment (before any `[].`) for validation since `is_valid_for_result`
            // only checks the first path component.
            let head = f.split("[].").next().unwrap_or(f);
            if !head.is_empty() && !field_resolver.is_valid_for_result(head) {
                let _ = writeln!(out, "    // skipped: field '{f}' not available on dart result type");
                return;
            }
        }
    }

    // Skip assertions that traverse a tagged-union variant boundary. FRB exposes
    // tagged unions like `FormatMetadata` as sealed classes whose variants are
    // accessed via pattern matching (`switch (m) { case FormatMetadata_Excel ... }`)
    // — there is no `.excel?` getter, so the fixture path cannot be expressed as
    // a simple chained accessor without language-specific pattern-matching codegen.
    if let Some(f) = assertion.field.as_deref() {
        if !f.is_empty() && field_resolver.tagged_union_split(f).is_some() {
            let _ = writeln!(
                out,
                "    // skipped: field '{f}' crosses a tagged-union variant boundary (not expressible in Dart)"
            );
            return;
        }
    }

    // Handle array traversal (e.g. "links[].link_type" → any() expression).
    if let Some(f) = assertion.field.as_deref() {
        if let Some(dot) = f.find("[].") {
            // Apply the alias mapping to the full `xxx[].yyy` path first so renamed
            // sub-fields (e.g. `assets[].category` → `assets[].asset_category`) resolve
            // correctly. Split *after* resolving so both the array head and the element
            // path reflect any alias rewrites.
            let resolved_full = field_resolver.resolve(f);
            let (array_part, elem_part) = match resolved_full.find("[].") {
                Some(rdot) => (&resolved_full[..rdot], &resolved_full[rdot + 3..]),
                // Resolver mapped the path away from `[].` form — fall back to the original
                // split, since generated code expects the array/elem structure.
                None => (&f[..dot], &f[dot + 3..]),
            };
            let array_accessor = if array_part.is_empty() {
                result_var.to_string()
            } else {
                field_resolver.accessor(array_part, "dart", result_var)
            };
            let elem_accessor = field_to_dart_accessor(elem_part);
            match assertion.assertion_type.as_str() {
                "contains" => {
                    if let Some(expected) = &assertion.value {
                        let dart_val = dart_format_value(expected);
                        let _ = writeln!(
                            out,
                            "    expect({array_accessor}.any((e) => e.{elem_accessor}.toString().contains({dart_val})), isTrue);"
                        );
                    }
                }
                "contains_all" => {
                    if let Some(values) = &assertion.values {
                        for val in values {
                            let dart_val = dart_format_value(val);
                            let _ = writeln!(
                                out,
                                "    expect({array_accessor}.any((e) => e.{elem_accessor}.toString().contains({dart_val})), isTrue);"
                            );
                        }
                    }
                }
                "not_contains" => {
                    if let Some(expected) = &assertion.value {
                        let dart_val = dart_format_value(expected);
                        let _ = writeln!(
                            out,
                            "    expect({array_accessor}.any((e) => e.{elem_accessor}.toString().contains({dart_val})), isFalse);"
                        );
                    } else if let Some(values) = &assertion.values {
                        for val in values {
                            let dart_val = dart_format_value(val);
                            let _ = writeln!(
                                out,
                                "    expect({array_accessor}.any((e) => e.{elem_accessor}.toString().contains({dart_val})), isFalse);"
                            );
                        }
                    }
                }
                "not_empty" => {
                    let _ = writeln!(
                        out,
                        "    expect({array_accessor}.any((e) => e.{elem_accessor}.toString().isNotEmpty), isTrue);"
                    );
                }
                other => {
                    let _ = writeln!(
                        out,
                        "    // skipped: unsupported traversal assertion '{other}' on '{f}'"
                    );
                }
            }
            return;
        }
    }

    let field_accessor = if result_is_simple {
        // Whole-result assertion path: the dart return is a scalar (e.g. a
        // `Uint8List` for speech/file_content), so any `field` on the
        // assertion resolves to the whole value rather than a sub-accessor.
        result_var.to_string()
    } else {
        match assertion.field.as_deref() {
            // Use the shared accessor builder (`FieldResolver::accessor`) — it applies the
            // alias mapping (e.g. `robots.is_allowed` → `is_allowed`), expands array
            // segments to `[0]` lookups, and injects `!` after optional intermediates so
            // chained access compiles under sound null safety.
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "dart", result_var),
            _ => result_var.to_string(),
        }
    };

    let format_value = |val: &serde_json::Value| -> String { dart_format_value(val) };

    match assertion.assertion_type.as_str() {
        "equals" | "field_equals" => {
            if let Some(expected) = &assertion.value {
                let dart_val = format_value(expected);
                // Check if this field is an enum field. Enum fields need _alefE2eText for serde
                // wire format conversion (e.g. FinishReason.toolCalls → "tool_calls").
                let is_enum_field = assertion
                    .field
                    .as_deref()
                    .map(|f| {
                        let resolved = field_resolver.resolve(f);
                        enum_fields.contains(f) || enum_fields.contains(resolved)
                    })
                    .unwrap_or(false);

                // Match the rust codegen's behaviour: trim both sides for string equality
                // so trailing-newline differences between generated text output and the
                // fixture's expected value don't produce false positives.
                if expected.is_string() {
                    if is_enum_field {
                        // For enum fields, use _alefE2eText to normalize the enum value to its
                        // serde wire format before comparison.
                        let _ = writeln!(
                            out,
                            "    expect(_alefE2eText({field_accessor}).trim(), equals({dart_val}.toString().trim()));"
                        );
                    } else {
                        // When result_is_simple is true and the field_accessor is nullable (e.g. String?),
                        // use null-coalescing operator (?? '') to handle null gracefully.
                        let safe_accessor = if result_is_simple && assertion.field.is_none() {
                            format!("({field_accessor} ?? '').toString().trim()")
                        } else {
                            format!("{field_accessor}.toString().trim()")
                        };
                        let _ = writeln!(
                            out,
                            "    expect({safe_accessor}, equals({dart_val}.toString().trim()));"
                        );
                    }
                } else {
                    let _ = writeln!(out, "    expect({field_accessor}, equals({dart_val}));");
                }
            } else {
                let _ = writeln!(
                    out,
                    "    // skipped: '{}' assertion missing value",
                    assertion.assertion_type
                );
            }
        }
        "not_equals" => {
            if let Some(expected) = &assertion.value {
                let dart_val = format_value(expected);
                // Check if this field is an enum field.
                let is_enum_field = assertion
                    .field
                    .as_deref()
                    .map(|f| {
                        let resolved = field_resolver.resolve(f);
                        enum_fields.contains(f) || enum_fields.contains(resolved)
                    })
                    .unwrap_or(false);

                if expected.is_string() {
                    if is_enum_field {
                        let _ = writeln!(
                            out,
                            "    expect(_alefE2eText({field_accessor}).trim(), isNot(equals({dart_val}.toString().trim())));"
                        );
                    } else {
                        // When result_is_simple is true and the field_accessor is nullable (e.g. String?),
                        // use null-coalescing operator (?? '') to handle null gracefully.
                        let safe_accessor = if result_is_simple && assertion.field.is_none() {
                            format!("({field_accessor} ?? '').toString().trim()")
                        } else {
                            format!("{field_accessor}.toString().trim()")
                        };
                        let _ = writeln!(
                            out,
                            "    expect({safe_accessor}, isNot(equals({dart_val}.toString().trim())));"
                        );
                    }
                } else {
                    let _ = writeln!(out, "    expect({field_accessor}, isNot(equals({dart_val})));");
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let dart_val = format_value(expected);
                // Try the "stringy aggregator" path first: when the field is a list of DTOs
                // with multiple text-bearing accessors (e.g. List<ImportInfo> with
                // source/items/alias), emit code that walks every accessor and does
                // substring matching. This avoids the brittle "primary accessor" guess.
                let aggregator = dart_stringy_aggregator_contains_assert(
                    assertion.field.as_deref(),
                    result_var,
                    field_resolver,
                    &dart_val,
                );
                if let Some(line) = aggregator {
                    let _ = writeln!(out, "{line}");
                } else {
                    let _ = writeln!(out, "    expect({field_accessor}, contains({dart_val}));");
                }
            } else {
                let _ = writeln!(out, "    // skipped: 'contains' assertion missing value");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let dart_val = format_value(val);
                    let _ = writeln!(out, "    expect({field_accessor}, contains({dart_val}));");
                }
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let dart_val = format_value(v);
                        format!("{field_accessor}.contains({dart_val})")
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(out, "    expect({joined}, isTrue);");
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let dart_val = format_value(expected);
                let _ = writeln!(out, "    expect({field_accessor}, isNot(contains({dart_val})));");
            } else if let Some(values) = &assertion.values {
                for val in values {
                    let dart_val = format_value(val);
                    let _ = writeln!(out, "    expect({field_accessor}, isNot(contains({dart_val})));");
                }
            }
        }
        "not_empty" => {
            // `isNotEmpty` only applies to types with a `.isEmpty` getter (collections,
            // strings, maps). For struct-shaped fields (e.g. `document: DocumentStructure`)
            // we instead assert the value is non-null — those types have no notion of
            // "empty" and the fixture intent is "the field is present".
            let is_collection = assertion.field.as_deref().is_some_and(|f| {
                let resolved = field_resolver.resolve(f);
                field_resolver.is_array(f) || field_resolver.is_array(resolved)
            });
            if is_collection {
                let _ = writeln!(out, "    expect({field_accessor}, isNotEmpty);");
            } else {
                let _ = writeln!(out, "    expect({field_accessor}, isNotNull);");
            }
        }
        "is_empty" => {
            // FRB models `Option<String>` / `Option<Vec<T>>` as nullable in Dart. The `isEmpty`
            // matcher throws `NoSuchMethodError` on `null`. Accept `null` as semantically
            // empty by combining `isNull` with `isEmpty` via `anyOf`.
            let _ = writeln!(out, "    expect({field_accessor}, anyOf(isNull, isEmpty));");
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let dart_val = format_value(expected);
                let _ = writeln!(out, "    expect({field_accessor}, startsWith({dart_val}));");
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let dart_val = format_value(expected);
                let _ = writeln!(out, "    expect({field_accessor}, endsWith({dart_val}));");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let length_expr = dart_length_expr(&field_accessor, assertion.field.as_deref(), field_resolver);
                    let _ = writeln!(out, "    expect({length_expr}, greaterThanOrEqualTo({n}));");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let length_expr = dart_length_expr(&field_accessor, assertion.field.as_deref(), field_resolver);
                    let _ = writeln!(out, "    expect({length_expr}, lessThanOrEqualTo({n}));");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let length_expr = dart_length_expr(&field_accessor, assertion.field.as_deref(), field_resolver);
                    let _ = writeln!(out, "    expect({length_expr}, equals({n}));");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let length_expr = dart_length_expr(&field_accessor, assertion.field.as_deref(), field_resolver);
                    let _ = writeln!(out, "    expect({length_expr}, greaterThanOrEqualTo({n}));");
                }
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let dart_val = format_value(expected);
                let _ = writeln!(out, "    expect({field_accessor}, matches(RegExp({dart_val})));");
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    expect({field_accessor}, isTrue);");
        }
        "is_false" => {
            let _ = writeln!(out, "    expect({field_accessor}, isFalse);");
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let dart_val = format_value(val);
                let _ = writeln!(out, "    expect({field_accessor}, greaterThan({dart_val}));");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let dart_val = format_value(val);
                let _ = writeln!(out, "    expect({field_accessor}, lessThan({dart_val}));");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let dart_val = format_value(val);
                let _ = writeln!(out, "    expect({field_accessor}, greaterThanOrEqualTo({dart_val}));");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let dart_val = format_value(val);
                let _ = writeln!(out, "    expect({field_accessor}, lessThanOrEqualTo({dart_val}));");
            }
        }
        "not_null" => {
            let _ = writeln!(out, "    expect({field_accessor}, isNotNull);");
        }
        "not_error" => {
            // The `await` already guarantees no thrown error reaches this point — if
            // the call throws, the test fails before reaching here. Don't emit
            // `expect(result, isNotNull)`: for void-returning trait-bridge fns
            // (clear_*) Dart rejects `expect(<void>, ...)` with "expression has type
            // 'void' and can't be used". The implicit exception handling proves
            // success.
        }
        "error" => {
            // Handled at the test method level via throwsA(anything).
        }
        "method_result" => {
            if let Some(method) = &assertion.method {
                let dart_method = method.to_lower_camel_case();
                let check = assertion.check.as_deref().unwrap_or("not_null");
                let method_call = format!("{field_accessor}.{dart_method}()");
                match check {
                    "equals" => {
                        if let Some(expected) = &assertion.value {
                            let dart_val = format_value(expected);
                            let _ = writeln!(out, "    expect({method_call}, equals({dart_val}));");
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "    expect({method_call}, isTrue);");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    expect({method_call}, isFalse);");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let dart_val = format_value(val);
                            let _ = writeln!(out, "    expect({method_call}, greaterThanOrEqualTo({dart_val}));");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            if let Some(n) = val.as_u64() {
                                let _ = writeln!(out, "    expect({method_call}.length, greaterThanOrEqualTo({n}));");
                            }
                        }
                    }
                    _ => {
                        let _ = writeln!(out, "    expect({method_call}, isNotNull);");
                    }
                }
            }
        }
        other => {
            let _ = writeln!(out, "    // skipped: unknown assertion type '{other}'");
        }
    }
}

/// Render a single fixture assertion for a streaming result.
///
/// `result_var` is the `List<T>` collected via `.toList()` on the stream.
/// Supports:
/// - `not_error`: `expect(result, isNotNull)` (a thrown error would already fail
///   the test; the explicit expect keeps the test body non-empty).
/// - `count_min` with `field = "chunks"`: assert `result_var.length >= value`.
/// - `equals` with `field = "stream_content"`: concatenate `delta.content` and compare.
///
/// Other assertion types are emitted as comments.
pub(super) fn render_streaming_assertion_dart(out: &mut String, assertion: &Assertion, result_var: &str) {
    match assertion.assertion_type.as_str() {
        "not_error" => {
            // `.toList()` would have thrown to fail the test on error; emit an
            // explicit `expect` so the test body isn't empty and the collected
            // stream variable is consumed.
            let _ = writeln!(out, "    expect({result_var}, isNotNull);");
        }
        "count_min" if assertion.field.as_deref() == Some("chunks") => {
            if let Some(serde_json::Value::Number(n)) = &assertion.value {
                let _ = writeln!(out, "    expect({result_var}.length, greaterThanOrEqualTo({n}));");
            }
        }
        "equals" if assertion.field.as_deref() == Some("stream_content") => {
            if let Some(serde_json::Value::String(expected)) = &assertion.value {
                let escaped = escape_dart(expected);
                let _ = writeln!(
                    out,
                    "    final _content = {result_var}.map((c) => c.choices.firstOrNull?.delta.content ?? '').join();"
                );
                let _ = writeln!(out, "    expect(_content, equals('{escaped}'));");
            }
        }
        other => {
            let _ = writeln!(out, "    // skipped streaming assertion: '{other}'");
        }
    }
}

/// Converts a snake_case JSON key to Dart camelCase.
pub(super) fn snake_to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut next_upper = false;
    for ch in s.chars() {
        if ch == '_' {
            next_upper = true;
        } else if next_upper {
            result.extend(ch.to_uppercase());
            next_upper = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert a dot-separated fixture field path to a Dart accessor expression.
///
/// Each segment is converted to camelCase (FRB v2 convention); array-index brackets
/// (e.g. `choices[0]`) and map-key brackets (e.g. `tags[name]`) are preserved.
/// This replaces the former single-pass `snake_to_camel` call which incorrectly
/// treated the entire path string as one identifier.
///
/// Examples:
/// - `"choices"` → `"choices"`
/// - `"choices[0].message.content"` → `"choices[0].message.content"`
/// - `"metadata.document_title"` → `"metadata.documentTitle"`
/// - `"model_id"` → `"modelId"`
pub(super) fn field_to_dart_accessor(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    for (i, segment) in path.split('.').enumerate() {
        if i > 0 {
            result.push('.');
        }
        // Separate a trailing `[...]` bracket from the field name so we only
        // camelCase the identifier part, not the bracket content. The owning
        // collection may be `List<T>?` when the underlying Rust field is
        // `Option<Vec<T>>`; force-unwrap with `!` so the `[N]` lookup and any
        // subsequent member access compile under sound null safety.
        if let Some(bracket_pos) = segment.find('[') {
            let name = &segment[..bracket_pos];
            let bracket = &segment[bracket_pos..];
            result.push_str(&name.to_lower_camel_case());
            result.push('!');
            result.push_str(bracket);
        } else {
            result.push_str(&segment.to_lower_camel_case());
        }
    }
    result
}

/// Emit a `expect(array.where(...).any(...), isTrue)` line that aggregates
/// every accessor on the element type of a `List<T>` field, mirroring
/// python's `_alef_e2e_item_texts` helper.
///
/// Since Dart e2e codegen doesn't currently carry type information for per-type
/// field classification, this fallback always tries to aggregate common text-bearing
/// accessors (kind, name, source, items, alias, and similar snake_case names) on any
/// element type. This is lenient and works well with opaque DTOs from FRB binding
/// generation where we can't statically determine the exact field structure.
///
/// Returns `None` when:
///   - `field` is missing or the field doesn't look like an array field
///
/// When matched, emits code that tries to gather text from a set of known
/// accessor names into a `[String]` and substring-matches the expected value
/// against every entry. The matcher is lenient so that fixtures asserting `"os"`
/// against the `imports` field succeed regardless of which accessor surfaces
/// the value (`ImportInfo.source`, `ImportInfo.items`, etc.).
///
/// First tries the "stringy aggregator" path: when the array element is an
/// opaque DTO with several text-bearing accessors, emit a `where(...)`
/// closure that walks every accessor and does substring matching. Falls back
/// to the catch-all path if no stringy fields are recorded for the element type.
pub(super) fn dart_stringy_aggregator_contains_assert(
    field: Option<&str>,
    result_var: &str,
    field_resolver: &crate::e2e::field_access::FieldResolver,
    dart_val: &str,
) -> Option<String> {
    use crate::e2e::field_access::StringyFieldKind;
    let field = field?;
    let resolved = field_resolver.resolve(field);

    // Only handle simple top-level array fields (no nested chains).
    if resolved.contains('.') || resolved.contains('[') {
        return None;
    }

    // Check if this is a known array field. If not, we can't tell if it's a
    // list of DTOs so bail out and let the scalar list path handle it.
    if !field_resolver.is_array(field) && !field_resolver.is_array(resolved) {
        return None;
    }

    let array_accessor = field_resolver.accessor(field, "dart", result_var);

    // Try the stringy aggregator path: if the element type has multiple
    // text-bearing accessors, emit a proper aggregator instead of a catch-all.
    let root_type = field_resolver.dart_root_type().cloned();
    if let Some(elem_type) = field_resolver.dart_advance(root_type.as_deref(), resolved) {
        if let Some(stringy) = field_resolver.dart_stringy_fields(&elem_type) {
            // Only emit the aggregator if the element type has 2+ stringy fields.
            // Single-field types are better served by the simpler single-accessor path.
            if stringy.len() >= 2 {
                // flutter_rust_bridge renders struct DTOs as plain Dart classes
                // with `final` fields, so accessors are property reads (no
                // parens). Dart is statically typed — calling `item.field()` on
                // a non-callable field, or naming a field the type lacks, is a
                // compile error, not a runtime miss.
                let mut texts_lines: Vec<String> = Vec::new();
                for sf in stringy {
                    let call = sf.name.to_lower_camel_case();
                    match sf.kind {
                        StringyFieldKind::Plain => {
                            texts_lines.push(format!("            texts.add(item.{call}.toString());"));
                        }
                        StringyFieldKind::Optional => {
                            texts_lines.push(format!(
                                "            final v_{call} = item.{call};\n            if (v_{call} != null) texts.add(v_{call}.toString());"
                            ));
                        }
                        StringyFieldKind::Vec => {
                            texts_lines.push(format!(
                                "            texts.addAll(item.{call}.map((e) => e.toString()));"
                            ));
                        }
                    }
                }
                let texts_block = texts_lines.join("\n");
                // Case-insensitive substring match: enum/sealed-class fields
                // stringify to `EnumName.variant()` (lowerCamelCase variant),
                // while fixture node-type values are PascalCase (`Function`).
                return Some(format!(
                    "    expect({array_accessor}.where((item) {{\n            final texts = <String>[];\n{texts_block}\n            return texts.any((t) => t.toLowerCase().contains(({dart_val}).toString().toLowerCase()));\n          }}).isEmpty, isFalse);"
                ));
            }
        }
    }

    // Fallback: the element type's fields could not be resolved from the IR
    // (unknown root type, or fewer than two recorded stringy fields). Dart is
    // statically typed, so probing arbitrary accessor names cannot compile —
    // emit a lenient whole-object stringification match that always compiles.
    Some(format!(
        "    expect({array_accessor}.where((item) => item.toString().toLowerCase().contains(({dart_val}).toString().toLowerCase())).isEmpty, isFalse);"
    ))
}
