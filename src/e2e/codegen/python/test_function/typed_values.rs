//! Typed Python value rendering for generated test functions.

use std::collections::{BTreeSet, HashMap};

use heck::ToSnakeCase;

use crate::e2e::escape::escape_python;
use crate::e2e::fixture::FixtureDocsFileInput;

use super::super::json::json_to_python_literal;

/// Read-only rendering environment shared, unchanged, across every level of the
/// `render_kwarg_field_value` recursion (through `render_struct_constructor` and the
/// uniform `render_value_for_type_ref` core) and reused by the `json_object` arg emitters below.
/// All five fields are borrows or a `Copy` enum the whole call tree reads but never writes, so
/// the struct derives `Copy` -- passing it down the recursion is a pointer-width copy, not a
/// clone of `type_defs`/`enums`/`docs_files` themselves.
///
/// `leaf_source` decides how a leaf (non-container, non-nested-struct) field renders: the
/// common `Literal` case embeds the field's JSON value directly, while the `$mock_url` case
/// (see `emit_json_object_arg_with_mock_url`) needs every leaf to instead index into a runtime
/// dict already holding the placeholder-substituted values. Vector index segments carry the
/// private `~2` tag so numeric map keys remain distinguishable from array positions. It is
/// itself read-only, invariant
/// for the whole recursion, so it belongs here rather than as a parallel argument threaded
/// through every function in the call tree.
///
/// `used_types` is deliberately NOT a field here: it is a per-call *output* accumulator that
/// every level mutates, not read-only shared state, so it stays its own `&mut` argument at each
/// call site. Folding a mutable accumulator into an otherwise `Copy`, read-only bundle would
/// force every function to take `&mut KwargRenderContext` and forfeit the very simplicity --
/// cheap, ordinary reborrows -- this struct exists to buy. ~keep
#[derive(Clone, Copy)]
pub(in crate::e2e::codegen::python) struct KwargRenderContext<'a> {
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub enums: &'a [crate::core::ir::EnumDef],
    pub enum_fields: &'a HashMap<String, String>,
    pub docs_files: &'a [FixtureDocsFileInput],
    pub leaf_source: LeafSource<'a>,
}

/// Output accumulator for one `render_kwarg_field_value` traversal: every nested config/struct
/// class and every enum class it actually constructs, kept in two separate sets so a caller
/// merging them into an import list can keep the existing grouped ordering (config classes,
/// then enum classes) instead of collapsing both kinds into one alphabetically-interleaved list
/// -- a class-only rename here would otherwise reorder the import line of every generated file
/// that references more than one enum, not just the ones that were missing an import. ~keep
#[derive(Default)]
pub(in crate::e2e::codegen::python) struct UsedTypeNames {
    pub structs: BTreeSet<String>,
    pub enums: BTreeSet<String>,
}

/// Where a leaf field value's Python expression comes from -- see the `leaf_source` doc on
/// [`KwargRenderContext`] for why this lives there rather than as its own argument.
#[derive(Clone, Copy)]
pub(in crate::e2e::codegen::python) enum LeafSource<'a> {
    /// The value is known at codegen time; render its JSON literal directly.
    Literal,
    /// The value must be read at runtime from `holder`, a dict already holding the
    /// `$mock_url`-substituted data (`json.loads` of the placeholder-replaced JSON string) --
    /// render a chain of Python subscripts on `holder` derived from the field's JSON pointer
    /// instead of embedding the (still placeholder-laden) literal.
    RuntimeDict { holder: &'a str },
}

const RUNTIME_ARRAY_INDEX_PREFIX: &str = "~2";

/// Output accumulator for one fixture argument's emission: the setup lines it needs
/// (`bindings`) and the expression that becomes its slot in the call's keyword-argument list
/// (`kwarg_exprs`). Bundled because every `json_object` arg emitter below appends to both
/// together, in lockstep, never to one alone. Unlike `KwargRenderContext` this is mutated on
/// every call, so it holds `&mut` fields and is passed by `&mut` reference rather than `Copy`.
pub(in crate::e2e::codegen::python) struct ArgSink<'a> {
    pub bindings: &'a mut Vec<String>,
    pub kwarg_exprs: &'a mut Vec<String>,
}

