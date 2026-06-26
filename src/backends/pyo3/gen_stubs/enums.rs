use super::{pyi_docstring, python_safe_name};
use crate::backends::pyo3::type_map::python_type;
use crate::core::ir::EnumDef;

fn to_python_enum_variant(name: &str) -> String {
    use heck::ToShoutySnakeCase;
    crate::core::keywords::python_str_enum_ident(&name.to_shouty_snake_case())
}

/// Generate a Python enum stub.
pub(super) fn gen_enum_stub(enum_def: &EnumDef, emit_docstrings: bool) -> String {
    use crate::codegen::generators::enum_has_data_variants;
    let mut lines = vec![];

    if enum_has_data_variants(enum_def) {
        // Data enums: emit a TypedDict per variant and a Union type alias.
        gen_data_enum_typeddicts(&mut lines, enum_def);
    } else {
        lines.push(format!("class {}:", enum_def.name));
        // Enum-level docstring — gated behind emit_docstrings (ruff PYI021).
        if emit_docstrings {
            if let Some(docstring) = pyi_docstring(&enum_def.doc, "    ") {
                lines.push(docstring);
            }
        }
        for variant in &enum_def.variants {
            // Emit snake_case attribute names to match the #[pyo3(name = "...")] rename
            // on the Rust pyclass variant, following Python idiom (PEP 8: enum members are lowercase).
            lines.push(format!(
                "    {}: {} = ...",
                to_python_enum_variant(&variant.name),
                enum_def.name
            ));
            // Variant-level docstring — gated behind emit_docstrings (ruff PYI021).
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
fn gen_data_enum_typeddicts(lines: &mut Vec<String>, enum_def: &EnumDef) {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let rename_all = enum_def.serde_rename_all.as_deref();

    let mut variant_class_names = vec![];

    for variant in &enum_def.variants {
        let class_name = format!("{}{}Variant", enum_def.name, variant.name);
        variant_class_names.push(class_name.clone());

        let tag_value =
            crate::codegen::naming::wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all);

        lines.push(format!("class {}(TypedDict):", class_name));

        // Tag field with Literal type
        lines.push(format!("    {}: Literal[\"{}\"]", tag_field, tag_value));

        // Data fields
        for field in &variant.fields {
            let field_type = python_type(&field.ty);
            let field_type = if field.optional && !field_type.contains("| None") {
                format!("{} | None", field_type)
            } else {
                field_type
            };
            lines.push(format!("    {}: {}", python_safe_name(&field.name), field_type));
        }

        // If no data fields, the TypedDict only has the tag
        if variant.fields.is_empty() && lines.last().is_some_and(|l| l.ends_with("):")) {
            // Empty body — TypedDict needs at least the tag field which is already added
        }

        lines.push("".to_string());
    }

    // Emit a class stub for the opaque pyo3 wrapper.
    // The wrapper exposes the serde tag as a readable attribute and delegates __str__
    // to the inner serde serialization. Variant TypedDicts above are kept for documentation.
    lines.push(format!("class {}:", enum_def.name));
    lines.push(format!("    {}: str", tag_field));
    // Per-variant `@staticmethod` constructors, declared between the tag attribute and the
    // dunder stubs so type-checkers and IDEs see `Shape.circle(...)` factory calls. The variant
    // selection is shared with the runtime binding via `collect_variant_constructors`, so the
    // declared surface stays in lockstep with the methods PyO3 actually exposes.
    gen_data_enum_variant_constructor_stubs(lines, enum_def);
    // PYI029: __str__/__repr__ stubs are needed because the pyo3 wrapper implements them
    // via Display/Debug, and downstream callers rely on str(value) returning the serde tag.
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
fn gen_data_enum_variant_constructor_stubs(lines: &mut Vec<String>, enum_def: &EnumDef) {
    use crate::codegen::generators::collect_variant_constructors;

    let ctors = collect_variant_constructors(enum_def);

    // A per-variant factory method named after a builtin container (e.g. a `List` variant → `def
    // list(...)`) shadows that builtin within the class body, so a sibling annotation like
    // `entries: list[MetadataEntry]` resolves to the factory rather than `builtins.list` and mypy
    // rejects it (`Function ... is not valid as a type`). Qualify the shadowed builtins as
    // `builtins.<name>` in the factory annotations to break the cycle.
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
                // A param is optional in the emitted signature when it is naturally optional OR was
                // promoted because it follows an optional param — the same rule the runtime PyO3
                // binding applies (`is_promoted_optional`), which widens such params to `Optional[T]`
                // with a `= None` default. Mirroring it keeps the stub's required/optional split and
                // defaults identical to the runtime constructor signature.
                let optional = p.optional || crate::codegen::shared::is_promoted_optional(&ctor.params, idx);
                let mut py_type = python_type(&p.ty);
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
