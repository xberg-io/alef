use crate::codegen::shared::binding_fields;
use crate::core::config::{DtoConfig, PythonDtoStyle};
use crate::core::ir::{TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;

type OptionsFieldBridges<'a> = AHashMap<&'a str, (&'a str, &'a str, Option<&'a str>)>;

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_converters(
    out: &mut String,
    needed_converters: &[String],
    default_types: &AHashMap<String, &TypeDef>,
    options_field_bridges: &OptionsFieldBridges<'_>,
    enum_names: &AHashSet<&str>,
    data_enum_names: &AHashSet<&str>,
    dto: &DtoConfig,
    reexported_types: &[String],
) {
    let output_style = dto.python_output_style();
    let reexported_names: AHashSet<&str> = reexported_types.iter().map(|s| s.as_str()).collect();
    // Emit a helper that coerces strings or PyO3 enum aliases into the
    // canonical enum class instance. PyO3 enums do not expose a `__new__`
    // that accepts strings, so the wrapper must do the lookup itself.
    out.push_str("_E = TypeVar(\"_E\")\n\n");
    out.push_str(
    "def _pascal_to_snake(value: str) -> str:\n    \"\"\"Convert PascalCase/camelCase to snake_case (AtxClosed -> atx_closed).\"\"\"\n    out_chars: list[str] = []\n    for index, ch in enumerate(value):\n        if ch.isupper() and index > 0 and (value[index - 1].islower() or (index + 1 < len(value) and value[index + 1].islower())):\n            out_chars.append(\"_\")\n        out_chars.append(ch.lower())\n    return \"\".join(out_chars)\n\n\n",
);
    out.push_str(
    "def _coerce_enum(enum_cls: type[_E], value: object) -> _E:\n    \"\"\"Coerce a string/alias value into the matching pyclass enum instance.\"\"\"\n    if isinstance(value, enum_cls):\n        return value\n    if value is None:\n        msg = f\"unknown {getattr(enum_cls, '__name__', enum_cls)!s} value: {value!r}\"\n        raise ValueError(msg)\n    s = str(value).replace(\"-\", \"_\").replace(\" \", \"_\")\n    snake = _pascal_to_snake(s)\n    candidates = (\n        s,\n        s.upper(),\n        s.lower(),\n        snake,\n        snake.upper(),\n        \"\".join(part.capitalize() for part in s.split(\"_\")),\n        \"\".join(part.capitalize() for part in snake.split(\"_\")),\n    )\n    for candidate in candidates:\n        attr = getattr(enum_cls, candidate, None)\n        if isinstance(attr, enum_cls):\n            return attr\n    msg = f\"unknown {getattr(enum_cls, '__name__', enum_cls)!s} value: {value!r}\"\n    raise ValueError(msg)\n\n\n",
);

    // Generate converter functions for each needed has_default type
    for type_name in needed_converters {
        let typ = default_types[type_name];
        let snake = type_name.to_snake_case();

        // `_to_rust_*` converters handle has_default config structs. A type is emitted as a
        // TypedDict (a dict at runtime, requiring `value.get("field")` access) only when it is a
        // return type under the structural output style and is NOT re-exported as a native
        // pyclass. This mirrors `gen_typeddict` in types.rs and the `options_type_names`
        // classification in orchestration.rs. Pure input configs (e.g. ChunkingConfig) are
        // always emitted as @dataclass, so they keep `value.field` attribute access.
        let is_typeddict = output_style == PythonDtoStyle::TypedDict
            && typ.is_return_type
            && !reexported_names.contains(type_name.as_str());
        // A reexported-native type IS the native class — the str/dict branches above already turned
        // the input into a `_rust.{type_name}` instance, and an already-native input is itself the
        // result. Reconstructing it field-by-field would call `_to_rust_*` on fields that are already
        // native (a type error). So the converter returns the value directly instead.
        let is_reexported = reexported_names.contains(type_name.as_str());

        // Helper: emit `value.field` or `value.get("field")` depending on the type kind.
        let field_access = |name: &str| -> String {
            if is_typeddict {
                format!("value.get(\"{name}\")")
            } else {
                format!("value.{name}")
            }
        };

        // Check if this type has an options-field bridge (e.g. ParseOptions.visitor).
        // If so, the converter gains a `_visitor_override: {type_alias} | None = None` param.
        let bridge_visitor_field = options_field_bridges.get(type_name.as_str()).copied();
        let bridge_visitor_type = bridge_visitor_field.and_then(|(_, _, alias)| alias).unwrap_or("object");

        // Emit @overload signatures for mypy narrowing. These allow callers to pass
        // None explicitly and have mypy narrow the return type to None, vs. passing
        // a non-None config and getting a non-None return type. Without overloads,
        // callers that do `_to_rust_process_config(config) if config is not None`
        // still see the return type as `ProcessConfig | None` (not narrowed), causing
        // mypy errors when passing the result directly.
        let has_visitor_override = bridge_visitor_field.is_some();
        // Record position before emitting overloads so the dict-helper splice can
        // insert the helper BEFORE the first @overload. Without this, the helper
        // ends up between the last @overload stub and the actual implementation,
        // which mypy rejects (overloads must be immediately followed by impl).
        let overloads_start_pos = out.len();
        out.push_str(&crate::backends::pyo3::template_env::render(
            "converters/overload_none.jinja",
            minijinja::context! {
                snake => &snake,
                type_name => type_name,
                has_visitor_override => has_visitor_override,
                bridge_visitor_type => bridge_visitor_type,
            },
        ));
        out.push_str(&crate::backends::pyo3::template_env::render(
            "converters/overload_some.jinja",
            minijinja::context! {
                snake => &snake,
                type_name => type_name,
                has_visitor_override => has_visitor_override,
                bridge_visitor_type => bridge_visitor_type,
            },
        ));

        // Build the converter signature.
        // When there's a visitor override param, always use multi-line form.
        if bridge_visitor_field.is_some() {
            out.push_str(&crate::backends::pyo3::template_env::render(
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
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "converters/signature_multiline.jinja",
                    minijinja::context! {
                        snake => &snake,
                        type_name => type_name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "converters/signature_singleline.jinja",
                    minijinja::context! {
                        snake => &snake,
                        type_name => type_name,
                    },
                ));
            }
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "converters/docstring.jinja",
            minijinja::context! {
                type_name => type_name,
            },
        ));
        out.push_str("    if isinstance(value, str):\n        value = json.loads(value)\n");

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
            // Emit `_coerce_dict_{snake}` BEFORE the `@overload` stubs for `_to_rust_{snake}`.
            //
            // Strategy: insert at `overloads_start_pos`, recorded just before the overload
            // stubs were appended. This guarantees the helper appears BEFORE the overloads
            // (so mypy still sees overloads → impl with no non-overload functions between
            // them). Previously we used `rfind("def _to_rust_X(")` which matched the LAST
            // overload stub, placing the helper between overloads and implementation —
            // mypy rejects that arrangement with `no-overload-impl`.
            let insert_pos = overloads_start_pos;

            let mut helper = String::new();
            helper.push_str(&crate::backends::pyo3::template_env::render(
                "converters/dict_coercer_header.jinja",
                minijinja::context! {
                    snake => &snake,
                    type_name => type_name,
                },
            ));
            helper.push_str(&crate::backends::pyo3::template_env::render(
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
                    helper.push_str(&crate::backends::pyo3::template_env::render(
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
                    helper.push_str(&crate::backends::pyo3::template_env::render(
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
                    helper.push_str(&crate::backends::pyo3::template_env::render(
                        "converters/enum_coercion_entry.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            enum_name => enum_name,
                        },
                    ));
                }
                helper.push_str("    }\n");
                // Distinct loop variable from the unit-enum loop above: reusing `_cls` makes a type
                // checker infer a single `type[...]` for it and flag the second dict's classes as an
                // incompatible reassignment.
                helper.push_str("    for _k, _data_cls in _data_enum_coercions.items():\n");
                helper.push_str(
                "        if _k in value and value[_k] is not None and not isinstance(value[_k], _data_cls):\n            value[_k] = _data_cls(value[_k])\n",
            );
            }

            helper.push_str(&crate::backends::pyo3::template_env::render(
                "converters/return_coerced_type.jinja",
                minijinja::context! {
                    type_name => type_name,
                    is_typeddict => is_typeddict,
                },
            ));
            out.insert_str(insert_pos, &helper);
        }

        // Allow dict input as a convenience (callers may pass a literal `{...}` instead
        // of constructing the dataclass). Coerce enum fields in the dict before constructing.
        out.push_str("    if isinstance(value, dict):\n");

        // Alias serde-renamed dict keys back to Rust field names.
        // Fixtures and config files use serde-renamed wire names (e.g., "max_chars"),
        // but the Python dataclass constructor expects Rust field names (e.g., "max_characters").
        // When a field has #[serde(rename = "...")], map the serde name back.
        let serde_renamed_fields: Vec<_> = typ
            .fields
            .iter()
            .filter_map(|f| f.serde_rename.as_ref().map(|sr| (f.name.as_str(), sr.as_str())))
            .collect();
        if !serde_renamed_fields.is_empty() {
            out.push_str("        # Alias serde-renamed keys back to Rust field names\n");
            for (field_name, serde_name) in &serde_renamed_fields {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "converters/serde_alias.jinja",
                    minijinja::context! {
                        field_name => field_name,
                        serde_name => serde_name,
                    },
                ));
            }
        }

        if use_dict_helper {
            // Delegate all dict coercion to the extracted helper.
            out.push_str(&crate::backends::pyo3::template_env::render(
                "converters/call_dict_helper.jinja",
                minijinja::context! {
                    snake => &snake,
                    is_typeddict => is_typeddict,
                },
            ));
        } else {
            // Inline coercions for types with few coercible fields (stays within ruff C901 limit).
            let has_enum_field = !simple_enum_coercible.is_empty();
            if has_enum_field {
                for field in &simple_enum_coercible {
                    let enum_name = get_inner_name(&field.ty).unwrap();
                    out.push_str(&crate::backends::pyo3::template_env::render(
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
                    out.push_str(&crate::backends::pyo3::template_env::render(
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
                    out.push_str(&crate::backends::pyo3::template_env::render(
                        "converters/inline_data_enum_coerce.jinja",
                        minijinja::context! {
                            field_name => &field.name,
                            enum_name => enum_name,
                        },
                    ));
                }
            }
            out.push_str(&crate::backends::pyo3::template_env::render(
                "converters/construct_type.jinja",
                minijinja::context! {
                    type_name => type_name,
                },
            ));
        }
        out.push_str("    if value is None:\n");
        if let Some((kwarg_name, _field_name, _)) = bridge_visitor_field {
            // When value is None but visitor override is provided, construct a default instance.
            out.push_str(&crate::backends::pyo3::template_env::render(
                "visitor_override_none_case.jinja",
                minijinja::context! {
                    type_name => type_name,
                    kwarg_name => kwarg_name,
                },
            ));
        } else {
            out.push_str("        return None\n");
        }
        // TypedDict configs (total=False) carry only the keys the caller set, so reading every
        // field with `value.get(...)` would pass `None` for omitted keys and the PyO3 constructor
        // rejects `None` for non-Optional fields. The dict branch above already coerced nested
        // structs/enums in place, so splat the present keys and let PyO3 apply its own defaults
        // for the rest. (Skip when a visitor bridge needs an explicit override kwarg.)
        if is_typeddict && bridge_visitor_field.is_none() {
            out.push_str("    value = cast(dict[str, Any], value)\n");
            out.push_str(&crate::backends::pyo3::template_env::render(
                "converters/typeddict_splat_return.jinja",
                minijinja::context! {
                    type_name => type_name,
                },
            ));
            out.push_str("\n\n");
            continue;
        }
        // Narrow `value` to the concrete dataclass for mypy. The signature accepts
        // `{type_name} | dict[str, Any] | str | None` so callers may pass JSON
        // strings or raw dicts, but by this point the str/dict branches above
        // have rebuilt `value` into a `{type_name}` instance. mypy cannot follow
        // those reassignments and would otherwise flag every `value.<field>`
        // access below as `Item "str" of "<TypeName> | str" has no attribute …`.
        // Using `cast` instead of `assert isinstance` avoids ruff S101 in
        // generated code while still narrowing for mypy at no runtime cost.
        out.push_str(&crate::backends::pyo3::template_env::render(
            "converters/cast_value.jinja",
            minijinja::context! {
                type_name => type_name,
            },
        ));
        if is_reexported {
            // `value` is already the native class — return it without field-by-field reconstruction.
            out.push_str("    return value\n\n\n");
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "converters/return_constructed.jinja",
            minijinja::context! {
                type_name => type_name,
            },
        ));

        // Skip cfg-gated fields: they are conditionally compiled out of the native `#[new]`
        // constructor (and omitted from the `.pyi` stub, which cannot express `#[cfg]`), so passing
        // them as keyword arguments would be an unknown-kwarg error. Mirrors the stub's filter.
        for field in binding_fields(&typ.fields).filter(|f| f.cfg.is_none()) {
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
                    out.push_str(&crate::backends::pyo3::template_env::render(
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
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "data_enum_dict_coerce_guard.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    accessor => &accessor,
                                    enum_name => nested_name,
                                },
                            ));
                        } else {
                            // For non-optional data enums with #[serde(default)], the user-facing
                            // dataclass may carry None (because Python users often omit it).
                            // Passing the coercion expression unconditionally to the PyO3
                            // constructor would call `_rust.<Enum>(None)` and raise TypeError.
                            // Use dict-splat to omit the kwarg when None and let PyO3's
                            // #[pyo3(signature = ...)] default apply.
                            let has_serde_default = field.default.as_deref() == Some("/* serde(default) */");
                            if has_serde_default {
                                out.push_str(&crate::backends::pyo3::template_env::render(
                                    "data_enum_dict_coerce_optional_default.jinja",
                                    minijinja::context! {
                                        name => &field.name,
                                        accessor => &accessor,
                                        enum_name => nested_name,
                                    },
                                ));
                            } else {
                                out.push_str(&crate::backends::pyo3::template_env::render(
                                    "data_enum_dict_coerce_no_guard.jinja",
                                    minijinja::context! {
                                        name => &field.name,
                                        accessor => &accessor,
                                        enum_name => nested_name,
                                    },
                                ));
                            }
                        }
                    } else {
                        // Simple unit enum: callers pass a string/alias or a _rust.<Enum>
                        // instance. PyO3 enums do not provide a string `__init__`, so use the
                        // shared `_coerce_enum` helper to look up the canonical variant.
                        let accessor = field_access(&field.name);

                        // If this enum field has #[serde(default)] and is non-optional in Rust,
                        // the Python dataclass may have it as Optional[T]. When the value is None,
                        // we must omit the kwarg so PyO3's default applies (not call _coerce_enum(None)).
                        let has_serde_default = field.default.as_deref() == Some("/* serde(default) */");
                        let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional;

                        if has_serde_default && !is_optional {
                            // Use dict-splat to omit the kwarg when None
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "simple_enum_dict_coerce_optional_default.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => nested_name,
                                    accessor => &accessor,
                                },
                            ));
                        } else {
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "simple_enum_dict_coerce.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => nested_name,
                                    accessor => &accessor,
                                },
                            ));
                        }
                    }
                    continue;
                }
            }

            // Vec<Enum> field: convert list[str] -> list[RustEnum].
            // Unwrap an optional wrapper so `Option<Vec<Enum>>` (e.g. `modalities`) is also
            // handled, and remember the optionality so the comprehension is None-guarded.
            let vec_field = match &field.ty {
                TypeRef::Vec(inner) => Some((inner, matches!(&field.ty, TypeRef::Optional(_)) || field.optional)),
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Vec(inner) => Some((inner, true)),
                    _ => None,
                },
                _ => None,
            };
            if let Some((inner, is_optional)) = vec_field {
                if let TypeRef::Named(enum_name) = inner.as_ref() {
                    if enum_names.contains(&enum_name.as_str()) {
                        let accessor = field_access(&field.name);
                        if data_enum_names.contains(&enum_name.as_str()) {
                            // Data enum list: each element is a dict passed to the PyO3 constructor.
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "data_enum_vec_coerce.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => enum_name.as_str(),
                                    accessor => &accessor,
                                    optional => is_optional,
                                },
                            ));
                        } else {
                            // Simple unit enum list: each element is a string/alias or a
                            // _rust.<Enum> instance — coerce via the shared helper.
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "simple_enum_vec_coerce.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => enum_name.as_str(),
                                    accessor => &accessor,
                                    optional => is_optional,
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
                    out.push_str(&crate::backends::pyo3::template_env::render(
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

            // For optional data enum fields, guard against None before instantiation.
            // If the field is Optional and is a data enum (tagged union), we need to check
            // for None because the PyO3 constructor will fail if passed None directly.
            let final_accessor = if let Some(inner_named) = match &field.ty {
                TypeRef::Named(n) => Some(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            } {
                if (matches!(&field.ty, TypeRef::Optional(_)) || field.optional)
                    && data_enum_names.contains(inner_named)
                {
                    // Optional data enum: guard with None check
                    format!(
                        "None if {accessor} is None else ({accessor} if isinstance({accessor}, _rust.{inner_named}) else _rust.{inner_named}({accessor}))",
                        accessor = accessor,
                        inner_named = inner_named
                    )
                } else {
                    accessor.clone()
                }
            } else {
                accessor.clone()
            };

            // When a field has serde_rename, use it for pyo3 binding compatibility.
            // The pyo3 constructor parameter names match serde-renamed field names.
            let pyo3_param_name = field.serde_rename.as_deref().unwrap_or(&field.name);

            // If this field has a #[serde(default)] and is non-optional in the binding,
            // we need to omit the kwarg when the Python value is None. Otherwise, passing
            // None explicitly causes a TypeError at the PyO3 constructor call.
            // The marker string "/* serde(default) */" indicates the field has #[serde(default)].
            // This only applies to Named types (enums, structs) that the Python backend
            // generates as Optional[T] in the dataclass even though Rust declares them non-optional.
            let has_serde_default = field.default.as_deref() == Some("/* serde(default) */");
            let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional;
            let is_named_type = matches!(field.ty, TypeRef::Named(_));

            if has_serde_default && !is_optional && is_named_type {
                // For Named fields with #[serde(default)] that are non-optional in the binding,
                // use dict-splat to conditionally omit the kwarg when the source value is None.
                // We need to check the raw field value (before conversion) to decide
                // whether to include the kwarg.
                let raw_field_accessor = field_access(&field.name);
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "field_kwarg_optional_default.jinja",
                    minijinja::context! {
                        name => pyo3_param_name,
                        raw_accessor => &raw_field_accessor,
                        final_accessor => &final_accessor,
                    },
                ));
            } else {
                // Normal kwarg rendering
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "field_kwarg.jinja",
                    minijinja::context! {
                        name => pyo3_param_name,
                        accessor => &final_accessor,
                    },
                ));
            }
        }

        out.push_str("    )\n\n\n");
    }
}
