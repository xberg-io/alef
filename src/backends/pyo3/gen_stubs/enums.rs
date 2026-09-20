use super::{
    SHADOWABLE_BUILTIN_TYPES, pyi_docstring, python_safe_name, qualify_parameter_type, qualify_shadowed_builtin_types,
};
use crate::backends::pyo3::type_map::python_type;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use ahash::AHashSet;

/// Map a data-enum factory-constructor param type to its stub annotation, widening dataclass-backed
/// config DTOs. The bare name (`LlmConfig`) resolves to the compiled `#[pyclass]` this stub declares,
/// but the public name users pass is the `options.py` `@dataclass`, so such a param is widened to
/// `options.<Name> | dict[str, Any]` — the public wrapper or a dict, matching what the runtime
/// `__alef_coerce_dto` accepts. Non-DTO types (and the inner of containers) fall back to
/// [`python_type`], so the output is byte-identical to it wherever no coercible DTO is present.
fn factory_param_type(ty: &TypeRef, coercible_dtos: &AHashSet<&str>) -> String {
    match ty {
        TypeRef::Named(name) if coercible_dtos.contains(name.as_str()) => {
            format!("options.{name} | dict[str, Any]")
        }
        TypeRef::Optional(inner) => format!("{} | None", factory_param_type(inner, coercible_dtos)),
        TypeRef::Vec(inner) => format!("list[{}]", factory_param_type(inner, coercible_dtos)),
        TypeRef::Map(k, v) => format!(
            "dict[{}, {}]",
            factory_param_type(k, coercible_dtos),
            factory_param_type(v, coercible_dtos)
        ),
        _ => python_type(ty),
    }
}

fn to_python_enum_variant(name: &str) -> String {
    crate::core::keywords::python_str_enum_ident(&crate::codegen::naming::pascal_to_screaming_snake(name))
}

/// Generate a Python enum stub.
///
/// `api_types` is the surface's `TypeDef` list: a variant whose payload serde flattens into the
/// tag object contributes that payload type's OWN fields to the wire dict, and they can only be
/// resolved from there (the variant itself holds one synthetic `_0` field, not the payload's
/// keys). ~keep
pub(super) fn gen_enum_stub(
    enum_def: &EnumDef,
    emit_docstrings: bool,
    coercible_dtos: &AHashSet<&str>,
    is_host_enum: bool,
    api_types: &[TypeDef],
) -> String {
    use crate::codegen::generators::enum_has_data_variants;
    let mut lines = vec![];

    if enum_has_data_variants(enum_def) {
        gen_data_enum_typeddicts(&mut lines, enum_def, coercible_dtos, is_host_enum, api_types);
    } else {
        lines.push(format!("class {}:", enum_def.name));
        if emit_docstrings && let Some(docstring) = pyi_docstring(&enum_def.doc, "    ") {
            lines.push(docstring);
        }
        for variant in &enum_def.variants {
            // Emit snake_case attribute names to match the #[pyo3(name = "...")] rename
            lines.push(format!(
                "    {}: {} = ...",
                to_python_enum_variant(&variant.name),
                enum_def.name
            ));
            if emit_docstrings && let Some(docstring) = pyi_docstring(&variant.doc, "    ") {
                lines.push(docstring);
            }
        }
        lines.push("    def __init__(self, value: int | str) -> None: ...".to_string());
        lines.push("    def __hash__(self) -> int: ...".to_string());
    }

    lines.join("\n")
}

/// serde's default discriminator key, kept for enums that reach the tagged path without one.
const DEFAULT_TAG_FIELD: &str = "type";

/// The stub annotation for an adjacently tagged variant's payload, or `None` when the variant is
/// a unit variant and serde writes no content key for it at all.
///
/// A struct variant's payload is an object of its own named fields, which a TypedDict cannot spell
/// inline, so a companion TypedDict is emitted into `lines` and referenced by name.
fn adjacent_payload_type(lines: &mut Vec<String>, enum_def: &EnumDef, variant: &EnumVariant) -> Option<String> {
    if variant.fields.is_empty() {
        return None;
    }
    if variant.is_tuple && variant.fields.len() == 1 {
        return Some(python_type(&variant.fields[0].ty));
    }
    let payload_class = format!("{}{}Payload", enum_def.name, variant.name);
    lines.push(format!("class {payload_class}(TypedDict):"));
    for field in &variant.fields {
        lines.push(typed_dict_field_line(field));
    }
    lines.push(String::new());
    Some(payload_class)
}

