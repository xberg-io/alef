use crate::core::ir::{CoreWrapper, EnumDef, FieldDef, TypeRef};

pub(super) fn emit_from_mirror_to_core_enum(out: &mut String, en: &EnumDef, source_crate_name: &str) {
    let name = &en.name;
    let core_ty = if en.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        en.rust_path.replace('-', "_")
    };

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_mirror_enum_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => en.cfg.as_deref().unwrap_or(""),
        },
    ));

    let has_cfg_variants = en.variants.iter().any(|v| v.cfg.is_some());

    for variant in &en.variants {
        let vname = &variant.name;
        let cfg = variant.cfg.as_deref();
        if let Some(condition) = cfg {
            out.push_str("            #[cfg(");
            out.push_str(condition);
            out.push_str(")]\n");
        }
        if variant.originally_had_data_fields {
            let stripped_fields: Vec<&crate::core::ir::FieldDef> =
                variant.fields.iter().filter(|f| f.binding_excluded).collect();
            if variant.is_tuple {
                let args: Vec<String> = stripped_fields
                    .iter()
                    .map(|_| "Default::default()".to_string())
                    .collect();
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_enum_stripped_tuple_to_core_arm.jinja",
                    minijinja::context! {
                        name => name.as_str(),
                        vname => vname.as_str(),
                        core_ty => core_ty.as_str(),
                        args => args.join(", "),
                    },
                ));
            } else {
                let args: Vec<String> = stripped_fields
                    .iter()
                    .map(|f| format!("{}: Default::default()", f.name))
                    .collect();
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_enum_stripped_struct_to_core_arm.jinja",
                    minijinja::context! {
                        name => name.as_str(),
                        vname => vname.as_str(),
                        core_ty => core_ty.as_str(),
                        args => args.join(", "),
                    },
                ));
            }
        } else {
            let visible_fields: Vec<&crate::core::ir::FieldDef> =
                variant.fields.iter().filter(|f| !f.binding_excluded).collect();
            if visible_fields.is_empty() {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_enum_unit_to_core_arm.jinja",
                    minijinja::context! {
                        name => name.as_str(),
                        vname => vname.as_str(),
                        core_ty => core_ty.as_str(),
                    },
                ));
            } else if variant.is_tuple {
                let mirror_bindings: Vec<String> = (0..visible_fields.len()).map(|i| format!("field{i}")).collect();
                let core_args: Vec<String> = visible_fields
                    .iter()
                    .enumerate()
                    .map(|(i, field)| enum_variant_field_conv_to_core(&format!("field{i}"), field))
                    .collect();
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_enum_tuple_to_core_arm.jinja",
                    minijinja::context! {
                        name => name.as_str(),
                        vname => vname.as_str(),
                        core_ty => core_ty.as_str(),
                        mirror_bindings => mirror_bindings.join(", "),
                        core_args => core_args.join(", "),
                    },
                ));
            } else {
                let mirror_field_names: Vec<&str> = visible_fields.iter().map(|f| f.name.as_str()).collect();
                let mut core_args: Vec<String> = visible_fields
                    .iter()
                    .map(|field| {
                        let fname = &field.name;
                        let conv = enum_variant_field_conv_to_core(fname, field);
                        format!("{fname}: {conv}")
                    })
                    .collect();
                let excluded_args: Vec<String> = variant
                    .fields
                    .iter()
                    .filter(|f| f.binding_excluded)
                    .map(|f| format!("{}: Default::default()", f.name))
                    .collect();
                core_args.extend(excluded_args);
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_enum_struct_to_core_arm.jinja",
                    minijinja::context! {
                        name => name.as_str(),
                        vname => vname.as_str(),
                        core_ty => core_ty.as_str(),
                        field_names => mirror_field_names.join(", "),
                        core_args => core_args.join(", "),
                    },
                ));
            }
        }
    }

    // When any variant carries a `#[cfg(feature = "X")]` attribute, the cfg is
    // every feature combination; `#![allow(unreachable_patterns)]` at the crate
    if has_cfg_variants {
        out.push_str(&format!(
            "            _ => unreachable!(\"cfg-gated variant of {} not active in this build\"),\n",
            name
        ));
    }

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_impl_close.jinja",
        minijinja::context! {},
    ));
}

