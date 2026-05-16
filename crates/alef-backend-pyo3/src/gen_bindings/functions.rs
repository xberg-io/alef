//! Python API wrapper function generation: `api.py`.

use ahash::{AHashMap, AHashSet};
use alef_codegen::doc_emission::doc_first_paragraph_joined;
use alef_codegen::generators;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::ApiSurface;

use super::enums::{Wrapping, sanitize_python_doc};
use super::types::collect_named_types;

/// Generate api.py — wrapper functions that convert Python types to Rust binding types.
///
/// For each function parameter whose type is a `has_default` struct (e.g. `ConversionOptions`),
/// we generate a `_to_rust_{snake_name}` converter that maps the Python `@dataclass` instance
/// to the Rust binding's pyclass by passing every field as a keyword argument.
pub(super) fn gen_api_py(
    api: &ApiSurface,
    module_name: &str,
    package_name: &str,
    trait_bridges: &[alef_core::config::TraitBridgeConfig],
    dto: &alef_core::config::DtoConfig,
    capsule_types: &std::collections::HashMap<String, alef_core::config::CapsuleTypeConfig>,
) -> String {
    use alef_core::config::PythonDtoStyle;
    use alef_core::ir::TypeRef;
    use heck::ToSnakeCase;

    // Collect bridge param names so they can be typed as `object | None` instead of
    // `str | None`. The IR sanitizes trait handle types to String, but callers pass
    // arbitrary Python objects implementing the visitor protocol.
    let bridge_param_names: ahash::AHashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    // Build lookup for options-field bridges: options_type_name → (visitor_kwarg_name, field_name, type_alias).
    // When a function parameter's type matches an options-field bridge's `options_type`, we add
    // a `visitor: {type_alias} | None = None` convenience kwarg to the Python wrapper.
    // The type_alias (e.g. "VisitorHandle") is the handle type exported from the native module.
    let options_field_bridges: AHashMap<&str, (&str, &str, Option<&str>)> = trait_bridges
        .iter()
        .filter(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField)
        .filter_map(|b| {
            let options_type = b.options_type.as_deref()?;
            let param_name = b.param_name.as_deref()?;
            let field_name = b.resolved_options_field()?;
            let type_alias = b.type_alias.as_deref();
            Some((options_type, (param_name, field_name, type_alias)))
        })
        .collect();

    // Build lookup: type_name → TypeDef for has_default types
    let default_types: AHashMap<String, &alef_core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update"))
        .map(|t| (t.name.clone(), t))
        .collect();

    // Collect enum names for conversion detection
    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    // Separate data enums (tagged unions exposed as dict-accepting structs) from simple int enums.
    // Data enums are passed through as dicts; simple enums need string→variant lookup.
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();

    // Determine which has_default types are referenced by function parameters (directly or nested)
    let mut needed_converters: Vec<String> = Vec::new();
    let mut visited: AHashSet<String> = AHashSet::new();

    fn collect_needed(
        type_name: &str,
        default_types: &AHashMap<String, &alef_core::ir::TypeDef>,
        needed: &mut Vec<String>,
        visited: &mut AHashSet<String>,
    ) {
        if !visited.insert(type_name.to_string()) {
            return;
        }
        if let Some(typ) = default_types.get(type_name) {
            // First collect nested types so they appear before the parent converter.
            // `classify_param_type` recursively unwraps Optional/Vec layers so a
            // `Vec<HasDefault>` field still discovers the leaf converter.
            for field in &typ.fields {
                if let Some((name, _)) = classify_param_type(&field.ty) {
                    if default_types.contains_key(name) {
                        collect_needed(name, default_types, needed, visited);
                    }
                }
            }
            needed.push(type_name.to_string());
        }
    }

    for func in &api.functions {
        for param in &func.params {
            // `classify_param_type` unwraps Optional/Vec/Optional<Vec> layers
            // so a `Vec<HasDefault>` parameter still triggers converter emission
            // for the leaf type.
            if let Some((name, _)) = classify_param_type(&param.ty) {
                collect_needed(name, &default_types, &mut needed_converters, &mut visited);
            }
        }
    }

    // Collect all type names referenced in function signatures (params + returns)
    // that aren't converters — these need to be imported too.
    let mut all_type_imports: AHashSet<String> = AHashSet::new();
    for type_name in &needed_converters {
        all_type_imports.insert(type_name.clone());
    }
    for func in &api.functions {
        for param in &func.params {
            collect_named_types(&param.ty, &mut all_type_imports);
        }
        // Collect return type references so they are imported and can be used as bare
        // names in annotations. This avoids `_rust.`-prefixed return types which cause
        // type checkers to see a different type than the public re-export.
        collect_named_types(&func.return_type, &mut all_type_imports);
    }
    // Also collect type_alias names from options-field bridges so they can be used in
    // function signature annotations for visitor parameters.
    for bridge in trait_bridges {
        if let Some(alias) = &bridge.type_alias {
            all_type_imports.insert(alias.clone());
        }
    }

    // Detect whether any function or method returns a capsule type — drives whether the
    // api.py needs `cast` in typing imports (used to bridge `Any` from the native stub to
    // the public third-party return annotation, e.g. `tree_sitter.Language`).
    let needs_cast = api.functions.iter().any(|f| {
        let leaf = match &f.return_type {
            alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
            alef_core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                _ => None,
            },
            _ => None,
        };
        leaf.is_some_and(|n| capsule_types.contains_key(n))
    });

    let mut out = String::with_capacity(4096);
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str("\"\"\"Public API for conversion.\"\"\"\n\n");
    // stdlib first (isort section 1)
    let typing_imports = if needs_cast {
        "from typing import Any, TypeVar, cast"
    } else {
        "from typing import Any, TypeVar"
    };
    out.push_str(typing_imports);
    out.push_str("\n\n");
    // third-party / package self-import (isort section 3)
    out.push_str(&crate::template_env::render(
        "import_as_module.jinja",
        minijinja::context! {
            package_name => package_name,
            module_name => module_name,
        },
    ));

    // Split type imports: opaque/error types and non-options types come from the native module,
    // has_default dataclass types come from .options.
    let opaque_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let error_names: AHashSet<String> = api.errors.iter().map(|e| e.name.clone()).collect();
    // Types that exist in options.py: has_default structs that are not direct return types
    // of free functions. Update types are output-only. `t.is_return_type` is set during IR
    // extraction for types returned directly by a public free function — that's the only
    // case where a has_default type must live in the native module rather than .options.
    // Don't try to widen this with method returns or transitive field walks: a builder
    // method like `PackConfig::from_toml_file -> PackConfig` is a constructor, not evidence
    // that the type leaves through the native return surface, and falsely excluding it from
    // .options is what produced alef#72 (PackConfig/ProcessConfig imported from ._native
    // despite being dataclasses re-exported from .options).
    let options_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update") && !t.is_return_type)
        .map(|t| t.name.clone())
        .collect();
    // All non-enum IR type names (used to distinguish structs from enums in classification).
    let all_ir_type_names: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    // Enums that options.py actually exports: plain (non-data) unit enums referenced by
    // has_default struct fields. Data enums and enums not referenced by config structs live
    // in the native module, not options.py — so they must be imported from the native module.
    let options_enum_names: AHashSet<String> = {
        let mut set = AHashSet::new();
        for typ in api
            .types
            .iter()
            .filter(|t| t.has_default && !t.name.ends_with("Update"))
        {
            for field in &typ.fields {
                let inner_name = match &field.ty {
                    TypeRef::Named(n) => Some(n.as_str()),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(name) = inner_name {
                    if enum_names.contains(name) && !data_enum_names.contains(name) {
                        set.insert(name.to_string());
                    }
                }
            }
        }
        set
    };

    let all_enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let mut options_imports: Vec<&str> = Vec::new();
    let mut native_imports: Vec<&str> = Vec::new();
    for name in &all_type_imports {
        // Capsule types are not registered as #[pyclass] in the native module; skip them
        // here so api.py doesn't try `from ._native import <CapsuleType>` and crash at import.
        if capsule_types.contains_key(name) {
            continue;
        }
        let is_options = options_type_names.contains(name) || options_enum_names.contains(name);
        let is_native = !is_options
            && (opaque_names.contains(name)
                || error_names.contains(name)
                || all_ir_type_names.contains(name)
                // Enums not in options_enum_names live in the native module.
                || (all_enum_names.contains(name) && !options_enum_names.contains(name)));
        if is_native {
            native_imports.push(name.as_str());
        } else {
            options_imports.push(name.as_str());
        }
    }

    // Import types used in function signatures at runtime (not under TYPE_CHECKING)
    // since they appear as parameter/return type annotations in generated wrapper functions.
    // Sort for deterministic codegen — `all_type_imports` is an AHashSet, so iteration
    // order changes between runs; without sorting, hash-based caching always misses.
    native_imports.sort_unstable();
    options_imports.sort_unstable();
    if !native_imports.is_empty() {
        out.push_str(&crate::template_env::render(
            "import_from_module.jinja",
            minijinja::context! {
                module_name => module_name,
                imports => native_imports.join(", "),
            },
        ));
    }
    if !options_imports.is_empty() {
        out.push_str(&crate::template_env::render(
            "import_from_options.jinja",
            minijinja::context! {
                imports => options_imports.join(", "),
            },
        ));
    }
    // Capsule type imports: group by module path, emit one `from {module} import {names}` per group.
    // Capsule types (e.g. tree_sitter.Language) are not in ._native or .options; they need their
    // own first-party import so bare names in function signatures resolve (ruff F821).
    {
        use std::collections::BTreeMap;
        let mut capsule_imports: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (rust_name, cfg) in capsule_types {
            let python_type = cfg.python_type();
            if let Some((module_path, _class_name)) = python_type.rsplit_once('.') {
                capsule_imports
                    .entry(module_path.to_string())
                    .or_default()
                    .push(rust_name.clone());
            }
        }
        if !capsule_imports.is_empty() {
            for (module_path, mut names) in capsule_imports {
                names.sort_unstable();
                out.push_str(&format!("from {} import {}\n", module_path, names.join(", ")));
            }
        }
    }
    out.push('\n');

    // Emit a helper that coerces strings or PyO3 enum aliases into the
    // canonical enum class instance. PyO3 enums do not expose a `__new__`
    // that accepts strings, so the wrapper must do the lookup itself.
    out.push_str("_E = TypeVar(\"_E\")\n\n");
    out.push_str(
        "def _coerce_enum(enum_cls: type[_E], value: object) -> _E:\n    \"\"\"Coerce a string/alias value into the matching pyclass enum instance.\"\"\"\n    if isinstance(value, enum_cls):\n        return value\n    if value is None:\n        msg = f\"unknown {getattr(enum_cls, '__name__', enum_cls)!s} value: {value!r}\"\n        raise ValueError(msg)\n    s = str(value).replace(\"-\", \"_\").replace(\" \", \"_\")\n    candidates = (\n        s,\n        s.upper(),\n        s.lower(),\n        \"\".join(part.capitalize() for part in s.split(\"_\")),\n    )\n    for candidate in candidates:\n        attr = getattr(enum_cls, candidate, None)\n        if isinstance(attr, enum_cls):\n            return attr\n    msg = f\"unknown {getattr(enum_cls, '__name__', enum_cls)!s} value: {value!r}\"\n    raise ValueError(msg)\n\n\n",
    );

    // Generate converter functions for each needed has_default type
    for type_name in &needed_converters {
        let typ = default_types[type_name];
        let snake = type_name.to_snake_case();

        // `_to_rust_*` converters handle INPUT types (has_default config structs). These are
        // typed according to the INPUT style (`dto.python`), NOT the output style. Use
        // `value.get("field")` dict access only when the input style is TypedDict; otherwise
        // use `value.field` attribute access (safe for dataclasses/pydantic).
        let is_typeddict = dto.python == PythonDtoStyle::TypedDict;

        // Helper: emit `value.field` or `value.get("field")` depending on the type kind.
        let field_access = |name: &str| -> String {
            if is_typeddict {
                format!("value.get(\"{name}\")")
            } else {
                format!("value.{name}")
            }
        };

        // Check if this type has an options-field bridge (e.g. ConversionOptions.visitor).
        // If so, the converter gains a `_visitor_override: {type_alias} | None = None` param.
        let bridge_visitor_field = options_field_bridges.get(type_name.as_str()).copied();
        let bridge_visitor_type = bridge_visitor_field.and_then(|(_, _, alias)| alias).unwrap_or("object");

        // Build the converter signature.
        // When there's a visitor override param, always use multi-line form.
        if bridge_visitor_field.is_some() {
            out.push_str(&crate::template_env::render(
                "converters/signature_with_visitor.jinja",
                minijinja::context! {
                    snake => &snake,
                    type_name => type_name,
                    bridge_visitor_type => bridge_visitor_type,
                },
            ));
        } else {
            // Single-line: "def _to_rust_{snake}(value: {type_name} | None) -> _rust.{type_name} | None:"
            // Prefix "def _to_rust_" (13) + snake + "(value: " (8) + type_name + " | None) -> _rust." (18)
            // + type_name + " | None:" (8) = 47 + snake.len + 2 * type_name.len
            let sig_len = 47 + snake.len() + 2 * type_name.len();
            if sig_len > 100 {
                out.push_str(&crate::template_env::render(
                    "converters/signature_multiline.jinja",
                    minijinja::context! {
                        snake => &snake,
                        type_name => type_name,
                    },
                ));
            } else {
                out.push_str(&crate::template_env::render(
                    "converters/signature_singleline.jinja",
                    minijinja::context! {
                        snake => &snake,
                        type_name => type_name,
                    },
                ));
            }
        }
        out.push_str(&crate::template_env::render(
            "converters/docstring.jinja",
            minijinja::context! {
                type_name => type_name,
            },
        ));

        // Helper fn: extract the leaf Named type name from Named(n) or Optional(Named(n)).
        fn get_inner_name(ty: &TypeRef) -> Option<&str> {
            match ty {
                TypeRef::Named(n) => Some(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }

        // Collect the three categories of dict-coercible fields.
        let struct_coercible: Vec<_> = typ
            .fields
            .iter()
            .filter(|f| get_inner_name(&f.ty).is_some_and(|n| default_types.contains_key(n)))
            .collect();
        let simple_enum_coercible: Vec<_> = typ
            .fields
            .iter()
            .filter(|f| get_inner_name(&f.ty).is_some_and(|n| enum_names.contains(n) && !data_enum_names.contains(n)))
            .collect();
        let data_enum_coercible: Vec<_> = typ
            .fields
            .iter()
            .filter(|f| get_inner_name(&f.ty).is_some_and(|n| data_enum_names.contains(n)))
            .collect();
        let total_coercible = struct_coercible.len() + simple_enum_coercible.len() + data_enum_coercible.len();

        // When total coercible fields exceed the threshold, extract coercion into a dedicated
        // `_coerce_dict_{snake}` helper to keep `_to_rust_{snake}` under ruff's C901/PLR0912
        // complexity limit (15 branches).
        const DICT_HELPER_THRESHOLD: usize = 5;
        let use_dict_helper = total_coercible > DICT_HELPER_THRESHOLD;

        if use_dict_helper {
            // Emit `_coerce_dict_{snake}` BEFORE `_to_rust_{snake}`.
            // Insert the helper function text before the current function's docstring by
            // prepending it to a temporary buffer and then inserting into `out` just before
            // the function header we already emitted.  Because we are mid-emit, it is simpler
            // to build the helper in a separate string and splice it in before the `def` line.
            //
            // Strategy: find the last occurrence of `def _to_rust_{snake}` in `out` and
            // insert the helper immediately before it.
            let helper_marker = format!("def _to_rust_{snake}(");
            let insert_pos = out.rfind(&helper_marker).unwrap_or(out.len());

            let mut helper = String::new();
            helper.push_str(&crate::template_env::render(
                "converters/dict_coercer_header.jinja",
                minijinja::context! {
                    snake => &snake,
                    type_name => type_name,
                },
            ));
            helper.push_str(&crate::template_env::render(
                "converters/dict_coercer_docstring.jinja",
                minijinja::context! {
                    type_name => type_name,
                },
            ));

            if !struct_coercible.is_empty() {
                helper.push_str("    _struct_coercions = {\n");
                for field in &struct_coercible {
                    let nested_name = get_inner_name(&field.ty).unwrap();
                    let nested_snake = nested_name.to_snake_case();
                    helper.push_str(&crate::template_env::render(
                        "converters/struct_coercion_entry.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            nested_snake => &nested_snake,
                        },
                    ));
                }
                helper.push_str("    }\n");
                helper.push_str("    for _k, _fn in _struct_coercions.items():\n");
                helper.push_str(
                    "        if _k in value and value[_k] is not None:\n            value[_k] = _fn(value[_k])\n",
                );
            }

            if !simple_enum_coercible.is_empty() {
                helper.push_str("    _enum_coercions = {\n");
                for field in &simple_enum_coercible {
                    let enum_name = get_inner_name(&field.ty).unwrap();
                    helper.push_str(&crate::template_env::render(
                        "converters/enum_coercion_entry.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            enum_name => enum_name,
                        },
                    ));
                }
                helper.push_str("    }\n");
                helper.push_str("    for _k, _cls in _enum_coercions.items():\n");
                helper.push_str(
                    "        if _k in value and value[_k] is not None:\n            value[_k] = _coerce_enum(_cls, value[_k])\n",
                );
            }

            if !data_enum_coercible.is_empty() {
                helper.push_str("    _data_enum_coercions = {\n");
                for field in &data_enum_coercible {
                    let enum_name = get_inner_name(&field.ty).unwrap();
                    helper.push_str(&crate::template_env::render(
                        "converters/enum_coercion_entry.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            enum_name => enum_name,
                        },
                    ));
                }
                helper.push_str("    }\n");
                helper.push_str("    for _k, _cls in _data_enum_coercions.items():\n");
                helper.push_str(
                    "        if _k in value and value[_k] is not None and not isinstance(value[_k], _cls):\n            value[_k] = _cls(value[_k])\n",
                );
            }

            helper.push_str(&crate::template_env::render(
                "converters/return_coerced_type.jinja",
                minijinja::context! {
                    type_name => type_name,
                },
            ));
            out.insert_str(insert_pos, &helper);
        }

        // Allow dict input as a convenience (callers may pass a literal `{...}` instead
        // of constructing the dataclass). Coerce enum fields in the dict before constructing.
        out.push_str("    if isinstance(value, dict):\n");

        if use_dict_helper {
            // Delegate all dict coercion to the extracted helper.
            out.push_str(&crate::template_env::render(
                "converters/call_dict_helper.jinja",
                minijinja::context! {
                    snake => &snake,
                },
            ));
        } else {
            // Inline coercions for types with few coercible fields (stays within ruff C901 limit).
            let has_enum_field = !simple_enum_coercible.is_empty();
            if has_enum_field {
                for field in &simple_enum_coercible {
                    let enum_name = get_inner_name(&field.ty).unwrap();
                    out.push_str(&crate::template_env::render(
                        "converters/inline_enum_coerce.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            enum_name => enum_name,
                        },
                    ));
                }
            }
            // Also coerce nested has_default struct types in the dict before constructing the dataclass.
            if !struct_coercible.is_empty() {
                for field in &struct_coercible {
                    let nested_name = get_inner_name(&field.ty).unwrap();
                    let nested_snake = nested_name.to_snake_case();
                    out.push_str(&crate::template_env::render(
                        "converters/inline_struct_coerce.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            nested_snake => &nested_snake,
                        },
                    ));
                }
            }
            // Coerce data-enum fields: when a field's type is a data enum (e.g. `OutputFormat`)
            // the PyO3 #[pyclass] reconstruction at `{type_name}(**value)` requires the field
            // value to be an instance of that class, not a raw string/dict. Wrap any non-None,
            // non-instance value in a constructor call so `output_format="markdown"` becomes
            // `_rust.OutputFormat("markdown")`.
            if !data_enum_coercible.is_empty() {
                for field in &data_enum_coercible {
                    let enum_name = get_inner_name(&field.ty).unwrap();
                    out.push_str(&crate::template_env::render(
                        "converters/inline_data_enum_coerce.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            enum_name => enum_name,
                        },
                    ));
                }
            }
            out.push_str(&crate::template_env::render(
                "converters/construct_type.jinja",
                minijinja::context! {
                    type_name => type_name,
                },
            ));
        }
        out.push_str("    if value is None:\n");
        if let Some((kwarg_name, _field_name, _)) = bridge_visitor_field {
            // When value is None but visitor override is provided, construct a default instance.
            out.push_str(&crate::template_env::render(
                "visitor_override_none_case.jinja",
                minijinja::context! {
                    type_name => type_name,
                    kwarg_name => kwarg_name,
                },
            ));
        } else {
            out.push_str("        return None\n");
        }
        out.push_str(&crate::template_env::render(
            "converters/return_constructed.jinja",
            minijinja::context! {
                type_name => type_name,
            },
        ));

        for field in &typ.fields {
            // Check if the field's type is itself a has_default Named type (needs nested conversion)
            let inner_named = match &field.ty {
                TypeRef::Named(n) => Some(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(nested_name) = inner_named {
                if default_types.contains_key(nested_name) {
                    let nested_snake = nested_name.to_snake_case();
                    let accessor = field_access(&field.name);
                    out.push_str(&crate::template_env::render(
                        "converters/field_accessor.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            accessor => format!("_to_rust_{nested_snake}({accessor})"),
                        },
                    ));
                    continue;
                }
                // Single enum field: convert str -> Rust enum
                if enum_names.contains(&nested_name) {
                    if data_enum_names.contains(&nested_name) {
                        // Data enum (tagged union): PyO3 constructor accepts a dict directly.
                        // If the caller already holds a _rust.{EnumName} instance (e.g. from a
                        // previous conversion), pass it through to avoid a double-wrap error;
                        // otherwise wrap the dict via the PyO3 constructor.
                        let accessor = field_access(&field.name);
                        // Guard with None check when the field is optional OR when we are in
                        // TypedDict mode (where `value.get("field")` returns None for absent fields
                        // even if the IR marks the field as non-optional). Without the guard,
                        // `_rust.OutputFormat(None)` raises a TypeError.
                        let needs_none_guard =
                            matches!(&field.ty, TypeRef::Optional(_)) || field.optional || is_typeddict;
                        if needs_none_guard {
                            out.push_str(&crate::template_env::render(
                                "data_enum_dict_coerce_guard.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    accessor => &accessor,
                                    enum_name => nested_name,
                                },
                            ));
                        } else {
                            out.push_str(&crate::template_env::render(
                                "data_enum_dict_coerce_no_guard.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    accessor => &accessor,
                                    enum_name => nested_name,
                                },
                            ));
                        }
                    } else {
                        // Simple unit enum: callers pass a string/alias or a _rust.<Enum>
                        // instance. PyO3 enums do not provide a string `__init__`, so use the
                        // shared `_coerce_enum` helper to look up the canonical variant.
                        let accessor = field_access(&field.name);
                        out.push_str(&crate::template_env::render(
                            "simple_enum_dict_coerce.jinja",
                            minijinja::context! {
                                name => &field.name,
                                enum_name => nested_name,
                                accessor => &accessor,
                            },
                        ));
                    }
                    continue;
                }
            }

            // Vec<Enum> field: convert list[str] -> list[RustEnum]
            if let TypeRef::Vec(inner) = &field.ty {
                if let TypeRef::Named(enum_name) = inner.as_ref() {
                    if enum_names.contains(&enum_name.as_str()) {
                        let accessor = field_access(&field.name);
                        if data_enum_names.contains(&enum_name.as_str()) {
                            // Data enum list: each element is a dict passed to the PyO3 constructor.
                            out.push_str(&crate::template_env::render(
                                "data_enum_vec_coerce.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => enum_name.as_str(),
                                    accessor => &accessor,
                                },
                            ));
                        } else {
                            // Simple unit enum list: each element is a string/alias or a
                            // _rust.<Enum> instance — coerce via the shared helper.
                            out.push_str(&crate::template_env::render(
                                "simple_enum_vec_coerce.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => enum_name.as_str(),
                                    accessor => &accessor,
                                },
                            ));
                        }
                        continue;
                    }
                }
            }

            // Check if this field is the options-field bridge field (visitor handle).
            // When it is, use the _visitor_override if provided, else fall back to value.field.
            if let Some((kwarg_name, field_name, _)) = bridge_visitor_field {
                if field.name == field_name {
                    out.push_str(&crate::template_env::render(
                        "visitor_override_param.jinja",
                        minijinja::context! {
                            field_name => field_name,
                            accessor => field_access(field_name),
                        },
                    ));
                    let _ = kwarg_name; // used above in the None branch
                    continue;
                }
            }
            let accessor = field_access(&field.name);
            out.push_str(&crate::template_env::render(
                "field_kwarg.jinja",
                minijinja::context! {
                    name => &field.name,
                    accessor => &accessor,
                },
            ));
        }

        out.push_str("    )\n\n\n");
    }

    // Generate wrapper for each function
    for func in &api.functions {
        // Build Python-side params applying seen_optional promotion.
        //
        // Python syntax requires params with defaults to follow params without defaults.
        // The PyO3 binding uses seen_optional promotion: once any optional param appears
        // in the Rust function signature, all subsequent params also get `= None` defaults
        // (wrapped in Option<T>). The Python wrapper must mirror this so callers can omit
        // those trailing params.
        //
        // Algorithm:
        //   1. Walk params in IR order, track seen_optional.
        //   2. A param is "promoted" if it is NOT optional in the IR but seen_optional is
        //      already true (an earlier param was optional).
        //   3. Partition into truly-required (not optional, not promoted) and
        //      all-with-defaults (optional || promoted).
        //   4. Emit truly-required first, then all-with-defaults — satisfying Python syntax.
        let mut seen_optional_so_far = false;
        let mut promoted_params: ahash::AHashSet<String> = ahash::AHashSet::new();
        for param in &func.params {
            if param.optional {
                seen_optional_so_far = true;
            } else if seen_optional_so_far {
                // This param is not optional in the IR but comes after an optional param
                // → the PyO3 binding promotes it to Option<T>; the Python wrapper must too.
                promoted_params.insert(param.name.clone());
            }
        }

        let mut sig_parts = Vec::new();
        let is_with_default = |p: &&alef_core::ir::ParamDef| p.optional || promoted_params.contains(&p.name);
        let (required, optional): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !is_with_default(p));
        for param in required.iter().chain(optional.iter()) {
            // Bridge params have their IR type sanitized to String, but callers pass
            // arbitrary Python objects implementing the visitor protocol — use `object`.
            let base_type = if bridge_param_names.contains(param.name.as_str()) {
                "object".to_string()
            } else {
                crate::type_map::python_type(&param.ty)
            };
            let needs_default = param.optional || promoted_params.contains(&param.name);
            // Required params whose type is a has-default struct are treated as optional
            // at the Python wrapper level: callers may omit them and the wrapper substitutes
            // a Rust default-constructed instance (e.g. `_rust.ExtractionConfig()`).
            // This prevents panics in the PyO3 binding when `None` is passed to a
            // function whose Rust signature wraps the param in `Option<T>` but immediately
            // calls `.expect("'param' is required")`.
            let is_has_default_param = !bridge_param_names.contains(param.name.as_str()) && {
                let leaf_name = match &param.ty {
                    alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                leaf_name.is_some_and(|n| default_types.contains_key(n))
            };
            let py_type = if needs_default || is_has_default_param {
                if base_type.ends_with("| None") {
                    format!("{} = None", base_type)
                } else {
                    format!("{} | None = None", base_type)
                }
            } else {
                base_type
            };
            sig_parts.push(format!("{}: {}", param.name, py_type));
        }

        // Detect if this function has an options-field bridge (visitor embedded in options).
        // When it does, add a convenience `visitor: {type_alias} | None = None` kwarg.
        // We track: (options_param_name, options_type_name, visitor_kwarg_name, type_alias).
        let options_field_visitor_kwarg: Option<(&str, &str, &str, Option<&str>)> = func.params.iter().find_map(|p| {
            let type_name = match &p.ty {
                alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                alef_core::ir::TypeRef::Optional(inner) => {
                    if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            }?;
            let (kwarg_name, _field_name, type_alias) = options_field_bridges.get(type_name)?;
            Some((p.name.as_str(), type_name, *kwarg_name, *type_alias))
        });
        if let Some((_, _, kwarg_name, type_alias)) = options_field_visitor_kwarg {
            let visitor_type = type_alias.unwrap_or("object");
            sig_parts.push(format!("{kwarg_name}: {visitor_type} | None = None"));
        }

        let return_type_str = crate::type_map::python_type(&func.return_type);
        // Async pyo3 functions return a coroutine — the Python wrapper must be `async def`
        // so that `result = await fn(...)` works correctly and type checkers see the right type.
        let def_keyword = if func.is_async { "async def" } else { "def" };
        let has_builtin_param = sig_parts
            .iter()
            .any(|p| crate::gen_stubs::is_python_builtin_name(p.split(':').next().unwrap_or("").trim()));
        let single_line = format!(
            "{def_keyword} {}({}) -> {}:\n",
            func.name,
            sig_parts.join(", "),
            return_type_str
        );
        if single_line.len() <= 100 && !has_builtin_param {
            out.push_str(&crate::template_env::render(
                "function_signature_single_line.jinja",
                minijinja::context! {
                    def_keyword => def_keyword,
                    name => &func.name,
                    params => sig_parts.join(", "),
                    return_type => &return_type_str,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "function_signature_multiline_start.jinja",
                minijinja::context! {
                    def_keyword => def_keyword,
                    name => &func.name,
                },
            ));
            for param in &sig_parts {
                let name = param.split(':').next().unwrap_or("").trim();
                if crate::gen_stubs::is_python_builtin_name(name) {
                    out.push_str(&crate::template_env::render(
                        "function_signature_multiline_param_noqa.jinja",
                        minijinja::context! { param => param },
                    ));
                } else {
                    out.push_str(&crate::template_env::render(
                        "function_signature_multiline_param.jinja",
                        minijinja::context! { param => param },
                    ));
                }
            }
            out.push_str(&crate::template_env::render(
                "function_signature_multiline_end.jinja",
                minijinja::context! { return_type => &return_type_str },
            ));
        }
        {
            let doc_with_period = if !func.doc.is_empty() {
                let doc_first_para = doc_first_paragraph_joined(&func.doc);
                let doc_sanitized = sanitize_python_doc(&doc_first_para);
                // `    """..."""` is 10 chars of overhead; period may add 1 more char.
                // Limit content to 89 chars so that with a trailing period the full line stays ≤100.
                let doc_content = if doc_sanitized.len() > 89 {
                    doc_sanitized[..89].to_string()
                } else {
                    doc_sanitized
                };
                if doc_content.ends_with('.') {
                    doc_content
                } else {
                    format!("{}.", doc_content)
                }
            } else {
                use heck::ToSnakeCase;
                let snake = func.name.to_snake_case();
                let sentence = snake.replace('_', " ");
                let mut chars = sentence.chars();
                let capitalized = match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                };
                format!("{}.", capitalized)
            };
            out.push_str(&crate::template_env::render(
                "function_docstring.jinja",
                minijinja::context! { doc => &doc_with_period },
            ));
        }

        // For each param that has a converter, emit a local conversion variable.
        // Use the same required-first, optional-last order as the Python signature so that
        // positional calls to the native function match the pyo3 signature declaration.
        //
        // We classify the param's type by unwrapping `Optional`/`Vec` layers down to the
        // leaf `Named` type. The classification determines whether a scalar conversion or
        // a list-comprehension conversion is generated.
        // Each entry is (param_name, value_expr) — used to build keyword-argument calls so
        // that the generated `_rust.fn(path=path, config=_rust_config, ...)` form is
        // independent of the pyo3 signature parameter order.
        let mut call_args: Vec<(String, String)> = Vec::new();
        let (req_params, opt_params): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !is_with_default(p));
        for param in req_params.iter().chain(opt_params.iter()) {
            let class = classify_param_type(&param.ty);

            if let Some((name, wrapping)) = class {
                let pname = &param.name;
                let var = format!("_rust_{pname}");
                // A param is "optional" for the conversion guard when:
                //   - its IR type is Optional/OptionalVec, OR
                //   - the IR param itself is optional, OR
                //   - it was promoted to optional via seen_optional (comes after an optional param).
                let is_promoted = promoted_params.contains(pname.as_str());
                let optional =
                    matches!(wrapping, Wrapping::Optional | Wrapping::OptionalVec) || param.optional || is_promoted;
                let is_collection = matches!(wrapping, Wrapping::Vec | Wrapping::OptionalVec);

                // has_default struct: Python-side conversion via _to_rust_<snake>().
                if default_types.contains_key(name) {
                    let snake = name.to_snake_case();
                    // When this param is the options param of an options-field bridge, pass the
                    // visitor kwarg name as _visitor_override so the converter injects it.
                    let scalar_expr = if options_field_bridges.contains_key(name) {
                        if let Some((_, _, kwarg_name, _)) = options_field_visitor_kwarg {
                            format!("_to_rust_{snake}({pname}, _visitor_override={kwarg_name})")
                        } else {
                            format!("_to_rust_{snake}({pname})")
                        }
                    } else {
                        format!("_to_rust_{snake}({pname})")
                    };
                    if is_collection {
                        let element_expr = format!("_to_rust_{snake}(__item)");
                        let body = format!("[{element_expr} for __item in {pname}]");
                        emit_param_conversion(&mut out, &var, pname, &body, optional);
                    } else {
                        // When this param is the options param of an options-field bridge, the
                        // converter handles all None cases itself — emit an unconditional call
                        // so that `visitor=visitor` is forwarded even when `options is None`.
                        let bridge_optional = optional
                            && !(options_field_bridges.contains_key(name) && options_field_visitor_kwarg.is_some());
                        if bridge_optional {
                            // Optional has-default param: use Rust default constructor when None
                            // instead of passing None to the Rust binding (which may panic on
                            // `.expect("'config' is required")`).
                            out.push_str(&crate::template_env::render(
                                "config_conversion_ternary.jinja",
                                minijinja::context! {
                                    var => &var,
                                    body => &scalar_expr,
                                    pname => pname,
                                    name => name,
                                },
                            ));
                        } else {
                            emit_param_conversion(&mut out, &var, pname, &scalar_expr, false);
                        }
                        // Required scalar (not optional and not promoted): when the converter
                        // returns None (caller passed None for a required param), substitute the
                        // Rust default constructor instead of raising ValueError.  This lets
                        // callers omit the config argument naturally.
                        if !param.optional && !is_promoted && !is_collection {
                            out.push_str(&crate::template_env::render(
                                "config_default_on_none.jinja",
                                minijinja::context! {
                                    var => &var,
                                    name => name,
                                },
                            ));
                        }
                    }
                    call_args.push((pname.clone(), var));
                    continue;
                }
                // Data enum (tagged union): wrap with `_rust.<EnumName>(value)` if not already.
                if data_enum_names.contains(name) {
                    let scalar_expr =
                        format!("(_rust.{name}({pname}) if not isinstance({pname}, _rust.{name}) else {pname})");
                    if is_collection {
                        let element_expr =
                            format!("(_rust.{name}(__item) if not isinstance(__item, _rust.{name}) else __item)");
                        let body = format!("[{element_expr} for __item in {pname}]");
                        emit_param_conversion(&mut out, &var, pname, &body, optional);
                    } else {
                        emit_param_conversion(&mut out, &var, pname, &scalar_expr, optional);
                    }
                    call_args.push((pname.clone(), var));
                    continue;
                }
            }
            call_args.push((param.name.clone(), param.name.clone()));
        }

        // Bridge `bind_via = "options_field"`: the Rust function has an additional visitor
        // kwarg (appended by gen_bridge_field_function) that is NOT in `func.params`. The
        // python wrapper takes a convenience `visitor=` kwarg and stuffs it into options
        // via `_visitor_override`, but the Rust function body actually reads the explicit
        // kwarg — pass it through as well so the visitor handle reaches the bridge.
        if let Some((_, _, kwarg_name, _)) = options_field_visitor_kwarg {
            call_args.push((kwarg_name.to_string(), kwarg_name.to_string()));
        }

        // Use keyword arguments so the call is independent of the pyo3 signature order.
        // This ensures wrapper-side required/optional reordering doesn't misalign slots.
        let kwargs: Vec<String> = call_args.iter().map(|(k, v)| format!("{k}={v}")).collect();
        // Async pyo3 functions return a coroutine that must be awaited by the Python caller.
        let return_prefix = if func.is_async { "await " } else { "" };

        // Check if this function returns Unit (void). Void-returning functions should emit
        // a bare call without `return`.
        let is_void_return = matches!(&func.return_type, alef_core::ir::TypeRef::Unit);

        if is_void_return {
            // Emit bare call without return statement for void-returning functions
            out.push_str(&format!(
                "    {return_prefix}_rust.{}({})\n",
                &func.name,
                kwargs.join(", ")
            ));
        }
        // When the return type is a capsule type, the _native stub returns Any (the actual
        // value is a PyCapsule wrapped into the third-party type via the capsule codegen).
        // Wrap the call in `cast(ReturnType, ...)` so mypy --strict (warn_return_any) is happy
        // without weakening the public api.py annotation.
        else if {
            let returns_capsule = match &func.return_type {
                alef_core::ir::TypeRef::Named(n) => capsule_types.contains_key(n),
                alef_core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                    alef_core::ir::TypeRef::Named(n) => capsule_types.contains_key(n),
                    _ => false,
                },
                _ => false,
            };
            returns_capsule
        } {
            let cast_target = match &func.return_type {
                alef_core::ir::TypeRef::Named(n) => n.clone(),
                alef_core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                    alef_core::ir::TypeRef::Named(n) => format!("{n} | None"),
                    _ => crate::type_map::python_type(&func.return_type),
                },
                _ => crate::type_map::python_type(&func.return_type),
            };
            out.push_str(&format!(
                "    return cast(\"{cast_target}\", {return_prefix}_rust.{name}({kwargs}))\n",
                name = &func.name,
                kwargs = kwargs.join(", ")
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "function_call.jinja",
                minijinja::context! {
                    return_prefix => return_prefix,
                    name => &func.name,
                    kwargs => kwargs.join(", "),
                },
            ));
        }
        out.push_str("\n\n");
    }

    // Emit pass-through wrappers for trait-bridge registration functions.
    // These functions are emitted as #[pyfunction] in the native Rust module but are not in
    // api.functions — they must be re-exported via api.py so callers can use the public package
    // path (e.g. `kreuzberg.register_ocr_backend`) rather than `kreuzberg._kreuzberg.register_ocr_backend`.
    for register_fn in crate::trait_bridge::collect_bridge_register_fns(trait_bridges) {
        out.push_str(&crate::template_env::render(
            "bridge_register_fn.jinja",
            minijinja::context! { register_fn => &register_fn },
        ));
    }

    // Emit pass-through wrappers for trait-bridge unregistration functions.
    // These allow callers to unregister a named backend via the public package path.
    for unregister_fn in crate::trait_bridge::collect_bridge_unregister_fns(trait_bridges) {
        out.push_str(&crate::template_env::render(
            "bridge_unregister_fn.jinja",
            minijinja::context! { unregister_fn => &unregister_fn },
        ));
    }

    // Emit pass-through wrappers for trait-bridge clear functions.
    // These allow callers to clear all registered backends for a plugin type.
    for clear_fn in crate::trait_bridge::collect_bridge_clear_fns(trait_bridges) {
        out.push_str(&crate::template_env::render(
            "bridge_clear_fn.jinja",
            minijinja::context! { clear_fn => &clear_fn },
        ));
    }

    out
}
pub(super) fn classify_param_type(ty: &alef_core::ir::TypeRef) -> Option<(&str, Wrapping)> {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Named(n) => Some((n.as_str(), Wrapping::Plain)),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some((n.as_str(), Wrapping::Optional)),
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) => Some((n.as_str(), Wrapping::OptionalVec)),
                _ => None,
            },
            _ => None,
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some((n.as_str(), Wrapping::Vec)),
            _ => None,
        },
        _ => None,
    }
}

