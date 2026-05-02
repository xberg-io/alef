//! Python type generation: `options.py` (enums, dataclasses, TypedDicts) and helpers.

use ahash::{AHashMap, AHashSet};
use alef_codegen::doc_emission::doc_first_paragraph_joined;
use alef_codegen::generators;
use alef_core::config::{DtoConfig, PythonDtoStyle};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::ApiSurface;

use super::enums::{EmitContext, class_name_to_docstring, sanitize_python_doc};

/// Convert a Rust variant name to SCREAMING_SNAKE_CASE for Python (str, Enum) members.
///
/// Handles acronym-style names correctly: names with 2+ leading uppercase characters
/// followed only by lowercase (e.g. `RDFa`) are fully uppercased to `RDFA` rather than
/// incorrectly split to `RD_FA` by `to_shouty_snake_case`.
fn to_python_screaming(name: &str) -> String {
    use heck::ToShoutySnakeCase;
    let chars: Vec<char> = name.chars().collect();
    let upper_prefix_len = chars.iter().take_while(|c| c.is_uppercase()).count();
    // Acronym: 2+ leading uppercase chars with only lowercase (or empty) remainder
    if upper_prefix_len >= 2 && chars[upper_prefix_len..].iter().all(|c| c.is_lowercase()) {
        name.to_ascii_uppercase()
    } else {
        name.to_shouty_snake_case()
    }
}