/// One `    name: annotation` line of a TypedDict body, widening an optional field's annotation
/// with `| None` unless [`python_type`] already spelled it.
fn typed_dict_field_line(field: &FieldDef) -> String {
    let field_type = python_type(&field.ty);
    let field_type = if field.optional && !field_type.contains("| None") {
        format!("{field_type} | None")
    } else {
        field_type
    };
    format!("    {}: {}", python_safe_name(&field.name), field_type)
}

/// The fields a variant serde FLATTENS contributes to the tag object — the payload type's own
/// fields, looked up in the surface by the payload's declared name.
///
/// `None` means the payload's fields are not knowable here: a payload that is not a plain
/// [`TypeRef::Named`], or a named type this binding's surface does not declare. The caller then
/// emits the discriminator alone, which is incomplete but true, rather than the synthetic `_0`
/// key serde never writes. ~keep
fn flattened_payload_fields<'a>(variant: &EnumVariant, api_types: &'a [TypeDef]) -> Option<&'a [FieldDef]> {
    let TypeRef::Named(payload_type) = &variant.fields.first()?.ty else {
        return None;
    };
    api_types
        .iter()
        .find(|typ| &typ.name == payload_type)
        .map(|typ| typ.fields.as_slice())
}

/// Generate the native enum class and wire-shape helpers for explicitly tagged variants.
fn gen_data_enum_typeddicts(
    lines: &mut Vec<String>,
    enum_def: &EnumDef,
    coercible_dtos: &AHashSet<&str>,
    is_host_enum: bool,
    api_types: &[TypeDef],
) {
    let repr = crate::codegen::serde_enum_repr::serde_enum_repr(enum_def);
    let tag_field = repr.tag().unwrap_or(DEFAULT_TAG_FIELD);
    let rename_all = enum_def.serde_rename_all.as_deref();

    // Untagged payloads and externally tagged variants have no discriminator field.
    // Do not invent `type: Literal[...]` wire dictionaries for those representations.
    for variant in enum_def.variants.iter().filter(|_| repr.tag().is_some()) {
        let class_name = format!("{}{}Variant", enum_def.name, variant.name);

        let tag_value =
            crate::codegen::naming::wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all);

        // serde's adjacent form nests the whole payload under the content key, so the variant's
        // TypedDict declares that one key. A struct variant's payload is an object in its own
        // right and gets its own TypedDict; a newtype variant's is whatever it wraps. ~keep
        let payload_type_name = repr
            .content()
            .and_then(|_| adjacent_payload_type(lines, enum_def, variant));

        lines.push(format!("class {}(TypedDict):", class_name));

        lines.push(format!("    {}: Literal[\"{}\"]", tag_field, tag_value));

        // A flattened newtype variant carries the PAYLOAD's fields beside the tag and no key of
        // its own, so declaring the synthetic `_0` advertised a key no payload ever has. ~keep
        // Deliberately NOT the resolution-aware `serde_flattens_newtype_payload`: serde flattens
        // on the payload's own `Serialize` impl, so an internally tagged newtype variant is
        // flattened at runtime whether or not `api_types` can resolve it. A stub only describes
        // shape, so the unresolvable case must still take the flattened arm (tag alone) instead
        // of falling through to the verbatim arm and advertising `_0` as a real key. ~keep
        let is_internal_newtype_payload =
            matches!(repr, crate::codegen::serde_enum_repr::SerdeEnumRepr::Internal { .. })
                && variant.is_tuple
                && variant.fields.len() == 1
                && matches!(variant.fields[0].ty, TypeRef::Named(_));
        let flattened_fields =
            is_internal_newtype_payload.then(|| flattened_payload_fields(variant, api_types).unwrap_or_default());

        match (repr.content(), payload_type_name, flattened_fields) {
            (Some(content_field), Some(payload_type), _) => {
                lines.push(format!("    {content_field}: {payload_type}"));
            }
            (Some(_), None, _) => {}
            (None, _, Some(payload_fields)) => {
                for field in payload_fields {
                    if python_safe_name(&field.name) == tag_field {
                        continue;
                    }
                    lines.push(typed_dict_field_line(field));
                }
            }
            (None, _, None) => {
                // A single positional field is the variant's whole payload with no wire key of
                // its own: external tagging keys it on the variant name itself and untagged
                // serde writes it bare, so serde never puts a `_0` key on the wire in either
                // case. The class already exposes the value through a real runtime accessor --
                // the typed property (`gen_data_enum_variant_accessor_stubs`) for a `Named`
                // payload, or the untyped dict-shaped `#[getter]` otherwise -- so declaring `_0`
                // here described a key the constructor never accepts and nothing in this file
                // referenced. A multi-field tuple variant's shape is unaffected. ~keep
                // Matches the exact tuple-field-name test `variant_accessor`
                // (`codegen::generators::enums`) uses to recognize a synthesized positional
                // field, rather than trusting `variant.is_tuple` alone -- a genuine multi-field
                // tuple variant keeps every field the loop below still reaches.
                let is_single_tuple_field = variant.fields.len() == 1
                    && variant.fields[0]
                        .name
                        .strip_prefix('_')
                        .is_some_and(|suffix| suffix.chars().all(|c| c.is_ascii_digit()));
                if !is_single_tuple_field {
                    for field in &variant.fields {
                        lines.push(typed_dict_field_line(field));
                    }
                }
            }
        }

        lines.push("".to_string());
    }

    lines.push(format!("class {}:", enum_def.name));
    // `write_pyo3_serde_tag_getter` (`codegen::generators::enums`) only emits a `type`-style
    // getter when the enum has an explicit `#[serde(tag = "...")]` -- true for Internal and
    // Adjacent representations, both of which set `EnumDef::serde_tag`. External and Untagged
    // enums never set it, so the wrapper pyclass exposes no such attribute for them at all. ~keep
    if enum_def.serde_tag.is_some() {
        lines.push(format!("    {}: str", tag_field));
    }
    gen_data_enum_variant_accessor_stubs(lines, enum_def);
    gen_data_enum_variant_constructor_stubs(lines, enum_def, coercible_dtos, is_host_enum);
    // The runtime wrapper exposes a `#[new]` accepting a tag string, a `{"type": ...}` dict, or
    // serde-based `#[new]` is omitted and the type is return-only. Mirror that here so a converter
    if !crate::codegen::generators::enum_has_sanitized_fields(enum_def) {
        lines.push(
            "    def __init__(self, value: dict[str, Any] | str | None = None, **kwargs: Any) -> None: ...".to_string(),
        );
    }
    lines.push("    def __str__(self) -> str: ...".to_string());
    lines.push("    def __repr__(self) -> str: ...".to_string());
}

