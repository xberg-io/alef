//! Python options dataclass rendering, split out of types.rs.

use super::*;

pub(in crate::backends::pyo3::gen_bindings) fn gen_options_py(
    api: &ApiSurface,
    module_name: &str,
    dto: &DtoConfig,
    reexported_types: &[String],
    has_serde: bool,
) -> String {
    use crate::core::ir::TypeRef;

    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();
    let str_coercible_data_enums: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| data_enum_names.contains(e.name.as_str()) && e.variants.iter().any(|v| v.fields.is_empty()))
        .map(|e| e.name.as_str())
        .collect();

    let output_style = dto.python_output_style();
    let published_return_type_names = options_return_dataclass_names(api, dto, reexported_types);
    // Every `options.py`-published input type -- both the `has_default` seed and the closure
    // extension (a native type with no `Default` of its own, pulled in only because a required
    // field of it is itself in this set; see `options_dataclass_type_names`'s doc). The several
    // inline `typ.has_default` checks below each independently decided "is this type's body
    // rendered/imported/local here", so each is widened to also accept closure membership --
    // never narrowed, so every case that already worked when `has_default` alone was checked
    // keeps working identically. ~keep
    let dataclass_names = options_dataclass_type_names(api, reexported_types);

    // Must track `gen_from_native_converters`' own set exactly: a converter emitted for a
    // published return-type `@dataclass` annotates its parameter `Any` just like an input
    // dataclass one does, and an `Any` used without its import is a NameError in the file a
    // consumer installs. ~keep
    let emits_from_native_converters = {
        let mut options_types = dataclass_names.clone();
        options_types.extend(published_return_type_names.iter().cloned());
        api.types.iter().any(|t| options_types.contains(&t.name))
    };
    // A dataclass twin needs the native module imported by name (not just the specific
    // native symbols `runtime_native_imports` already pulls in) whenever at least one
    // `options.py`-published type gets a delegating `from_json` staticmethod below -- the
    // method body calls `{module_name}.{ClassName}.from_json(...)` directly. Scoped to the
    // exact same type set `gen_from_native_converters` emits `_from_native_*` for, so this
    // never imports a module nothing in the file ends up using. ~keep
    let emits_from_json = {
        let mut options_types = dataclass_names.clone();
        options_types.extend(published_return_type_names.iter().cloned());
        api.types
            .iter()
            .any(|t| options_types.contains(&t.name) && type_has_from_json(t, api, has_serde))
    };
    // Json-typed fields used to render as `dict[str, Any]` and so pulled in `Any`. They now
    // render as `str`, so a Json field alone no longer references `Any`; keeping the
    // old clause would emit an unused `from typing import Any` and trip ruff F401 in the
    // generated stubs. Only the `from_native` converters still need `Any`.
    let needs_any = emits_from_native_converters;

    let mut referenced_types: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if reexported_types.contains(&typ.name) {
            continue;
        }
        if (typ.has_default || dataclass_names.contains(&typ.name)) && !typ.name.ends_with("Update") {
            let is_emitted = !typ.is_return_type || output_style == PythonDtoStyle::TypedDict;
            if !is_emitted {
                continue;
            }
            for field in binding_fields(&typ.fields) {
                collect_named_types(&field.ty, &mut referenced_types);
            }
        }
    }

    let mut needed_enums: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.has_default || typ.is_return_type || dataclass_names.contains(&typ.name) {
            for field in binding_fields(&typ.fields) {
                collect_named_types_filtered(&field.ty, &enum_names, &mut needed_enums);
            }
        }
    }

    let enum_defs_by_name: AHashMap<&str, &crate::core::ir::EnumDef> =
        api.enums.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut changed = true;
    while changed {
        changed = false;
        let current: Vec<String> = needed_enums.iter().cloned().collect();
        for name in current {
            if let Some(enum_def) = enum_defs_by_name.get(name.as_str())
                && generators::enum_has_data_variants(enum_def)
            {
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

    // Unit enums (needed_enums) are defined as #[pyclass] in the native module and imported
    let local_type_names: AHashSet<&str> = {
        let mut local = AHashSet::new();
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            if typ.name.ends_with("Update") || typ.fields.is_empty() || reexported_types.contains(&typ.name) {
                continue;
            }
            if (typ.has_default || dataclass_names.contains(&typ.name)) && !typ.is_return_type {
                local.insert(typ.name.as_str());
            }
            if published_return_type_names.contains(&typ.name) {
                local.insert(typ.name.as_str());
            }
        }
        local
    };
    let mut native_type_imports: Vec<String> = referenced_types
        .iter()
        .filter(|n| !local_type_names.contains(n.as_str()))
        .cloned()
        .collect();
    native_type_imports.sort();

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
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("from dataclasses import dataclass, field\n");
    let has_non_needed_str_enums = api
        .enums
        .iter()
        .any(|e| !needed_enums.contains(&e.name) && !data_enum_names.contains(e.name.as_str()));
    if has_non_needed_str_enums {
        out.push_str("from enum import Enum\n");
    }
    let needs_type_checking = !type_checking_only_imports.is_empty();
    // `options.py` never renders a literal `TypedDict` class anymore (see `gen_options_py`'s doc),
    // so nothing here ever needs to import the name.
    let needs_typing_import = needs_type_checking || needs_any;
    if needs_typing_import {
        let mut typing_names = Vec::new();
        if needs_type_checking {
            typing_names.push("TYPE_CHECKING");
        }
        if needs_any {
            typing_names.push("Any");
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "typing_import.jinja",
            crate::alef_context! { names => typing_names },
        ));
    }
    if !runtime_native_imports.is_empty() {
        out.push('\n');
        out.push_str(&crate::backends::pyo3::template_env::render(
            "import_from_module_header.jinja",
            crate::alef_context! { module_name => module_name },
        ));
        for name in &runtime_native_imports {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "import_item.jinja",
                crate::alef_context! { name => name },
            ));
        }
        out.push_str(")\n");
    }
    if emits_from_json {
        out.push_str(&crate::backends::pyo3::template_env::render(
            "import_module_relative.jinja",
            crate::alef_context! { module_name => module_name },
        ));
    }
    out.push('\n');
    if !type_checking_only_imports.is_empty() {
        out.push_str("if TYPE_CHECKING:\n");
        out.push_str(&crate::backends::pyo3::template_env::render(
            "type_checking_import_header.jinja",
            crate::alef_context! { module_name => module_name },
        ));
        for name in &type_checking_only_imports {
            out.push_str(&crate::backends::pyo3::template_env::render(
                "type_checking_import_item.jinja",
                crate::alef_context! { name => name },
            ));
        }
        out.push_str("    )\n");
    }
    out.push_str("\n\n");

    let field_defaults = OptionsFieldDefaults::new(api);

    // Unit enums (needed_enums) live as #[pyclass] in the native module. Each variant is
    // already exposed as UPPER_SNAKE_CASE via #[pyo3(name = "UPPER_SNAKE_CASE")] in the
    let mut sorted_needed_enums: Vec<&String> = needed_enums.iter().collect();
    sorted_needed_enums.sort();

    for enum_def in &api.enums {
        if needed_enums.contains(&enum_def.name) {
            continue;
        }
        if data_enum_names.contains(enum_def.name.as_str()) {
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "str_enum_class_header.jinja",
            crate::alef_context! { name => &enum_def.name },
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
            crate::alef_context! { doc => &enum_doc },
        ));
        out.push('\n');
        for variant in &enum_def.variants {
            let value = variant
                .serde_rename
                .clone()
                .unwrap_or_else(|| crate::codegen::naming::pascal_to_snake(&variant.name));
            out.push_str(&crate::backends::pyo3::template_env::render(
                "enum_variant.jinja",
                crate::alef_context! {
                    name => to_python_enum_variant(&variant.name),
                    value => &value,
                },
            ));
            out.push('\n');
        }
        out.push_str("\n\n");
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if reexported_types.contains(&typ.name) {
            continue;
        }
        if !typ.has_default && !dataclass_names.contains(&typ.name) {
            continue;
        }
        if typ.name.ends_with("Update") {
            continue;
        }

        // A closure-only type (no core `Default` impl of its own -- it is here purely because a
        // required field of it points at a type already in `dataclass_names`, e.g.
        // `CaptioningConfig { llm: LlmConfig, .. }`) cannot honestly give every field a literal
        // default the way a `has_default` type can (its own fields' defaults come from field-level
        // `typed_default`/`#[serde(default)]` only, never from a whole-struct `Default::default()`
        // fallback). Its genuinely required fields -- no `typed_default`, not `optional` -- render
        // with NO default at all rather than `OptionsFieldDefaults::literal`'s zero-value/`None`
        // fallback, which would silently let a caller omit them. Python requires every
        // no-default field to precede every defaulted field in a dataclass, so those fields are
        // moved first (a stable sort: their relative order, and the relative order of the
        // defaulted fields behind them, is otherwise untouched). `has_default` types are
        // completely unaffected by this branch and keep the original declaration order and the
        // existing fallback -- this only ever widens what closure types accept.
        //
        // A bare `#[serde(default)]` marker (`field.default == Some("/* serde(default) */")`,
        // `typed_default` unset) does NOT put this field in that "genuinely has a default"
        // category on a closure-only type: `constructors::should_option_for_nested_default`
        // requires `typ.has_default` as its very first condition, so on a closure-only type
        // (`!typ.has_default`) the native `#[new]` NEVER grants this field a default regardless
        // of the marker -- only `field.optional` does. Treating the marker as "has a default"
        // here (the old behaviour) made `OptionsFieldDefaults::literal`'s Named-type fallback
        // fabricate `None` as this field's public default even though the native constructor
        // still demands a real value, widening the type hint to `T | None` and reporting a
        // `T | None` return from the paired `_to_rust_*` converter where the native parameter is
        // `T` (e.g. `OcrPipelineConfig::quality_thresholds`, PyO3 signature `(stages,
        // quality_thresholds)` -- no default at all). `field.default.is_some()` is dropped from
        // both this sort key and `omit_default` below so the two conditions never disagree with
        // `should_option_for_nested_default`'s actual answer. ~keep
        let is_closure_only_type = !typ.has_default;
        let mut ordered_fields: Vec<&crate::core::ir::FieldDef> = binding_fields(&typ.fields).collect();
        if is_closure_only_type {
            ordered_fields.sort_by_key(|f| f.optional || f.typed_default.is_some());
        }

        // Return types are defined authoritatively by the Rust native module as #[pyclass],
        // unless this return type is one `options_return_dataclass_names` selects for
        // publication -- in which case it renders through the exact same `@dataclass` path as
        // every other type below (never `TypedDict`; see `gen_options_py`'s doc). ~keep
        if typ.is_return_type && !published_return_type_names.contains(&typ.name) {
            continue;
        }

        out.push_str("@dataclass(frozen=True, slots=True)\n");
        out.push_str(&crate::backends::pyo3::template_env::render(
            "dataclass_header.jinja",
            crate::alef_context! { name => &typ.name },
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
            crate::alef_context! { doc => &class_doc },
        ));
        out.push('\n');

        let native_delegation_py = render_native_delegation_methods(typ, api, has_serde, module_name);

        if ordered_fields.is_empty() {
            out.push_str(&native_delegation_py);
            out.push('\n');
            continue;
        }

        for field in ordered_fields.iter().copied() {
            let type_hint = python_field_type(
                &field.ty,
                field.optional,
                &enum_names,
                &data_enum_names,
                &str_coercible_data_enums,
                EmitContext::OptionsModule,
            );

            // Only a closure-only type's genuinely required fields skip the default entirely --
            // see the comment above `ordered_fields`. Checking `typed_default` alone is not
            // enough: a bare `#[serde(default)]` field (e.g. `OcrPipelineConfig::quality_thresholds`)
            // leaves `typed_default` unset but still records the wire-level defer marker in
            // `field.default` (`"/* serde(default) */"`, per `defers_to_rust_default` in
            // `functions::converters`). On a closure-only type that marker is NOT honest evidence
            // of an omittable field (see the `is_closure_only_type` doc above): the native
            // `#[new]` only grants a default when `field.optional`, never merely because of the
            // marker, so this field must render with no default at all -- exactly like a field
            // with no marker -- rather than fabricating a `None` fallback the constructor would
            // reject. ~keep
            let omit_default = is_closure_only_type && !field.optional && field.typed_default.is_none();

            let safe_name = crate::core::keywords::python_ident(&field.name);
            let field_declaration = if omit_default {
                if field.sensitive {
                    crate::backends::pyo3::template_env::render(
                        "trait_bridge/dataclass_field_with_default.jinja",
                        crate::alef_context! {
                            name => &safe_name,
                            type_hint => &type_hint,
                            default => "field(repr=False)",
                        },
                    )
                } else {
                    crate::backends::pyo3::template_env::render(
                        "trait_bridge/dataclass_field_no_default.jinja",
                        crate::alef_context! { name => &safe_name, type_hint => &type_hint },
                    )
                }
            } else {
                let mut default = field_defaults.literal(field);
                let type_hint_with_none = if field.typed_default.is_none() && field.optional {
                    if !type_hint.contains("None") && matches!(&field.ty, TypeRef::Named(_)) {
                        format!("{} | None", type_hint)
                    } else {
                        type_hint.clone()
                    }
                } else if default == "None" && !type_hint.contains("None") {
                    format!("{} | None", type_hint)
                } else {
                    type_hint.clone()
                };
                if field.sensitive {
                    default = python_repr_hidden_default(&default);
                }
                crate::backends::pyo3::template_env::render(
                    "trait_bridge/dataclass_field_with_default.jinja",
                    crate::alef_context! { name => &safe_name, type_hint => &type_hint_with_none, default => &default },
                )
            };

            if !field.doc.is_empty() {
                out.push_str(&field_declaration);
                out.push('\n');
                let doc_line = sanitize_python_doc(&doc_first_paragraph_joined(&field.doc));
                let safe_doc = if doc_line.ends_with('"') {
                    format!("{doc_line} ")
                } else {
                    doc_line
                };
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "trait_bridge/python_docstring.jinja",
                    crate::alef_context! { text => &safe_doc },
                ));
                out.push('\n');
            } else {
                out.push_str(&field_declaration);
                out.push('\n');
            }
        }
        out.push_str(&native_delegation_py);
        out.push('\n');
    }

    out.push_str(&gen_from_native_converters(api, dto, reexported_types));

    out
}

/// Add `repr=False` to a dataclass default without changing its construction semantics. ~keep
fn python_repr_hidden_default(default: &str) -> String {
    if let Some(arguments) = default.strip_prefix("field(").and_then(|value| value.strip_suffix(')')) {
        if arguments.is_empty() {
            "field(repr=False)".to_string()
        } else {
            format!("field({arguments}, repr=False)")
        }
    } else {
        format!("field(default={default}, repr=False)")
    }
}