/// Generate options.py — Python-side enums (StrEnum) and @dataclass / TypedDict config types.
///
/// Enum fields in dataclasses use `str` type (not enum class) so users can pass
/// plain strings like `"atx"` instead of `HeadingStyle.Atx`.
/// Default values come from `typed_default` if available, otherwise type-appropriate zeros.
///
/// When `dto.python_output_style() == TypedDict` and a type has `is_return_type = true`,
/// it is emitted as a `TypedDict` (with `total=False`) instead of a `@dataclass`.
pub(super) fn gen_options_py(api: &ApiSurface, module_name: &str, dto: &DtoConfig) -> String {
    use alef_core::ir::TypeRef;
    use heck::ToSnakeCase;

    // Collect enum names for type detection (plain unit enums vs data enums)
    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();
    // Data enums (tagged unions) are exposed as dict-accepting structs, not str enums.
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();

    // Determine whether any type will be emitted as TypedDict so we know which imports to add.
    let output_style = dto.python_output_style();
    let any_typeddict = output_style == PythonDtoStyle::TypedDict
        && api
            .types
            .iter()
            .any(|t| t.has_default && t.is_return_type && !t.fields.is_empty() && !t.name.ends_with("Update"));

    // Check whether `Any` is needed: TypeRef::Json maps to `dict[str, Any]`.
    // Data enums now use their concrete type names (imported from native module),
    // so they no longer contribute to the `Any` requirement.
    let needs_any = api
        .types
        .iter()
        .filter(|t| !t.is_trait && t.has_default)
        .any(|t| t.fields.iter().any(|f| type_contains_json(&f.ty)));

    // Collect all Named types referenced by has_default types (including inside Vec/Optional).
    let mut referenced_types: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.has_default {
            for field in &typ.fields {
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
            for field in &typ.fields {
                collect_named_types_filtered(&field.ty, &enum_names, &mut needed_enums);
            }
        }
    }

    // Transitively expand needed_enums: data enums referenced by other data enum variants
    // also need to be defined (either as union aliases or str,Enum classes) in options.py.
    // Example: `ToolChoice` variants reference `ToolChoiceMode` (simple enum) and
    // `SpecificToolChoice` (struct); `UserContent` variants reference `ContentPart` (data enum).
    let enum_defs_by_name: AHashMap<&str, &alef_core::ir::EnumDef> =
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
            // When output_style == TypedDict, return types are also emitted locally
            // as TypedDicts — they must NOT be imported from the native module.
            if output_style == PythonDtoStyle::TypedDict && typ.is_return_type {
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

    // Unit enums (needed_enums) that are in native_type_imports must be imported at runtime
    // (not just under TYPE_CHECKING) so the monkey-patching code below can add SCREAMING_SNAKE_CASE
    // aliases to them. Split native_type_imports into runtime and TYPE_CHECKING-only groups.
    let mut runtime_native_imports: Vec<String> = native_type_imports
        .iter()
        .filter(|n| needed_enums.contains(*n) && !data_enum_names.contains(n.as_str()))
        .cloned()
        .collect();
    runtime_native_imports.sort();
    let mut type_checking_only_imports: Vec<String> = native_type_imports
        .iter()
        .filter(|n| !needed_enums.contains(*n))
        .cloned()
        .collect();
    type_checking_only_imports.sort();

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
        out.push_str(&format!(
            "from typing import {}  # noqa: F401\n",
            typing_names.join(", ")
        ));
    }
    // Runtime imports for unit enums — needed both for monkey-patching aliases and for
    // users who import from options.py (e.g. `from html_to_markdown.options import NewlineStyle`).
    if !runtime_native_imports.is_empty() {
        out.push_str(&format!("from .{module_name} import (\n"));
        for name in &runtime_native_imports {
            out.push_str(&format!("    {},\n", name));
        }
        out.push_str(")\n");
    }
    out.push('\n');
    // Import non-enum native-module types for static analysis only (TYPE_CHECKING guard).
    if !type_checking_only_imports.is_empty() {
        out.push_str("if TYPE_CHECKING:\n");
        out.push_str(&format!("    from .{module_name} import (\n"));
        for name in &type_checking_only_imports {
            out.push_str(&format!("        {},\n", name));
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

    // Unit enums (needed_enums) live as #[pyclass] in the native module under their Rust
    // variant names (PascalCase, e.g. NewlineStyle.Backslash). Emit SCREAMING_SNAKE_CASE
    // aliases on the native class so callers can use NewlineStyle.BACKSLASH as they would
    // with a regular Python (str, Enum). The values are the native enum instances, so
    // passing them to native-typed constructor parameters works without a TypeError.
    let mut sorted_needed_enums: Vec<&String> = needed_enums.iter().collect();
    sorted_needed_enums.sort();
    for enum_name in &sorted_needed_enums {
        if let Some(enum_def) = api.enums.iter().find(|e| &e.name == *enum_name) {
            if data_enum_names.contains(enum_def.name.as_str()) {
                continue;
            }
            // Add SCREAMING_SNAKE_CASE class-level attributes to the native pyclass.
            // When the Rust variant name is a Python keyword (e.g. `None`), bare
            // attribute access `HighlightStyle.None` is a SyntaxError, so use
            // `setattr` + `getattr` to bridge the keyword.
            for variant in &enum_def.variants {
                let rust_name = &variant.name;
                let py_name = to_python_screaming(rust_name);
                // PyO3 escapes Python-keyword variant names by appending `_`
                // (matching `python_safe_name`), so `None` becomes `None_` on
                // the runtime class. We must `getattr` from the escaped form.
                let runtime_name = alef_core::keywords::python_ident(rust_name);
                let needs_setattr = runtime_name.as_str() != rust_name.as_str()
                    || alef_core::keywords::PYTHON_KEYWORDS.contains(&py_name.as_str());
                if needs_setattr {
                    out.push_str(&format!(
                        "setattr({}, \"{}\", getattr({}, \"{}\"))\n",
                        enum_def.name, py_name, enum_def.name, runtime_name
                    ));
                } else {
                    out.push_str(&format!(
                        "{}.{} = {}.{}\n",
                        enum_def.name, py_name, enum_def.name, rust_name
                    ));
                }
            }
            out.push('\n');
        }
    }

    for enum_def in &api.enums {
        // Unit enums are handled above — do not emit a duplicate (str, Enum) subclass.
        if needed_enums.contains(&enum_def.name) {
            continue;
        }
        // Data enums are dict-accepting structs on the Rust side; skip str,Enum generation.
        if data_enum_names.contains(enum_def.name.as_str()) {
            continue;
        }
        out.push_str(&format!("class {}(str, Enum):\n", enum_def.name));
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
        out.push_str(&format!("    \"\"\"{enum_doc}\"\"\"\n\n"));
        for variant in &enum_def.variants {
            let value = variant.name.to_snake_case();
            out.push_str(&format!("    {} = \"{}\"\n", to_python_screaming(&variant.name), value));
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
        let use_typeddict = output_style == PythonDtoStyle::TypedDict && typ.is_return_type;

        // Return types are defined authoritatively by the Rust native module as #[pyclass]
        // structs. Emitting a @dataclass with the same name creates a shadow class that breaks
        // static analysis — Pylance reports a type mismatch because the @dataclass and the
        // native PyO3 class are unrelated types even though they share a name.
        // Only emit a TypedDict when explicitly configured; otherwise skip entirely.
        if typ.is_return_type && !use_typeddict {
            continue;
        }

        if use_typeddict {
            out.push_str(&gen_typeddict(typ, &enum_names, &data_enum_names));
        } else {
            out.push_str("@dataclass\n");
            out.push_str(&format!("class {}:\n", typ.name));
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
            out.push_str(&format!("    \"\"\"{class_doc}\"\"\"\n\n"));

            if typ.fields.is_empty() {
                out.push_str("    pass\n\n");
                continue;
            }

            for field in &typ.fields {
                // Determine Python type hint
                let type_hint = python_field_type(
                    &field.ty,
                    field.optional,
                    &enum_names,
                    &data_enum_names,
                    EmitContext::OptionsModule,
                );

                // Determine default value and check if we need | None
                let (type_hint_with_none, default) = if let Some(td) = &field.typed_default {
                    // For optional fields with Empty default, use None — not a zero value.
                    // This ensures Option<usize> defaults to None (not 0), preventing
                    // "max_concurrent must be > 0" validation errors.
                    let default = if field.optional && matches!(td, alef_core::ir::DefaultValue::Empty) {
                        "None".to_string()
                    } else {
                        typed_default_to_python(td, &field.ty, &enum_defaults, &data_enum_names)
                    };
                    // When the effective default is None (e.g. Duration with Empty typed_default),
                    // add | None to the type hint so the annotation matches the default value.
                    let hint = if default == "None" && !type_hint.contains('|') {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (hint, default)
                } else if field.optional {
                    // If default is None but type is Named (not already Optional), add | None
                    let final_hint = if !type_hint.contains('|') && matches!(&field.ty, TypeRef::Named(_)) {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (final_hint, "None".to_string())
                } else {
                    let default = python_zero_value(&field.ty, &enum_names, &data_enum_names);
                    // When the zero value is None (e.g. data enum fields), add | None so the
                    // annotation matches — `dict[str, Any] = None` is a mypy type error.
                    let hint = if default == "None" && !type_hint.contains('|') {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    };
                    (hint, default)
                };

                let safe_name = alef_core::keywords::python_ident(&field.name);
                if !field.doc.is_empty() {
                    out.push_str(&format!("    {}: {} = {}\n", safe_name, type_hint_with_none, default));
                    let doc_line = sanitize_python_doc(&doc_first_paragraph_joined(&field.doc));
                    // Avoid `""""` when docstring ends with `"` — add trailing space.
                    let safe_doc = if doc_line.ends_with('"') {
                        format!("{doc_line} ")
                    } else {
                        doc_line
                    };
                    out.push_str(&format!("    \"\"\"{safe_doc}\"\"\"\n\n"));
                } else {
                    out.push_str(&format!("    {}: {} = {}\n", safe_name, type_hint_with_none, default));
                }
            }
            out.push('\n');
        }
    }

    // Emit union type aliases for data enums referenced by has_default types.
    // These are tagged-union enums whose variants map to Python dataclasses or primitive types.
    // Example: `Message = SystemMessage | UserMessage | AssistantMessage | ...`
    let mut needed_data_enum_aliases: Vec<&alef_core::ir::EnumDef> = api
        .enums
        .iter()
        .filter(|e| needed_enums.contains(&e.name) && data_enum_names.contains(e.name.as_str()))
        .collect();
    // Topological sort: emit enums that are referenced by other enums first.
    // A data enum that appears in another data enum's variant fields must be
    // defined before the referencing enum (e.g., ContentPart before UserContent).
    let alias_names: AHashSet<&str> = needed_data_enum_aliases.iter().map(|e| e.name.as_str()).collect();
    let refs_name = |e: &alef_core::ir::EnumDef, name: &str| -> bool {
        e.variants
            .iter()
            .any(|v| v.fields.iter().any(|f| f.ty.references_named(name)))
    };
    needed_data_enum_aliases.sort_by(|a, b| {
        let a_refs_b = refs_name(a, &b.name);
        let b_refs_a = refs_name(b, &a.name);
        if a_refs_b {
            std::cmp::Ordering::Greater
        } else if b_refs_a {
            std::cmp::Ordering::Less
        } else {
            // Stable: enums with fewer cross-references among aliases go first
            let a_deps = a
                .variants
                .iter()
                .flat_map(|v| &v.fields)
                .filter(|f| alias_names.iter().any(|n| f.ty.references_named(n)))
                .count();
            let b_deps = b
                .variants
                .iter()
                .flat_map(|v| &v.fields)
                .filter(|f| alias_names.iter().any(|n| f.ty.references_named(n)))
                .count();
            a_deps.cmp(&b_deps)
        }
    });
    for enum_def in needed_data_enum_aliases {
        let member_types: Vec<String> = enum_def
            .variants
            .iter()
            .flat_map(|v| {
                if v.fields.is_empty() {
                    // Tag-only variant with no payload: represents a string literal on the wire.
                    vec!["str".to_string()]
                } else if v.fields.len() == 1 {
                    // Single-field variant (positional or named): use the Python type of that field.
                    vec![python_field_type(
                        &v.fields[0].ty,
                        v.fields[0].optional,
                        &enum_names,
                        &data_enum_names,
                        EmitContext::OptionsModule,
                    )]
                } else {
                    // Multi-field variant: emit a Python type per field, joined as a tuple-like union.
                    v.fields
                        .iter()
                        .map(|f| python_field_type(&f.ty, f.optional, &enum_names, &data_enum_names, EmitContext::OptionsModule))
                        .collect()
                }
            })
            // Deduplicate while preserving order (e.g. two tag-only variants both map to `str`).
            .fold(Vec::<String>::new(), |mut acc, t| {
                if !acc.contains(&t) {
                    acc.push(t);
                }
                acc
            });

        if member_types.is_empty() {
            continue;
        }

        let doc = if !enum_def.doc.is_empty() {
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
        out.push_str(&format!("# {doc}\n"));

        if member_types.len() <= 3 {
            out.push_str(&format!("{} = {}\n\n", enum_def.name, member_types.join(" | ")));
        } else {
            out.push_str(&format!("{} = (\n", enum_def.name));
            for (i, ty) in member_types.iter().enumerate() {
                if i < member_types.len() - 1 {
                    out.push_str(&format!("    {} |\n", ty));
                } else {
                    out.push_str(&format!("    {}\n", ty));
                }
            }
            out.push_str(")\n\n");
        }
    }

    out
}

/// Generate a `TypedDict` class for a return type.
///
/// TypedDict is emitted with `total=False` because all fields are optional at the
/// call site — the caller may receive only a subset of keys.  Default values are
/// not supported by TypedDict, so we only emit field name + type hint.
///
/// ```python
/// class ConversionResult(TypedDict, total=False):
///     """One-line doc."""
///
///     content: str | None
///     tables: list[ExtractedTable]
/// ```
fn gen_typeddict(
    typ: &alef_core::ir::TypeDef,
    enum_names: &AHashSet<&str>,
    data_enum_names: &AHashSet<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("class {}(TypedDict, total=False):\n", typ.name));
    let typeddict_doc = if !typ.doc.is_empty() {
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
    out.push_str(&format!("    \"\"\"{typeddict_doc}\"\"\"\n\n"));
    for field in &typ.fields {
        let type_hint = python_field_type(
            &field.ty,
            field.optional,
            enum_names,
            data_enum_names,
            EmitContext::OptionsModule,
        );
        // Ensure Optional-like fields always include `| None`
        let type_hint_with_none = if field.optional && !type_hint.contains('|') {
            if matches!(&field.ty, alef_core::ir::TypeRef::Named(_)) {
                format!("{} | None", type_hint)
            } else {
                type_hint
            }
        } else {
            type_hint
        };
        let safe_name = alef_core::keywords::python_ident(&field.name);
        if !field.doc.is_empty() {
            out.push_str(&format!("    {}: {}\n", safe_name, type_hint_with_none));
            let doc_line = sanitize_python_doc(&doc_first_paragraph_joined(&field.doc));
            // A triple-quoted docstring that ends with `"` would produce `""""` (4 quotes),
            // which Python parses as an empty string followed by a stray `"`.
            // Add a trailing space to prevent the collision.
            let safe_doc = if doc_line.ends_with('"') {
                format!("{doc_line} ")
            } else {
                doc_line
            };
            out.push_str(&format!("    \"\"\"{safe_doc}\"\"\"\n\n"));
        } else {
            out.push_str(&format!("    {}: {}\n", safe_name, type_hint_with_none));
        }
    }
    out.push('\n');
    out
}

pub(super) fn python_field_type(
    ty: &alef_core::ir::TypeRef,
    optional: bool,
    enum_names: &AHashSet<&str>,
    data_enum_names: &AHashSet<&str>,
    context: EmitContext,
) -> String {
    use alef_core::ir::TypeRef;
    let base = match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "bool".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "float".to_string(),
            _ => "int".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "str".to_string(),
        TypeRef::Bytes => "bytes".to_string(),
        TypeRef::Vec(inner) => {
            format!(
                "list[{}]",
                python_field_type(inner, false, enum_names, data_enum_names, context)
            )
        }
        TypeRef::Map(k, v) => format!(
            "dict[{}, {}]",
            python_field_type(k, false, enum_names, data_enum_names, context),
            python_field_type(v, false, enum_names, data_enum_names, context)
        ),
        TypeRef::Named(name) if data_enum_names.contains(name.as_str()) => match context {
            // In options.py the data-enum type is defined locally as a union alias; use
            // the bare name so it resolves to that alias rather than a native import.
            EmitContext::OptionsModule => name.clone(),
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
                python_field_type(inner, false, enum_names, data_enum_names, context)
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
    td: &alef_core::ir::DefaultValue,
    ty: &alef_core::ir::TypeRef,
    enum_defaults: &AHashMap<String, String>,
    data_enum_names: &AHashSet<&str>,
) -> String {
    use alef_core::ir::{DefaultValue, TypeRef};
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
                    alef_core::ir::PrimitiveType::Bool => "False".to_string(),
                    alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
                    _ => "0".to_string(),
                },
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"".to_string(),
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
    ty: &alef_core::ir::TypeRef,
    enum_names: &AHashSet<&str>,
    data_enum_names: &AHashSet<&str>,
) -> String {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "False".to_string(),
            alef_core::ir::PrimitiveType::F32 | alef_core::ir::PrimitiveType::F64 => "0.0".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "\"\"".to_string(),
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
fn type_contains_json(ty: &alef_core::ir::TypeRef) -> bool {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Json => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_contains_json(inner),
        TypeRef::Map(k, v) => type_contains_json(k) || type_contains_json(v),
        _ => false,
    }
}

/// Recursively collect all Named type references from a TypeRef.
pub(super) fn collect_named_types(ty: &alef_core::ir::TypeRef, out: &mut AHashSet<String>) {
    use alef_core::ir::TypeRef;
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
    ty: &alef_core::ir::TypeRef,
    enum_names: &AHashSet<&str>,
    out: &mut AHashSet<String>,
) {
    use alef_core::ir::TypeRef;
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
mod tests {
    use super::{EmitContext, python_field_type};
    use ahash::AHashSet;
    use alef_core::ir::{PrimitiveType, TypeRef};

    fn make_sets<'a>(enum_names: &[&'a str], data_enum_names: &[&'a str]) -> (AHashSet<&'a str>, AHashSet<&'a str>) {
        let enums: AHashSet<&'a str> = enum_names.iter().copied().collect();
        let data_enums: AHashSet<&'a str> = data_enum_names.iter().copied().collect();
        (enums, data_enums)
    }

    /// `Map<String, Named("ExtractionPattern")>` in OptionsModule context resolves to the
    /// locally defined union type alias — bare, unqualified name.
    #[test]
    fn test_map_named_data_enum_options_module() {
        let (enum_names, data_enum_names) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"]);
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("ExtractionPattern".to_string())),
        );
        let result = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::OptionsModule);
        assert_eq!(result, "dict[str, ExtractionPattern]");
    }

    /// `Map<String, Named("ExtractionPattern")>` in NativeStub context resolves to the
    /// native PyO3 class — also bare name (no `_native.` prefix needed in a .pyi file that
    /// IS the native module).
    #[test]
    fn test_map_named_data_enum_native_stub() {
        let (enum_names, data_enum_names) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"]);
        let ty = TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("ExtractionPattern".to_string())),
        );
        let result = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::NativeStub);
        assert_eq!(result, "dict[str, ExtractionPattern]");
    }

    /// `Vec<Named("Message")>` in OptionsModule context uses the bare union-alias name.
    #[test]
    fn test_vec_named_data_enum_options_module() {
        let (enum_names, data_enum_names) = make_sets(&["Message"], &["Message"]);
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string())));
        let result = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::OptionsModule);
        assert_eq!(result, "list[Message]");
    }

    /// `Vec<Named("Message")>` in NativeStub context uses the bare native-class name.
    #[test]
    fn test_vec_named_data_enum_native_stub() {
        let (enum_names, data_enum_names) = make_sets(&["Message"], &["Message"]);
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Message".to_string())));
        let result = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::NativeStub);
        assert_eq!(result, "list[Message]");
    }

    /// `Optional<Named("ExtractionPattern")>` in OptionsModule context appends `| None`.
    #[test]
    fn test_optional_named_data_enum_options_module() {
        let (enum_names, data_enum_names) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"]);
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("ExtractionPattern".to_string())));
        let result = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::OptionsModule);
        assert_eq!(result, "ExtractionPattern | None");
    }

    /// `Optional<Named("ExtractionPattern")>` in NativeStub context appends `| None`.
    #[test]
    fn test_optional_named_data_enum_native_stub() {
        let (enum_names, data_enum_names) = make_sets(&["ExtractionPattern"], &["ExtractionPattern"]);
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("ExtractionPattern".to_string())));
        let result = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::NativeStub);
        assert_eq!(result, "ExtractionPattern | None");
    }

    /// Plain (non-data) enum field always uses `EnumName | str` regardless of context.
    #[test]
    fn test_plain_enum_field_both_contexts() {
        let (enum_names, data_enum_names) = make_sets(&["HeadingStyle"], &[]);
        let ty = TypeRef::Named("HeadingStyle".to_string());
        let options = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::OptionsModule);
        let native = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::NativeStub);
        assert_eq!(options, "HeadingStyle | str");
        assert_eq!(native, "HeadingStyle | str");
    }

    /// Primitive types are unaffected by context.
    #[test]
    fn test_primitive_unaffected_by_context() {
        let (enum_names, data_enum_names) = make_sets(&[], &[]);
        let ty = TypeRef::Primitive(PrimitiveType::Bool);
        let options = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::OptionsModule);
        let native = python_field_type(&ty, false, &enum_names, &data_enum_names, EmitContext::NativeStub);
        assert_eq!(options, "bool");
        assert_eq!(native, "bool");
    }
}