/// Emit a bare-annotation stub property for each typed `#[getter]` the PyO3 binding exposes on a
/// data enum's wrapper `#[pyclass]` -- `write_pyo3_variant_accessors`
/// (`codegen::generators::enums`) emits one such getter per single-tuple-field variant whose
/// payload is a `TypeRef::Named` type (e.g. `FormatMetadata::Excel(ExcelMetadata)` becomes
/// `metadata.format.excel: ExcelMetadata | None`). `collect_variant_accessors` shares the exact
/// eligibility rule with that emitter, so this can never declare a property the runtime pyclass
/// lacks or omit one it has.
///
/// A dict-shaped variant (unit, multi-field tuple, struct, or non-`Named` payload) also gets a
/// runtime `#[getter]` via `write_pyo3_variant_accessors`, but it returns an untyped
/// `dict[str, Any] | None` the stub can't usefully narrow beyond what mypy already infers for an
/// unannotated attribute, so — matching every other stub emitter in this module, which declares
/// only the typed surface — it is left undeclared here.
///
/// Bare annotation (not `@property`) matches the precedent `gen_type_stub`
/// (`backends::pyo3::gen_stubs::classes`) already sets for `#[pyo3(get)]` struct fields.
///
/// The runtime getter is unconditional per variant: `write_pyo3_variant_accessors` iterates every
/// variant with no cfg filter on the loop itself, and only drops the match ARM inside a
/// foreign-cfg-gated variant's getter body (falling back to `_ => None`), never the getter method.
/// So, unlike `gen_data_enum_variant_constructor_stubs`'s `@staticmethod`s (which really can be
/// absent under some cfg), this property is always present and needs no `is_host_enum` filter.
fn gen_data_enum_variant_accessor_stubs(lines: &mut Vec<String>, enum_def: &EnumDef) {
    use crate::codegen::generators::collect_variant_accessors;

    for accessor in collect_variant_accessors(enum_def) {
        lines.push(format!(
            "    {}: {} | None",
            accessor.py_name,
            python_type(&TypeRef::Named(accessor.inner_type_name.to_string()))
        ));
    }
}

