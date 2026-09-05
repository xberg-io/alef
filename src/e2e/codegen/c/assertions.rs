//! C e2e assertion and accessor rendering helpers.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::escape::escape_c;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

/// The IR type name a C parameter carries as an opaque `AlefHandle` rather than as a literal.
/// Defined in `c::optional_arg` -- the single seam every C e2e call site checks a parameter's
/// handle-ness through, so the free-function path here and the client-method path in
/// `c/test_function.rs` cannot independently drift on the same question. ~keep
use super::optional_arg::handle_param_type_name;
use super::{
    NestedLeafOutcome, is_primitive_c_type, is_skipped_c_field, json_to_c, render_wildcard_assertion,
    resolve_optional_sentinel, try_emit_enum_accessor,
};

/// Emit chained FFI accessor calls for a nested resolved field path.
///
/// For a path like `metadata.document.title`, this generates:
/// ```c
/// HTMHtmlMetadata* metadata_handle = htm_conversion_result_metadata(result);
/// assert(metadata_handle != NULL);
/// HTMDocumentMetadata* doc_handle = htm_html_metadata_document(metadata_handle);
/// assert(doc_handle != NULL);
/// char* metadata_title = htm_document_metadata_title(doc_handle);
/// ```
///
/// The type chain is looked up from `fields_c_types` which maps
/// `"{parent_snake_type}.{field}"` -> `"PascalCaseType"`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_nested_accessor(
    out: &mut String,
    prefix: &str,
    resolved: &str,
    local_var: &str,
    result_var: &str,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    intermediate_handles: &mut Vec<(String, String)>,
    result_type_name: &str,
    raw_field: &str,
    type_defs: &[crate::core::ir::TypeDef],
    config_sources: &FieldConfigSources,
) -> anyhow::Result<Option<NestedLeafOutcome>> {
    let segments: Vec<&str> = resolved.split('.').collect();
    // cbindgen's `[export] prefix` is shouty-snake, not uppercase; re-deriving it here as
    // `to_uppercase` names types the generated header never declares for any prefix carrying an
    // internal word boundary (`SampleCore` -> `SAMPLECORE` vs the header's `SAMPLE_CORE`). ~keep
    let prefix_upper = crate::codegen::c_consumer::export_type_prefix(prefix);

    // Walk the path, starting from the root result type.
    let mut walk = SegmentWalk {
        current_snake_type: result_type_name.to_snake_case(),
        current_handle: result_var.to_string(),
        current_type_from_ir: type_defs.iter().any(|type_def| type_def.name == result_type_name),
        json_extract_mode: false,
        is_wildcard: false,
    };

    for (i, segment) in segments.iter().enumerate() {
        let is_leaf = i + 1 == segments.len();

        // JSON-extract mode and bracket ("field[key]"/"field[]") segments are both handled
        // entirely by `step_prefixed_segment`; only a plain field segment falls through to
        // `step_plain_segment` below.
        let step = match step_prefixed_segment(
            &mut *out,
            prefix,
            &mut *intermediate_handles,
            &mut walk,
            segment,
            is_leaf,
            local_var,
        ) {
            Some(step) => step,
            None => step_plain_segment(PlainSegmentArgs {
                out: &mut *out,
                prefix,
                prefix_upper: &prefix_upper,
                raw_field,
                resolved,
                segment,
                is_leaf,
                segments: &segments,
                i,
                walk: &mut walk,
                local_var,
                fields_c_types,
                fields_enum,
                intermediate_handles: &mut *intermediate_handles,
                result_type_name,
                type_defs,
                config_sources,
            })?,
        };
        match step {
            SegmentStep::Continue => continue,
            SegmentStep::Done(outcome) => return Ok(outcome),
        }
    }
    Ok(None)
}

/// One step of [`emit_nested_accessor`]'s walk for a segment handled without reaching the
/// plain-field branch: while `walk.json_extract_mode` is set, every segment is a JSON-extract
/// step; otherwise a segment shaped `field[key]`/`field[]` is a bracket-access step. Returns
/// `None` when neither applies, so the caller falls through to [`step_plain_segment`] exactly
/// as the original `if walk.json_extract_mode {..} if let Some(bracket_pos) = ... {..}` did.
fn step_prefixed_segment(
    out: &mut String,
    prefix: &str,
    intermediate_handles: &mut Vec<(String, String)>,
    walk: &mut SegmentWalk,
    segment: &str,
    is_leaf: bool,
    local_var: &str,
) -> Option<SegmentStep> {
    // In JSON extraction mode, the current_handle is a JSON string and all
    // segments name keys to extract via alef_json_get_string (for primitive
    // leaves) or alef_json_get_object (for intermediate object hops).
    if walk.json_extract_mode {
        return Some(step_json_extract_segment(out, intermediate_handles, walk, segment, is_leaf, local_var));
    }
    // Check for map access: "field[key]" or array element access: "field[]"
    step_bracket_segment(out, prefix, intermediate_handles, walk, segment, is_leaf, local_var)
}

/// Inputs for [`step_plain_segment`]. A struct, not a dozen positional arguments, for the
/// same reason as [`StepLeafSegmentArgs`].
struct PlainSegmentArgs<'a> {
    out: &'a mut String,
    prefix: &'a str,
    prefix_upper: &'a str,
    raw_field: &'a str,
    resolved: &'a str,
    segment: &'a str,
    is_leaf: bool,
    segments: &'a [&'a str],
    i: usize,
    walk: &'a mut SegmentWalk,
    local_var: &'a str,
    fields_c_types: &'a HashMap<String, String>,
    fields_enum: &'a HashSet<String>,
    intermediate_handles: &'a mut Vec<(String, String)>,
    result_type_name: &'a str,
    type_defs: &'a [crate::core::ir::TypeDef],
    config_sources: &'a FieldConfigSources,
}

/// One step of [`emit_nested_accessor`]'s walk for a plain field segment (`segment` carries no
/// `[` and we are not already in JSON-extract mode) -- computes the accessor name, applies the
/// `fields_c_types` skip sentinel, then dispatches to the leaf or intermediate branch.
/// Extracted verbatim from the walk loop's tail -- same emitted lines, same conditions, same
/// ordering.
fn step_plain_segment(args: PlainSegmentArgs<'_>) -> anyhow::Result<SegmentStep> {
    let PlainSegmentArgs {
        out,
        prefix,
        prefix_upper,
        raw_field,
        resolved,
        segment,
        is_leaf,
        segments,
        i,
        walk,
        local_var,
        fields_c_types,
        fields_enum,
        intermediate_handles,
        result_type_name,
        type_defs,
        config_sources,
    } = args;

    let seg_snake = segment.to_snake_case();
    let accessor_fn = format!("{prefix}_{}_{seg_snake}", walk.current_snake_type);

    // Skip any assertion that touches a field marked "skip" in fields_c_types.
    if is_skipped_c_field(fields_c_types, &walk.current_snake_type, &seg_snake) {
        // Sentinel: no accessor emitted, assertion skipped later.
        return Ok(SegmentStep::Done(Some(NestedLeafOutcome::Typed("__skip__".to_string()))));
    }

    if is_leaf {
        step_leaf_segment(StepLeafSegmentArgs {
            out,
            prefix,
            prefix_upper,
            raw_field,
            resolved,
            segment,
            seg_snake: &seg_snake,
            walk: &*walk,
            local_var,
            accessor_fn: &accessor_fn,
            fields_c_types,
            fields_enum,
            intermediate_handles,
            result_type_name,
            type_defs,
            config_sources,
        })
    } else {
        step_intermediate_segment(StepIntermediateSegmentArgs {
            out,
            prefix,
            prefix_upper,
            raw_field,
            resolved,
            segment,
            seg_snake: &seg_snake,
            segments,
            i,
            walk,
            local_var,
            accessor_fn: &accessor_fn,
            fields_c_types,
            intermediate_handles,
            result_type_name,
            type_defs,
            config_sources,
        })
    }
}

/// Where [`emit_nested_accessor`]'s per-segment walk goes next: keep consuming the path, or
/// stop and hand this value back to the caller. Every branch of the original loop body either
/// mutated the walk state and looped, or returned -- this names the two shapes so the
/// extracted per-branch helpers below can hand control back to the loop instead of inlining a
/// `continue`/`return` themselves. ~keep
enum SegmentStep {
    /// Keep walking; `SegmentWalk` has already been updated in place.
    Continue,
    /// Stop the walk and return this value from `emit_nested_accessor`.
    Done(Option<NestedLeafOutcome>),
}

/// Mutable state threaded through [`emit_nested_accessor`]'s per-segment walk loop.
struct SegmentWalk {
    current_snake_type: String,
    current_handle: String,
    /// True only while `current_snake_type` names a type the IR actually declares, which
    /// is the precondition for using the IR as an oracle for the next segment. The `char*`
    /// hop below sets `current_snake_type` from a *field* name rather than a type name, and
    /// a `fields_c_types` value may name a C type with no IR counterpart at all; in either
    /// case an IR type that happens to share the name is a coincidence, not the parent. ~keep
    current_type_from_ir: bool,
    /// Set to true when we've traversed a `[]` array element accessor and subsequent
    /// fields must be extracted via alef_json_get_string rather than FFI function calls.
    json_extract_mode: bool,
    /// Set to true only when that `[]` had an EMPTY key — a true wildcard ("every element"),
    /// as opposed to an explicit numeric index (`[N]`) that also enables `json_extract_mode`
    /// but names one concrete element. Distinguishes the two at the leaf below: an indexed
    /// leaf still resolves to one scalar value, a wildcard leaf does not.
    ///
    /// `assertions.rs` and `test_function.rs` are both already over the repo's 1,000-line cap
    /// (`file-modularization`), and this fix necessarily touches both: the mis-selection lives
    /// in THIS function, and each of its three call sites (`call_patterns.rs`,
    /// `test_function.rs` x2) has to learn about the new `wildcard_locals` bucket to stop
    /// freeing a C local that was never declared. The new logic itself — the quantifier
    /// renderer and the primitive/opaque/wildcard classification — lives in
    /// `collection_wildcard.rs` instead of growing either capped file further; what remains
    /// here and in `test_function.rs` is the minimum wiring needed to reach it. ~keep
    is_wildcard: bool,
}

/// One step of [`emit_nested_accessor`]'s walk while `walk.json_extract_mode` is set: the
/// current handle is a JSON string and all segments name keys to extract via
/// `alef_json_get_string` (for primitive leaves) or `alef_json_get_object` (for intermediate
/// object hops). Extracted verbatim from the walk loop's `json_extract_mode` branch -- same
/// emitted lines, same conditions, same ordering.
fn step_json_extract_segment(
    out: &mut String,
    intermediate_handles: &mut Vec<(String, String)>,
    walk: &mut SegmentWalk,
    segment: &str,
    is_leaf: bool,
    local_var: &str,
) -> SegmentStep {
    let current_handle = walk.current_handle.clone();
    // Decompose `field` or `field[N]`/`field[]`. Numeric indexing must
    // extract the Nth element so later key lookups don't ambiguously
    // pick the first occurrence (matters for fixtures with multiple
    // array elements like `data[0]`/`data[1]`).
    let (bare_segment, bracket_key): (&str, Option<&str>) = match segment.find('[') {
        Some(pos) => (&segment[..pos], Some(segment[pos + 1..].trim_end_matches(']'))),
        None => (segment, None),
    };
    let seg_snake = bare_segment.to_snake_case();
    if is_leaf {
        // `field[].key`: `current_handle` names the ARRAY's own JSON text (the `[]`
        // branch below set it and never drilled into one element), so a scalar
        // `alef_json_get_string(current_handle, ...)` here would look up "key" as a
        // property of the array itself — never present, making every "contains"-shaped
        // assertion built from the (buggy) scalar local unsatisfiable by construction.
        // Defer to a per-element quantifier at assertion-render time instead. ~keep
        if walk.is_wildcard && bracket_key.is_none() {
            return SegmentStep::Done(Some(NestedLeafOutcome::Wildcard {
                array_var: current_handle.clone(),
                key_snake: seg_snake,
            }));
        }
        let _ = writeln!(
            out,
            "    char* {local_var} = alef_json_get_string({current_handle}, \"{seg_snake}\");"
        );
        return SegmentStep::Done(None); // JSON key leaf — char*.
    }
    // Intermediate JSON key — must be an object/array value. Use the
    // object extractor so the substring includes braces/brackets and
    // later primitive lookups against it find their keys
    // (alef_json_get_string would return NULL on non-string values).
    let json_var = format!("{seg_snake}_json");
    if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
        let _ = writeln!(
            out,
            "    char* {json_var} = alef_json_get_object({current_handle}, \"{seg_snake}\");"
        );
        intermediate_handles.push((json_var.clone(), "free".to_string()));
    }
    // If the segment also includes a numeric index `[N]`, drill into
    // the Nth element of the extracted array; otherwise stay on the
    // object/array substring.
    if let Some(key) = bracket_key
        && let Ok(idx) = key.parse::<usize>()
    {
        let elem_var = format!("{seg_snake}_{idx}_json");
        if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
            let _ = writeln!(
                out,
                "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
            );
            intermediate_handles.push((elem_var.clone(), "free".to_string()));
        }
        walk.current_handle = elem_var;
        return SegmentStep::Continue;
    }
    walk.current_handle = json_var;
    SegmentStep::Continue
}