/// Build conversion expression for one enum variant field in the mirror-to-core direction.
fn enum_variant_field_conv_to_core(binding: &str, field: &FieldDef) -> String {
    if field.sanitized {
        return "Default::default()".to_string();
    }
    match &field.ty {
        TypeRef::Named(_) => match field.core_wrapper {
            CoreWrapper::Arc | CoreWrapper::ArcMutex => {
                if field.optional {
                    format!("{binding}.map(|x| std::sync::Arc::new(x.into()))")
                } else {
                    format!("std::sync::Arc::new({binding}.into())")
                }
            }
            _ if field.is_boxed => {
                if field.optional {
                    format!("{binding}.map(|x| Box::new(x.into()))")
                } else {
                    format!("Box::new({binding}.into())")
                }
            }
            _ => {
                if field.optional {
                    format!("{binding}.map(Into::into)")
                } else {
                    format!("{binding}.into()")
                }
            }
        },
        TypeRef::String => {
            if field.optional {
                if matches!(field.core_wrapper, CoreWrapper::Cow) {
                    format!("if {binding}.is_empty() {{ None }} else {{ Some({binding}.into()) }}")
                } else {
                    format!("if {binding}.is_empty() {{ None }} else {{ Some({binding}) }}")
                }
            } else if matches!(field.core_wrapper, CoreWrapper::Cow) {
                format!("{binding}.into()")
            } else {
                binding.to_string()
            }
        }
        TypeRef::Char => {
            if field.optional {
                format!("{binding}.as_deref().and_then(|s| s.chars().next())")
            } else {
                format!("{binding}.chars().next().unwrap_or_default()")
            }
        }
        TypeRef::Path => {
            if field.optional {
                format!("if {binding}.is_empty() {{ None }} else {{ Some(std::path::PathBuf::from({binding})) }}")
            } else {
                format!("std::path::PathBuf::from({binding})")
            }
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!("{binding}.into_iter().map(Into::into).collect()"),
            TypeRef::String => binding.to_string(),
            _ => format!("{binding}.into_iter().map(|x| x as _).collect::<Vec<_>>()"),
        },
        TypeRef::Primitive(prim) => {
            use crate::core::ir::PrimitiveType;
            if matches!(prim, PrimitiveType::Bool) {
                return match field.optional {
                    true => format!("if {binding} {{ Some({binding}) }} else {{ None }}"),
                    false => binding.to_string(),
                };
            }
            match (&field.newtype_wrapper, field.optional) {
                (Some(nw), true) => format!("if {binding} == 0 {{ None }} else {{ Some({nw}({binding} as _)) }}"),
                (Some(nw), false) => format!("{nw}({binding} as _)"),
                (None, true) => format!("if {binding} == 0 {{ None }} else {{ Some({binding} as _) }}"),
                (None, false) => format!("{binding} as _"),
            }
        }
        _ => {
            if field.optional {
                format!("{binding}.map(Into::into)")
            } else {
                format!("{binding}.into()")
            }
        }
    }
}

pub(super) fn emit_from_impl_for_enum(out: &mut String, en: &EnumDef, source_crate_name: &str) {
    let name = &en.name;
    let core_ty = if en.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        en.rust_path.replace('-', "_")
    };

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_core_enum_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => en.cfg.as_deref().unwrap_or(""),
        },
    ));

    let has_cfg_variants = en.variants.iter().any(|v| v.cfg.is_some());

    for variant in &en.excluded_variants {
        let vname = &variant.name;
        let template = if variant.is_tuple || !variant.fields.is_empty() {
            "rust_enum_excluded_variant_tuple_arm.jinja"
        } else {
            "rust_enum_excluded_variant_unit_arm.jinja"
        };
        out.push_str(&crate::backends::dart::template_env::render(
            template,
            minijinja::context! {
                core_ty => core_ty.as_str(),
                vname => vname.as_str(),
                name => name.as_str(),
            },
        ));
    }

    for variant in &en.variants {
        let vname = &variant.name;
        let cfg = variant.cfg.as_deref();
        if let Some(condition) = cfg {
            out.push_str("            #[cfg(");
            out.push_str(condition);
            out.push_str(")]\n");
        }
        let visible_fields: Vec<&crate::core::ir::FieldDef> =
            variant.fields.iter().filter(|f| !f.binding_excluded).collect();
        if variant.originally_had_data_fields {
            let template = if variant.is_tuple {
                "rust_enum_tuple_stripped_from_core_arm.jinja"
            } else {
                "rust_enum_struct_stripped_from_core_arm.jinja"
            };
            out.push_str(&crate::backends::dart::template_env::render(
                template,
                minijinja::context! {
                    core_ty => core_ty.as_str(),
                    vname => vname.as_str(),
                    name => name.as_str(),
                },
            ));
        } else if visible_fields.is_empty() {
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_enum_unit_from_core_arm.jinja",
                minijinja::context! {
                    core_ty => core_ty.as_str(),
                    vname => vname.as_str(),
                    name => name.as_str(),
                },
            ));
        } else if variant.is_tuple {
            let field_patterns: Vec<String> = (0..visible_fields.len()).map(|i| format!("f{i}")).collect();
            let mirror_fields: Vec<String> = visible_fields
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let conv = enum_variant_field_conv(&format!("f{i}"), field, source_crate_name);
                    format!("field{i}: {conv}")
                })
                .collect();
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_enum_tuple_from_core_arm.jinja",
                minijinja::context! {
                    core_ty => core_ty.as_str(),
                    vname => vname.as_str(),
                    name => name.as_str(),
                    field_patterns => field_patterns.join(", "),
                    mirror_fields => mirror_fields.join(", "),
                },
            ));
        } else {
            let field_names: Vec<&str> = visible_fields.iter().map(|f| f.name.as_str()).collect();
            let field_convs: Vec<String> = visible_fields
                .iter()
                .map(|field| {
                    let fname = &field.name;
                    let conv = enum_variant_field_conv(fname, field, source_crate_name);
                    format!("{fname}: {conv}")
                })
                .collect();
            out.push_str(&crate::backends::dart::template_env::render(
                "rust_enum_struct_from_core_arm.jinja",
                minijinja::context! {
                    core_ty => core_ty.as_str(),
                    vname => vname.as_str(),
                    name => name.as_str(),
                    field_names => field_names.join(", "),
                    field_convs => field_convs.join(", "),
                },
            ));
        }
    }

    // When any variant carries a `#[cfg(feature = "X")]` attribute, the cfg is
    // every feature combination; `#![allow(unreachable_patterns)]` at the crate
    if has_cfg_variants {
        out.push_str(&format!(
            "            _ => unreachable!(\"cfg-gated variant of {} not active in this build\"),\n",
            name
        ));
    }

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_impl_close.jinja",
        minijinja::context! {},
    ));
}