/// Emit a `@staticmethod` stub for each per-variant constructor the PyO3 binding exposes.
///
/// The runtime binding declares these under the bare snake_case host name (via
/// `#[pyo3(name = "<snake>")]`), so the stub declares the same public name. Each param type maps
/// through [`python_type`] — the same mapper the surrounding stub uses for fields — and the return
/// type is the enum itself. `collect_pyo3_variant_constructors` owns the skip rules (unit / tuple /
/// `binding_excluded` / sanitized-field variants) so the stub and runtime binding stay aligned —
/// including for a variant whose snake_case name collides with a hand-written `impl EnumType { .. }`
/// method, since the runtime binding never forwards that method either (see its doc comment).
///
/// `is_host_enum` additionally excludes any variant
/// `gen_pyo3_enum_variant_constructors_content` (`codegen::generators::enums`) itself drops as an
/// unreachable FOREIGN cfg-gated variant -- without this the stub would advertise a
/// `@staticmethod` PyO3 never registers.
fn gen_data_enum_variant_constructor_stubs(
    lines: &mut Vec<String>,
    enum_def: &EnumDef,
    coercible_dtos: &AHashSet<&str>,
    is_host_enum: bool,
) {
    use crate::codegen::generators::{collect_pyo3_variant_constructors, variant_constructor_is_reachable};

    let ctors: Vec<_> = collect_pyo3_variant_constructors(enum_def)
        .into_iter()
        .filter(|ctor| {
            enum_def
                .variants
                .iter()
                .find(|v| v.name == ctor.variant_name)
                .is_some_and(|v| variant_constructor_is_reachable(v, is_host_enum))
        })
        .collect();

    let shadowed: Vec<&str> = SHADOWABLE_BUILTIN_TYPES
        .iter()
        .copied()
        .filter(|b| ctors.iter().any(|c| c.snake_name == *b))
        .collect();

    for ctor in &ctors {
        let params: Vec<String> = ctor
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let optional = p.optional || crate::codegen::shared::is_promoted_optional(&ctor.params, idx);
                let py_type = factory_param_type(&p.ty, coercible_dtos);
                let py_type = qualify_parameter_type(&python_safe_name(&p.name), &py_type);
                let py_type = qualify_shadowed_builtin_types(&py_type, &shadowed);
                let py_type = if optional && !py_type.contains("| None") {
                    format!("{py_type} | None")
                } else {
                    py_type
                };
                crate::backends::pyo3::template_env::render(
                    "stub_enum_variant_constructor_param.jinja",
                    minijinja::context! {
                        name => python_safe_name(&p.name),
                        py_type => py_type,
                        optional => optional,
                    },
                )
            })
            .collect();
        lines.push(crate::backends::pyo3::template_env::render(
            "stub_enum_variant_constructor.jinja",
            minijinja::context! {
                method_name => python_safe_name(&ctor.snake_name),
                params => params.join(", "),
                return_type => &enum_def.name,
            },
        ));
    }
}

#[cfg(test)]
mod tests;