/// How a `json_object` argument's JSON value should become a Python expression -- the three
/// pieces of `alef.toml` call-config the branches of `emit_json_object_arg` dispatch on
/// together. Bundled because every branch needs some subset of exactly these three fields and
/// nothing else about the call.
pub(in crate::e2e::codegen::python) struct ConstructorSpec<'a> {
    pub options_type: Option<&'a str>,
    pub options_via: &'a str,
    pub element_type: &'a Option<String>,
}

/// Identifies the mock-server fixture a `json_object` argument's placeholder URL resolves
/// against. Only consulted once `value_contains_mock_url_placeholder` finds a placeholder to
/// substitute, so it stays its own small struct rather than folding into `ConstructorSpec`
/// (which every call needs) or `KwargRenderContext` (which describes IR, not the fixture).
pub(in crate::e2e::codegen::python) struct MockUrlInfo<'a> {
    pub fixture_id: &'a str,
    pub has_host_root_route: bool,
}

/// Resolve the enum type name for a field if it's an enum type in the TypeDef,
/// and return None if it's not an enum or the type cannot be resolved.
pub(in crate::e2e::codegen::python) fn resolve_field_enum_type(
    field_name: &str,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> Option<String> {
    use crate::core::ir::TypeRef;

    let opts_type = options_type?;
    let type_def = type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;

    // Unwrap Optional and Vec wrappers to get the inner type
    let inner_name = match &field.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }?;

    // Check if this is an enum type
    if enums.iter().any(|e| e.name == inner_name) {
        Some(inner_name.to_string())
    } else {
        None
    }
}

/// Render one field's JSON value as a Python expression for a `kwargs`-mode constructor call,
/// recursing into nested config/struct fields so a field whose type is itself a generated
/// pyclass (e.g. `nested: NestedConfig` inside `ExtractionConfig`) is constructed with
/// that class instead of a raw dict literal. `used_types` records every nested constructor name
/// this rendering references -- struct classes in `used_types.structs`, enum classes in
/// `used_types.enums` -- so a caller collecting imports can run the identical traversal instead
/// of a second copy that could disagree with what actually gets emitted (the same technique
/// `handle_values::collect_used_nested_types` uses). ~keep
pub(in crate::e2e::codegen::python) fn render_kwarg_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_types: &mut UsedTypeNames,
) -> String {
    if let Some(rendered) = render_enum_field_value(field_name, value, containing_type, context, used_types) {
        return rendered;
    }
    if let Some(rendered) = render_docs_file_field_value(pointer, context.docs_files) {
        return rendered;
    }
    if let Some(rendered) = render_typed_field_value(field_name, value, containing_type, pointer, context, used_types) {
        return rendered;
    }

    render_leaf_value(value, pointer, context.leaf_source)
}

/// Render a leaf (non-container, non-nested-struct) field value per [`LeafSource`]: the
/// codegen-time JSON literal in the common case, or a runtime subscript chain into a
/// `$mock_url`-substituted dict.
fn render_leaf_value(value: &serde_json::Value, pointer: &str, leaf_source: LeafSource<'_>) -> String {
    match leaf_source {
        LeafSource::Literal => json_to_python_literal(value),
        LeafSource::RuntimeDict { holder } => runtime_dict_index_expression(holder, pointer),
    }
}

