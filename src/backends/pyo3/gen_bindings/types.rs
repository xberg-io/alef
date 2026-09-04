//! Python type generation: `options.py` (enums and dataclasses) and helpers.

use crate::codegen::doc_emission::doc_first_paragraph_joined;
use crate::codegen::generators;
use crate::codegen::shared::binding_fields;
use crate::core::config::{DtoConfig, PythonDtoStyle, ResolvedCrateConfig, detect_serde_available, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeDef};
use ahash::{AHashMap, AHashSet};

use super::enums::{EmitContext, class_name_to_docstring, sanitize_python_doc};

/// Convert a Rust variant name to snake_case for Python enum members (PEP 8),
/// escaping any result that collides with a Python reserved keyword or `str` method name
/// (e.g. `Del` → `del_`, `Title` → `title_`).
fn to_python_enum_variant(name: &str) -> String {
    use heck::ToSnakeCase;
    crate::core::keywords::python_str_enum_ident(&name.to_snake_case())
}

/// Generate options.py — Python-side enums (StrEnum) and `@dataclass` config types.
///
/// Enum fields in dataclasses use `str` type (not enum class) so users can pass
/// plain strings like `"atx"` instead of `HeadingStyle.Atx`.
/// Default values come from `typed_default` if available, otherwise type-appropriate zeros.
///
/// When `dto.python_output_style() == TypedDict` and a type has `is_return_type = true`, it is
/// (when selected -- see [`options_return_dataclass_names`]) still emitted as a `@dataclass`, the
/// same representation every other type here uses -- never as a `TypedDict`. `options.py` no
/// longer has a `TypedDict` rendering path at all: a `TypedDict` is a plain `dict` at runtime, so
/// it cannot give the attribute access (`result.field`) the native `#[pyclass]` this type mirrors
/// already provides, the `.pyi` stub already declares, and every consumer README documents.
/// Rendering it as a `TypedDict` discarded that access for no benefit (a downstream project's
/// issue #183: `ProcessResult.chunks` raised `AttributeError`, breaking chonkie's
/// `CodeChunker._process_code` and every downstream `agno-agi/agno` CI run pinned against it).
/// ~keep
/// Names of the types `options.py` emits as `@dataclass` config DTOs: non-trait,
/// `has_default`, not a return type, not an internal `*Update` type, and not
/// re-exported as a native pyclass -- PLUS the fixed-point closure of that seed set: any other
/// eligible type that has a *required* (non-`Optional`) field whose named type is itself in the
/// set. This is the public *input* type family — the trait-callback marshalling and the Protocol
/// stubs use the same set so the type a host is handed is the type the package exports under
/// that name.
///
/// The closure step exists because `has_default` is a fact about the CORE Rust type (does it
/// derive/impl `Default`), not about whether its public Python spelling is native or a
/// dataclass. A type can be `has_default == false` purely because one of its fields is required
/// (e.g. `CaptioningConfig { llm: LlmConfig, .. }` has no sensible default `LlmConfig`), while
/// that required field's type IS in the dataclass set (`LlmConfig` has a `Default` impl and
/// stands on its own, so it gets a twin). Without the closure, `CaptioningConfig` stays native
/// and its `#[new]` demands a native `LlmConfig` -- but the public name `LlmConfig` now resolves
/// to the dataclass twin, so `CaptioningConfig(llm=LlmConfig(...))` raises `TypeError: 'LlmConfig'
/// object is not an instance of 'LlmConfig'` (xberg-io/xberg -- CaptioningConfig/LlmConfig). A
/// Python dataclass field may be required (unlike a Rust struct literal, it needs no
/// container-level default), so lacking a core `Default` impl is not a reason to withhold a
/// twin. Once `CaptioningConfig` joins the set, the generated `_to_rust_captioning_config`
/// converter (see `functions::orchestration`, which must consult this same set, not a raw
/// `has_default` filter) builds the native object by recursively converting `llm` through
/// `_to_rust_llm_config` first, so the native `#[new]` still only ever receives native
/// instances. ~keep
///
/// `pub(crate)`: `e2e::codegen::python` calls this (alongside [`options_return_dataclass_names`])
/// to know when a type's public spelling is this method-less dataclass rather than the native
/// `#[pyclass]`, before trusting [`crate::codegen::conversions::pyo3_from_json_eligible`]'s
/// verdict that the native class carries `from_json` -- that verdict says nothing about which
/// class the public name actually resolves to. ~keep
pub(crate) fn options_dataclass_type_names(
    api: &ApiSurface,
    reexported_types: &[String],
) -> std::collections::HashSet<String> {
    use crate::core::ir::TypeRef;

    let reexported: AHashSet<&str> = reexported_types.iter().map(String::as_str).collect();
    let eligible = |t: &&TypeDef| -> bool {
        !t.is_trait && !t.is_return_type && !t.name.ends_with("Update") && !reexported.contains(t.name.as_str())
    };
    let mut names: std::collections::HashSet<String> = api
        .types
        .iter()
        .filter(|t| eligible(t) && t.has_default)
        .map(|t| t.name.clone())
        .collect();

    // Fixed-point closure: a type not yet in the set joins it once it has a required field
    // whose named type already is in the set -- see the doc comment above for why `has_default`
    // alone under-counts this family. A multi-level chain (Z requires Y requires X, where only X
    // seeded the set) needs more than one pass: pass 1 can only see X and adds Y, pass 2 sees the
    // now-updated set and adds Z. Each pass snapshots candidates against the set as it stood
    // *before* that pass (collected into an owned `Vec` first, so the borrow of `names` used to
    // find candidates ends before the loop that mutates `names`) -- the repeated outer `loop`,
    // not same-pass visibility, is what makes the closure converge over the whole chain.
    loop {
        let mut grew = false;
        let candidates: Vec<String> = api
            .types
            .iter()
            .filter(|t| eligible(t) && !names.contains(&t.name))
            .filter(|t| {
                binding_fields(&t.fields).any(|f| !f.optional && matches!(&f.ty, TypeRef::Named(inner) if names.contains(inner)))
            })
            .map(|t| t.name.clone())
            .collect();
        for name in candidates {
            if names.insert(name) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    names
}

/// Names of the return types `options.py` defines itself, as public `@dataclass`es, instead of
/// leaving them to the native module's `#[pyclass]`.
///
/// Every emitter that has to decide whether a return type's public spelling lives in `.options`
/// or in the extension module asks this. `api.py` restating it as "is a return type and is not
/// config-re-exported" is how `-> _rust.<Name>` came to annotate a function whose name
/// `__init__.py` re-exports from `.options` -- two different types under one public name.
///
/// Named `..._dataclass_names`, not `..._typeddict_names`: this used to select the types
/// `options.py` renders as `TypedDict` under the `typed-dict` output style. It still selects
/// exactly the same types (this function's own logic is unchanged, and still honors
/// `reexported_types` as the per-type escape hatch to keep a specific return type native even
/// under that style -- xberg-io/alef#134), but every selected type now renders as `@dataclass`
/// like every other type `options.py` defines. See [`gen_options_py`]'s doc for why.
///
/// `pub(crate)`: see [`options_dataclass_type_names`]'s visibility note -- the two together are
/// the complete set of names `e2e::codegen::python` must treat as method-less. ~keep
pub(crate) fn options_return_dataclass_names(
    api: &ApiSurface,
    dto: &DtoConfig,
    reexported_types: &[String],
) -> std::collections::HashSet<String> {
    let output_style = dto.python_output_style();
    let reexported: AHashSet<&str> = reexported_types.iter().map(String::as_str).collect();
    api.types
        .iter()
        .filter(|t| t.is_return_type && super::errors::is_dataclass_backed_config(t, output_style, &reexported))
        .map(|t| t.name.clone())
        .collect()
}

/// Name of the `options._from_native_<snake>` converter that turns the native `#[pyclass]` a
/// function returns into the public type `options.py` publishes under the same name.
pub(in crate::backends::pyo3) fn from_native_converter_name(type_name: &str) -> String {
    use heck::ToSnakeCase;
    format!("_from_native_{}", type_name.to_snake_case())
}

pub(super) fn gen_options_py(
    api: &ApiSurface,
    module_name: &str,
    dto: &DtoConfig,
    reexported_types: &[String],
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
    // Json-typed fields used to render as `dict[str, Any]` and so pulled in `Any`. They now
    // render as `str`, so a Json field alone no longer references `Any`; keeping the
    // old clause would emit an unused `from typing import Any` and trip ruff F401 in the
    // generated stubs. Only the `from_native` converters still need `Any`.
    let needs_any = emits_from_native_converters;

    let mut referenced_types: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
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
            if typ.name.ends_with("Update") || typ.fields.is_empty() {
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
            minijinja::context! { names => typing_names },
        ));
    }
    if !runtime_native_imports.is_empty() {
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

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
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
        // existing fallback -- this only ever widens what closure types accept. ~keep
        let is_closure_only_type = !typ.has_default;
        let mut ordered_fields: Vec<&crate::core::ir::FieldDef> = binding_fields(&typ.fields).collect();
        if is_closure_only_type {
            ordered_fields.sort_by_key(|f| f.optional || f.typed_default.is_some() || f.default.is_some());
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
        out.push('\n');

        if ordered_fields.is_empty() {
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
            // `functions::converters`) -- that field DOES have a default, just not one this
            // renderer can spell as a Python literal, and it must keep the existing zero-value
            // fallback (and stay omittable), not suddenly become a required constructor argument. ~keep
            let omit_default =
                is_closure_only_type && !field.optional && field.typed_default.is_none() && field.default.is_none();

            let safe_name = crate::core::keywords::python_ident(&field.name);
            let field_declaration = if omit_default {
                crate::backends::pyo3::template_env::render(
                    "trait_bridge/dataclass_field_no_default.jinja",
                    minijinja::context! { name => &safe_name, type_hint => &type_hint },
                )
            } else {
                let default = field_defaults.literal(field);
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
                crate::backends::pyo3::template_env::render(
                    "trait_bridge/dataclass_field_with_default.jinja",
                    minijinja::context! { name => &safe_name, type_hint => &type_hint_with_none, default => &default },
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
                    minijinja::context! { text => &safe_doc },
                ));
                out.push('\n');
            } else {
                out.push_str(&field_declaration);
                out.push('\n');
            }
        }
        out.push('\n');
    }

    out.push_str(&gen_from_native_converters(api, dto, reexported_types));

    out
}

