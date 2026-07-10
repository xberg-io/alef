use super::{pyi_docstring, python_safe_name};
use crate::backends::pyo3::type_map::python_type;
use crate::core::ir::{EnumDef, TypeRef};
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
    use heck::ToShoutySnakeCase;
    crate::core::keywords::python_str_enum_ident(&name.to_shouty_snake_case())
}

/// Generate a Python enum stub.
pub(super) fn gen_enum_stub(enum_def: &EnumDef, emit_docstrings: bool, coercible_dtos: &AHashSet<&str>) -> String {
    use crate::codegen::generators::enum_has_data_variants;
    let mut lines = vec![];

    if enum_has_data_variants(enum_def) {
        gen_data_enum_typeddicts(&mut lines, enum_def, coercible_dtos);
    } else {
        lines.push(format!("class {}:", enum_def.name));
        if emit_docstrings {
            if let Some(docstring) = pyi_docstring(&enum_def.doc, "    ") {
                lines.push(docstring);
            }
        }
        for variant in &enum_def.variants {
            // Emit snake_case attribute names to match the #[pyo3(name = "...")] rename
            lines.push(format!(
                "    {}: {} = ...",
                to_python_enum_variant(&variant.name),
                enum_def.name
            ));
            if emit_docstrings {
                if let Some(docstring) = pyi_docstring(&variant.doc, "    ") {
                    lines.push(docstring);
                }
            }
        }
        lines.push("    def __init__(self, value: int | str) -> None: ...".to_string());
    }

    lines.join("\n")
}

/// Generate TypedDicts for each variant of a data enum, plus a Union type alias.
fn gen_data_enum_typeddicts(lines: &mut Vec<String>, enum_def: &EnumDef, coercible_dtos: &AHashSet<&str>) {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let rename_all = enum_def.serde_rename_all.as_deref();

    let mut variant_class_names = vec![];

    for variant in &enum_def.variants {
        let class_name = format!("{}{}Variant", enum_def.name, variant.name);
        variant_class_names.push(class_name.clone());

        let tag_value =
            crate::codegen::naming::wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all);

        lines.push(format!("class {}(TypedDict):", class_name));

        lines.push(format!("    {}: Literal[\"{}\"]", tag_field, tag_value));

        for field in &variant.fields {
            let field_type = python_type(&field.ty);
            let field_type = if field.optional && !field_type.contains("| None") {
                format!("{} | None", field_type)
            } else {
                field_type
            };
            lines.push(format!("    {}: {}", python_safe_name(&field.name), field_type));
        }

        lines.push("".to_string());
    }

    lines.push(format!("class {}:", enum_def.name));
    lines.push(format!("    {}: str", tag_field));
    gen_data_enum_variant_constructor_stubs(lines, enum_def, coercible_dtos);
    // The runtime wrapper exposes a `#[new]` accepting a tag string, a `{"type": ...}` dict, or
    // serde-based `#[new]` is omitted and the type is return-only. Mirror that here so a converter
    if !crate::codegen::generators::enum_has_sanitized_fields(enum_def) {
        lines.push(
            "    def __init__(self, value: dict[str, Any] | str | None = None, **kwargs: Any) -> None: ...".to_string(),
        );
    }
    lines.push("    def __str__(self) -> str: ...  # noqa: PYI029".to_string());
    lines.push("    def __repr__(self) -> str: ...  # noqa: PYI029".to_string());
}

/// Emit a `@staticmethod` stub for each per-variant constructor the PyO3 binding exposes.
///
/// The runtime binding declares these under the bare snake_case host name (via
/// `#[pyo3(name = "<snake>")]`), so the stub declares the same public name. Each param type maps
/// through [`python_type`] — the same mapper the surrounding stub uses for fields — and the return
/// type is the enum itself. `collect_variant_constructors` owns the skip rules (unit / tuple /
/// `binding_excluded` / sanitized-field variants and hand-written method collisions) so the stub
/// and runtime binding stay aligned.
fn gen_data_enum_variant_constructor_stubs(
    lines: &mut Vec<String>,
    enum_def: &EnumDef,
    coercible_dtos: &AHashSet<&str>,
) {
    use crate::codegen::generators::collect_variant_constructors;

    let ctors = collect_variant_constructors(enum_def);

    const SHADOWABLE_BUILTINS: &[&str] = &["list", "dict", "set", "tuple", "frozenset", "type"];
    let shadowed: Vec<&str> = SHADOWABLE_BUILTINS
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
                let mut py_type = factory_param_type(&p.ty, coercible_dtos);
                for builtin in &shadowed {
                    py_type = py_type.replace(&format!("{builtin}["), &format!("builtins.{builtin}["));
                }
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
