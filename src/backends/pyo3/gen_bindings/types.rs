//! Python type generation: `options.py` (enums, dataclasses, TypedDicts) and helpers.

use crate::codegen::doc_emission::doc_first_paragraph_joined;
use crate::codegen::generators;
use crate::codegen::shared::binding_fields;
use crate::core::config::{DtoConfig, PythonDtoStyle};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::ApiSurface;
use ahash::{AHashMap, AHashSet};

use super::enums::{EmitContext, class_name_to_docstring, sanitize_python_doc};

mod typeddict;
use typeddict::gen_typeddict;

/// Convert a Rust variant name to snake_case for Python enum members (PEP 8),
/// escaping any result that collides with a Python reserved keyword or `str` method name
/// (e.g. `Del` → `del_`, `Title` → `title_`).
fn to_python_enum_variant(name: &str) -> String {
    use heck::ToSnakeCase;
    crate::core::keywords::python_str_enum_ident(&name.to_snake_case())
}

/// Generate options.py — Python-side enums (StrEnum) and @dataclass / TypedDict config types.
///
/// Enum fields in dataclasses use `str` type (not enum class) so users can pass
/// plain strings like `"atx"` instead of `HeadingStyle.Atx`.
/// Default values come from `typed_default` if available, otherwise type-appropriate zeros.
///
/// When `dto.python_output_style() == TypedDict` and a type has `is_return_type = true`,
/// it is emitted as a `TypedDict` (with `total=False`) instead of a `@dataclass`.
/// Names of the types `options.py` emits as `@dataclass` config DTOs: non-trait,
/// `has_default`, not a return type, not an internal `*Update` type, and not
/// re-exported as a native pyclass. This is the public *input* type family — the
/// trait-callback marshalling and the Protocol stubs use the same set so the type
/// a host is handed is the type the package exports under that name.
pub(in crate::backends::pyo3) fn options_dataclass_type_names(
    api: &ApiSurface,
    reexported_types: &[String],
) -> std::collections::HashSet<String> {
    let reexported: AHashSet<&str> = reexported_types.iter().map(String::as_str).collect();
    api.types
        .iter()
        .filter(|t| {
            !t.is_trait
                && t.has_default
                && !t.is_return_type
                && !t.name.ends_with("Update")
                && !reexported.contains(t.name.as_str())
        })
        .map(|t| t.name.clone())
        .collect()
}