/// Convert a JSON-pointer-style path built by the recursion below (e.g.
/// `/profiles/first/model`) into a chain of Python subscript expressions on `holder` (e.g.
/// `holder["profiles"]["first"]["model"]`). A segment tagged with the private `~2` prefix
/// renders as an integer subscript; an untagged numeric segment remains a quoted map key.
fn runtime_dict_index_expression(holder: &str, pointer: &str) -> String {
    let mut expression = holder.to_string();
    if pointer.is_empty() {
        return expression;
    }
    let path = pointer.strip_prefix('/').unwrap_or(pointer);
    for segment in path.split('/') {
        if let Some(index) = segment
            .strip_prefix(RUNTIME_ARRAY_INDEX_PREFIX)
            .filter(|index| !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
        {
            expression.push_str(&format!("[{index}]"));
            continue;
        }
        let unescaped = segment.replace("~1", "/").replace("~0", "~");
        expression.push_str(&format!("[\"{}\"]", escape_python(&unescaped)));
    }
    expression
}

fn array_item_pointer(pointer: &str, index: usize, leaf_source: LeafSource<'_>) -> String {
    match leaf_source {
        LeafSource::Literal => format!("{pointer}/{index}"),
        LeafSource::RuntimeDict { .. } => format!("{pointer}/{RUNTIME_ARRAY_INDEX_PREFIX}{index}"),
    }
}

/// Resolve `field_name`'s declared `TypeRef` on `containing_type` and render `value` against it
/// via the uniform recursive core [`render_value_for_type_ref`]. Returns `None` when the field
/// isn't declared in IR, so the caller falls through to [`render_leaf_value`].
fn render_typed_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_types: &mut UsedTypeNames,
) -> Option<String> {
    let opts_type = containing_type?;
    let type_def = context.type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;
    render_value_for_type_ref(&field.ty, value, pointer, context, used_types)
}

/// Uniform recursive core replacing the former shape-by-shape dispatch (one arm each for a
/// direct struct field, `Vec<Struct>`, and `Map<K, Struct>`): unwraps `Optional`/`Vec`/`Map`
/// around a `Named` type at any depth and combination, so `Map<String, Optional<Named>>`,
/// `Map<String, Vec<Named>>`, `Vec<Optional<Named>>`, and nestings none of the former arms
/// enumerated all resolve through the same three arms below, with no per-combination code.
///
/// A `null` value under an `Optional` wrapper always renders as Python `None`, independent of
/// nesting depth -- e.g. one entry of a `Map<String, Optional<Named>>` may be null while a
/// sibling entry is an object; each entry resolves on its own rather than one null entry
/// reverting the whole map to a raw-dict fallback.
///
/// Returns `None` when `type_ref` doesn't bottom out on a `Named` type known to
/// `context.type_defs` (a plain scalar, or a container of one), or the JSON shape doesn't match
/// the declared type (e.g. a non-object where a struct is expected) -- both fall through to the
/// caller's plain leaf rendering. ~keep
fn render_value_for_type_ref(
    type_ref: &crate::core::ir::TypeRef,
    value: &serde_json::Value,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_types: &mut UsedTypeNames,
) -> Option<String> {
    use crate::core::ir::TypeRef;

    match type_ref {
        TypeRef::Named(name) => {
            let type_def = context.type_defs.iter().find(|t| &t.name == name)?;
            let obj = value.as_object()?;
            Some(render_struct_constructor(type_def, obj, pointer, context, used_types))
        }
        TypeRef::Optional(_) if value.is_null() => Some("None".to_string()),
        TypeRef::Optional(inner) => render_value_for_type_ref(inner, value, pointer, context, used_types),
        TypeRef::Vec(inner) => {
            let items = value
                .as_array()?
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let item_pointer = array_item_pointer(pointer, index, context.leaf_source);
                    render_value_for_type_ref(inner, item, &item_pointer, context, used_types)
                })
                .collect::<Option<Vec<String>>>()?;
            Some(format!("[{}]", items.join(", ")))
        }
        TypeRef::Map(_, value_ty) => {
            let items = value
                .as_object()?
                .iter()
                .map(|(key, entry)| {
                    let entry_pointer = format!("{pointer}/{}", escape_json_pointer(key));
                    let rendered = render_value_for_type_ref(value_ty, entry, &entry_pointer, context, used_types)?;
                    Some(format!("\"{}\": {rendered}", escape_python(key)))
                })
                .collect::<Option<Vec<String>>>()?;
            Some(format!("{{{}}}", items.join(", ")))
        }
        _ => None,
    }
}