/// One step of [`emit_nested_accessor`]'s walk for a segment shaped `field[key]` or `field[]`
/// (map access or array-element access), while not already in JSON-extract mode. Extracted
/// verbatim from the walk loop's bracket-segment branch -- same emitted lines, same
/// conditions, same ordering. Returns `None` when `segment` carries no `[`, so the caller
/// falls through to the plain-field branch exactly as the original `if let Some(...)` did.
fn step_bracket_segment(
    out: &mut String,
    prefix: &str,
    intermediate_handles: &mut Vec<(String, String)>,
    walk: &mut SegmentWalk,
    segment: &str,
    is_leaf: bool,
    local_var: &str,
) -> Option<SegmentStep> {
    let bracket_pos = segment.find('[')?;
    let current_handle = walk.current_handle.clone();
    let current_snake_type = walk.current_snake_type.clone();
    let field_name = &segment[..bracket_pos];
    let key = segment[bracket_pos + 1..].trim_end_matches(']');
    let field_snake = field_name.to_snake_case();
    let accessor_fn = format!("{prefix}_{current_snake_type}_{field_snake}");

    // The accessor returns a char* (JSON object/array string).
    let json_var = format!("{field_snake}_json");
    if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
        let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
        let _ = writeln!(out, "    assert({json_var} != NULL);");
        // Track for freeing — use prefix_free_string since it's a char*.
        intermediate_handles.push((json_var.clone(), "free_string".to_string()));
    }

    // Empty key `[]`: array-element substring access (any element matches).
    // Numeric key `[N]` (e.g. `choices[0]`, `data[1]`): extract the exact
    // Nth top-level element so subsequent key lookups don't ambiguously
    // pick the first occurrence — required for fixtures whose results
    // contain multiple array elements (e.g. `data[0].index`/`data[1].index`).
    if key.is_empty() {
        if !is_leaf {
            walk.current_handle = json_var;
            walk.json_extract_mode = true;
            walk.is_wildcard = true;
            return Some(SegmentStep::Continue);
        }
        return Some(SegmentStep::Done(None));
    }
    if let Ok(idx) = key.parse::<usize>() {
        let elem_var = format!("{field_snake}_{idx}_json");
        if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
            let _ = writeln!(
                out,
                "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
            );
            intermediate_handles.push((elem_var.clone(), "free".to_string()));
        }
        if !is_leaf {
            walk.current_handle = elem_var;
            walk.json_extract_mode = true;
            return Some(SegmentStep::Continue);
        }
        // Trailing `[N]` — caller asserts on the element JSON.
        return Some(SegmentStep::Done(None));
    }

    // Named map key access: extract the key value from the JSON object.
    let _ = writeln!(
        out,
        "    char* {local_var} = alef_json_get_string({json_var}, \"{key}\");"
    );
    Some(SegmentStep::Done(None)) // Map access leaf — char*.
}

/// Inputs for [`step_leaf_segment`]. A struct, not a dozen positional arguments, following
/// this file's existing convention (see [`MissingIntermediateType`], [`LeafFieldCheck`]) for
/// a helper with this many related-but-distinct parameters.
struct StepLeafSegmentArgs<'a> {
    out: &'a mut String,
    prefix: &'a str,
    prefix_upper: &'a str,
    raw_field: &'a str,
    resolved: &'a str,
    segment: &'a str,
    seg_snake: &'a str,
    walk: &'a SegmentWalk,
    local_var: &'a str,
    accessor_fn: &'a str,
    fields_c_types: &'a HashMap<String, String>,
    fields_enum: &'a HashSet<String>,
    intermediate_handles: &'a mut Vec<(String, String)>,
    result_type_name: &'a str,
    type_defs: &'a [crate::core::ir::TypeDef],
    config_sources: &'a FieldConfigSources,
}

/// One step of [`emit_nested_accessor`]'s walk for a leaf plain-field segment (`is_leaf` is
/// true, and `segment` carries no `[` and we are not already in JSON-extract mode). Extracted
/// verbatim from the walk loop's leaf branch -- same emitted lines, same conditions, same
/// ordering.
fn step_leaf_segment(args: StepLeafSegmentArgs<'_>) -> anyhow::Result<SegmentStep> {
    let StepLeafSegmentArgs {
        out,
        prefix,
        prefix_upper,
        raw_field,
        resolved,
        segment,
        seg_snake,
        walk,
        local_var,
        accessor_fn,
        fields_c_types,
        fields_enum,
        intermediate_handles,
        result_type_name,
        type_defs,
        config_sources,
    } = args;
    let current_handle = walk.current_handle.as_str();
    // Leaf may be a primitive scalar (uint64_t, double, ...) when
    // configured in `fields_c_types`. Otherwise default to char*.
    let lookup_key = format!("{}.{seg_snake}", walk.current_snake_type);
    if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
        let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({current_handle});");
        return Ok(SegmentStep::Done(Some(NestedLeafOutcome::Typed(t.clone()))));
    }
    // Enum leaf: opaque enum pointer that needs `_to_string` conversion. Must run
    // BEFORE the opaque-struct-leaf check below: `try_emit_enum_accessor` gates
    // itself on `fields_enum` membership, but its `fields_c_types` value (the
    // enum's PascalCase type name, e.g. `DataNodeKind`) is indistinguishable in
    // shape from a struct's opaque type name -- both are non-primitive PascalCase
    // strings. Checking the opaque-struct filter first would swallow every
    // dotted-path enum leaf (it never inspects `fields_enum`) and hand back a bare
    // handle for the caller to `strcmp` against, which aborts at runtime. The flat
    // (single-segment) leaf path a few lines below in `test_function.rs` already
    // orders enum-before-opaque; this nested-path leaf must match it. ~keep
    if try_emit_enum_accessor(
        out,
        prefix,
        prefix_upper,
        raw_field,
        seg_snake,
        &walk.current_snake_type,
        accessor_fn,
        current_handle,
        local_var,
        fields_c_types,
        fields_enum,
        intermediate_handles,
    ) {
        return Ok(SegmentStep::Done(None));
    }
    // Opaque struct leaf: when fields_c_types maps "{parent}.{field}" to a
    // PascalCase type name (not a primitive, not "char*", not "skip"), the
    // accessor returns a struct pointer rather than a string. Emit the typed
    // handle declaration and register it for freeing.
    if let Some(opaque_type) = fields_c_types.get(&lookup_key).filter(|t| {
        *t != "char*" && *t != "skip" && !is_primitive_c_type(t) && t.chars().next().is_some_and(|c| c.is_uppercase())
    }) {
        let handle_var = format!("{seg_snake}_handle");
        let opaque_snake = opaque_type.to_snake_case();
        if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
            let _ = writeln!(
                out,
                "    {prefix_upper}AlefHandle {handle_var} = {accessor_fn}({current_handle});"
            );
            intermediate_handles.push((handle_var.clone(), opaque_snake.clone()));
        }
        // Treat the handle itself as the local_var for later assertions.
        // Map local_var → handle_var so render_assertion uses the handle name.
        if local_var != handle_var {
            let _ = writeln!(out, "    {prefix_upper}AlefHandle {local_var} = {handle_var};");
        }
        // return type name so caller can register opaque handle cleanup
        return Ok(SegmentStep::Done(Some(NestedLeafOutcome::Typed(opaque_snake))));
    }
    // Every branch above proved the leaf exists — an explicit `fields_c_types`
    // declaration, or an enum registration. This default proves nothing: it emits
    // `{accessor_fn}()` on faith. When the IR knows the type the walk is standing
    // on and that type has no such field, cbindgen never generated that symbol, so
    // the assertion is rendered against a function that does not exist and the
    // failure surfaces at `cc` time inside a consumer — or, if the generated suite
    // is never compiled, not at all. Nothing upstream catches it either:
    // `FieldResolver::is_valid_for_result` only inspects a path's FIRST segment, so
    // `metadata.<anything>` passes as long as `metadata` is a real field, and the
    // `fail_on_unavailable_field_markers` scan only sees skip comments that this
    // path never writes. Fail here, matching the intermediate arm below. ~keep
    ensure_leaf_field_exists(LeafFieldCheck {
        prefix,
        accessor_fn,
        resolved,
        raw_field,
        segment,
        parent_snake_type: &walk.current_snake_type,
        parent_is_ir_type: walk.current_type_from_ir,
        declared_in_fields_c_types: fields_c_types.contains_key(&lookup_key),
        result_type_name,
        type_defs,
        result_fields_source: &config_sources.result_fields,
        fields_source: &config_sources.fields,
    })?;
    let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({current_handle});");
    Ok(SegmentStep::Continue)
}

/// Inputs for [`step_intermediate_segment`]. A struct, not a dozen positional arguments, for
/// the same reason as [`StepLeafSegmentArgs`].
struct StepIntermediateSegmentArgs<'a> {
    out: &'a mut String,
    prefix: &'a str,
    prefix_upper: &'a str,
    raw_field: &'a str,
    resolved: &'a str,
    segment: &'a str,
    seg_snake: &'a str,
    segments: &'a [&'a str],
    i: usize,
    walk: &'a mut SegmentWalk,
    local_var: &'a str,
    accessor_fn: &'a str,
    fields_c_types: &'a HashMap<String, String>,
    intermediate_handles: &'a mut Vec<(String, String)>,
    result_type_name: &'a str,
    type_defs: &'a [crate::core::ir::TypeDef],
    config_sources: &'a FieldConfigSources,
}