/// Build the conversion expression for one enum variant field.
///
/// For enum struct variant fields extracted from core, the binding is the actual
/// core type (which may be optional, a newtype, etc.). The mirror variant always
/// uses concrete types (String not Option<String>, i64 not usize).
fn enum_variant_field_conv(binding: &str, field: &FieldDef, source_crate_name: &str) -> String {
    let _ = source_crate_name;
    if field.sanitized {
        match &field.ty {
            TypeRef::Primitive(_) => {
                if field.optional {
                    return format!("{binding}.map(|x| x as _).unwrap_or_default()");
                }
                return format!("{binding} as _");
            }
            TypeRef::Vec(inner) => {
                if matches!(inner.as_ref(), TypeRef::Vec(inner_inner) if matches!(inner_inner.as_ref(), TypeRef::String))
                {
                    if field.optional {
                        return format!(
                            "{binding}.map(|v| v.into_iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect()).unwrap_or_default()"
                        );
                    }
                    return format!("{binding}.into_iter().map(|(a, b)| vec![a.to_string(), b.to_string()]).collect()");
                }
                if field.optional {
                    return format!(
                        "{binding}.map(|v| v.into_iter().map(|e| serde_json::to_string(&e).unwrap_or_default()).collect()).unwrap_or_default()"
                    );
                }
                return format!(
                    "{binding}.into_iter().map(|e| serde_json::to_string(&e).unwrap_or_default()).collect()"
                );
            }
            _ => {
                if field.optional {
                    return format!(
                        "{binding}.map(|v| serde_json::to_string(&v).unwrap_or_default()).unwrap_or_default()"
                    );
                }
                return format!("serde_json::to_string(&{binding}).unwrap_or_default()");
            }
        }
    }

    match &field.ty {
        TypeRef::Named(inner_name) => {
            if field.is_boxed && field.optional {
                format!("{binding}.map(|b| {inner_name}::from(*b)).unwrap_or_default()")
            } else if field.is_boxed {
                format!("{inner_name}::from(*{binding})")
            } else if field.optional {
                format!("{binding}.map({inner_name}::from).unwrap_or_default()")
            } else {
                format!("{inner_name}::from({binding})")
            }
        }
        TypeRef::Vec(inner) => {
            let item_conv = match inner.as_ref() {
                TypeRef::Named(inner_name) => Some(format!("{inner_name}::from")),
                TypeRef::Primitive(_) => Some("|x| x as _".to_string()),
                TypeRef::String => None,
                _ => Some("|s| s.into()".to_string()),
            };
            match (item_conv, field.optional) {
                (None, true) => format!("{binding}.unwrap_or_default()"),
                (None, false) => binding.to_string(),
                (Some(conv), true) => {
                    format!("{binding}.map(|v| v.into_iter().map({conv}).collect()).unwrap_or_default()")
                }
                (Some(conv), false) => format!("{binding}.into_iter().map({conv}).collect()"),
            }
        }
        TypeRef::String => {
            if field.optional {
                format!("{binding}.unwrap_or_default()")
            } else if matches!(field.core_wrapper, CoreWrapper::Cow) {
                format!("{binding}.into()")
            } else {
                // (clippy::useless_conversion flags `.into()` here).
                binding.to_string()
            }
        }
        TypeRef::Char => {
            if field.optional {
                format!("{binding}.map(|c| c.to_string()).unwrap_or_default()")
            } else {
                format!("{binding}.to_string()")
            }
        }
        TypeRef::Path => {
            if field.optional {
                format!("{binding}.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()")
            } else {
                format!("{binding}.to_string_lossy().into_owned()")
            }
        }
        TypeRef::Json => {
            if field.optional {
                format!("{binding}.map(|j| serde_json::to_string(&j).unwrap_or_default()).unwrap_or_default()")
            } else {
                format!("serde_json::to_string(&{binding}).unwrap_or_default()")
            }
        }
        TypeRef::Primitive(_) => {
            if let Some(_nw) = &field.newtype_wrapper {
                if field.optional {
                    format!("{binding}.map(|x| x.0 as _).unwrap_or_default()")
                } else {
                    format!("{binding}.0 as _")
                }
            } else if field.optional {
                format!("{binding}.map(|x| x as _).unwrap_or_default()")
            } else {
                format!("{binding} as _")
            }
        }
        TypeRef::Map(_, v_ty) => {
            let needs_value_conv = matches!(v_ty.as_ref(), TypeRef::Json | TypeRef::Named(_));
            if needs_value_conv {
                format!(
                    "{binding}.into_iter().map(|(k, v)| (k.into(), serde_json::to_string(&v).unwrap_or_default())).collect()"
                )
            } else {
                format!("{binding}.into_iter().map(|(k, v)| (k.into(), v.into())).collect()")
            }
        }
        _ => binding.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant};

    fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    /// A cfg-gated variant on a mirror enum emits a catch-all `_ => unreachable!()`
    /// arm so the `From<CoreType>` match is exhaustive even when the feature is not
    /// declared in the binding crate (E0004 guard).
    #[test]
    fn cfg_gated_variant_emits_catch_all_in_from_core_impl() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Native", None),
                make_unit_variant("Png", None),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
            ],
            ..Default::default()
        };
        let mut out = String::new();
        emit_from_impl_for_enum(&mut out, &en, "mylib");
        assert!(
            out.contains("_ => unreachable!"),
            "expected catch-all `_ => unreachable!` arm in From<CoreType> impl, got:\n{out}"
        );
        assert!(
            out.contains("cfg-gated variant of ImageOutputFormat"),
            "expected enum name in catch-all message, got:\n{out}"
        );
    }

    /// The same catch-all is emitted in the mirror→core direction.
    #[test]
    fn cfg_gated_variant_emits_catch_all_in_from_mirror_impl() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Native", None),
                make_unit_variant("Png", None),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
            ],
            ..Default::default()
        };
        let mut out = String::new();
        emit_from_mirror_to_core_enum(&mut out, &en, "mylib");
        assert!(
            out.contains("_ => unreachable!"),
            "expected catch-all `_ => unreachable!` arm in From<Mirror> impl, got:\n{out}"
        );
        assert!(
            out.contains("cfg-gated variant of ImageOutputFormat"),
            "expected enum name in catch-all message, got:\n{out}"
        );
    }

    /// When no variant has a cfg attribute, no catch-all is emitted (the match
    /// remains fully exhaustive without it, and we do not want spurious arms).
    #[test]
    fn no_cfg_variants_does_not_emit_catch_all() {
        let en = EnumDef {
            name: "SimpleEnum".to_string(),
            variants: vec![make_unit_variant("A", None), make_unit_variant("B", None)],
            ..Default::default()
        };
        let mut out_core = String::new();
        emit_from_impl_for_enum(&mut out_core, &en, "mylib");
        let mut out_mirror = String::new();
        emit_from_mirror_to_core_enum(&mut out_mirror, &en, "mylib");

        assert!(
            !out_core.contains("_ => unreachable!"),
            "unexpected catch-all in From<CoreType> impl for no-cfg enum:\n{out_core}"
        );
        assert!(
            !out_mirror.contains("_ => unreachable!"),
            "unexpected catch-all in From<Mirror> impl for no-cfg enum:\n{out_mirror}"
        );
    }
}
