use crate::codegen::shared::binding_fields;
use crate::core::config::{DtoConfig, PythonDtoStyle};
use crate::core::ir::{TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;

type OptionsFieldBridges<'a> = AHashMap<&'a str, (&'a str, &'a str, Option<&'a str>)>;

/// Check if a cfg condition is present in the pyo3 build (i.e., the field should be
/// included in the pyo3-compiled binding). This mirrors the logic in gen_stubs/classes.rs
/// to ensure the converter includes the same fields as the .pyi stub.
fn cfg_present_for_pyo3(cfg: &str) -> bool {
    let normalized: String = cfg.chars().filter(|c| !c.is_whitespace()).collect();
    if normalized == "not(target_arch=\"wasm32\")" {
        return true;
    }
    if normalized.starts_with("feature=") {
        return true;
    }
    if normalized.starts_with("any(") && normalized.ends_with(')') {
        let inner = &normalized[4..normalized.len() - 1];
        return inner
            .split(',')
            .all(|part| part.starts_with("feature=") || part == "not(target_arch=\"wasm32\")");
    }
    false
}

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
    out.push_str("_E = TypeVar(\"_E\")\n\n");
    out.push_str(
    "def _pascal_to_snake(value: str) -> str:\n    \"\"\"Convert PascalCase/camelCase to snake_case (AtxClosed -> atx_closed).\"\"\"\n    out_chars: list[str] = []\n    for index, ch in enumerate(value):\n        if ch.isupper() and index > 0 and (value[index - 1].islower() or (index + 1 < len(value) and value[index + 1].islower())):\n            out_chars.append(\"_\")\n        out_chars.append(ch.lower())\n    return \"\".join(out_chars)\n\n\n",
);
    out.push_str(
    "def _coerce_enum(enum_cls: type[_E], value: object) -> _E:\n    \"\"\"Coerce a string/alias value into the matching pyclass enum instance.\"\"\"\n    if isinstance(value, enum_cls):\n        return value\n    if value is None:\n        msg = f\"unknown {getattr(enum_cls, '__name__', enum_cls)!s} value: {value!r}\"\n        raise ValueError(msg)\n    s = str(value).replace(\"-\", \"_\").replace(\" \", \"_\")\n    snake = _pascal_to_snake(s)\n    candidates = (\n        s,\n        s.upper(),\n        s.lower(),\n        snake,\n        snake.upper(),\n        \"\".join(part.capitalize() for part in s.split(\"_\")),\n        \"\".join(part.capitalize() for part in snake.split(\"_\")),\n    )\n    for candidate in candidates:\n        attr = getattr(enum_cls, candidate, None)\n        if isinstance(attr, enum_cls):\n            return attr\n    msg = f\"unknown {getattr(enum_cls, '__name__', enum_cls)!s} value: {value!r}\"\n    raise ValueError(msg)\n\n\n",
);

    for type_name in needed_converters {
        let typ = default_types[type_name];
        let snake = type_name.to_snake_case();

        let is_typeddict = output_style == PythonDtoStyle::TypedDict
            && typ.is_return_type
            && !reexported_names.contains(type_name.as_str());
        let is_reexported = reexported_names.contains(type_name.as_str());

        let field_access = |name: &str| -> String {
            if is_typeddict {
                format!("value.get(\"{name}\")")
            } else {
                format!("value.{name}")
            }
        };

        let bridge_visitor_field = options_field_bridges.get(type_name.as_str()).copied();
        let bridge_visitor_type = bridge_visitor_field.and_then(|(_, _, alias)| alias).unwrap_or("object");

        let has_visitor_override = bridge_visitor_field.is_some();
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

        const DICT_HELPER_THRESHOLD: usize = 5;
        let use_dict_helper = total_coercible > DICT_HELPER_THRESHOLD;

        if use_dict_helper {
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

        out.push_str("    if isinstance(value, dict):\n");

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
            out.push_str(&crate::backends::pyo3::template_env::render(
                "converters/call_dict_helper.jinja",
                minijinja::context! {
                    snake => &snake,
                    is_typeddict => is_typeddict,
                },
            ));
        } else {
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
            // the PyO3 #[pyclass] reconstruction at `{type_name}(**value)` requires the field
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
        out.push_str(&crate::backends::pyo3::template_env::render(
            "converters/cast_value.jinja",
            minijinja::context! {
                type_name => type_name,
            },
        ));
        if is_reexported {
            out.push_str("    return value\n\n\n");
            continue;
        }
        out.push_str(&crate::backends::pyo3::template_env::render(
            "converters/return_constructed.jinja",
            minijinja::context! {
                type_name => type_name,
            },
        ));

        // Include fields that are in the pyo3 `#[new]` constructor: all non-binding-excluded fields
        // #[serde(skip)] does NOT affect this — it only affects serialization, not construction.
        for field in binding_fields(&typ.fields).filter(|f| f.cfg.as_deref().is_none_or(cfg_present_for_pyo3)) {
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
                if enum_names.contains(&nested_name) {
                    if data_enum_names.contains(&nested_name) {
                        let accessor = field_access(&field.name);
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
                        let accessor = field_access(&field.name);

                        // If this enum field is optional (may be None) or has #[serde(default)]
                        let has_serde_default = field.default.as_deref() == Some("/* serde(default) */");
                        let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional;
                        let needs_none_guard = is_optional || has_serde_default;

                        if needs_none_guard {
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

            if let Some((kwarg_name, field_name, _)) = bridge_visitor_field {
                if field.name == field_name {
                    out.push_str(&crate::backends::pyo3::template_env::render(
                        "visitor_override_param.jinja",
                        minijinja::context! {
                            field_name => field_name,
                            accessor => field_access(field_name),
                        },
                    ));
                    let _ = kwarg_name;
                    continue;
                }
            }
            let accessor = field_access(&field.name);

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

            let is_json_field = matches!(field.ty, TypeRef::Json)
                || matches!(&field.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Json));
            let final_accessor = if is_json_field {
                format!(
                    "(json.dumps({final_accessor}) if isinstance({final_accessor}, (dict, list)) else {final_accessor})"
                )
            } else {
                final_accessor
            };

            let pyo3_param_name = field.serde_rename.as_deref().unwrap_or(&field.name);

            // If this field has a #[serde(default)] and is non-optional in the binding,
            // The marker string "/* serde(default) */" indicates the field has #[serde(default)].
            let has_serde_default = field.default.as_deref() == Some("/* serde(default) */");
            let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional;
            let is_named_type = matches!(field.ty, TypeRef::Named(_));

            if has_serde_default && !is_optional && is_named_type {
                // For Named fields with #[serde(default)] that are non-optional in the binding,
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

#[cfg(test)]
mod tests {
    use super::cfg_present_for_pyo3;

    #[test]
    fn cfg_present_for_pyo3_accepts_feature_gates() {
        assert!(cfg_present_for_pyo3("feature = \"my-feature\""));
        assert!(cfg_present_for_pyo3("feature=\"my-feature\""));
    }

    #[test]
    fn cfg_present_for_pyo3_accepts_non_wasm_gate() {
        assert!(cfg_present_for_pyo3("not(target_arch = \"wasm32\")"));
    }

    #[test]
    fn cfg_present_for_pyo3_accepts_any_of_feature_gates() {
        // The `crawl` field: #[cfg(any(feature = "url-ingestion", feature = "url-config-types"))].
        assert!(cfg_present_for_pyo3(
            "any(feature = \"url-ingestion\", feature = \"url-config-types\")"
        ));
    }

    #[test]
    fn cfg_present_for_pyo3_rejects_unsupported_gates() {
        assert!(!cfg_present_for_pyo3("target_os = \"windows\""));
        assert!(!cfg_present_for_pyo3("target_arch = \"x86_64\""));
        assert!(!cfg_present_for_pyo3("any(feature = \"x\", target_os = \"windows\")"));
    }
}