/// Enum branch of [`render_kwarg_field_value`]: an explicitly configured `enum_fields` entry, or
/// an auto-detected enum field type, renders as `EnumType("variant")`. Mirrors the original
/// inline logic exactly -- an `enum_fields` hit with a non-string value falls through to the
/// remaining branches rather than trying auto-detection.
///
/// Records the resolved enum type name into `used_types.enums` -- the enum half of the same
/// [`UsedTypeNames`] accumulator [`render_struct_constructor`] records nested config classes
/// into (`used_types.structs`) -- so a caller collecting import candidates by running this
/// traversal sees every type it constructs, enum or struct, through one shared output instead of
/// a second hand-maintained scan that could disagree with it. Before this, an enum field nested
/// inside a nested config object (e.g. `PreprocessingOptions.preset: PreprocessingPreset` inside
/// `ConversionOptions`) was rendered correctly here but never recorded anywhere, so the import
/// list omitted it even though the emitted body referenced it. Kept in its own set rather than
/// merged into `used_types.structs` so a caller can still emit config classes and enum classes
/// as two separately-ordered groups, matching the import line's existing shape instead of
/// re-sorting every name in the file together. ~keep
fn render_enum_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    context: KwargRenderContext<'_>,
    used_types: &mut UsedTypeNames,
) -> Option<String> {
    if let Some(enum_type) = context.enum_fields.get(field_name) {
        if let Some(s) = value.as_str() {
            used_types.enums.insert(enum_type.clone());
            return Some(format!("{enum_type}(\"{s}\")"));
        }
    } else if let Some(auto_enum_type) =
        resolve_field_enum_type(field_name, containing_type, context.type_defs, context.enums)
        && let Some(s) = value.as_str()
    {
        used_types.enums.insert(auto_enum_type.clone());
        return Some(format!("{auto_enum_type}(\"{s}\")"));
    }
    None
}

/// Docs-file branch of [`render_kwarg_field_value`]: a field whose JSON pointer matches a
/// configured fixture docs-file input renders as a file-read expression instead of its JSON value.
fn render_docs_file_field_value(pointer: &str, docs_files: &[FixtureDocsFileInput]) -> Option<String> {
    let pointer = canonical_docs_pointer(pointer);
    docs_files
        .iter()
        .find(|file| file.field == pointer)
        .map(|file| docs_file_expression(&file.path))
}