/// One step of [`emit_nested_accessor`]'s walk for a non-leaf plain-field segment (`segment`
/// carries no `[` and we are not already in JSON-extract mode). Extracted verbatim from the
/// walk loop's intermediate branch -- same emitted lines, same conditions, same ordering.
fn step_intermediate_segment(args: StepIntermediateSegmentArgs<'_>) -> anyhow::Result<SegmentStep> {
    let StepIntermediateSegmentArgs {
        out,
        prefix,
        prefix_upper,
        raw_field,
        resolved,
        segment,
        seg_snake,
        segments,
        i,
        walk,
        local_var,
        accessor_fn,
        fields_c_types,
        intermediate_handles,
        result_type_name,
        type_defs,
        config_sources,
    } = args;
    let current_handle = walk.current_handle.clone();
    // Intermediate field — check if it's a char* (JSON string/array) or an opaque handle.
    let lookup_key = format!("{}.{seg_snake}", walk.current_snake_type);
    let return_type_pascal = match fields_c_types
        .get(&lookup_key)
        .cloned()
        .or_else(|| resolve_intermediate_type(&walk.current_snake_type, seg_snake, type_defs))
    {
        Some(return_type) => return_type,
        None => {
            // No silent fallback: deriving the C type from the field name only
            // works when the Rust return type is the literal PascalCase of the
            // field identifier. For accessors whose return type carries a
            // suffix (e.g. `data` -> `DataNode`, `metadata` -> `MetadataConfig`)
            // the guessed name does not match what cbindgen emits and the
            // generated C fails to compile with `unknown type name`. Fail loud
            // here so the operator declares the correct C type explicitly. ~keep
            anyhow::bail!(
                "{}",
                missing_intermediate_type_diagnostic(MissingIntermediateType {
                    prefix,
                    lookup_key: &lookup_key,
                    accessor_fn,
                    resolved,
                    raw_field,
                    segment,
                    seg_snake,
                    segments_walked: &segments[..=i],
                    current_snake_type: &walk.current_snake_type,
                    result_type_name,
                    type_defs,
                    fields_source: &config_sources.fields,
                })
            );
        }
    };

    // Special case: intermediate char* fields (e.g. links, assets) are JSON
    // strings/arrays, not opaque handles. For a `.length` suffix, emit alef_json_array_count.
    if return_type_pascal == "char*" {
        let json_var = format!("{seg_snake}_json");
        if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
            let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
            intermediate_handles.push((json_var.clone(), "free_string".to_string()));
        }
        // If the next (and final) segment is "length", emit the count accessor.
        if i + 2 == segments.len() && segments[i + 1] == "length" {
            let _ = writeln!(out, "    int {local_var} = alef_json_array_count({json_var});");
            return Ok(SegmentStep::Done(Some(NestedLeafOutcome::Typed("int".to_string()))));
        }
        walk.current_snake_type = seg_snake.to_string();
        walk.current_type_from_ir = false;
        walk.current_handle = json_var;
        return Ok(SegmentStep::Continue);
    }

    let return_snake = return_type_pascal.to_snake_case();
    let handle_var = format!("{seg_snake}_handle");

    // Only emit the handle if we haven't already (multiple fields may
    // share the same intermediate path prefix).
    if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
        let _ = writeln!(
            out,
            "    {prefix_upper}AlefHandle {handle_var} = \
             {accessor_fn}({current_handle});"
        );
        let _ = writeln!(out, "    assert({handle_var} != 0);");
        intermediate_handles.push((handle_var.clone(), return_snake.clone()));
    }

    walk.current_type_from_ir = type_defs.iter().any(|type_def| type_def.name == return_type_pascal);
    walk.current_snake_type = return_snake;
    walk.current_handle = handle_var;
    Ok(SegmentStep::Continue)
}

fn resolve_intermediate_type(
    parent_snake: &str,
    field_snake: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    let parent = type_defs
        .iter()
        .find(|type_def| type_def.name.to_snake_case() == parent_snake)?;
    let field = parent
        .fields
        .iter()
        .find(|field| field.name.to_snake_case() == field_snake)?;
    super::named_type(&field.ty).map(str::to_string)
}

/// How deep [`find_field_path`] will search for a field name below the result type.
///
/// The bound exists to terminate on a self-referential IR, not to trade off cost -- this
/// only ever runs on the way to returning an error. Six comfortably clears the chains that
/// motivated it (a real consumer's `ScrapeResult.metadata.article.tags` is three hops); a chain
/// deeper than this just loses the "here is where the field really lives" hint, it does not
/// change the error.
const MAX_FIELD_PATH_SEARCH_DEPTH: usize = 6;

/// Where a field named `field_snake` really lives below some root type.
struct ResolvedFieldChain {
    /// The dotted path from the root type down to the field, e.g. `metadata.article.tags`.
    path: String,
    /// The IR type that actually declares the field. The C accessor symbol is built from
    /// this type, not from the root -- naming it is the difference between the diagnostic
    /// pointing at `cberg_article_metadata_tags` and at the `cberg_scrape_result_tags` that
    /// does not exist.
    owner_type: String,
}

/// Every dotted path from `root_type` down to a field whose snake_case name is
/// `field_snake`, one entry per distinct declaring type, shallowest first.
///
/// Only through `TypeRef::Named` struct fields — the same hops [`emit_nested_accessor`]
/// itself can walk, so a path this returns is one the C codegen could actually emit
/// accessors for.
///
/// More than one entry means the field name is ambiguous below `root_type`: two unrelated
/// types happen to share a field name (e.g. `kind` declared on both `DataNode`, values
/// `object`/`array`/`scalar`, and `StructureItem`, values `function`/`class`). A caller that
/// would otherwise propose a single alias fix MUST check `len() > 1` first and refuse to
/// guess — silently picking one binds the fixture to a field with a different value domain
/// instead of failing loudly. Finding this required tslp-owner to catch, by hand, a
/// generated diagnostic that suggested exactly that corrupting alias. ~keep
fn find_all_field_paths(
    root_type: &str,
    field_snake: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Vec<ResolvedFieldChain> {
    fn walk(
        type_name: &str,
        field_snake: &str,
        type_defs: &[crate::core::ir::TypeDef],
        depth: usize,
        seen: &mut HashSet<String>,
        out: &mut Vec<ResolvedFieldChain>,
    ) {
        if depth == 0 || !seen.insert(type_name.to_string()) {
            return;
        }
        let Some(type_def) = type_defs.iter().find(|type_def| type_def.name == type_name) else {
            return;
        };
        if let Some(field) = type_def
            .fields
            .iter()
            .find(|field| field.name.to_snake_case() == field_snake)
        {
            out.push(ResolvedFieldChain {
                path: field.name.to_snake_case(),
                owner_type: type_def.name.clone(),
            });
        }
        // Keep walking nested fields even after a direct hit above: a distinct type
        // reachable through a sibling or deeper field may ALSO declare `field_snake`, and
        // that collision is exactly what this function exists to surface.
        for field in &type_def.fields {
            let Some(nested) = super::named_type(&field.ty) else {
                continue;
            };
            let before = out.len();
            walk(nested, field_snake, type_defs, depth - 1, seen, out);
            for chain in &mut out[before..] {
                chain.path = format!("{}.{}", field.name.to_snake_case(), chain.path);
            }
        }
    }

    let mut out = Vec::new();
    walk(
        root_type,
        field_snake,
        type_defs,
        MAX_FIELD_PATH_SEARCH_DEPTH,
        &mut HashSet::new(),
        &mut out,
    );
    out.sort_by_key(|chain| chain.path.matches('.').count());
    out
}

/// The dotted path from `root_type` down to a field whose snake_case name is
/// `field_snake`, when the name is declared by exactly one reachable type.
///
/// Returns `None` both when no type reachable from `root_type` has such a field AND when
/// more than one distinct type does — see [`find_all_field_paths`] for why an ambiguous
/// name cannot collapse to a single answer here. Callers that need to tell those two cases
/// apart (to phrase a different diagnostic for each) must call `find_all_field_paths`
/// directly instead of this wrapper.
///
/// Test-only: every production caller needs the ambiguous and absent cases phrased differently, so
/// they all call `find_all_field_paths`. The wrapper stays to pin the collapse rule itself. ~keep
#[cfg(test)]
fn find_field_path(
    root_type: &str,
    field_snake: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<ResolvedFieldChain> {
    let mut chains = find_all_field_paths(root_type, field_snake, type_defs);
    if chains.len() == 1 { chains.pop() } else { None }
}

/// The leading segments `resolved` lost to virtual-namespace stripping, if any.
///
/// [`emit_nested_accessor`] is handed the already-stripped path (the callers in
/// `test_function.rs`/`call_patterns.rs` strip before calling), so the only surviving record
/// of a stripped prefix is that `raw_field` ends with `resolved`. Recovering it is what lets
/// the diagnostic tell "add an alias" apart from "add a type mapping": a path that lost a
/// segment is almost always a missing `[crates.e2e.fields]` alias, and declaring the C type
/// the message names would instead emit a call to a symbol that does not exist. ~keep
fn stripped_namespace_prefix<'a>(raw_field: &'a str, resolved: &str) -> Option<&'a str> {
    let prefix_len = raw_field.len().checked_sub(resolved.len())?;
    if prefix_len == 0 || !raw_field.ends_with(resolved) {
        return None;
    }
    raw_field
        .get(..prefix_len)
        .and_then(|prefix| prefix.strip_suffix('.'))
        .filter(|prefix| !prefix.is_empty())
}

/// Why `resolve_intermediate_type` could not derive a C type for `{parent_snake}.{field_snake}`.
///
/// The three ways it returns `None` need three different fixes, and the missing key alone
/// cannot tell them apart: an unknown parent type means the walk arrived somewhere it should
/// never have been (usually namespace stripping), a missing field means the path is wrong,
/// and a non-`Named` field type means the path is right but the accessor returns something
/// no opaque handle can carry. ~keep
fn why_the_type_is_unknown(parent_snake: &str, field_snake: &str, type_defs: &[crate::core::ir::TypeDef]) -> String {
    let Some(parent) = type_defs
        .iter()
        .find(|type_def| type_def.name.to_snake_case() == parent_snake)
    else {
        return format!("No IR type has the snake_case name `{parent_snake}`");
    };
    let Some(field) = parent
        .fields
        .iter()
        .find(|field| field.name.to_snake_case() == field_snake)
    else {
        return format!("Type `{}` has no field `{field_snake}`", parent.name);
    };
    if super::named_type(&field.ty).is_none() {
        return format!(
            "Field `{}.{field_snake}` is not a named struct type, so no opaque accessor type can be derived from it",
            parent.name
        );
    }
    format!("Type `{}` does have a field `{field_snake}`", parent.name)
}

/// Inputs for [`missing_intermediate_type_diagnostic`]. A struct, not a dozen positional
/// `&str`s, so two of them cannot be swapped without the compiler noticing.
struct MissingIntermediateType<'a> {
    /// The crate's FFI symbol prefix, for naming the accessor that really exists.
    prefix: &'a str,
    /// The `"{parent_snake}.{field_snake}"` key that was looked up and missed.
    lookup_key: &'a str,
    /// The C symbol the walk would call if that key were simply declared.
    accessor_fn: &'a str,
    /// The (already namespace-stripped) path being walked.
    resolved: &'a str,
    /// The fixture's own field path, before alias resolution and namespace stripping.
    raw_field: &'a str,
    segment: &'a str,
    seg_snake: &'a str,
    /// `resolved`'s segments up to and including the failing one.
    segments_walked: &'a [&'a str],
    /// The snake_case type the walk is standing on.
    current_snake_type: &'a str,
    /// The type the walk started from.
    result_type_name: &'a str,
    type_defs: &'a [crate::core::ir::TypeDef],
    /// Which `fields` (alias table) governs the call this hop belongs to — threaded
    /// through so the diagnostic can name the one config key an edit will actually reach.
    fields_source: &'a EffectiveConfigSource,
}