/// Emit a `{var} = {body}` line, guarded by `if {pname} is not None else None`
/// when the parameter is optional.
pub(super) fn emit_param_conversion(out: &mut String, var: &str, pname: &str, body: &str, optional: bool) {
    if optional {
        out.push_str(&crate::template_env::render(
            "param_conversion_optional.jinja",
            minijinja::context! {
                var => var,
                body => body,
                pname => pname,
            },
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "param_conversion.jinja",
            minijinja::context! {
                var => var,
                body => body,
            },
        ));
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::{classify_param_type, emit_param_conversion};
    use alef_core::ir::TypeRef;

    /// classify_param_type returns Plain for a bare Named type.
    #[test]
    fn classify_param_type_returns_plain_for_named() {
        let ty = TypeRef::Named("Foo".to_string());
        let result = classify_param_type(&ty);
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "Foo");
    }

    /// classify_param_type returns None for a primitive type.
    #[test]
    fn classify_param_type_returns_none_for_primitive() {
        let ty = TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool);
        assert!(classify_param_type(&ty).is_none());
    }

    /// emit_param_conversion emits a guarded None check when optional.
    #[test]
    fn emit_param_conversion_guards_optional() {
        let mut out = String::new();
        emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", true);
        assert!(out.contains("if x is not None else None"));
    }

    /// emit_param_conversion emits a direct assignment when not optional.
    #[test]
    fn emit_param_conversion_direct_when_required() {
        let mut out = String::new();
        emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", false);
        assert!(!out.contains("if x is not None"));
        assert!(out.contains("_rust_x = convert(x)"));
    }
}