fn canonical_docs_pointer(pointer: &str) -> String {
    pointer
        .split('/')
        .map(|segment| {
            segment
                .strip_prefix(RUNTIME_ARRAY_INDEX_PREFIX)
                .filter(|index| !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
                .unwrap_or(segment)
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Build a `TypeName(field=value, ...)` constructor call for `type_def`, recursing through
/// [`render_kwarg_field_value`] for each field so arbitrarily deep nested config types resolve
/// the same way at every depth.
fn render_struct_constructor(
    type_def: &crate::core::ir::TypeDef,
    obj: &serde_json::Map<String, serde_json::Value>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_types: &mut UsedTypeNames,
) -> String {
    used_types.structs.insert(type_def.name.clone());
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(field_name, field_value)| {
            let snake_key = field_name.to_snake_case();
            let field_pointer = format!("{pointer}/{}", escape_json_pointer(field_name));
            let rendered = render_kwarg_field_value(
                field_name,
                field_value,
                Some(type_def.name.as_str()),
                &field_pointer,
                context,
                used_types,
            );
            format!("{snake_key}={rendered}")
        })
        .collect();
    format!("{}({})", type_def.name, kwargs.join(", "))
}

/// Returns `true` if the arg was fully emitted (caller should `continue`).
pub(super) fn emit_json_object_arg(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    spec: &ConstructorSpec<'_>,
    mock: &MockUrlInfo<'_>,
    context: KwargRenderContext<'_>,
) -> bool {
    if crate::e2e::codegen::value_contains_mock_url_placeholder(value) {
        return emit_json_object_arg_with_mock_url(sink, value, var_name, spec, mock, context);
    }

    match spec.options_via {
        "dict" => emit_json_object_arg_dict_mode(sink, value, var_name, spec.element_type),
        "json" => emit_json_object_arg_json_mode(sink, value, var_name),
        "from_json" => emit_json_object_arg_from_json_mode(sink, value, var_name, spec.options_type),
        _ => emit_json_object_arg_default_mode(sink, value, var_name, spec, context),
    }
}

/// `options_via = "dict"` branch: an array of objects paired with `element_type` emits plain
/// dict literals (the bindings expect `[{"type": "click", ...}, ...]`, not constructor calls);
/// anything else falls back to a single JSON literal for the whole value.
fn emit_json_object_arg_dict_mode(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    element_type: &Option<String>,
) -> bool {
    if let (Some(_elem_type), Some(arr)) = (element_type, value.as_array())
        && !arr.is_empty()
        && arr.iter().all(|v| v.is_object())
    {
        let items: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_object())
            .map(emit_python_object_item)
            .collect();
        sink.bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
        sink.kwarg_exprs.push(var_name.to_string());
        return true;
    }
    let literal = json_to_python_literal(value);
    let noqa = if literal.contains("/tmp/") {
        "  # noqa: S108"
    } else {
        ""
    };
    sink.bindings.push(format!("    {var_name} = {literal}{noqa}"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// `options_via = "json"` branch: the value round-trips through `json.loads(...)`.
fn emit_json_object_arg_json_mode(sink: &mut ArgSink<'_>, value: &serde_json::Value, var_name: &str) -> bool {
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    sink.bindings
        .push(format!("    {var_name} = json.loads(\"{escaped}\")"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// `options_via = "from_json"` branch: the value round-trips through the configured type's
/// `from_json(...)` classmethod. Requires `options_type`; without it there is no method to call,
/// so the caller falls back to the remaining arg-emission paths.
fn emit_json_object_arg_from_json_mode(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
) -> bool {
    let Some(opts_type) = options_type else {
        return false;
    };
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    sink.bindings
        .push(format!("    {var_name} = {opts_type}.from_json(\"{escaped}\")"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// Default (`options_via` unset or unrecognized) branch: either a "batch" array of typed items
/// (`element_type`), or a single "kwargs"-mode constructor call (`options_type`).
fn emit_json_object_arg_default_mode(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    spec: &ConstructorSpec<'_>,
    context: KwargRenderContext<'_>,
) -> bool {
    if emit_json_object_arg_typed_array(sink, value, var_name, spec.element_type, context) {
        return true;
    }
    emit_json_object_arg_typed_kwargs(sink, value, var_name, spec.options_type, context)
}

/// Batch-array sub-branch of the default mode: an array of objects paired with `element_type`
/// constructs a typed instance per item via [`emit_python_typed_instance`].
fn emit_json_object_arg_typed_array(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    element_type: &Option<String>,
    context: KwargRenderContext<'_>,
) -> bool {
    let Some(elem_type) = element_type else {
        return false;
    };
    if value.is_null() {
        return false;
    }
    let Some(arr) = value.as_array() else {
        return false;
    };
    if !arr.iter().all(|item| item.is_object()) {
        return false;
    }
    let items: Vec<String> = arr
        .iter()
        .filter_map(|item| item.as_object())
        .enumerate()
        .map(|(index, obj)| {
            let pointer = format!("/{index}");
            emit_python_typed_instance(obj, elem_type, &pointer, context)
        })
        .collect();
    sink.bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// Build a `TypeName(field=value, ...)` constructor call for a "kwargs"-mode `json_object` arg,
/// recursing through [`render_kwarg_field_value`] for every field. Shared by the plain
/// kwargs-mode emitter and the `$mock_url` emitter below, which differ only in `context`'s
/// `leaf_source` -- codegen-time literals for the former, runtime-dict subscripts for the
/// latter -- so this is the one place that walks `obj`'s fields.
fn build_typed_kwargs_constructor(
    value: &serde_json::Value,
    opts_type: &str,
    context: KwargRenderContext<'_>,
) -> Option<String> {
    let obj = value.as_object()?;
    let mut used_types = UsedTypeNames::default();
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            let snake_key = k.to_snake_case();
            let field_pointer = format!("/{}", escape_json_pointer(k));
            let py_val = render_kwarg_field_value(k, v, Some(opts_type), &field_pointer, context, &mut used_types);
            format!("{snake_key}={py_val}")
        })
        .collect();
    Some(format!("{opts_type}({})", kwargs.join(", ")))
}

/// Single-object sub-branch of the default mode: a "kwargs"-mode constructor call.
fn emit_json_object_arg_typed_kwargs(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    context: KwargRenderContext<'_>,
) -> bool {
    let Some(opts_type) = options_type else {
        return false;
    };
    let Some(constructor) = build_typed_kwargs_constructor(value, opts_type, context) else {
        return false;
    };
    sink.bindings.push(format!("    {var_name} = {constructor}"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

fn emit_json_object_arg_with_mock_url(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    spec: &ConstructorSpec<'_>,
    mock: &MockUrlInfo<'_>,
    context: KwargRenderContext<'_>,
) -> bool {
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    let env_key = crate::e2e::codegen::mock_url_env_key(mock.fixture_id);
    let fallback = format!("os.environ['MOCK_SERVER_URL'] + '/fixtures/{}'", mock.fixture_id);
    let base_expr = if mock.has_host_root_route {
        format!("os.environ.get('{env_key}') or {fallback}")
    } else {
        fallback
    };
    sink.bindings
        .push(format!("    {var_name}_mock_base_url = {base_expr}"));
    sink.bindings.push(format!(
        "    {var_name}_json = \"{escaped}\".replace(\"{}\", {var_name}_mock_base_url)",
        crate::e2e::codegen::MOCK_URL_PLACEHOLDER
    ));

    if !matches!(spec.options_via, "dict" | "json" | "from_json")
        && let Some(element_type) = spec.element_type
        && emit_mock_url_typed_array(sink, value, var_name, element_type, context)
    {
        sink.kwarg_exprs.push(var_name.to_string());
        return true;
    }

    match (spec.options_via, spec.options_type) {
        ("from_json", Some(opts_type)) => {
            sink.bindings
                .push(format!("    {var_name} = {opts_type}.from_json({var_name}_json)"));
        }
        ("dict", _) | (_, None) | ("json", _) => {
            sink.bindings
                .push(format!("    {var_name} = json.loads({var_name}_json)"));
        }
        (_, Some(opts_type)) => {
            emit_mock_url_typed_kwargs(sink, value, var_name, opts_type, context);
        }
    }
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

fn emit_mock_url_typed_array(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    element_type: &str,
    context: KwargRenderContext<'_>,
) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if !items.iter().all(serde_json::Value::is_object) {
        return false;
    }

    sink.bindings
        .push(format!("    {var_name}_data = json.loads({var_name}_json)"));
    let holder = format!("{var_name}_data");
    let runtime_context = KwargRenderContext {
        leaf_source: LeafSource::RuntimeDict { holder: &holder },
        ..context
    };
    let rendered = items
        .iter()
        .filter_map(serde_json::Value::as_object)
        .enumerate()
        .map(|(index, item)| {
            let pointer = array_item_pointer("", index, runtime_context.leaf_source);
            emit_python_typed_instance(item, element_type, &pointer, runtime_context)
        })
        .collect::<Vec<_>>()
        .join(", ");
    sink.bindings.push(format!("    {var_name} = [{rendered}]"));
    true
}

/// `$mock_url` counterpart of [`emit_json_object_arg_typed_kwargs`]: the runtime-substituted
/// JSON string (`{var_name}_json`, already built by the caller) is parsed into a runtime dict
/// (`{var_name}_data`), then [`build_typed_kwargs_constructor`] builds the same typed
/// constructor call the non-mock-url path would, with every leaf indexing into that dict
/// instead of embedding its (still placeholder-laden) literal -- so a nested struct or map
/// field survives placeholder substitution instead of reverting to a raw dict. Falls back to
/// unpacking the runtime dict directly (the pre-fix behavior) when `opts_type` or the JSON
/// shape doesn't resolve to a constructor -- e.g. `value` isn't an object.
fn emit_mock_url_typed_kwargs(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    opts_type: &str,
    context: KwargRenderContext<'_>,
) {
    sink.bindings
        .push(format!("    {var_name}_data = json.loads({var_name}_json)"));
    let holder = format!("{var_name}_data");
    let runtime_context = KwargRenderContext {
        leaf_source: LeafSource::RuntimeDict { holder: &holder },
        ..context
    };
    let constructor = build_typed_kwargs_constructor(value, opts_type, runtime_context)
        .unwrap_or_else(|| format!("{opts_type}(**{holder})"));
    sink.bindings.push(format!("    {var_name} = {constructor}"));
}

pub(super) fn emit_bytes_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
) {
    if let Some(raw) = value.as_str() {
        match super::super::helpers::classify_bytes_value(raw) {
            super::super::helpers::BytesKind::FilePath => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = Path(\"{escaped}\").read_bytes()"));
            }
            super::super::helpers::BytesKind::InlineText => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = b\"{escaped}\""));
            }
            super::super::helpers::BytesKind::Base64 => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = base64.b64decode(\"{escaped}\")"));
            }
        }
    } else {
        arg_bindings.push(format!("    {var_name} = None"));
    }
    kwarg_exprs.push(var_name.to_string());
}

/// Emit a Python dict literal for a typed object-array element.
fn emit_python_object_item(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    let items: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            format!(
                "{}: {}",
                json_to_python_literal(&serde_json::Value::String(k.clone())),
                json_to_python_literal(v)
            )
        })
        .collect();
    format!("{{{}}}", items.join(", "))
}

/// Emit a Python constructor call for a typed instance (e.g., BatchFileItem(...)), recursing
/// into any of its own fields that are themselves generated pyclasses (e.g. a batch item whose
/// `nested` field is a `NestedConfig`) via [`render_kwarg_field_value`].
fn emit_python_typed_instance(
    obj: &serde_json::Map<String, serde_json::Value>,
    elem_type: &str,
    pointer: &str,
    context: KwargRenderContext<'_>,
) -> String {
    let mut used_types = UsedTypeNames::default();
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            let snake_key = k.to_snake_case();
            let field_pointer = format!("{pointer}/{}", escape_json_pointer(k));
            let rendered = render_kwarg_field_value(k, v, Some(elem_type), &field_pointer, context, &mut used_types);
            format!("{snake_key}={rendered}")
        })
        .collect();
    format!("{}({})", elem_type, kwargs.join(", "))
}

fn docs_file_expression(path: &str) -> String {
    crate::e2e::template_env::render(
        "python/docs_file_expression.py.jinja",
        minijinja::context! { path => escape_python(path) },
    )
    .trim_end()
    .to_string()
}

fn escape_json_pointer(field: &str) -> String {
    field.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
#[path = "typed_values_tests.rs"]
mod tests;

// Sibling of `typed_values_tests.rs` rather than more cases inside it: that file is already
// close to the file-size cap, and this group has its own concern -- how the runtime-dict
// lowering picks list indices apart from map keys. ~keep
#[cfg(test)]
#[path = "mock_url_index_tests.rs"]
mod mock_url_index_tests;