/// Explain a missing `fields_c_types` key in terms of the chain that produced it.
///
/// The bare "missing key `{parent}.{field}`" this replaced implied its own remedy — declare
/// that key — and the implied remedy is wrong whenever the key names a field the parent type
/// does not have. Adding it silences the failure and emits a call to a C function that was
/// never generated, which then fails at `cc` time (or, worse, links against an unrelated
/// symbol). So the message has to carry three things the key alone cannot: which prefix
/// alef stripped as a virtual namespace to arrive at this path, which symbol declaring the
/// key would conjure, and where the field really lives under the result type. ~keep
fn missing_intermediate_type_diagnostic(context: MissingIntermediateType<'_>) -> String {
    let MissingIntermediateType {
        prefix,
        lookup_key,
        accessor_fn,
        resolved,
        raw_field,
        segment,
        seg_snake,
        segments_walked,
        current_snake_type,
        result_type_name,
        type_defs,
        fields_source,
    } = context;

    let mut message = format!(
        "e2e c codegen: fields_c_types is missing key \"{lookup_key}\" (path \"{resolved}\", segment \"{segment}\"), \
         reached while walking fixture field \"{raw_field}\" from result type `{result_type_name}`. {why}, so \
         declaring \"{lookup_key}\" would make the generated test call `{accessor_fn}()`. (The old fallback guessed \
         `{guess}` from the field name, which silently miscompiled whenever the Rust return type differed, e.g. \
         `DataNode` vs `Data`.)",
        why = why_the_type_is_unknown(current_snake_type, seg_snake, type_defs),
        guess = segment.to_pascal_case(),
    );

    if let Some(namespace) = stripped_namespace_prefix(raw_field, resolved) {
        let _ = write!(
            message,
            " alef stripped the leading \"{namespace}\" from \"{raw_field}\" as a virtual namespace, because no \
             `[crates.e2e.fields]` alias maps it onto a real path and its first segment is not a `result_fields` \
             entry -- which is why the walk started at `{result_type_name}` instead of inside `{namespace}`."
        );
    }

    match find_all_field_paths(result_type_name, seg_snake, type_defs).as_slice() {
        [chain] => {
            append_single_field_chain_fix(SingleFieldChainFixArgs {
                message: &mut message,
                prefix,
                lookup_key,
                accessor_fn,
                raw_field,
                resolved,
                seg_snake,
                segments_walked,
                result_type_name,
                fields_source,
                chain,
            });
        }
        [] => {
            let _ = write!(
                message,
                " No type reachable from `{result_type_name}` has a field named `{seg_snake}` either, so the \
                 fixture's field path is the thing to check first -- declaring \"{lookup_key}\" cannot make \
                 `{accessor_fn}()` exist."
            );
        }
        chains => {
            let _ = write!(
                message,
                "{}",
                ambiguous_field_name_suffix(seg_snake, result_type_name, chains, fields_source)
            );
        }
    }

    message
}

/// Inputs for [`append_single_field_chain_fix`]. A struct, not eleven positional arguments,
/// following this file's existing convention (see [`MissingIntermediateType`]) for a helper
/// with this many related-but-distinct parameters.
struct SingleFieldChainFixArgs<'a> {
    message: &'a mut String,
    prefix: &'a str,
    lookup_key: &'a str,
    accessor_fn: &'a str,
    raw_field: &'a str,
    resolved: &'a str,
    seg_snake: &'a str,
    segments_walked: &'a [&'a str],
    result_type_name: &'a str,
    fields_source: &'a EffectiveConfigSource,
    chain: &'a ResolvedFieldChain,
}

/// Append the "field does exist at exactly one location, add this alias" suffix to `message`
/// for [`missing_intermediate_type_diagnostic`]. Extracted verbatim -- same emitted text, same
/// conditions, same ordering.
fn append_single_field_chain_fix(args: SingleFieldChainFixArgs<'_>) {
    let SingleFieldChainFixArgs {
        message,
        prefix,
        lookup_key,
        accessor_fn,
        raw_field,
        resolved,
        seg_snake,
        segments_walked,
        result_type_name,
        fields_source,
        chain,
    } = args;
    let alias_key = match stripped_namespace_prefix(raw_field, resolved) {
        Some(namespace) => format!("{namespace}.{}", segments_walked.join(".")),
        None => segments_walked.join("."),
    };
    let real_path = &chain.path;
    let real_symbol = format!("{prefix}_{}_{seg_snake}", chain.owner_type.to_snake_case());
    // Same shadowing rule as the leaf diagnostic's alias-fix branch, `fields`
    // instead of `result_fields`: a non-empty per-call `fields` override
    // replaces the global alias table outright (`E2eConfig::effective_fields`),
    // so the alias must be spelled under whichever one actually governs this
    // call. ~keep
    let fields_key = match fields_source {
        EffectiveConfigSource::Global => "`[crates.e2e.fields]`".to_string(),
        EffectiveConfigSource::PerCall(label) => format!("`{label}.fields`"),
    };
    let _ = write!(
        message,
        " Field `{seg_snake}` does exist below `{result_type_name}`, at \"{real_path}\" -- it is declared on \
         `{owner}`, so the accessor that really exists is `{real_symbol}()`. Fix: add \
         \"{alias_key}\" = \"{real_path}\" under {fields_key} so the fixture path resolves to the \
         real chain. Only add \"{lookup_key}\" to `[crates.e2e.fields_c_types]` if `{accessor_fn}()` really \
         is in the generated header.",
        owner = chain.owner_type,
    );
}

/// Describe an ambiguous field name (declared by more than one distinct type reachable from
/// `result_type_name`) without picking one for the caller.
///
/// Shared by both diagnostics that would otherwise call [`find_field_path`] and silently take
/// its `None` for "field does not exist" -- an ambiguous name is a different failure mode
/// entirely, and conflating the two is how this diagnostic once recommended a corrupting
/// fix: `find_field_path` returned whichever same-named field it found first (e.g.
/// `DataNode.kind`, values `object`/`array`/`scalar`, vs an unrelated `StructureItem.kind`,
/// values `function`/`class`), and the message confidently suggested aliasing to it. Naming
/// every candidate chain, and refusing to recommend any single one of them, is the fix: the
/// operator -- who knows which chain the fixture actually means -- has to pick. ~keep
fn ambiguous_field_name_suffix(
    seg_snake: &str,
    result_type_name: &str,
    chains: &[ResolvedFieldChain],
    fields_source: &EffectiveConfigSource,
) -> String {
    let candidates: Vec<String> = chains
        .iter()
        .map(|chain| format!("\"{}\" (declared on `{}`)", chain.path, chain.owner_type))
        .collect();
    // Same shadowing rule as every other alias-fix branch in this file: a non-empty
    // per-call `fields` override replaces the global alias table outright
    // (`E2eConfig::effective_fields`), so the manual alias this suggests has to be
    // spelled under whichever one actually governs this call. ~keep
    let fields_key = match fields_source {
        EffectiveConfigSource::Global => "`[crates.e2e.fields]`".to_string(),
        EffectiveConfigSource::PerCall(label) => format!("`{label}.fields`"),
    };
    format!(
        " Field `{seg_snake}` is declared on {count} unrelated types reachable from `{result_type_name}`, with \
         different chains: {candidates} -- alef cannot tell which one the fixture means, and guessing risks \
         binding the assertion to a field with a different value domain than intended. Fix: add \
         \"<fixture path>\" = \"<the correct chain from the list above>\" under {fields_key} yourself, \
         after checking which candidate actually matches this fixture's data.",
        count = chains.len(),
        candidates = candidates.join(", "),
    )
}

/// Where a per-call-overridable e2e config collection (`result_fields`, `fields`, ...)
/// actually came from for a given call: the per-call override, or the global
/// `[crates.e2e]` default that only applies when the call declares no override of its
/// own.
///
/// Exists so a diagnostic can name the ONE config key an edit will actually reach.
/// Every `E2eConfig::effective_*` method (`effective_result_fields`, `effective_fields`,
/// ...) REPLACES the global collection outright when a call's own collection is
/// non-empty — it never merges the two — so a message that always names the global key
/// is actively wrong for every call with an override. That exact wrongness shipped once
/// already for `result_fields`: it told a consumer with a per-call override to edit the
/// global key, they did, nothing changed, and they filed it as a codegen blocker. The
/// same shape lived on, unfixed, in every diagnostic that names `[crates.e2e.fields]` —
/// this type is shared by both so the two checks cannot drift onto different resolution
/// logic the way the two hand-rolled versions of it did before this. ~keep
pub(super) enum EffectiveConfigSource {
    /// The global `[crates.e2e]` collection is what's in effect for this call.
    Global,
    /// A per-call override is what's in effect, named by its TOML table path (e.g.
    /// `"[crates.e2e.calls.crawl]"`, or the unnamed default `"[crates.e2e.call]"`).
    PerCall(String),
}

/// Determine which instance of a per-call-overridable collection governs `call`: pass
/// `call_has_override` as `!call.result_fields.is_empty()`, `!call.fields.is_empty()`,
/// etc. — whichever collection the caller is resolving — since that emptiness check is
/// the only part of [`E2eConfig::effective_result_fields`]/[`E2eConfig::effective_fields`]
/// (and siblings) that differs per collection; the "which key names it" logic that
/// follows is identical for all of them.
///
/// `call` is matched against `e2e_config.calls`/`e2e_config.call` by pointer identity
/// rather than by name, because a caller that reached `call` through
/// `resolve_call_for_fixture`'s `select_when` auto-routing does not get the matched key
/// back — the resolved `&CallConfig` reference is the only thing both the explicit-name
/// path and the auto-routed path have in common. ~keep
pub(super) fn describe_effective_config_source(
    e2e_config: &E2eConfig,
    call: &CallConfig,
    call_has_override: bool,
) -> EffectiveConfigSource {
    if !call_has_override {
        return EffectiveConfigSource::Global;
    }
    match e2e_config
        .calls
        .iter()
        .find(|(_, candidate)| std::ptr::eq(*candidate, call))
    {
        Some((name, _)) => EffectiveConfigSource::PerCall(format!("[crates.e2e.calls.{name}]")),
        None => EffectiveConfigSource::PerCall("[crates.e2e.call]".to_string()),
    }
}

/// The `result_fields` and `fields` sources actually in effect for one call, resolved
/// once per fixture and threaded through every nested-field diagnostic for it. Bundled
/// rather than passed as two loose parameters so a diagnostic that needs both (the leaf
/// diagnostic proposes a `result_fields` fix on one path and a `fields` alias fix on
/// another) cannot accidentally receive one resolved against a different call than the
/// other. ~keep
pub(super) struct FieldConfigSources {
    pub result_fields: EffectiveConfigSource,
    pub fields: EffectiveConfigSource,
}

impl FieldConfigSources {
    pub(super) fn resolve(e2e_config: &E2eConfig, call: &CallConfig) -> Self {
        Self {
            result_fields: describe_effective_config_source(e2e_config, call, !call.result_fields.is_empty()),
            fields: describe_effective_config_source(e2e_config, call, !call.fields.is_empty()),
        }
    }
}

/// Inputs for [`ensure_leaf_field_exists`]. A struct, not a handful of positional
/// `&str`s, for the same reason [`MissingIntermediateType`] is one.
pub(super) struct LeafFieldCheck<'a> {
    /// The crate's FFI symbol prefix, for naming the accessor that really exists.
    pub prefix: &'a str,
    /// The C symbol the caller is about to emit for this leaf.
    pub accessor_fn: &'a str,
    /// The (alias-resolved, already namespace-stripped) path being walked.
    pub resolved: &'a str,
    /// The fixture's own field path, before alias resolution and namespace stripping.
    pub raw_field: &'a str,
    /// The leaf segment itself, in its fixture spelling.
    pub segment: &'a str,
    /// The snake_case name of the type the accessor will be called on.
    pub parent_snake_type: &'a str,
    /// Whether `parent_snake_type` really names an IR type. False after a `char*` hop,
    /// where it holds a *field* name, and for a result type the IR does not model — in
    /// both cases an IR type sharing the name is a coincidence, not the parent.
    pub parent_is_ir_type: bool,
    /// Whether the operator declared this exact leaf in `[crates.e2e.fields_c_types]`.
    /// An explicit declaration is a claim that the accessor exists, and stays authoritative.
    pub declared_in_fields_c_types: bool,
    /// The type the walk started from.
    pub result_type_name: &'a str,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    /// Which `result_fields` set governs the call this leaf belongs to — threaded through
    /// so the diagnostic can name the one config key an edit will actually reach.
    pub result_fields_source: &'a EffectiveConfigSource,
    /// Which `fields` (alias table) governs the call this leaf belongs to — same reason
    /// as `result_fields_source`, for the diagnostic's alias-fix branches.
    pub fields_source: &'a EffectiveConfigSource,
}