pub(super) fn gen_options_py(
    api: &ApiSurface,
    module_name: &str,
    dto: &DtoConfig,
    reexported_types: &[String],
) -> String {
    use crate::core::ir::TypeRef;
    use heck::ToSnakeCase;
    // A type re-exported as a native pyclass is native everywhere — never emit a parallel TypedDict
    // for it, so a field of this type carries a single, consistent identity (the native class).
    let reexported_names: AHashSet<&str> = reexported_types.iter().map(String::as_str).collect();

    // Collect enum names for type detection (plain unit enums vs data enums)
    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();
    // Data enums (tagged unions) are exposed as dict-accepting structs, not str enums.
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();
    // Data enums with at least one unit (tag-only) variant additionally accept a bare string
    // tag on the wire (e.g. `output_format="native"`), so their config fields are typed
    // `<Class> | str`. Payload-only data enums (e.g. EmbeddingModelType) accept only the class.
    let str_coercible_data_enums: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| data_enum_names.contains(e.name.as_str()) && e.variants.iter().any(|v| v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    // Determine whether any type will be emitted as TypedDict so we know which imports to add.
    let output_style = dto.python_output_style();
    let any_typeddict = output_style == PythonDtoStyle::TypedDict
        && api.types.iter().any(|t| {
            t.has_default
                && t.is_return_type
                && !t.fields.is_empty()
                && !t.name.ends_with("Update")
                && !reexported_names.contains(t.name.as_str())
        });

    // Check whether `Any` is needed: TypeRef::Json maps to `dict[str, Any]`.
    // Data enums now use their concrete type names (imported from native module),
    // so they no longer contribute to the `Any` requirement.
    let needs_any = api
        .types
        .iter()
        .filter(|t| !t.is_trait && t.has_default)
        .any(|t| binding_fields(&t.fields).any(|f| type_contains_json(&f.ty)));

    // Collect all Named types referenced by has_default types (including inside Vec/Optional).
    // Only include types that will actually be emitted in options.py:
    //   - Skip `*Update` types (internal, never emitted as @dataclass).
    //   - Skip return types unless TypedDict style is active (return types are native pyclasses).
    let mut referenced_types: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.has_default && !typ.name.ends_with("Update") {
            let is_emitted = !typ.is_return_type || output_style == PythonDtoStyle::TypedDict;
            if !is_emitted {
                continue;
            }
            for field in binding_fields(&typ.fields) {
                collect_named_types(&field.ty, &mut referenced_types);
            }
        }
    }

    // Generate only "public" enums — skip internal types like TextDirection, LinkType etc.
    // that aren't part of the user-facing config API.
    // Generate enums referenced by has_default type fields AND return type fields
    // (return types are emitted as TypedDicts and may reference enums like StructureKind).
    let mut needed_enums: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.has_default || typ.is_return_type {
            for field in binding_fields(&typ.fields) {
                collect_named_types_filtered(&field.ty, &enum_names, &mut needed_enums);
            }
        }
    }

    // Transitively expand needed_enums: data enums referenced by other data enum variants
    // also need to be defined (either as union aliases or str,Enum classes) in options.py.
    // Example: `ToolChoice` variants reference `ToolChoiceMode` (simple enum) and
    // `SpecificToolChoice` (struct); `UserContent` variants reference `ContentPart` (data enum).
    let enum_defs_by_name: AHashMap<&str, &crate::core::ir::EnumDef> =
        api.enums.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut changed = true;
    while changed {
        changed = false;
        let current: Vec<String> = needed_enums.iter().cloned().collect();
        for name in current {
            if let Some(enum_def) = enum_defs_by_name.get(name.as_str()) {
                if generators::enum_has_data_variants(enum_def) {
                    for variant in &enum_def.variants {
                        for field in &variant.fields {
                            let mut discovered = AHashSet::new();
                            collect_named_types_filtered(&field.ty, &enum_names, &mut discovered);
                            for discovered_name in discovered {
                                if needed_enums.insert(discovered_name) {
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Compute which Named types are defined locally in options.py and which must be imported
    // from the native extension module under TYPE_CHECKING.
    // Types defined locally: has_default dataclasses only.
    // Unit enums (needed_enums) are defined as #[pyclass] in the native module and imported
    // from there — they are NOT re-emitted as (str, Enum) subclasses in options.py.
    let local_type_names: AHashSet<&str> = {
        let mut local = AHashSet::new();
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            if typ.name.ends_with("Update") || typ.fields.is_empty() {
                continue;
            }
            // has_default types are emitted as @dataclass / TypedDict locally.
            if typ.has_default && !typ.is_return_type {
                local.insert(typ.name.as_str());
            }
            // When output_style == TypedDict, return types are also emitted locally as TypedDicts —
            // they must NOT be imported from the native module. This must mirror the emission loop,
            // which only emits `has_default` types: a return type without `has_default` is a native
            // pyclass that is imported, not redefined locally. Marking it local here without the
            // `has_default` guard would leave it neither imported nor emitted (an undefined name).
            if output_style == PythonDtoStyle::TypedDict
                && typ.is_return_type
                && typ.has_default
                && !reexported_names.contains(typ.name.as_str())
            {
                local.insert(typ.name.as_str());
            }
        }
        local
    };
    // Native types: referenced in has_default fields but not defined locally in options.py.
    // This includes data enum types, which are imported from the native module and used
    // with their concrete type names (proper typing instead of dict[str, Any]).
    let mut native_type_imports: Vec<String> = referenced_types
        .iter()
        .filter(|n| !local_type_names.contains(n.as_str()))
        .cloned()
        .collect();
    native_type_imports.sort();

    // Enums (unit AND data) are imported at runtime from the native module and referenced by class
    // name in field annotations (`model: EmbeddingModelType | None`) — the class users construct,
    // not a flattened alias. A runtime import keeps the annotation resolvable. Some enums are reached
    // only transitively through data-enum variants, so derive from needed_enums.
    let mut runtime_native_imports: Vec<String> = needed_enums.iter().cloned().collect();
    runtime_native_imports.sort();
    runtime_native_imports.dedup();
    let runtime_native_import_names: AHashSet<&str> = runtime_native_imports.iter().map(String::as_str).collect();
    let mut type_checking_only_imports: Vec<String> = native_type_imports
        .iter()
        .filter(|n| !runtime_native_import_names.contains(n.as_str()))
        .cloned()
        .collect();
    type_checking_only_imports.sort();
    type_checking_only_imports.dedup();

    let mut out = String::with_capacity(4096);
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str("\"\"\"Configuration options for the conversion API.\"\"\"\n\n");
    // Use deferred annotation evaluation so forward references to types defined in the
    // native extension module (e.g. HeaderMetadata, GridCell) do not raise NameError.
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("from dataclasses import dataclass, field\n");
    // Emit `from enum import Enum` only when there are still (str, Enum) subclasses to define.
    // Unit enums are re-exported from the native module, so Enum is not needed for them.
    let has_non_needed_str_enums = api
        .enums
        .iter()
        .any(|e| !needed_enums.contains(&e.name) && !data_enum_names.contains(e.name.as_str()));
    if has_non_needed_str_enums {
        out.push_str("from enum import Enum\n");
    }
    // Build typing imports: TYPE_CHECKING is needed for native-module forward refs;
    // Any is needed when TypeRef::Json fields exist; TypedDict is needed when output_style == TypedDict.
    let needs_type_checking = !type_checking_only_imports.is_empty();
    let needs_typing_import = needs_type_checking || needs_any || any_typeddict;
    if needs_typing_import {
        let mut typing_names = Vec::new();
        if needs_type_checking {
            typing_names.push("TYPE_CHECKING");
        }
        if needs_any {
            typing_names.push("Any");
        }
        if any_typeddict {
            typing_names.push("TypedDict");
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "typing_import.jinja",
            minijinja::context! { names => typing_names },
        ));
    }
    // Runtime imports for native enums (unit enums and data-enum classes) — referenced by config
    // field annotations and importable by users (e.g. `from sample_markdown.options import NewlineStyle`).
    if !runtime_native_imports.is_empty() {
        // Blank line separates stdlib imports from the relative package import (isort E401).
        out.push('\n');
        out.push_str(&crate::backends::pyo3::template_env::render(
            "import_from_module_header.jinja",
            minijinja::context! { module_name => module_name },
        ));
        for name in &runtime_native_imports {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "import_item.jinja",
                minijinja::context! { name => name },
            ));
        }
        out.push_str(")\n");
    }
    out.push('\n');
    // Import non-enum native-module types for static analysis only (TYPE_CHECKING guard).
    if !type_checking_only_imports.is_empty() {
        out.push_str("if TYPE_CHECKING:\n");
        out.push_str(&crate::backends::pyo3::template_env::render(
            "type_checking_import_header.jinja",
            minijinja::context! { module_name => module_name },
        ));
        for name in &type_checking_only_imports {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "type_checking_import_item.jinja",
                minijinja::context! { name => name },
            ));
        }
        out.push_str("    )\n");
    }
    out.push_str("\n\n");

    // Build map of enum name → default variant string value.
    // Uses the variant with is_default=true (#[default] attr), falls back to first variant.
    let enum_defaults: AHashMap<String, String> = api
        .enums
        .iter()
        .filter_map(|e| {
            let default_v = e.variants.iter().find(|v| v.is_default).or(e.variants.first());
            default_v.map(|v| (e.name.clone(), v.name.to_snake_case()))
        })
        .collect();

    // Unit enums (needed_enums) live as #[pyclass] in the native module. Each variant is
    // already exposed as UPPER_SNAKE_CASE via #[pyo3(name = "UPPER_SNAKE_CASE")] in the
    // generated Rust binding (see alef-codegen::generators::enums::gen_enum), so no
    // runtime monkey-patching aliases are needed here.
    let mut sorted_needed_enums: Vec<&String> = needed_enums.iter().collect();
    sorted_needed_enums.sort();

    for enum_def in &api.enums {
        // Unit enums are handled above — do not emit a duplicate (str, Enum) subclass.
        if needed_enums.contains(&enum_def.name) {
            continue;
        }
        // Data enums are dict-accepting structs on the Rust side; skip str,Enum generation.
        if data_enum_names.contains(enum_def.name.as_str()) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "str_enum_class_header.jinja",
            minijinja::context! { name => &enum_def.name },
        ));
        let enum_doc = if !enum_def.doc.is_empty() {
            let raw = doc_first_paragraph_joined(&enum_def.doc);
            let first = sanitize_python_doc(&raw);
            let content = if first.len() > 89 {
                first[..89].to_string()
            } else {
                first
            };
            if content.ends_with(['.', '?', '!']) {
                content
            } else {
                format!("{}.", content)
            }
        } else {
            class_name_to_docstring(&enum_def.name)
        };
        out.push_str(&crate::backends::pyo3::template_env::render(
            "enum_docstring.jinja",
            minijinja::context! { doc => &enum_doc },
        ));
        // ruff format (E303): blank line required after class docstring before first member.
        out.push('\n');
        for variant in &enum_def.variants {
            let value = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| crate::codegen::naming::pascal_to_snake(&variant.name));
            out.push_str(&crate::backends::pyo3::template_env::render(
                "enum_variant.jinja",
                minijinja::context! {
                    name => to_python_enum_variant(&variant.name),
                    value => &value,
                },
            ));
            out.push('\n');
        }
        out.push_str("\n\n");
    }

    // Generate @dataclass or TypedDict for types with has_default (user-facing config types)
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if !typ.has_default {
            continue;
        }
        // Skip "Update" types — they're internal
        if typ.name.ends_with("Update") {
            continue;
        }

        // Use TypedDict for return types when the output style is configured as TypedDict.
        // A reexported-native return type is imported and referenced as the native pyclass — never
        // emitted as a parallel TypedDict here (that second identity is what breaks field typing).
        let use_typeddict = output_style == PythonDtoStyle::TypedDict
            && typ.is_return_type
            && !reexported_names.contains(typ.name.as_str());

        // Return types are defined authoritatively by the Rust native module as #[pyclass]
        // structs. Emitting a @dataclass with the same name creates a shadow class that breaks
        // static analysis — Pylance reports a type mismatch because the @dataclass and the
        // native PyO3 class are unrelated types even though they share a name.
        // Only emit a TypedDict when explicitly configured; otherwise skip entirely.
        if typ.is_return_type && !use_typeddict {
            continue;
        }

        if use_typeddict {
            out.push_str(&gen_typeddict(
                typ,
                &enum_names,
                &data_enum_names,
                &str_coercible_data_enums,
            ));
        } else {
            out.push_str("@dataclass(frozen=True, slots=True)\n");
            out.push_str(&crate::backends::pyo3::template_env::render(
                "dataclass_header.jinja",
                minijinja::context! { name => &typ.name },
            ));
            let class_doc = if !typ.doc.is_empty() {
                let raw = doc_first_paragraph_joined(&typ.doc);
                let first = sanitize_python_doc(&raw);
                let content = if first.len() > 89 {
                    first[..89].to_string()
                } else {
                    first
                };
                if content.ends_with(['.', '?', '!']) {
                    content
                } else {
                    format!("{}.", content)
                }
            } else {
                class_name_to_docstring(&typ.name)
            };
            out.push_str(&crate::backends::pyo3::template_env::render(
                "class_docstring.jinja",
                minijinja::context! { doc => &class_doc },
            ));
            // ruff format (E303): blank line required after class docstring before first field.
            out.push('\n');

            if binding_fields(&typ.fields).next().is_none() {
                // Empty class body — docstring already emitted, so no `pass` needed
                // (the docstring itself serves as the class body). Avoids ruff PIE790.
                out.push('\n');
                continue;
            }

            for field in binding_fields(&typ.fields) {
                // Determine Python type hint
                let type_hint = python_field_type(
                    &field.ty,
                    field.optional,
                    &enum_names,
                    &data_enum_names,
                    &str_coercible_data_enums,
                    EmitContext::OptionsModule,
                );

                // Determine default value and check if we need | None
                let (type_hint_with_none, default) = if let Some(td) = &field.typed_default {
                    // For optional fields with Empty default, use None — not a zero value.
                    // This ensures Option<usize> defaults to None (not 0), preventing
                    // "max_concurrent must be > 0" validation errors.
                    let default = if field.optional && matches!(td, crate::core::ir::DefaultValue::Empty) {
                        "None".to_string()
                    } else {
                        typed_default_to_python(td, &field.ty, &enum_defaults, &data_enum_names)
                    };
                    // When the effective default is None (e.g. Duration with Empty typed_default),
                    // add | None to the type hint so the annotation matches the default value.
                    // Append `| None` unless the hint already admits None. Checking for `None`
                    // (not just any `|`) matters for str-coercible data enums whose hint is
                    // already a union (`ChunkSizing | str`) but still needs `| None` for a None default.
                    let hint = if default == "None" && !type_hint.contains("None") {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (hint, default)
                } else if field.optional {
                    // If default is None but type is Named (not already Optional), add | None.
                    // A str-coercible data enum's hint is `<Class> | str` (a union without None),
                    // so guard on the absence of `None` rather than the absence of any `|`.
                    let final_hint = if !type_hint.contains("None") && matches!(&field.ty, TypeRef::Named(_)) {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (final_hint, "None".to_string())
                } else {
                    let default = python_zero_value(&field.ty, &enum_names, &data_enum_names);
                    // When the zero value is None (e.g. data enum fields), add | None so the
                    // annotation matches — `dict[str, Any] = None` is a mypy type error.
                    // Append `| None` unless the hint already admits None. Checking for `None`
                    // (not just any `|`) matters for str-coercible data enums whose hint is
                    // already a union (`ChunkSizing | str`) but still needs `| None` for a None default.
                    let hint = if default == "None" && !type_hint.contains("None") {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (hint, default)
                };

                let safe_name = crate::core::keywords::python_ident(&field.name);
                if !field.doc.is_empty() {
                    out.push_str(&crate::backends::pyo3::template_env::render(
                        "trait_bridge/dataclass_field_with_default.jinja",
                        minijinja::context! { name => &safe_name, type_hint => &type_hint_with_none, default => &default },
                    ));
                    out.push('\n');
                    let doc_line = sanitize_python_doc(&doc_first_paragraph_joined(&field.doc));
                    // Avoid `""""` when docstring ends with `"` — add trailing space.
                    let safe_doc = if doc_line.ends_with('"') {
                        format!("{doc_line} ")
                    } else {
                        doc_line
                    };
                    out.push_str(&crate::backends::pyo3::template_env::render(
                        "trait_bridge/python_docstring.jinja",
                        minijinja::context! { text => &safe_doc },
                    ));
                    out.push('\n');
                } else {
                    out.push_str(&crate::backends::pyo3::template_env::render(
                        "trait_bridge/dataclass_field_with_default.jinja",
                        minijinja::context! { name => &safe_name, type_hint => &type_hint_with_none, default => &default },
                    ));
                    out.push('\n');
                }
            }
            out.push('\n');
        }
    }

    // Data enums are imported from the native module and referenced by their class name in the
    // field annotations above — not redefined here as the old flattened union alias.

    // Converters from the native pyclass to the public dataclass, one per emitted
    // dataclass. The trait-callback bridges call these so a host method receives the
    // same type the package publicly exports (the options dataclass), not the
    // private native class.
    out.push_str(&gen_from_native_converters(api, reexported_types));

    out
}

/// Emit `_from_native_<snake>(native)` module-level converters for every emitted
/// options dataclass. Nested dataclass fields recurse (including through
/// `Optional`/`Vec`/`Map` wrappers); every other field passes through unchanged —
/// enums and re-exported types keep their single native identity.
fn gen_from_native_converters(api: &ApiSurface, reexported_types: &[String]) -> String {
    use heck::ToSnakeCase;
    let options_types = options_dataclass_type_names(api, reexported_types);
    let mut out = String::new();
    let mut emitted: Vec<&crate::core::ir::TypeDef> =
        api.types.iter().filter(|t| options_types.contains(&t.name)).collect();
    emitted.sort_by(|a, b| a.name.cmp(&b.name));

    for typ in emitted {
        // Same field filter as the dataclass emission above: a binding-excluded
        // field is not a dataclass field, so it must not become a converter kwarg.
        let fields: Vec<minijinja::Value> = binding_fields(&typ.fields)
            .map(|f| {
                let safe_name = crate::core::keywords::python_ident(&f.name);
                let src = format!("native.{safe_name}");
                let inner_expr = from_native_field_expr(&f.ty, &options_types, &src);
                // Fields are commonly `Named` + `optional: true` in the IR rather than
                // `TypeRef::Optional` — a converting expression must still be None-guarded
                // so a config with an absent nested section survives the lift.
                let expr = if f.optional && inner_expr != src && !inner_expr.starts_with("(None if ") {
                    format!("(None if {src} is None else {inner_expr})")
                } else {
                    inner_expr
                };
                minijinja::context! {
                    name => &safe_name,
                    expr => expr,
                }
            })
            .collect();
        out.push_str(&crate::backends::pyo3::template_env::render(
            "trait_bridge/options_from_native.jinja",
            minijinja::context! {
                fn_name => format!("_from_native_{}", typ.name.to_snake_case()),
                class_name => &typ.name,
                fields => fields,
            },
        ));
        out.push('\n');
    }
    out
}

/// Python expression converting one field value from the native object to the
/// options-dataclass shape. Returns `src` unchanged when no conversion applies.
fn from_native_field_expr(
    ty: &crate::core::ir::TypeRef,
    options_types: &std::collections::HashSet<String>,
    src: &str,
) -> String {
    use crate::core::ir::TypeRef;
    use heck::ToSnakeCase;
    match ty {
        TypeRef::Named(n) if options_types.contains(n) => {
            format!("_from_native_{}({src})", n.to_snake_case())
        }
        TypeRef::Optional(inner) => {
            let inner_expr = from_native_field_expr(inner, options_types, src);
            if inner_expr == src {
                src.to_string()
            } else {
                format!("(None if {src} is None else {inner_expr})")
            }
        }
        TypeRef::Vec(inner) => {
            let inner_expr = from_native_field_expr(inner, options_types, "__v");
            if inner_expr == "__v" {
                src.to_string()
            } else {
                format!("[{inner_expr} for __v in {src}]")
            }
        }
        TypeRef::Map(_, value) => {
            let value_expr = from_native_field_expr(value, options_types, "__val");
            if value_expr == "__val" {
                src.to_string()
            } else {
                format!("{{__k: {value_expr} for __k, __val in {src}.items()}}")
            }
        }
        _ => src.to_string(),
    }
}

pub(super) fn python_field_type(
    ty: &crate::core::ir::TypeRef,
    optional: bool,
    enum_names: &AHashSet<&str>,
    data_enum_names: &AHashSet<&str>,
    str_coercible_data_enums: &AHashSet<&str>,
    context: EmitContext,
) -> String {
    use crate::core::ir::TypeRef;
    let base = match ty {
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "bool".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "float".to_string(),
            _ => "int".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path => "str".to_string(),
        // A JSON value (serde_json::Value) is exposed as `dict[str, Any]`, matching the native
        // module's type mapper. Mapping it to `str` instead made the options field disagree with
        // the compiled config it is converted into (e.g. `paddle_ocr_config`, `additional`).
        TypeRef::Json => "dict[str, Any]".to_string(),
        TypeRef::Bytes => "bytes".to_string(),
        TypeRef::Vec(inner) => {
            format!(
                "list[{}]",
                python_field_type(
                    inner,
                    false,
                    enum_names,
                    data_enum_names,
                    str_coercible_data_enums,
                    context
                )
            )
        }
        TypeRef::Map(k, v) => format!(
            "dict[{}, {}]",
            python_field_type(k, false, enum_names, data_enum_names, str_coercible_data_enums, context),
            python_field_type(v, false, enum_names, data_enum_names, str_coercible_data_enums, context)
        ),
        TypeRef::Named(name) if data_enum_names.contains(name.as_str()) => match context {
            // In options.py the bare name resolves to the data-enum class imported from the
            // native module (under TYPE_CHECKING) — the same class users construct via
            // `EmbeddingModelType.preset(...)`. Enums with a unit (tag-only) variant also
            // accept a bare string tag (e.g. `output_format="native"`), so those are widened
            // to `<Class> | str`; payload-only enums (e.g. EmbeddingModelType) accept only
            // the class.
            EmitContext::OptionsModule => {
                if str_coercible_data_enums.contains(name.as_str()) {
                    format!("{name} | str")
                } else {
                    name.clone()
                }
            }
            // In a _native.pyi stub the same bare name resolves to the PyO3 class
            // exported by the native extension — no qualification needed there either,
            // but the branch is kept separate to allow future divergence (e.g. prefixing).
            EmitContext::NativeStub => name.clone(),
        },
        // Plain enums: use `EnumName | str` so string literals like `"atx"` are accepted
        // alongside enum member values without mypy assignment errors.
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => format!("{name} | str"),
        TypeRef::Named(name) => name.clone(), // Use the concrete type name
        TypeRef::Optional(inner) => {
            return format!(
                "{} | None",
                python_field_type(
                    inner,
                    false,
                    enum_names,
                    data_enum_names,
                    str_coercible_data_enums,
                    context
                )
            );
        }
        TypeRef::Unit => "None".to_string(),
        TypeRef::Duration => "int".to_string(),
    };
    if optional { format!("{} | None", base) } else { base }
}

/// Convert a typed default value to Python literal.
/// For `Empty` on enum-typed fields, resolves to the enum's default (first) variant.
/// For `Empty` on data enum-typed fields, resolves to None (no sensible default dict).
fn typed_default_to_python(
    td: &crate::core::ir::DefaultValue,
    ty: &crate::core::ir::TypeRef,
    enum_defaults: &AHashMap<String, String>,
    data_enum_names: &AHashSet<&str>,
) -> String {
    use crate::core::ir::{DefaultValue, TypeRef};
    match td {
        DefaultValue::BoolLiteral(true) => "True".to_string(),
        DefaultValue::BoolLiteral(false) => "False".to_string(),
        DefaultValue::StringLiteral(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('\"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r");
            format!("\"{}\"", escaped)
        }
        DefaultValue::IntLiteral(i) => i.to_string(),
        DefaultValue::FloatLiteral(f) => format!("{}", f),
        DefaultValue::EnumVariant(v) => {
            use heck::ToSnakeCase;
            format!("\"{}\"", v.to_snake_case())
        }
        DefaultValue::Empty => {
            // For data enum-typed fields, use None (no sensible default dict).
            if let TypeRef::Named(name) = ty {
                if data_enum_names.contains(name.as_str()) {
                    return "None".to_string();
                }
            }
            // For plain enum-typed fields, resolve to the default variant's string value.
            // For other Named types, use None (Rust binding applies its own default).
            if let TypeRef::Named(name) = ty {
                if let Some(default_variant) = enum_defaults.get(name) {
                    return format!("\"{}\"", default_variant);
                }
            }
            // Type-appropriate zero values for Python
            match ty {
                TypeRef::Primitive(p) => match p {
                    crate::core::ir::PrimitiveType::Bool => "False".to_string(),
                    crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "0.0".to_string(),
                    _ => "0".to_string(),
                },
                TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
                // A JSON field is `dict[str, Any]`; its zero value is None (the field-emitter then
                // widens the annotation to `... | None`), not the empty string.
                TypeRef::Json => "None".to_string(),
                TypeRef::Bytes => "b\"\"".to_string(),
                // Duration fields with Empty default are Option<u64> in the binding;
                // use None so the core type's Default provides the real default value.
                TypeRef::Duration => "None".to_string(),
                TypeRef::Vec(_) => "field(default_factory=list)".to_string(),
                TypeRef::Map(_, _) => "field(default_factory=dict)".to_string(),
                _ => "None".to_string(),
            }
        }
        DefaultValue::None => "None".to_string(),
    }
}

/// Generate a Python zero value for a type (when no typed_default is available).
fn python_zero_value(
    ty: &crate::core::ir::TypeRef,
    enum_names: &AHashSet<&str>,
    data_enum_names: &AHashSet<&str>,
) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "False".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "0.0".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
        // A JSON field is `dict[str, Any]`; its zero value is None (annotation widened to `| None`).
        TypeRef::Json => "None".to_string(),
        TypeRef::Bytes => "b\"\"".to_string(),
        TypeRef::Vec(_) => "field(default_factory=list)".to_string(),
        TypeRef::Map(_, _) => "field(default_factory=dict)".to_string(),
        // Data enums have no simple zero value; default to None (they're typically Optional).
        TypeRef::Named(name) if data_enum_names.contains(name.as_str()) => "None".to_string(),
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => "\"\"".to_string(),
        TypeRef::Named(_) => "None".to_string(),
        TypeRef::Optional(_) => "None".to_string(),
        TypeRef::Unit => "None".to_string(),
        // Duration fields are stored as Option<u64> in has_default binding structs,
        // so None is the correct zero value (falls back to core Default).
        TypeRef::Duration => "None".to_string(),
    }
}

/// Check if a TypeRef transitively contains TypeRef::Json (which maps to `Any` in Python).
fn type_contains_json(ty: &crate::core::ir::TypeRef) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_contains_json(inner),
        TypeRef::Map(k, v) => type_contains_json(k) || type_contains_json(v),
        _ => false,
    }
}

/// Recursively collect all Named type references from a TypeRef.
pub(super) fn collect_named_types(ty: &crate::core::ir::TypeRef, out: &mut AHashSet<String>) {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(n) => {
            out.insert(n.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Collect only Named types that appear in `enum_names` (recursing into Vec/Optional/Map).
/// Used to find enum-typed fields in has_default types for generating (str, Enum) classes.
pub(super) fn collect_named_types_filtered(
    ty: &crate::core::ir::TypeRef,
    enum_names: &AHashSet<&str>,
    out: &mut AHashSet<String>,
) {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(n) if enum_names.contains(n.as_str()) => {
            out.insert(n.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types_filtered(inner, enum_names, out),
        TypeRef::Map(k, v) => {
            collect_named_types_filtered(k, enum_names, out);
            collect_named_types_filtered(v, enum_names, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
