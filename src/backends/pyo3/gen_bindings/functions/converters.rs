use crate::codegen::shared::binding_fields;
use crate::core::ir::{TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToSnakeCase;

use super::optional_kwargs::emit_optional_kwarg_helper;

type OptionsFieldBridges<'a> = AHashMap<&'a str, (&'a str, &'a str, Option<&'a str>)>;

/// True when the options dataclass may hand this field a `None` that the *native* constructor is
/// expected to replace with the Rust default, so the Python converter must pass it through
/// untouched instead of eagerly coercing it.
///
/// Two serde spellings reach that state and only one used to be recognised here. Bare
/// `#[serde(default)]` is stored as the `"/* serde(default) */"` marker in `FieldDef::default`,
/// while `#[serde(default = "path")]` is stored as `serde(default = "path")` there and as
/// `DefaultValue::FunctionCall`/`PublicFunctionCall` in `typed_default`
/// (`extract::extractor::helpers::fields`). `typed_default_to_python` renders both function-call
/// variants as the Python literal `None`, so an equality test against the marker alone left
/// exactly those fields unguarded: `_coerce_enum(_rust.Mode, None)` raises `ValueError`, and the
/// `Vec<enum>` form raises `TypeError: 'NoneType' object is not iterable` — both before the value
/// ever reaches the `#[pyo3(signature = (field=None, ...))]` constructor that would have applied
/// the real default. `codegen::config_gen::default_value_for_field` already discriminates the two
/// spellings the same way. ~keep
fn defers_to_rust_default(field: &crate::core::ir::FieldDef) -> bool {
    use crate::core::ir::DefaultValue;
    field.default.as_deref() == Some("/* serde(default) */")
        || matches!(
            field.typed_default,
            Some(DefaultValue::FunctionCall(_) | DefaultValue::PublicFunctionCall(_))
        )
}

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
    reexported_types: &[String],
    config: &crate::core::config::ResolvedCrateConfig,
    field_defaults: &crate::backends::pyo3::gen_bindings::types::OptionsFieldDefaults<'_>,
) {
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

        // `options.py` never renders a literal `TypedDict` anymore (see
        // `crate::backends::pyo3::gen_bindings::types::gen_options_py`'s doc, fixed alongside
        // tree-sitter-language-pack#183): a published return-position type is a `@dataclass`,
        // exactly like every other type here, so a value handed to `_to_rust_*` is never a plain
        // `dict` and always supports attribute access. Kept as a named `let` (not deleted
        // outright) so every branch below that used to depend on it keeps reading as the "this
        // type could be a dict" check it always was, now permanently false. ~keep
        let is_typeddict = false;
        let is_reexported = reexported_names.contains(type_name.as_str());
        let helpers_insert_pos = out.len();
        let mut optional_kwarg_helpers = String::new();

        // The same per-type `rename_fields` lookup `gen_stubs/classes.rs::gen_type_init_stub`
        // builds before calling `resolve_param_ident` -- keeping both call sites' `config_renames`
        // construction identical is what makes `resolve_param_ident` an actual single source of
        // truth for the `#[new]` keyword name, rather than three independently-derived spellings
        // that happen to agree on the common case. ~keep
        let config_renames: std::collections::HashMap<String, String> = typ
            .fields
            .iter()
            .filter_map(|f| {
                config
                    .resolve_field_name(crate::core::config::Language::Python, type_name, &f.name)
                    .map(|renamed| (f.name.clone(), renamed))
            })
            .collect();
        let config_renames_ref = if config_renames.is_empty() {
            None
        } else {
            Some(&config_renames)
        };

        // `name` is a raw Rust field name, but what is being read here is the *emitted* Python
        // attribute name, and that is already escaped (`types.rs` builds it with `python_ident`).
        // Reading `value.global` would be a SyntaxError. `python_ident` is idempotent, so
        // non-keyword names are untouched. ~keep
        let field_access = |name: &str| -> String {
            let name = crate::core::keywords::python_ident(name);
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
                            if defers_to_rust_default(field) {
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

                        // Mirrors the data-enum branch above: a field that is already `Option<T>`
                        // in the native constructor (`is_optional`) has a real `None` default at
                        // that layer (see `constructors::replace_constructor_with_serde_rename`'s
                        // `defaults` -- `f.optional` alone forces `{param}=None`), so passing
                        // `None` straight through is exactly equivalent to omitting the kwarg and
                        // needs no `**{...}` unpack at all. Only a field that is NOT optional at
                        // that layer but still carries `#[serde(default)]` needs the omission
                        // trick, because its real default there is a non-`None` Rust expression
                        // (`Self::default().{field}`) that Python cannot represent inline; passing
                        // `None` to a non-`Option` parameter would fail extraction. Collapsing
                        // both cases into one `needs_none_guard` used to route the `is_optional`
                        // case through the omission template too, which put an unnecessary
                        // `**({...} if ... else {})` in the call -- and pyrefly's keyword-argument
                        // inference cross-assigns argument types when two such unpacks appear in
                        // the same call (alef bug: two `Option<Enum>` fields on one constructor
                        // reproduced `[bad-argument-type]` with the errors swapped between the two
                        // parameters). ~keep
                        //
                        // `#[serde(default)]` alone does not make the omission trick sound either:
                        // `options.py` renders such an enum field as `= "start"` (the enum's
                        // `#[default]` variant), never `None`, so `if value.x is not None` is
                        // statically always true and the unpack buys nothing while still costing
                        // one `[bad-argument-type]` per other unpack in the same call. Only a field
                        // `options.py` actually defaults to `None` can be absent. ~keep
                        let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional;

                        if is_optional || is_typeddict {
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "simple_enum_dict_coerce_guard.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => nested_name,
                                    accessor => &accessor,
                                },
                            ));
                        } else if defers_to_rust_default(field) && field_defaults.admits_none(field) {
                            let parameter_name = crate::backends::pyo3::gen_bindings::constructors::resolve_param_ident(
                                &field.name,
                                field.serde_rename.as_ref(),
                                config_renames_ref,
                            );
                            let parameter_name = parameter_name.strip_prefix("r#").unwrap_or(&parameter_name);
                            let helper_name = emit_optional_kwarg_helper(
                                &mut optional_kwarg_helpers,
                                type_name,
                                &snake,
                                field,
                                parameter_name,
                            );
                            out.push_str(&crate::backends::pyo3::template_env::render(
                                "simple_enum_dict_coerce_optional_default.jinja",
                                minijinja::context! {
                                    name => &field.name,
                                    enum_name => nested_name,
                                    accessor => &accessor,
                                    helper_name => helper_name,
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
            if let Some((inner, is_optional)) = vec_field
                && let TypeRef::Named(enum_name) = inner.as_ref()
                && enum_names.contains(&enum_name.as_str())
            {
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
            if let Some((inner, is_optional)) = vec_field
                && let TypeRef::Named(struct_name) = inner.as_ref()
                && default_types.contains_key(struct_name.as_str())
            {
                let accessor = field_access(&field.name);
                let struct_snake = struct_name.to_snake_case();
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "struct_vec_coerce.jinja",
                    minijinja::context! {
                        name => &field.name,
                        struct_snake => &struct_snake,
                        accessor => &accessor,
                        optional => is_optional,
                    },
                ));
                continue;
            }

            if let Some((kwarg_name, field_name, _)) = bridge_visitor_field
                && field.name == field_name
            {
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

            // This is the keyword-argument name in the emitted `_rust.{Type}(...)` call, so it must
            // be exactly what the generated `#[new]` accepts. Calling `resolve_param_ident`
            // directly -- the same function `gen_stubs/classes.rs` calls for the `.pyi` `__init__`
            // stub, and the same function `constructors.rs` calls to build the real `#[new]`
            // signature -- makes this an ASK of the single source of truth instead of a
            // re-derivation that can silently drift from it. The re-derivation that used to live
            // here (`python_ident` applied straight to the wire name) agreed with
            // `resolve_param_ident` for a Python keyword that is not ALSO a Rust keyword (`global`
            // -> `global_` either way), but diverged for a name that is a keyword in BOTH
            // languages (`type`, a `#[serde(rename = "type")]` wire name): `resolve_param_ident`
            // takes the Rust `r#type` escape (which PyO3 exposes to Python as bare `type`, not
            // `type_`), so the two spellings disagreed exactly there -- `_rust.T(type_=...)` here
            // vs `_rust.T(type=...)` (and the `.pyi` stub) is what pyrefly's
            // `[unexpected-keyword]` was catching. `resolve_param_ident` can return an `r#`-escaped
            // Rust identifier; PyO3 strips that prefix from the Python-visible name, so it must be
            // stripped here too before use as a Python keyword argument (mirrors
            // `gen_stubs/classes.rs`'s stub emission). ~keep
            let pyo3_param_name = crate::backends::pyo3::gen_bindings::constructors::resolve_param_ident(
                &field.name,
                field.serde_rename.as_ref(),
                config_renames_ref,
            );
            let pyo3_param_name = pyo3_param_name
                .strip_prefix("r#")
                .map(str::to_owned)
                .unwrap_or(pyo3_param_name);

            let is_optional = matches!(field.ty, TypeRef::Optional(_)) || field.optional;

            // Which `TypeRef` shape the field has is irrelevant to whether the omission is
            // needed: the only facts that matter are that `options.py` defaults the field to
            // `None` (`admits_none`) and that the native parameter is not an `Option`
            // (`!is_optional`), so the `None` has to be withheld rather than passed. Gating this
            // on `TypeRef::Named` additionally left every other shape passing that `None`
            // straight through to a non-`Option` pyo3 parameter -- a `Vec<String>` field whose
            // Rust default is a function call reached `_rust.T(field=None)` and failed extraction
            // with `TypeError: 'None' is not an instance of 'Sequence'`, far from the dataclass
            // that produced it. `admits_none` is the single fact both this branch and the
            // `T | None` widening in `types.rs` are derived from; the gate made this branch
            // disagree with that widening for exactly the non-`Named` shapes. ~keep
            if defers_to_rust_default(field) && !is_optional && field_defaults.admits_none(field) {
                let raw_field_accessor = field_access(&field.name);
                let helper_name =
                    emit_optional_kwarg_helper(&mut optional_kwarg_helpers, type_name, &snake, field, &pyo3_param_name);
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "field_kwarg_optional_default.jinja",
                    minijinja::context! {
                        name => &pyo3_param_name,
                        raw_accessor => &raw_field_accessor,
                        final_accessor => &final_accessor,
                        helper_name => helper_name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::pyo3::template_env::render(
                    "field_kwarg.jinja",
                    minijinja::context! {
                        name => &pyo3_param_name,
                        accessor => &final_accessor,
                    },
                ));
            }
        }

        out.push_str("    )\n\n\n");
        out.insert_str(helpers_insert_pos, &optional_kwarg_helpers);
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