/// Reject a leaf field the IR positively says the parent type does not have.
///
/// The C accessor for a leaf is `{prefix}_{parent_snake}_{leaf_snake}`, built from a name
/// rather than looked up, so nothing but the IR can tell a real accessor from a fabricated
/// one. Default-allow everywhere the IR cannot answer: silence is not evidence of absence,
/// and this is a hard generation failure. ~keep
pub(super) fn ensure_leaf_field_exists(check: LeafFieldCheck<'_>) -> anyhow::Result<()> {
    if !check.parent_is_ir_type || check.declared_in_fields_c_types || check.resolved.contains('[') {
        return Ok(());
    }
    let seg_snake = check.segment.to_snake_case();
    let Some(parent) = check
        .type_defs
        .iter()
        .find(|type_def| type_def.name.to_snake_case() == check.parent_snake_type)
    else {
        return Ok(());
    };
    if parent
        .fields
        .iter()
        .any(|field| field.name.to_snake_case() == seg_snake)
    {
        return Ok(());
    }
    anyhow::bail!(
        "{}",
        unknown_leaf_field_diagnostic(UnknownLeafField {
            prefix: check.prefix,
            accessor_fn: check.accessor_fn,
            resolved: check.resolved,
            raw_field: check.raw_field,
            segment: check.segment,
            seg_snake: &seg_snake,
            parent_type: &parent.name,
            result_type_name: check.result_type_name,
            type_defs: check.type_defs,
            result_fields_source: check.result_fields_source,
            fields_source: check.fields_source,
        })
    )
}

/// Inputs for [`unknown_leaf_field_diagnostic`], resolved from a [`LeafFieldCheck`].
struct UnknownLeafField<'a> {
    prefix: &'a str,
    accessor_fn: &'a str,
    resolved: &'a str,
    raw_field: &'a str,
    segment: &'a str,
    seg_snake: &'a str,
    /// The IR type the walk is standing on, in its declared PascalCase spelling.
    parent_type: &'a str,
    result_type_name: &'a str,
    type_defs: &'a [crate::core::ir::TypeDef],
    result_fields_source: &'a EffectiveConfigSource,
    fields_source: &'a EffectiveConfigSource,
}

/// Explain a leaf segment that names no field of the type the walk arrived at.
///
/// The intermediate arm can at least offer "declare the C type"; a leaf cannot, because the
/// leaf accessor is emitted from the parent type and the field name alone. So the only
/// honest remedies are the alias that reconnects the fixture path to the real chain, or
/// fixing the fixture path — and the message has to say which, by looking up where the field
/// really lives. Same three facts as [`missing_intermediate_type_diagnostic`], same
/// resolution machinery, different remedy. ~keep
fn unknown_leaf_field_diagnostic(context: UnknownLeafField<'_>) -> String {
    let UnknownLeafField {
        prefix,
        accessor_fn,
        resolved,
        raw_field,
        segment,
        seg_snake,
        parent_type,
        result_type_name,
        type_defs,
        result_fields_source,
        fields_source,
    } = context;

    let mut message = format!(
        "e2e c codegen: fixture field \"{raw_field}\" (path \"{resolved}\") ends at segment \"{segment}\", but IR \
         type `{parent_type}` has no field `{seg_snake}`. The walk was about to emit `{accessor_fn}()`, a C symbol \
         no binding generates, so this assertion would have been rendered against a function that does not exist. \
         Nothing upstream rejects it: the field-availability oracle (`FieldResolver::is_valid_for_result`) only \
         inspects a path's FIRST segment, which is a real field here."
    );

    let namespace = stripped_namespace_prefix(raw_field, resolved);
    if let Some(namespace) = namespace {
        let _ = write!(
            message,
            " alef stripped the leading \"{namespace}\" from \"{raw_field}\" as a virtual namespace, because no \
             `[crates.e2e.fields]` alias maps it onto a real path and its first segment is not a `result_fields` \
             entry -- which is why the walk started at `{result_type_name}` instead of inside `{namespace}`."
        );
    }

    let chains = find_all_field_paths(result_type_name, seg_snake, type_defs);
    let chain = match chains.as_slice() {
        [chain] => chain,
        [] => {
            let _ = write!(
                message,
                " No type reachable from `{result_type_name}` has a field named `{seg_snake}` either, so the \
                 fixture's field path is the thing to fix -- there is no config entry that can spell a chain which \
                 does not exist."
            );
            return message;
        }
        chains => {
            let _ = write!(
                message,
                "{}",
                ambiguous_field_name_suffix(seg_snake, result_type_name, chains, fields_source)
            );
            return message;
        }
    };

    let real_path = &chain.path;
    let real_symbol = format!("{prefix}_{}_{seg_snake}", chain.owner_type.to_snake_case());
    let _ = write!(
        message,
        " Field `{seg_snake}` does exist below `{result_type_name}`, at \"{real_path}\" -- it is declared on \
         `{owner}`, so the accessor that really exists is `{real_symbol}()`.",
        owner = chain.owner_type,
    );

    append_unknown_leaf_field_fix(&mut message, namespace, raw_field, real_path, result_fields_source, fields_source);

    message
}

/// Append the "here's how to fix it" suffix to `message` for
/// [`unknown_leaf_field_diagnostic`], once exactly one real chain for the field has been
/// found. Extracted verbatim -- same emitted text, same conditions, same ordering.
fn append_unknown_leaf_field_fix(
    message: &mut String,
    namespace: Option<&str>,
    raw_field: &str,
    real_path: &str,
    result_fields_source: &EffectiveConfigSource,
    fields_source: &EffectiveConfigSource,
) {
    // Two different config bugs produce this, and they take opposite fixes. When the real
    // chain starts with the prefix that was stripped, the fixture path was right all along
    // and the stripping was the mistake -- an alias would be an identity mapping and change
    // nothing, because `namespace_stripped_path` consults only `result_fields`. Otherwise the
    // fixture path genuinely names a chain that does not exist and needs an alias. ~keep
    match namespace.filter(|namespace| real_path.starts_with(&format!("{namespace}."))) {
        Some(namespace) => {
            // `result_fields` here means whichever set `effective_result_fields` actually
            // resolved for THIS call -- a non-empty per-call override replaces the global
            // default outright (see `E2eConfig::effective_result_fields`), so naming the
            // global key when a per-call override shadows it sends an edit nowhere: a
            // consumer followed exactly that instruction, edited the global key, and it
            // changed nothing because their call had its own `result_fields`. ~keep
            let result_fields_key = match result_fields_source {
                EffectiveConfigSource::Global => "`[crates.e2e].result_fields`".to_string(),
                EffectiveConfigSource::PerCall(label) => format!("`{label}.result_fields`"),
            };
            let _ = write!(
                message,
                " Fix: add \"{namespace}\" to {result_fields_key} so alef stops treating it as a virtual \
                 namespace prefix and walks it as the real field it is. An alias here would be an identity mapping \
                 and would not stop the stripping."
            );
        }
        None => {
            // Same shadowing rule, `[crates.e2e.fields]` instead of `.result_fields`: a
            // non-empty per-call `fields` override replaces the global alias table
            // outright (`E2eConfig::effective_fields`), so the alias must be spelled
            // under whichever one actually governs this call. ~keep
            let fields_key = match fields_source {
                EffectiveConfigSource::Global => "`[crates.e2e.fields]`".to_string(),
                EffectiveConfigSource::PerCall(label) => format!("`{label}.fields`"),
            };
            let _ = write!(
                message,
                " Fix: add \"{raw_field}\" = \"{real_path}\" under {fields_key} so the fixture path \
                 resolves to the real chain."
            );
        }
    }
}

/// The three-state view of the target's declared parameters this file renders against.
///
/// Defined in [`crate::e2e::codegen::call_ir`] because every backend needs the same three
/// states; the C-specific part is what this file *does* with them, not the states. ~keep
pub(super) use crate::e2e::codegen::call_ir::TargetParams;

/// The `alef.toml` key whose `args` list governs this fixture's call, named so every
/// diagnostic below points the operator at the table they actually have to edit -- the
/// per-call `[crates.e2e.calls.<name>]` one when the fixture selects a named call, the
/// default `[crates.e2e.call]` otherwise. ~keep
fn args_config_key(fixture: &Fixture) -> String {
    match fixture.call.as_deref() {
        Some(name) => format!("[crates.e2e.calls.{name}].args"),
        None => "[crates.e2e.call].args".to_string(),
    }
}

/// Fixture "{id}" calls `{function_name}` with no configured `args`, but the target's IR
/// signature declares real parameters -- an authoring gap, not a zero-argument call. ~keep
fn missing_args_for_known_params_diagnostic(
    fixture: &Fixture,
    function_name: &str,
    params: &[crate::core::ir::ParamDef],
) -> String {
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    let call_key = args_config_key(fixture);
    format!(
        "e2e c codegen: fixture \"{id}\" calls `{function_name}` with no configured `args`, but the Rust core \
         signature for `{function_name}` declares {count} parameter(s): {joined_names}. With no `args` \
         configured, alef used to splice the fixture's whole `input` JSON as a single C string literal \
         regardless of what the target actually takes, which does not compile against anything but a lone \
         string parameter. Fix: add an `args` entry under `{call_key}` for each parameter, mapping it to the \
         fixture input field that supplies it.",
        id = fixture.id,
        count = params.len(),
        joined_names = names.join(", "),
    )
}

/// Fixture "{id}" calls `{function_name}` with no configured `args`, and alef cannot resolve
/// the target's IR signature at all -- refuse rather than guess whether that is a genuine
/// zero-argument call or a missing `args` configuration. ~keep
fn missing_args_unresolvable_signature_diagnostic(fixture: &Fixture, function_name: &str) -> String {
    let call_key = args_config_key(fixture);
    format!(
        "e2e c codegen: fixture \"{id}\" calls `{function_name}` with no configured `args`, and alef could not \
         resolve `{function_name}` against the Rust core IR, so it cannot tell a genuine zero-argument call from \
         a missing `args` configuration -- guessing risks splicing the fixture's whole `input` JSON as one C \
         literal against a target that takes real, typed parameters. Fix: configure `args` under `{call_key}`, \
         one entry per parameter `{function_name}` actually takes. If it genuinely takes none, check that this \
         call's `function` name (and any per-language override) matches a real core function or method name -- \
         an unresolvable name is why alef cannot confirm that on its own.",
        id = fixture.id,
    )
}

/// How much of the offending literal [`handle_param_type_mismatch_diagnostic`] quotes back.
///
/// The value is named so the operator can find the `args` entry that produced it, but a
/// fixture `input` can be arbitrarily large and the diagnostic is not a place to reprint it. ~keep
const MAX_DIAGNOSTIC_VALUE_CHARS: usize = 80;

/// Fixture "{id}" maps an `args` entry onto a parameter the C ABI exports as an opaque
/// handle, but the fixture value lowers to a plain C literal. ~keep
fn handle_param_type_mismatch_diagnostic(
    fixture: &Fixture,
    function_name: &str,
    arg: &crate::e2e::config::ArgMapping,
    param: &crate::core::ir::ParamDef,
    param_type: &crate::core::ir::TypeDef,
    rendered: &str,
) -> String {
    let call_key = args_config_key(fixture);
    let quoted: String = rendered.chars().take(MAX_DIAGNOSTIC_VALUE_CHARS).collect();
    let elided = if quoted.len() < rendered.len() { "..." } else { "" };
    let type_name = &param_type.name;
    let mut message = format!(
        "e2e c codegen: fixture \"{id}\" maps `args` entry \"{arg_name}\" (type = \"{arg_type}\", field = \
         \"{field}\") onto parameter `{param_name}` of `{function_name}`, which the Rust core declares as \
         `{type_name}` and the C ABI exports as `AlefHandle` -- an unsigned integer handle, not a pointer or \
         a string. The fixture value lowers to the C literal {quoted}{elided}, and passing a literal where a \
         handle is expected does not compile (`incompatible pointer to integer conversion`). A handle only \
         exists once something constructs it, and alef will not fabricate one.",
        id = fixture.id,
        arg_name = arg.name,
        arg_type = arg.arg_type,
        field = arg.field,
        param_name = param.name,
    );
    if arg.arg_type == "json_object" {
        let _ = write!(
            message,
            " This entry already declares `type = \"json_object\"`, so the gap is on alef's side: this call \
             path rendered the arguments without constructing any typed handle first (the `returns_void` \
             snippet path in `c/test_function.rs` passes an empty handle map, unlike the free-function path, \
             which emits the `from_json` construction ahead of the call). Until that path constructs handles, \
             this fixture needs an extension-owned documentation recipe for C, or a documented \
             `coverage_exceptions` entry."
        );
    } else if param_type.has_serde {
        let _ = write!(
            message,
            " Fix: set `type = \"json_object\"` and `element_type = \"{type_name}\"` on that entry under \
             {call_key}, so alef constructs the handle with the generated `from_json` helper and passes that \
             instead of the literal."
        );
    } else {
        let _ = write!(
            message,
            " `{type_name}` derives no serde, so the FFI crate exports no `from_json` constructor for it and \
             `type = \"json_object\"` would name a symbol that does not exist. This fixture needs an \
             extension-owned documentation recipe for C, or a documented `coverage_exceptions` entry."
        );
    }
    message
}