/// Emit `_from_native_<snake>(native)` module-level converters for every public `@dataclass`
/// `options.py` defines itself — both the input dataclasses and the selected return-type ones.
/// Nested public fields recurse (including through `Optional`/`Vec`/`Map` wrappers); every
/// other field passes through unchanged — enums and re-exported types keep their single
/// native identity.
///
/// A published return-type `@dataclass` needs one for the same reason an input dataclass does:
/// `api.py` publishes that name as the function's return type, but the extension module hands
/// back a `#[pyclass]`. Both shapes are constructed by keyword identically, so one template
/// serves both. ~keep
fn gen_from_native_converters(api: &ApiSurface, dto: &DtoConfig, reexported_types: &[String]) -> String {
    let mut options_types = options_dataclass_type_names(api, reexported_types);
    options_types.extend(options_return_dataclass_names(api, dto, reexported_types));
    let mut out = String::new();
    let mut emitted: Vec<&crate::core::ir::TypeDef> =
        api.types.iter().filter(|t| options_types.contains(&t.name)).collect();
    emitted.sort_by(|a, b| a.name.cmp(&b.name));

    for typ in emitted {
        let fields: Vec<minijinja::Value> = binding_fields(&typ.fields)
            .map(|f| {
                let safe_name = crate::core::keywords::python_ident(&f.name);
                let src = format!("native.{safe_name}");
                let inner_expr = from_native_field_expr(&f.ty, &options_types, &src);
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
                fn_name => from_native_converter_name(&typ.name),
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
    match ty {
        TypeRef::Named(n) if options_types.contains(n) => {
            format!("{}({src})", from_native_converter_name(n))
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
        // `Pyo3Mapper::json()` (backends/pyo3/type_map.rs) maps `TypeRef::Json` to the Rust type
        // `String`, so a Json-typed field is emitted as `String`/`Option<String>` on the pyclass
        // and `#[pyo3(get)]` hands Python a `str` holding serialized JSON — never a `dict`.
        // Annotating it `dict[str, Any]` made every generated `.pyi` lie about the runtime type
        // PyO3 0.29 has no `IntoPyObject for serde_json::Value` (serde_json is only a
        // dev-dependency there), so a real dict would require `pythonize` and an API break.
        TypeRef::Json => "str".to_string(),
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
            EmitContext::OptionsModule => {
                if str_coercible_data_enums.contains(name.as_str()) {
                    format!("{name} | str")
                } else {
                    name.clone()
                }
            }
            EmitContext::NativeStub => name.clone(),
        },
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => format!("{name} | str"),
        TypeRef::Named(name) => name.clone(),
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

/// The default literal `options.py` writes for each field of a public config type.
///
/// `gen_options_py` emits the literal; `functions::converters` asks whether it is `None` to
/// decide whether a field genuinely admits `None`. Both must read one set of enum names,
/// data-enum names and per-enum default variants, or the two answers drift: `converters`'
/// own `data_enum_names` excludes sanitized data enums while `options.py`'s does not, and a
/// field that renders `= "start"` there but resolves as nullable here is exactly how the
/// unnecessary `**({...} if x is not None else {})` unpacks reached the emitted code. ~keep
pub(in crate::backends::pyo3) struct OptionsFieldDefaults<'a> {
    enum_names: AHashSet<&'a str>,
    data_enum_names: AHashSet<&'a str>,
    enum_defaults: AHashMap<String, String>,
}

impl<'a> OptionsFieldDefaults<'a> {
    pub(in crate::backends::pyo3) fn new(api: &'a ApiSurface) -> Self {
        use heck::ToSnakeCase;
        Self {
            enum_names: api.enums.iter().map(|e| e.name.as_str()).collect(),
            data_enum_names: api
                .enums
                .iter()
                .filter(|e| generators::enum_has_data_variants(e))
                .map(|e| e.name.as_str())
                .collect(),
            // Uses the variant with is_default=true (#[default] attr), falls back to first variant.
            enum_defaults: api
                .enums
                .iter()
                .filter_map(|e| {
                    let default_v = e.variants.iter().find(|v| v.is_default).or(e.variants.first());
                    default_v.map(|v| (e.name.clone(), v.name.to_snake_case()))
                })
                .collect(),
        }
    }

    /// The Python expression `options.py` assigns as this field's default.
    pub(in crate::backends::pyo3) fn literal(&self, field: &crate::core::ir::FieldDef) -> String {
        if let Some(td) = &field.typed_default {
            if field.optional && matches!(td, crate::core::ir::DefaultValue::Empty) {
                return "None".to_string();
            }
            return typed_default_to_python(td, &field.ty, &self.enum_defaults, &self.data_enum_names);
        }
        if field.optional {
            return "None".to_string();
        }
        python_zero_value(&field.ty, &self.enum_names, &self.data_enum_names)
    }

    /// True when the emitted public field can hold `None` — the only case in which omitting a
    /// keyword argument (rather than passing it) is meaningful at the native constructor call.
    pub(in crate::backends::pyo3) fn admits_none(&self, field: &crate::core::ir::FieldDef) -> bool {
        self.literal(field) == "None"
    }
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
        DefaultValue::ListLiteral(items) => {
            let element_ty = match ty {
                TypeRef::Vec(inner) => inner.as_ref(),
                other => other,
            };
            let rendered: Vec<String> = items
                .iter()
                .map(|item| typed_default_to_python(item, element_ty, enum_defaults, data_enum_names))
                .collect();
            // A dataclass cannot carry a mutable default directly, so even a fully known list
            // goes through `default_factory` — the same mechanism the empty case already uses. ~keep
            format!("field(default_factory=lambda: [{}])", rendered.join(", "))
        }
        DefaultValue::Empty => {
            if let TypeRef::Named(name) = ty
                && data_enum_names.contains(name.as_str())
            {
                return "None".to_string();
            }
            if let TypeRef::Named(name) = ty
                && let Some(default_variant) = enum_defaults.get(name)
            {
                return format!("\"{}\"", default_variant);
            }
            match ty {
                TypeRef::Primitive(p) => match p {
                    crate::core::ir::PrimitiveType::Bool => "False".to_string(),
                    crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "0.0".to_string(),
                    _ => "0".to_string(),
                },
                TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
                TypeRef::Json => "None".to_string(),
                TypeRef::Bytes => "b\"\"".to_string(),
                TypeRef::Duration => "None".to_string(),
                TypeRef::Vec(_) => "field(default_factory=list)".to_string(),
                TypeRef::Map(_, _) => "field(default_factory=dict)".to_string(),
                _ => "None".to_string(),
            }
        }
        // alef could not read the real default out of `impl Default`. Falling through to the
        // `Empty` arm above (as this used to) renders the *type's* zero — or even guesses an
        // enum's default variant — underneath a doc comment quoting the real (unreadable) Rust
        // default: a value the source never actually specified. `None` is the only honest
        // rendering with no fabrication; the call site already widens the type hint to `T | None`
        // whenever the default is `"None"`, so this reuses that existing mechanism rather than
        // guessing a type- or enum-specific zero. ~keep
        DefaultValue::Unresolved(_) => "None".to_string(),
        // alef DID read this value — it is a resolved tuple/struct-variant enum default, not
        // `Unresolved` — but this renderer has no Python expression for "construct enum variant
        // X with these field values" the way it does for a bare `EnumVariant`. `None` is still
        // the honest answer: guessing at the variant's arguments (or falling back to the
        // enum's `#[default]` variant, which may not even be this one) would render a value
        // this field's real default may not hold. ~keep
        DefaultValue::TupleVariant(..) | DefaultValue::StructVariant(..) => "None".to_string(),
        DefaultValue::None => "None".to_string(),
        DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_) => "None".to_string(),
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
        TypeRef::Json => "None".to_string(),
        TypeRef::Bytes => "b\"\"".to_string(),
        TypeRef::Vec(_) => "field(default_factory=list)".to_string(),
        TypeRef::Map(_, _) => "field(default_factory=dict)".to_string(),
        TypeRef::Named(name) if data_enum_names.contains(name.as_str()) => "None".to_string(),
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => "\"\"".to_string(),
        TypeRef::Named(_) => "None".to_string(),
        TypeRef::Optional(_) => "None".to_string(),
        TypeRef::Unit => "None".to_string(),
        TypeRef::Duration => "None".to_string(),
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

/// Crate-wide serde availability for the pyo3 binding output, detected exactly as
/// `generate_bindings` detects it. Lets callers that don't already have `has_serde` in scope
/// (the `.pyi` stub generator, and the e2e python snippet emitter that mirrors this gate) compute
/// the identical value instead of re-deriving it. `pub(crate)` so `e2e::codegen::python` can call
/// it too — see `crate::codegen::conversions::pyo3_from_json_eligible`. ~keep
pub(crate) fn crate_has_serde(config: &ResolvedCrateConfig) -> bool {
    let output_dir = resolve_output_dir(config.output_paths.get("python"), &config.name, "crates/{name}-py/src/");
    detect_serde_available(&output_dir)
}

/// True when the pyo3 backend gives `typ` a `from_json` staticmethod. Delegates to the shared
/// [`crate::codegen::conversions::pyo3_from_json_eligible`] predicate — the single source of
/// truth also consumed by the e2e python snippet emitter — so this call site never re-derives
/// the eligibility conditions on its own. This is the predicate behind both the raw-text
/// `#[pymethods]` injection in `gen_bindings::mod` and the `def from_json` declaration
/// `gen_stubs` emits — call it from both instead of re-checking the conditions separately, so
/// the emitted method and its stub can never drift apart. ~keep
pub(in crate::backends::pyo3) fn type_has_from_json(typ: &TypeDef, api: &ApiSurface, has_serde: bool) -> bool {
    let convertible = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
    crate::codegen::conversions::pyo3_from_json_eligible(typ, has_serde, &convertible)
}

/// The attribute name `field` is published under on the generated `#[pyclass]`.
///
/// `resolve_field_name` folds two different renames together and the runtime attribute follows
/// only one of them. A configured `rename_fields` entry renames the *Rust* field, and the emitted
/// `#[pyo3(get, name = ...)]` hands the original name back to Python; a reserved-word escape has
/// nowhere else to go (`obj.global` is a `SyntaxError` however the attribute was registered), so
/// the escaped spelling is what Python sees. Anything that needs to know what a caller can
/// actually read off an instance -- the `#[pyclass]` emitter itself, and the e2e generator that
/// probes a visitor-callback context object -- must ask here rather than re-deriving the rule and
/// naming an attribute that does not exist at runtime. `pub(crate)` for the same reason
/// [`crate_has_serde`] is. ~keep
pub(crate) fn python_visible_field_name(
    config: &ResolvedCrateConfig,
    type_name: &str,
    field: &crate::core::ir::FieldDef,
) -> String {
    match config.resolve_field_name(crate::core::config::Language::Python, type_name, &field.name) {
        Some(binding_name) if crate::core::keywords::python_safe_name(&field.name).is_some() => binding_name,
        _ => field.name.clone(),
    }
}

#[cfg(test)]
mod tests;