/// Refuse an argument whose lowering contradicts the type of the parameter it lands in.
///
/// The sibling refusal above covers the ABSENCE of `args` -- "no args configured, do not
/// fabricate an argument list". This covers the opposite case, which nothing checked: `args`
/// are present, so the arity is satisfied and no refusal fires, but the value is rendered by
/// `json_to_c` with no reference whatsoever to what the parameter is declared to be. A fixture
/// `input` object lowered that way becomes a C string literal, and against a parameter the FFI
/// exports as `AlefHandle` that is an int-conversion error, not a working call.
///
/// The check is deliberately narrow, because a false refusal deletes published documentation:
/// it fires only when the IR both names the parameter's type and carries a `TypeDef` for it.
/// An IR enum is an `EnumDef`, never a `TypeDef`, and enum-typed `Named` parameters cross as
/// `i32` rather than as a handle -- so a name that matches no `TypeDef` cannot be proven to be
/// a handle and is left alone. Parameter matching follows `resolve_call_info`'s `element_type`
/// backfill in `c.rs` exactly (by name, else positionally); the two must agree about which
/// parameter an `args` entry fills or they would be reasoning about different parameters. ~keep
/// Inputs for [`ensure_arg_matches_param_type`]. A struct, not seven positional arguments,
/// following this file's existing convention (see [`MissingIntermediateType`],
/// [`LeafFieldCheck`]) for a helper with this many related-but-distinct parameters.
struct EnsureArgMatchesParamTypeArgs<'a> {
    fixture: &'a Fixture,
    function_name: &'a str,
    arg: &'a crate::e2e::config::ArgMapping,
    index: usize,
    params: &'a [crate::core::ir::ParamDef],
    type_defs: &'a [crate::core::ir::TypeDef],
    rendered: &'a str,
}

fn ensure_arg_matches_param_type(args: EnsureArgMatchesParamTypeArgs<'_>) -> anyhow::Result<()> {
    let EnsureArgMatchesParamTypeArgs {
        fixture,
        function_name,
        arg,
        index,
        params,
        type_defs,
        rendered,
    } = args;
    let Some(param) = TargetParams::Known(params).param_for(&arg.name, index) else {
        return Ok(());
    };
    let Some(type_name) = handle_param_type_name(&param.ty) else {
        return Ok(());
    };
    let Some(param_type) = type_defs.iter().find(|type_def| type_def.name == type_name) else {
        return Ok(());
    };
    anyhow::bail!(
        "{}",
        handle_param_type_mismatch_diagnostic(fixture, function_name, arg, param, param_type, rendered)
    )
}

/// Build the C argument string for the function call.
/// When `has_options_handle` is true, json_object args are replaced with
/// the `options_handle` pointer (which was constructed via `from_json`).
///
/// `target_params` decides what an empty `args` renders as: a genuinely zero-argument target
/// (`TargetParams::Known(&[])`) emits `""` (an empty call), anything else refuses rather than
/// fabricate an argument list the target's real parameters (or the emitter's ignorance of
/// them) cannot justify. See [`TargetParams`].
///
/// It also decides whether a *present* argument may be rendered at all: a satisfied argument
/// count is not a satisfied argument type, so every value that would be lowered by `json_to_c`
/// is checked against the parameter it fills -- see [`ensure_arg_matches_param_type`].
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_string_c(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    typed_arg_handles: &HashMap<String, String>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
    function_name: &str,
    target_params: TargetParams<'_>,
) -> anyhow::Result<String> {
    if args.is_empty() {
        return match target_params {
            TargetParams::Known([]) => Ok(String::new()),
            TargetParams::Known(params) => {
                anyhow::bail!(
                    "{}",
                    missing_args_for_known_params_diagnostic(fixture, function_name, params)
                )
            }
            TargetParams::IrAbsent => Ok(json_to_c(input)),
            TargetParams::Unresolvable => {
                anyhow::bail!(
                    "{}",
                    missing_args_unresolvable_signature_diagnostic(fixture, function_name)
                )
            }
        };
    }

    // The parameters a rendered argument can be checked against, if any. `IrAbsent` and
    // `Unresolvable` learned nothing about the target, so they license no type claim -- the
    // same asymmetry the empty-`args` match above encodes. ~keep
    let known_params = target_params.known();

    let mut parts: Vec<String> = Vec::new();

    for (index, arg) in args.iter().enumerate() {
        // Handle test_backend args: emit the stub and use it.
        if arg.arg_type == "test_backend" {
            parts.push(build_test_backend_arg_expr(config, type_defs, fixture, arg));
            continue;
        }

        push_regular_arg_expr(&mut parts, RegularArgArgs {
            input,
            arg,
            index,
            typed_arg_handles,
            known_params,
            type_defs,
            fixture,
            function_name,
            target_params,
        })?;
    }

    Ok(parts.join(", "))
}

/// Build the C argument expression for a `test_backend` arg, which fills a C trait-bridge
/// vtable-pointer parameter. Extracted verbatim from [`build_args_string_c`]'s per-arg loop --
/// same panics, same trait/method resolution, same emitted stub.
fn build_test_backend_arg_expr(
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
    arg: &crate::e2e::config::ArgMapping,
) -> String {
    // A `test_backend` arg fills a C trait-bridge vtable-pointer parameter.
    // There is no fixture-supplied value to fall back to: an unregistered
    // trait has no vtable to point at, and `emit_test_backend` panics rather
    // than hand back a placeholder for `parts` to splice in as an
    // expression — splicing either would emit C that cannot compile. Unlike a non-null-typed
    // target language, C's type system would happily accept a `NULL` fallback
    // here too (any pointer type admits it), so the compiler can't be relied on
    // to catch a bad default the way it can elsewhere — fail loud here instead,
    // matching every other "cannot render this" case in this file (see
    // `resolve_intermediate_type`'s `None` arm above, and the assertion-type
    // panics below). ~keep
    let Some(trait_name) = &arg.trait_name else {
        panic!(
            "C e2e generator: fixture `{}` declares a `test_backend` arg `{}` with no `trait_name` configured; cannot generate a C stub without knowing which trait to implement",
            fixture.id, arg.name
        );
    };
    let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) else {
        panic!(
            "C e2e generator: fixture `{}` requires trait `{trait_name}` for its `test_backend` arg `{}`, but no `[[crates.trait_bridges]]` entry named `{trait_name}` is configured",
            fixture.id, arg.name
        );
    };
    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
        .iter()
        .find(|t| t.name == *trait_name)
        .map(|t| t.methods.iter().collect())
        .unwrap_or_default();
    if let Some(super_trait) = &trait_bridge.super_trait
        && let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait)
    {
        for method in &super_type.methods {
            if !methods.iter().any(|m| m.name == method.name) {
                methods.push(method);
            }
        }
    }
    // `emit_test_backend` panics rather than return a placeholder when the C
    // test-backend emitter is unimplemented — see `TestBackendEmission`'s and
    // `trait_bridge_snippet::emit_test_backend`'s doc comments. ~keep
    let emission = crate::e2e::codegen::emit_test_backend("c", trait_bridge, &methods, fixture, &[], "");
    emission.arg_expr
}

/// Inputs for [`push_regular_arg_expr`]. A struct, not nine positional arguments, for the same
/// reason as [`EnsureArgMatchesParamTypeArgs`].
struct RegularArgArgs<'a> {
    input: &'a serde_json::Value,
    arg: &'a crate::e2e::config::ArgMapping,
    index: usize,
    typed_arg_handles: &'a HashMap<String, String>,
    known_params: Option<&'a [crate::core::ir::ParamDef]>,
    type_defs: &'a [crate::core::ir::TypeDef],
    fixture: &'a Fixture,
    function_name: &'a str,
    target_params: TargetParams<'a>,
}

/// Build (and, via [`ensure_arg_matches_param_type`], validate) the C argument expression for
/// a non-`test_backend` arg, pushing it into `parts` -- or pushing nothing when the fixture
/// value resolves to a missing required field. Extracted verbatim from
/// [`build_args_string_c`]'s per-arg loop -- same emitted expression, same conditions, same
/// ordering.
fn push_regular_arg_expr(parts: &mut Vec<String>, args: RegularArgArgs<'_>) -> anyhow::Result<()> {
    let RegularArgArgs {
        input,
        arg,
        index,
        typed_arg_handles,
        known_params,
        type_defs,
        fixture,
        function_name,
        target_params,
    } = args;
    let val = crate::e2e::codegen::resolve_field(input, &arg.field);
    match val {
        // ~keep Explicit null on optional arg → pass the type-appropriate "none"
        // sentinel: `0` for a scalar `AlefHandle` arg, `NULL` for a real pointer.
        v if v.is_null() && arg.optional => {
            parts.push(resolve_optional_sentinel(target_params, &arg.name, index, &arg.arg_type).to_string())
        }
        // Missing required fields resolve to null; skip them so malformed
        // fixture configuration does not crash generation.
        v if v.is_null() => {}
        v => {
            // For json_object args, use the options_handle pointer
            // instead of the raw JSON string.
            if let Some(handle) = typed_arg_handles.get(&arg.name) {
                parts.push(handle.clone())
            } else {
                let rendered = json_to_c(v);
                // `json_to_c` answers only to the shape of the JSON value; nothing above
                // has consulted the parameter this expression lands in. This is the one
                // point where both facts are in hand. ~keep
                if let Some(params) = known_params {
                    ensure_arg_matches_param_type(EnsureArgMatchesParamTypeArgs {
                        fixture,
                        function_name,
                        arg,
                        index,
                        params,
                        type_defs,
                        rendered: &rendered,
                    })?;
                }
                parts.push(rendered)
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    ffi_prefix: &str,
    _field_resolver: &FieldResolver,
    accessed_fields: &[(String, String, bool)],
    primitive_locals: &HashMap<String, String>,
    opaque_handle_locals: &HashMap<String, String>,
    wildcard_locals: &HashMap<String, (String, String)>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !_field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
        return;
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => {
            // Use the local variable extracted from the opaque handle.
            accessed_fields
                .iter()
                .find(|(k, _, _)| k == f)
                .map(|(_, local, _)| local.clone())
                .unwrap_or_else(|| result_var.to_string())
        }
        _ => result_var.to_string(),
    };

    // `field[].key`: the extraction phase declared no scalar local for it (see
    // `emit_nested_accessor`'s wildcard leaf), only registered `array_var`/`key_snake` here.
    // Render the per-element quantifier and stop — none of the scalar branches below apply.
    if let Some((array_var, key_snake)) = wildcard_locals.get(&field_expr) {
        render_wildcard_assertion(out, assertion, array_var, key_snake);
        return;
    }

    // If the field was marked with the "__skip__" sentinel (fields_c_types = "skip"),
    // the accessor was never emitted — skip the assertion silently.
    if primitive_locals.get(&field_expr).is_some_and(|t| t == "__skip__") {
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::NotAvailableInCFfi.message(&field_expr)
        );
        return;
    }

    let ctx = build_assertion_field_context(
        assertion,
        field_expr,
        _field_resolver,
        accessed_fields,
        primitive_locals,
        opaque_handle_locals,
    );
    dispatch_assertion(out, assertion, result_var, ffi_prefix, &ctx);
}

/// Precomputed facts about the field an assertion targets, shared by every per-assertion-type
/// renderer below so each one does not recompute them from `accessed_fields`/`primitive_locals`.
struct AssertionFieldContext {
    field_expr: String,
    field_is_primitive: bool,
    field_primitive_type: Option<String>,
    /// Opaque-handle fields (e.g. `usage` → SAMPLELLMUsage*, or an enum field a missing
    /// `fields_enum`/IR-enum declaration failed to route through `try_emit_enum_accessor`)
    /// cannot be treated as C strings — `strlen`/`strcmp`/`strstr`/`regexec` on a scalar
    /// `AlefHandle` (`uint64_t`) is undefined behavior at best and a type error at worst.
    /// Every string-shaped assertion renderer below guards on this flag and falls back to a
    /// non-zero existence check (matching the sentinel the handle actually uses) rather
    /// than emitting a comparison against a value the ABI carries as an integer. ~keep
    field_is_opaque_handle: bool,
    field_is_map_access: bool,
    assertion_field_is_optional: bool,
}

/// Compute [`AssertionFieldContext`] for one assertion. Extracted verbatim from
/// `render_assertion`'s setup -- same conditions, same ordering.
fn build_assertion_field_context(
    assertion: &Assertion,
    field_expr: String,
    field_resolver: &FieldResolver,
    accessed_fields: &[(String, String, bool)],
    primitive_locals: &HashMap<String, String>,
    opaque_handle_locals: &HashMap<String, String>,
) -> AssertionFieldContext {
    let field_is_primitive = primitive_locals.contains_key(&field_expr);
    let field_primitive_type = primitive_locals.get(&field_expr).cloned();
    let field_is_opaque_handle = opaque_handle_locals.contains_key(&field_expr);
    // Map-access fields are extracted via `alef_json_get_string` and end up
    // as char*. When the assertion expects a numeric or boolean value, we
    // emit a parsed/literal comparison rather than `strcmp`.
    let field_is_map_access = if let Some(f) = &assertion.field {
        accessed_fields.iter().any(|(k, _, m)| k == f && *m)
    } else {
        false
    };

    // Check if the assertion field is optional — used to emit conditional assertions
    // for optional numeric fields (returns 0 when None, so 0 == "not set").
    // Check both the raw field name and its resolved alias.
    let assertion_field_is_optional = assertion
        .field
        .as_deref()
        .map(|f| {
            if f.is_empty() {
                return false;
            }
            if field_resolver.is_optional(f) {
                return true;
            }
            // Also check the resolved alias (e.g. "robots.crawl_delay" → "crawl_delay").
            let resolved = field_resolver.resolve(f);
            field_resolver.is_optional(resolved)
        })
        .unwrap_or(false);

    AssertionFieldContext {
        field_expr,
        field_is_primitive,
        field_primitive_type,
        field_is_opaque_handle,
        field_is_map_access,
        assertion_field_is_optional,
    }
}

/// Render the first half of `assertion.assertion_type`'s cases; anything else is handled by
/// [`dispatch_assertion_tail`]. Split in two purely to keep each dispatcher's cyclomatic
/// complexity down -- together they are exactly the original single `match`, same arms, same
/// order, same catch-all panic.
fn dispatch_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    ffi_prefix: &str,
    ctx: &AssertionFieldContext,
) {
    match assertion.assertion_type.as_str() {
        "equals" => render_equals_assertion(out, assertion, ctx),
        "contains" => render_contains_assertion(out, assertion, ctx),
        "contains_all" => render_contains_all_assertion(out, assertion, ctx),
        "not_contains" => render_not_contains_assertion(out, assertion, ctx),
        "not_empty" => render_not_empty_assertion(out, ctx),
        "is_empty" => render_is_empty_assertion(out, ctx),
        "contains_any" => render_contains_any_assertion(out, assertion, ctx),
        "greater_than" => render_greater_than_assertion(out, assertion, ctx),
        "less_than" => render_less_than_assertion(out, assertion, ctx),
        "greater_than_or_equal" => render_greater_than_or_equal_assertion(out, assertion, ctx),
        "less_than_or_equal" => render_less_than_or_equal_assertion(out, assertion, ctx),
        _ => dispatch_assertion_tail(out, assertion, result_var, ffi_prefix, ctx),
    }
}

/// The second half of `assertion.assertion_type`'s cases -- see [`dispatch_assertion`].
fn dispatch_assertion_tail(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    ffi_prefix: &str,
    ctx: &AssertionFieldContext,
) {
    match assertion.assertion_type.as_str() {
        "starts_with" => render_starts_with_assertion(out, assertion, ctx),
        "ends_with" => render_ends_with_assertion(out, assertion, ctx),
        "min_length" => render_min_length_assertion(out, assertion, ctx),
        "max_length" => render_max_length_assertion(out, assertion, ctx),
        "count_min" => render_count_min_assertion(out, assertion, ctx),
        "count_equals" => render_count_equals_assertion(out, assertion, ctx),
        "is_true" => {
            let field_expr = ctx.field_expr.clone();
            let _ = writeln!(out, "    assert({field_expr});");
        }
        "is_false" => {
            let field_expr = ctx.field_expr.clone();
            let _ = writeln!(out, "    assert(!{field_expr});");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                render_method_result_assertion(out, MethodResultAssertionArgs {
                    result_var,
                    ffi_prefix,
                    method_name,
                    args: assertion.args.as_ref(),
                    return_type: assertion.return_type.as_deref(),
                    check: assertion.check.as_deref().unwrap_or("is_true"),
                    value: assertion.value.as_ref(),
                });
            } else {
                panic!("C e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => render_matches_regex_assertion(out, assertion, ctx),
        "not_error" => {
            // Already handled — the NULL check above covers this.
        }
        "error" => {
            // Handled at the test function level.
        }
        other => {
            panic!("C e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Render the `field_is_primitive` branch of an `equals` assertion. Extracted verbatim from
/// `render_equals_assertion` -- same emitted lines, same conditions, same ordering.
fn render_equals_primitive_assertion(
    out: &mut String,
    field_expr: &str,
    expected: &serde_json::Value,
    c_val: String,
    field_primitive_type: Option<&str>,
    assertion_field_is_optional: bool,
) {
    let cmp_val = if field_primitive_type == Some("bool") {
        match expected.as_bool() {
            Some(true) => "1".to_string(),
            Some(false) => "0".to_string(),
            None => c_val,
        }
    } else {
        c_val
    };
    // For optional numeric fields, treat 0 as "not set" and allow it.
    // This mirrors Go's nil-pointer check for optional fields. Excludes a
    // boolean equals-assertion even when `field_primitive_type` spells the
    // field's real C type as `int32_t` rather than the literal string `bool`
    // (bool crosses the FFI ABI as `int32_t`, and `primitive_field_inference`'s
    // IR-derived entries record that exact spelling): `false` (`0`) is a real,
    // legitimate value for a boolean field, not an "unset" sentinel, so a
    // `equals: false` assertion against an optional bool field must never pass
    // merely because 0 also means "not set" for an unrelated numeric optional. ~keep
    let is_numeric = field_primitive_type.map(|t| t != "bool").unwrap_or(false) && !expected.is_boolean();
    if assertion_field_is_optional && is_numeric {
        let _ = writeln!(
            out,
            "    assert(({field_expr} == 0 || {field_expr} == {cmp_val}) && \"equals assertion failed\");"
        );
    } else {
        let _ = writeln!(
            out,
            "    assert({field_expr} == {cmp_val} && \"equals assertion failed\");"
        );
    }
}

/// Render an `equals` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_equals_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_primitive = ctx.field_is_primitive;
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    let field_is_map_access = ctx.field_is_map_access;

    if let Some(expected) = &assertion.value {
        let c_val = json_to_c(expected);
        if field_is_primitive {
            render_equals_primitive_assertion(
                out,
                &field_expr,
                expected,
                c_val,
                ctx.field_primitive_type.as_deref(),
                ctx.assertion_field_is_optional,
            );
        } else if field_is_opaque_handle {
            if expected.is_number() {
                // A numeric expected value compares exactly against the handle.
                let _ = writeln!(
                    out,
                    "    assert({field_expr} == {c_val} && \"equals assertion failed\");"
                );
            } else {
                // A string expected value against a handle means the field should
                // have been routed through `try_emit_enum_accessor` and wasn't;
                // `field_expr == "..."` would compile as a pointer comparison that
                // always lies, so weaken to existence instead of emitting that.
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            }
        } else if expected.is_string() {
            let _ = writeln!(
                out,
                "    assert({field_expr} != NULL && strcmp({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
            );
        } else if field_is_map_access && expected.is_boolean() {
            let lit = match expected.as_bool() {
                Some(true) => "\"true\"",
                _ => "\"false\"",
            };
            let _ = writeln!(
                out,
                "    assert({field_expr} != NULL && strcmp({field_expr}, {lit}) == 0 && \"equals assertion failed\");"
            );
        } else if field_is_map_access && expected.is_number() {
            if expected.is_f64() {
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && atof({field_expr}) == {c_val} && \"equals assertion failed\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && atoll({field_expr}) == {c_val} && \"equals assertion failed\");"
                );
            }
        } else {
            let _ = writeln!(
                out,
                "    assert(strcmp({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
            );
        }
    }
}

/// Render a `contains` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_contains_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(expected) = &assertion.value {
        let c_val = json_to_c(expected);
        let _ = writeln!(
            out,
            "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
        );
    }
}

/// Render a `contains_all` assertion. Extracted verbatim from `render_assertion`'s match --
/// same emitted lines, same conditions, same ordering.
fn render_contains_all_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(values) = &assertion.values {
        for val in values {
            let c_val = json_to_c(val);
            let _ = writeln!(
                out,
                "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
            );
        }
    }
}

/// Render a `not_contains` assertion. Extracted verbatim from `render_assertion`'s match --
/// same emitted lines, same conditions, same ordering.
fn render_not_contains_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(expected) = &assertion.value {
        let c_val = json_to_c(expected);
        let _ = writeln!(
            out,
            "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) == NULL && \"expected non-null value without substring\");"
        );
    }
}

/// Render a `not_empty` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_not_empty_assertion(out: &mut String, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        // ~keep Opaque handle: `strlen` on a scalar `AlefHandle` (uint64_t) is a
        // type error, not just UB on a struct pointer. Weaken to a
        // non-zero check — strictly weaker than the original intent but
        // matches the handle's actual "none" sentinel (`0`, not `NULL`).
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else {
        // A `char*` leaf can hold plain text OR the serialized JSON text of a
        // collection field (e.g. `alef_json_array_count`'s own input) — an empty
        // collection serializes as the two-byte string "[]"/"{}", not "", so `strlen`
        // alone reads it as non-empty. `c/scalar_or_collection_empty.jinja` accepts
        // either empty form. ~keep
        let condition = crate::e2e::template_env::render(
            "c/scalar_or_collection_empty.jinja",
            minijinja::context! { field_expr => field_expr, negate => true, allow_null => false },
        );
        let _ = writeln!(
            out,
            "    assert({} && \"expected non-empty value\");",
            condition.trim_end()
        );
    }
}

/// Render an `is_empty` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_is_empty_assertion(out: &mut String, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    let assertion_field_is_optional = ctx.assertion_field_is_optional;
    let field_is_primitive = ctx.field_is_primitive;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} == 0 && \"expected null handle\");");
    } else if assertion_field_is_optional || !field_is_primitive {
        // Optional string fields may return NULL — treat NULL as empty.
        let condition = crate::e2e::template_env::render(
            "c/scalar_or_collection_empty.jinja",
            minijinja::context! { field_expr => field_expr, negate => false, allow_null => true },
        );
        let _ = writeln!(out, "    assert({} && \"expected empty value\");", condition.trim_end());
    } else {
        let condition = crate::e2e::template_env::render(
            "c/scalar_or_collection_empty.jinja",
            minijinja::context! { field_expr => field_expr, negate => false, allow_null => false },
        );
        let _ = writeln!(out, "    assert({} && \"expected empty value\");", condition.trim_end());
    }
}

/// Render a `contains_any` assertion. Extracted verbatim from `render_assertion`'s match --
/// same emitted lines, same conditions, same ordering.
fn render_contains_any_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(values) = &assertion.values {
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        int found = 0;");
        for val in values {
            let c_val = json_to_c(val);
            let _ = writeln!(
                out,
                "        if (strstr({field_expr}, {c_val}) != NULL) {{ found = 1; }}"
            );
        }
        let _ = writeln!(
            out,
            "        assert(found && \"expected to contain at least one of the specified values\");"
        );
        let _ = writeln!(out, "    }}");
    }
}

/// Render a `greater_than` assertion. Extracted verbatim from `render_assertion`'s match --
/// same emitted lines, same conditions, same ordering.
fn render_greater_than_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_map_access = ctx.field_is_map_access;
    let field_is_primitive = ctx.field_is_primitive;
    if let Some(val) = &assertion.value {
        let c_val = json_to_c(val);
        if field_is_map_access && val.is_number() && !field_is_primitive {
            let _ = writeln!(
                out,
                "    assert({field_expr} != NULL && atof({field_expr}) > {c_val} && \"expected greater than\");"
            );
        } else {
            let _ = writeln!(out, "    assert({field_expr} > {c_val} && \"expected greater than\");");
        }
    }
}

/// Render a `less_than` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_less_than_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_map_access = ctx.field_is_map_access;
    let field_is_primitive = ctx.field_is_primitive;
    if let Some(val) = &assertion.value {
        let c_val = json_to_c(val);
        if field_is_map_access && val.is_number() && !field_is_primitive {
            let _ = writeln!(
                out,
                "    assert({field_expr} != NULL && atof({field_expr}) < {c_val} && \"expected less than\");"
            );
        } else {
            let _ = writeln!(out, "    assert({field_expr} < {c_val} && \"expected less than\");");
        }
    }
}

/// Render a `greater_than_or_equal` assertion. Extracted verbatim from `render_assertion`'s
/// match -- same emitted lines, same conditions, same ordering.
fn render_greater_than_or_equal_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_map_access = ctx.field_is_map_access;
    let field_is_primitive = ctx.field_is_primitive;
    if let Some(val) = &assertion.value {
        let c_val = json_to_c(val);
        if field_is_map_access && val.is_number() && !field_is_primitive {
            let _ = writeln!(
                out,
                "    assert({field_expr} != NULL && atof({field_expr}) >= {c_val} && \"expected greater than or equal\");"
            );
        } else {
            let _ = writeln!(
                out,
                "    assert({field_expr} >= {c_val} && \"expected greater than or equal\");"
            );
        }
    }
}

/// Render a `less_than_or_equal` assertion. Extracted verbatim from `render_assertion`'s match
/// -- same emitted lines, same conditions, same ordering.
fn render_less_than_or_equal_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_map_access = ctx.field_is_map_access;
    let field_is_primitive = ctx.field_is_primitive;
    if let Some(val) = &assertion.value {
        let c_val = json_to_c(val);
        if field_is_map_access && val.is_number() && !field_is_primitive {
            let _ = writeln!(
                out,
                "    assert({field_expr} != NULL && atof({field_expr}) <= {c_val} && \"expected less than or equal\");"
            );
        } else {
            let _ = writeln!(
                out,
                "    assert({field_expr} <= {c_val} && \"expected less than or equal\");"
            );
        }
    }
}

/// Render a `starts_with` assertion. Extracted verbatim from `render_assertion`'s match --
/// same emitted lines, same conditions, same ordering.
fn render_starts_with_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(expected) = &assertion.value {
        let c_val = json_to_c(expected);
        let _ = writeln!(
            out,
            "    assert(strncmp({field_expr}, {c_val}, strlen({c_val})) == 0 && \"expected to start with\");"
        );
    }
}

/// Render an `ends_with` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_ends_with_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(expected) = &assertion.value {
        let c_val = json_to_c(expected);
        let _ = writeln!(out, "    assert(strlen({field_expr}) >= strlen({c_val}) && ");
        let _ = writeln!(
            out,
            "           strcmp({field_expr} + strlen({field_expr}) - strlen({c_val}), {c_val}) == 0 && \"expected to end with\");"
        );
    }
}

/// Render a `min_length` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_min_length_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let _ = writeln!(
            out,
            "    assert(strlen({field_expr}) >= {n} && \"expected minimum length\");"
        );
    }
}

/// Render a `max_length` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_max_length_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let _ = writeln!(
            out,
            "    assert(strlen({field_expr}) <= {n} && \"expected maximum length\");"
        );
    }
}

/// Render a `count_min` assertion. Extracted verbatim from `render_assertion`'s match -- same
/// emitted lines, same conditions, same ordering.
fn render_count_min_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        /* count_min: count top-level JSON array elements */");
        let _ = writeln!(
            out,
            "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
        );
        let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
        let _ = writeln!(
            out,
            "        assert(elem_count >= {n} && \"expected at least {n} elements\");"
        );
        let _ = writeln!(out, "    }}");
    }
}

/// Render a `count_equals` assertion. Extracted verbatim from `render_assertion`'s match --
/// same emitted lines, same conditions, same ordering.
fn render_count_equals_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        /* count_equals: count elements in array */");
        let _ = writeln!(
            out,
            "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
        );
        let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
        let _ = writeln!(out, "        assert(elem_count == {n} && \"expected {n} elements\");");
        let _ = writeln!(out, "    }}");
    }
}

/// Render a `matches_regex` assertion. Extracted verbatim from `render_assertion`'s match --
/// same emitted lines, same conditions, same ordering.
fn render_matches_regex_assertion(out: &mut String, assertion: &Assertion, ctx: &AssertionFieldContext) {
    let field_expr = ctx.field_expr.clone();
    let field_is_opaque_handle = ctx.field_is_opaque_handle;
    if field_is_opaque_handle {
        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
    } else if let Some(expected) = &assertion.value {
        let c_val = json_to_c(expected);
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        regex_t _re;");
        let _ = writeln!(
            out,
            "        assert(regcomp(&_re, {c_val}, REG_EXTENDED) == 0 && \"regex compile failed\");"
        );
        let _ = writeln!(
            out,
            "        assert(regexec(&_re, {field_expr}, 0, NULL, 0) == 0 && \"expected value to match regex\");"
        );
        let _ = writeln!(out, "        regfree(&_re);");
        let _ = writeln!(out, "    }}");
    }
}

/// Inputs for [`render_method_result_assertion`]. A struct, not eight positional arguments,
/// following this file's existing convention (see [`MissingIntermediateType`],
/// [`LeafFieldCheck`]) for a helper with this many related-but-distinct parameters.
struct MethodResultAssertionArgs<'a> {
    result_var: &'a str,
    ffi_prefix: &'a str,
    method_name: &'a str,
    args: Option<&'a serde_json::Value>,
    return_type: Option<&'a str>,
    check: &'a str,
    value: Option<&'a serde_json::Value>,
}

/// Render a `method_result` assertion in C.
///
/// Dispatches generically using `{ffi_prefix}_{method_name}` for the FFI call.
/// The `return_type` fixture field controls how the return value is handled:
/// - `"string"` — the method returns a heap-allocated `char*`; the generator
///   emits a scoped block that asserts, then calls `free()`.
/// - absent/other — treated as a primitive integer (or pointer-as-bool); the
///   assertion is emitted inline without any heap management.
fn render_method_result_assertion(out: &mut String, args: MethodResultAssertionArgs<'_>) {
    let MethodResultAssertionArgs {
        result_var,
        ffi_prefix,
        method_name,
        args: call_args,
        return_type,
        check,
        value,
    } = args;
    let call_expr = build_c_method_call(result_var, ffi_prefix, method_name, call_args);

    if return_type == Some("string") {
        render_method_result_string_assertion(out, &call_expr, check, value);
        return;
    }

    render_method_result_primitive_assertion(out, &call_expr, check, value);
}

/// Render the string-return branch of a `method_result` assertion: a heap-allocated `char*`
/// return, emitted as a scoped block that asserts, then calls `free()`. Extracted verbatim
/// from `render_method_result_assertion` -- same emitted lines, same conditions, same
/// ordering.
fn render_method_result_string_assertion(
    out: &mut String,
    call_expr: &str,
    check: &str,
    value: Option<&serde_json::Value>,
) {
    let _ = writeln!(out, "    {{");
    let _ = writeln!(out, "        char* _method_result = {call_expr};");
    if check == "is_error" {
        let _ = writeln!(
            out,
            "        assert(_method_result == NULL && \"expected method to return error\");"
        );
        let _ = writeln!(out, "    }}");
        return;
    }
    let _ = writeln!(
        out,
        "        assert(_method_result != NULL && \"method_result returned NULL\");"
    );
    match check {
        "contains" => {
            if let Some(val) = value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "        assert(strstr(_method_result, {c_val}) != NULL && \"method_result contains assertion failed\");"
                );
            }
        }
        "equals" => {
            if let Some(val) = value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "        assert(strcmp(_method_result, {c_val}) == 0 && \"method_result equals assertion failed\");"
                );
            }
        }
        "is_true" => {
            let _ = writeln!(
                out,
                "        assert(_method_result != NULL && strlen(_method_result) > 0 && \"method_result is_true assertion failed\");"
            );
        }
        "count_min" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(out, "        int _elem_count = alef_json_array_count(_method_result);");
                let _ = writeln!(
                    out,
                    "        assert(_elem_count >= {n} && \"method_result count_min assertion failed\");"
                );
            }
        }
        other_check => {
            panic!("C e2e generator: unsupported method_result check type for string return: {other_check}");
        }
    }
    let _ = writeln!(out, "        free(_method_result);");
    let _ = writeln!(out, "    }}");
}

/// Render the primitive (integer / pointer-as-bool) return branch of a `method_result`
/// assertion: inline assert, no heap management. Extracted verbatim from
/// `render_method_result_assertion` -- same emitted lines, same conditions, same ordering.
fn render_method_result_primitive_assertion(
    out: &mut String,
    call_expr: &str,
    check: &str,
    value: Option<&serde_json::Value>,
) {
    match check {
        "equals" => {
            if let Some(val) = value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} == {c_val} && \"method_result equals assertion failed\");"
                );
            }
        }
        "is_true" => {
            let _ = writeln!(
                out,
                "    assert({call_expr} && \"method_result is_true assertion failed\");"
            );
        }
        "is_false" => {
            let _ = writeln!(
                out,
                "    assert(!{call_expr} && \"method_result is_false assertion failed\");"
            );
        }
        "greater_than_or_equal" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result >= {n} assertion failed\");"
                );
            }
        }
        "count_min" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result count_min assertion failed\");"
                );
            }
        }
        other_check => {
            panic!("C e2e generator: unsupported method_result check type: {other_check}");
        }
    }
}

/// Build a C call expression for a `method_result` assertion.
///
/// Uses generic dispatch: `{ffi_prefix}_{method_name}(result_var, args...)`.
/// Args from the fixture JSON object are emitted as positional C arguments in
/// insertion order, using best-effort type conversion (strings → C string literals,
/// numbers and booleans → verbatim literals).
fn build_c_method_call(
    result_var: &str,
    ffi_prefix: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    let extra_args = if let Some(args_val) = args {
        args_val
            .as_object()
            .map(|obj| {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => format!("\"{}\"", escape_c(s)),
                        serde_json::Value::Bool(true) => "1".to_string(),
                        serde_json::Value::Bool(false) => "0".to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => "NULL".to_string(),
                        other => format!("\"{}\"", escape_c(&other.to_string())),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if extra_args.is_empty() {
        format!("{ffi_prefix}_{method_name}({result_var})")
    } else {
        format!("{ffi_prefix}_{method_name}({result_var}, {extra_args})")
    }
}

#[cfg(test)]
mod tests;
