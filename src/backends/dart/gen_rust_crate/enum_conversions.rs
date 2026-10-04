use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::field_init::struct_field_init;
use crate::core::ir::{CoreWrapper, EnumDef, FieldDef, TypeRef};

pub(super) fn emit_from_mirror_to_core_enum(
    out: &mut String,
    en: &EnumDef,
    source_crate_name: &str,
    configured_features: Option<&[String]>,
) {
    let name = &en.name;
    let core_ty = if en.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        en.rust_path.replace('-', "_")
    };
    // A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's
    // own cfg gate; this dart crate never declares a Cargo feature for it (see
    // `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim as `#[cfg(...)]` produces an
    // `unexpected cfg condition value` warning for a feature this crate cannot control. See
    // `is_host_owned_rust_path`'s doc for why both halves must agree, and
    // `swift::gen_rust_crate::enums` for the sibling fix this mirrors. ~keep
    let is_host_enum = is_host_owned_rust_path(source_crate_name, &en.rust_path);

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_mirror_enum_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => en.cfg.as_deref().unwrap_or(""),
        },
    ));

    for variant in &en.variants {
        let vname = &variant.name;
        let cfg = variant.cfg.as_deref();
        if cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %en.name,
                enum_rust_path = %en.rust_path,
                variant_name = %variant.name,
                cfg = cfg.unwrap_or_default(),
                "dropping Dart bridge From<Mirror>-impl arm for a foreign-crate enum variant \
                 behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the variant \
                 is unreachable from this conversion"
            );
            continue;
        }
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
                        struct_field_init(fname, &conv)
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

    // A foreign cfg-gated variant's arm is dropped unconditionally above, so whether a catch-all
    // is still needed for it depends on whether this binding's own configured feature set proves
    // the variant unreachable -- delegated to
    // `codegen::conversions::enums::enum_conversion_needs_catch_all_for_features`, the same
    // resolver every other Rust-emitting backend's enum conversion uses, so this bespoke Dart
    // generator can't drift from that verdict (alef #547). `false` for `has_excluded_variants`:
    // this binding->core direction only ever matches the mirror's OWN declared variants, which by
    // construction never carries a gap the way the core->binding direction's `excluded_variants`
    // can. This match is over the Mirror enum this crate itself declares
    // (`mirror::emit_mirror_enum`), which now asks the identical `enum_variant_declaration`
    // authority (via the identical `configured_features`) to decide whether to declare a foreign
    // cfg-gated variant at all -- so `declaration_may_drop_variant` is `true` here too, matching
    // `emit_from_impl_for_enum` below: whenever `configured_features` proves the variant
    // unreachable, the mirror declaration drops it and this match needs no catch-all for it;
    // whenever it does not, the mirror keeps it and the catch-all is required. See
    // `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    if crate::codegen::conversions::enum_conversion_needs_catch_all_for_features(
        en,
        is_host_enum,
        false,
        configured_features,
        true,
    ) {
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
    if let Some(converted) = super::newtype_conversions::to_core(binding, field) {
        return converted;
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

pub(super) fn emit_from_impl_for_enum(
    out: &mut String,
    en: &EnumDef,
    source_crate_name: &str,
    configured_features: Option<&[String]>,
) {
    let name = &en.name;
    let core_ty = if en.rust_path.is_empty() {
        format!("{source_crate_name}::{name}")
    } else {
        en.rust_path.replace('-', "_")
    };
    // See the sibling comment in `emit_from_mirror_to_core_enum`: a foreign-crate cfg cannot be
    // forwarded as a Cargo feature this crate declares. ~keep
    let is_host_enum = is_host_owned_rust_path(source_crate_name, &en.rust_path);

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_core_enum_open.jinja",
        minijinja::context! {
            core_ty => core_ty.as_str(),
            name => name.as_str(),
            source_cfg => en.cfg.as_deref().unwrap_or(""),
        },
    ));

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
        if cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %en.name,
                enum_rust_path = %en.rust_path,
                variant_name = %variant.name,
                cfg = cfg.unwrap_or_default(),
                "dropping Dart bridge From<CoreType>-impl arm for a foreign-crate enum variant \
                 behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the variant \
                 is unreachable from this conversion"
            );
            continue;
        }
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
                    struct_field_init(fname, &conv)
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

    // A foreign cfg-gated variant's arm is dropped unconditionally above, so whether a catch-all
    // is still needed for it depends on whether this binding's own configured feature set proves
    // the variant unreachable -- delegated to
    // `codegen::conversions::enums::enum_conversion_needs_catch_all_for_features`, the same
    // resolver every other Rust-emitting backend's enum conversion uses, so this bespoke Dart
    // generator can't drift from that verdict (alef #547). `!en.excluded_variants.is_empty()`
    // covers the orthogonal gap this core->binding direction alone can have: a core variant this
    // binding never generates an arm for at all, regardless of cfg. This match is over the real
    // CORE type (`core_ty` above) -- a shape this crate does not declare and cannot influence, so
    // `configured_features`' proof about the dependency is already the complete answer regardless
    // of the Mirror declaration's own policy. `true` here, unlike `emit_from_mirror_to_core_enum`
    // above. See `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    if crate::codegen::conversions::enum_conversion_needs_catch_all_for_features(
        en,
        is_host_enum,
        !en.excluded_variants.is_empty(),
        configured_features,
        true,
    ) {
        out.push_str(&format!(
            "            _ => unreachable!(\"cfg-gated variant of {} not active in this build\"),\n",
            name
        ));
    }

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_impl_close.jinja",
        minijinja::context! {},
    ));

    if en.has_serde {
        emit_fallible_from_core_enum(out, en, &core_ty, is_host_enum, configured_features);
    }
}

pub(super) fn fallible_from_core_fn_name(enum_name: &str) -> String {
    let snake = crate::codegen::naming::public_host_identifier(
        crate::core::config::Language::Rust,
        crate::codegen::naming::PublicIdentifierKind::Function,
        enum_name,
    );
    format!("try_convert_{snake}_from_core")
}

fn emit_fallible_from_core_enum(
    out: &mut String,
    en: &EnumDef,
    core_ty: &str,
    is_host_enum: bool,
    configured_features: Option<&[String]>,
) {
    let fn_name = fallible_from_core_fn_name(&en.name);
    if let Some(cfg) = en.cfg.as_deref() {
        out.push_str(&format!("#[cfg({cfg})]\n"));
    }
    out.push_str(&format!(
        "fn {fn_name}(value: {core_ty}) -> Result<{}, String> {{\n    let representable = match &value {{\n",
        en.name
    ));
    for variant in &en.excluded_variants {
        let pattern = if variant.is_tuple {
            format!("{core_ty}::{}(..)", variant.name)
        } else if !variant.fields.is_empty() || variant.originally_had_data_fields {
            format!("{core_ty}::{} {{ .. }}", variant.name)
        } else {
            format!("{core_ty}::{}", variant.name)
        };
        out.push_str(&format!("        {pattern} => false,\n"));
    }
    for variant in &en.variants {
        if variant.cfg.is_some() && !is_host_enum {
            continue;
        }
        if let Some(cfg) = variant.cfg.as_deref() {
            out.push_str(&format!("        #[cfg({cfg})]\n"));
        }
        let pattern = if variant.is_tuple {
            format!("{core_ty}::{}(..)", variant.name)
        } else if !variant.fields.is_empty() || variant.originally_had_data_fields {
            format!("{core_ty}::{} {{ .. }}", variant.name)
        } else {
            format!("{core_ty}::{}", variant.name)
        };
        out.push_str(&format!("        {pattern} => true,\n"));
    }
    let needs_catch_all = crate::codegen::conversions::enum_conversion_needs_catch_all_for_features(
        en,
        is_host_enum,
        false,
        configured_features,
        true,
    );
    if needs_catch_all {
        out.push_str("        _ => false,\n");
    }
    out.push_str(&format!(
        "    }};\n    if !representable {{\n        return Err(\"{} contains a variant unavailable in the Dart binding\".to_string());\n    }}\n    Ok({}::from(value))\n}}\n",
        en.name, en.name
    ));
}

/// Build the conversion expression for one enum variant field.
///
/// For enum struct variant fields extracted from core, the binding is the actual
/// core type (which may be optional, a newtype, etc.). The mirror variant always
/// uses concrete types (String not Option<String>, i64 not usize).
fn enum_variant_field_conv(binding: &str, field: &FieldDef, source_crate_name: &str) -> String {
    let _ = source_crate_name;
    if let Some(converted) = super::newtype_conversions::from_core(binding, field) {
        return converted;
    }
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
    use super::super::mirror::emit_mirror_enum;
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant};

    fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    /// A FOREIGN cfg-gated variant (`rust_path` rooted in a crate other than "mylib") emits a
    /// catch-all `_ => unreachable!()` arm so the `From<CoreType>` match is exhaustive even when
    /// `configured_features` is `None` (unknown -- not proven unreachable) -- see
    /// `host_cfg_variant_keeps_its_arm_and_gate_in_both_directions` below for the sibling
    /// host-owned case, which needs NO catch-all since its arm carries the identical `#[cfg(...)]`
    /// guard as the variant itself. ~keep
    #[test]
    fn cfg_gated_variant_emits_catch_all_in_from_core_impl() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            rust_path: "dep_crate::ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Native", None),
                make_unit_variant("Png", None),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
            ],
            ..Default::default()
        };
        let mut out = String::new();
        emit_from_impl_for_enum(&mut out, &en, "mylib", None);
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
            rust_path: "dep_crate::ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Native", None),
                make_unit_variant("Png", None),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
            ],
            ..Default::default()
        };
        let mut out = String::new();
        emit_from_mirror_to_core_enum(&mut out, &en, "mylib", None);
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
        emit_from_impl_for_enum(&mut out_core, &en, "mylib", None);
        let mut out_mirror = String::new();
        emit_from_mirror_to_core_enum(&mut out_mirror, &en, "mylib", None);

        assert!(
            !out_core.contains("_ => unreachable!"),
            "unexpected catch-all in From<CoreType> impl for no-cfg enum:\n{out_core}"
        );
        assert!(
            !out_mirror.contains("_ => unreachable!"),
            "unexpected catch-all in From<Mirror> impl for no-cfg enum:\n{out_mirror}"
        );
    }

    #[test]
    fn fallible_from_core_rejects_excluded_struct_variant() {
        let excluded = EnumVariant {
            name: "Internal".to_string(),
            fields: vec![crate::core::ir::FieldDef {
                name: "reason".to_string(),
                ..Default::default()
            }],
            originally_had_data_fields: true,
            ..Default::default()
        };
        let en = EnumDef {
            name: "WorkflowStep".to_string(),
            rust_path: "mylib::WorkflowStep".to_string(),
            has_serde: true,
            variants: vec![make_unit_variant("Ready", None)],
            excluded_variants: vec![excluded],
            ..Default::default()
        };
        let mut out = String::new();
        emit_from_impl_for_enum(&mut out, &en, "mylib", None);

        assert!(
            out.contains("mylib::WorkflowStep::Internal { .. } => false"),
            "excluded struct variants must be rejected without invoking From: {out}"
        );
        assert!(
            out.contains("return Err(\"WorkflowStep contains a variant unavailable in the Dart binding\".to_string())"),
            "unrepresentable variants must return an error: {out}"
        );
        assert_eq!(
            out.matches("_ => false").count(),
            0,
            "fully enumerated variants must not emit an unreachable catch-all: {out}"
        );
    }

    #[test]
    fn non_serde_enum_omits_unused_fallible_from_core_helper() {
        let en = EnumDef {
            name: "Status".to_string(),
            rust_path: "mylib::Status".to_string(),
            variants: vec![make_unit_variant("Ready", None)],
            ..Default::default()
        };
        let mut out = String::new();
        emit_from_impl_for_enum(&mut out, &en, "mylib", None);

        assert!(
            !out.contains("try_convert_status_from_core"),
            "non-serde enums have no decoder caller: {out}"
        );
    }

    /// The regression this task fixes: a whole enum gated behind a Cargo feature (`EnumDef::cfg`,
    /// as opposed to a single variant's cfg) carries that gate through to both `impl From<...>`
    /// blocks, which name the host path directly. Before the fix, `source_cfg` was passed to the
    /// "open" templates but never used, so a wholly-gated enum's From impls were always emitted
    /// unconditionally -- an E0433 in a build excluding the feature.
    #[test]
    fn whole_enum_cfg_gates_both_from_impls() {
        let en = EnumDef {
            name: "OcrMode".to_string(),
            rust_path: "mylib::thumbnails::OcrMode".to_string(),
            cfg: Some(r#"feature = "thumbnails""#.to_string()),
            has_serde: true,
            variants: vec![make_unit_variant("Fast", None), make_unit_variant("Accurate", None)],
            ..Default::default()
        };
        let mut out_core = String::new();
        emit_from_impl_for_enum(&mut out_core, &en, "mylib", None);
        assert_eq!(
            out_core.matches("#[cfg(feature = \"thumbnails\")]").count(),
            2,
            "the whole-enum gate must cover the From<CoreType> impl and fallible decoder helper, got:\n{out_core}"
        );

        let mut out_mirror = String::new();
        emit_from_mirror_to_core_enum(&mut out_mirror, &en, "mylib", None);
        assert_eq!(
            out_mirror.matches("#[cfg(feature = \"thumbnails\")]").count(),
            1,
            "the whole-enum gate must land on the From<Mirror> impl exactly once, got:\n{out_mirror}"
        );
    }

    /// Negative control: an ungated enum (`EnumDef::cfg` is `None`) must emit no `#[cfg(...)]`
    /// on either impl header.
    #[test]
    fn ungated_enum_emits_no_cfg_on_either_impl_header() {
        let en = EnumDef {
            name: "PlainMode".to_string(),
            rust_path: "mylib::PlainMode".to_string(),
            variants: vec![make_unit_variant("A", None), make_unit_variant("B", None)],
            ..Default::default()
        };
        let mut out_core = String::new();
        emit_from_impl_for_enum(&mut out_core, &en, "mylib", None);
        assert!(
            !out_core.contains("#[cfg("),
            "ungated enum must not emit #[cfg(...)] in From<CoreType> impl, got:\n{out_core}"
        );

        let mut out_mirror = String::new();
        emit_from_mirror_to_core_enum(&mut out_mirror, &en, "mylib", None);
        assert!(
            !out_mirror.contains("#[cfg("),
            "ungated enum must not emit #[cfg(...)] in From<Mirror> impl, got:\n{out_mirror}"
        );
    }

    /// A variant merged in from a foreign `[[crates.source_crates]]` crate (`rust_path` rooted
    /// in a crate other than the host) carries that crate's own cfg. Forwarding it verbatim as
    /// `#[cfg(...)]` names a feature this dart crate never declares -- an `unexpected cfg
    /// condition value` warning (the second leak this task fixes) -- so the arm must be dropped
    /// entirely instead, mirroring `swift::gen_rust_crate::enums`.
    #[test]
    fn foreign_cfg_variant_arm_is_dropped_not_gated_in_both_directions() {
        let en = EnumDef {
            name: "TierStrategy".to_string(),
            rust_path: "dep_crate::TierStrategy".to_string(),
            variants: vec![
                make_unit_variant("Auto", None),
                make_unit_variant("Tier1", Some(r#"feature = "testkit""#)),
            ],
            ..Default::default()
        };

        let mut out_core = String::new();
        emit_from_impl_for_enum(&mut out_core, &en, "mylib", None);
        assert!(
            !out_core.contains("#[cfg(feature = \"testkit\")]"),
            "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{out_core}"
        );
        assert!(
            !out_core.contains("dep_crate::TierStrategy::Tier1 =>"),
            "a foreign-crate cfg-gated variant must not be referenced in the From<CoreType> match, got:\n{out_core}"
        );

        let mut out_mirror = String::new();
        emit_from_mirror_to_core_enum(&mut out_mirror, &en, "mylib", None);
        assert!(
            !out_mirror.contains("#[cfg(feature = \"testkit\")]"),
            "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{out_mirror}"
        );
        assert!(
            !out_mirror.contains("Tier1 =>"),
            "a foreign-crate cfg-gated variant must not be referenced in the From<Mirror> match, got:\n{out_mirror}"
        );
    }

    /// Superseded premise, corrected: this test used to assert that `emit_from_mirror_to_core_enum`
    /// (binding->core) always needs a catch-all for a proven-unreachable foreign cfg-gated
    /// variant, because `mirror::emit_mirror_enum` used to keep every foreign variant
    /// unconditionally regardless of `configured_features`. That premise is what the consumer's
    /// binding audit found wrong: `mirror::emit_mirror_enum` now asks the SAME
    /// `enum_variant_declaration` authority (with the SAME `configured_features`) as this
    /// function's own catch-all resolver, so the two can never disagree --
    /// `enum_variant_declaration`'s `Drop` branch and
    /// `enum_conversion_needs_catch_all_for_features`'s `declaration_may_drop_variant = true`
    /// branch both reduce to the identical `foreign_variant_proven_unreachable(cfg, features)`
    /// call. Concretely, for every variant: the mirror per-variant arm loop above drops a
    /// foreign cfg-gated variant's ARM unconditionally regardless of proof (it can never emit a
    /// `#[cfg(...)]` naming a feature this crate does not own), while the mirror's own
    /// DECLARATION keeps the variant unless proven unreachable. So a gap (declared but no arm)
    /// exists exactly when the variant is foreign, cfg-gated, and NOT proven unreachable -- which
    /// is exactly when this catch-all resolver reports `true`. When proven unreachable, both the
    /// declaration and the arm are absent, so the match is exhaustive over what the mirror can
    /// actually hold and a catch-all would be dead code (`unreachable_patterns`). This test now
    /// asserts the coupled, corrected invariant directly instead of hardcoding one side's
    /// now-obsolete assumption about the other. ~keep
    #[test]
    fn foreign_cfg_variant_proven_unreachable_drops_from_both_declaration_and_conversions() {
        let en = EnumDef {
            name: "External".to_string(),
            rust_path: "dep_crate::External".to_string(),
            variants: vec![
                make_unit_variant("Foo", None),
                make_unit_variant("Bar", Some(r#"feature = "extra""#)),
            ],
            ..Default::default()
        };
        // No feature configured, so `configured_features` proves the foreign "extra" gate
        // unsatisfied -- the real core type's `Bar` variant cannot exist in this build. Every
        // call below uses this SAME value, matching how the real generator (`gen_rust_crate::mod`)
        // threads one `configured_features` computation into both the declaration and every
        // conversion call for a given enum -- the coupling this test exists to prove. ~keep
        let configured_features: Option<&[String]> = Some(&[]);

        let mut out_mirror_decl = String::new();
        emit_mirror_enum(&mut out_mirror_decl, &en, "mylib", configured_features);
        assert!(
            !out_mirror_decl.contains("Bar"),
            "proven-unreachable foreign variant must not be declared on the mirror, got:\n{out_mirror_decl}"
        );
        assert!(
            out_mirror_decl.contains("Foo"),
            "the always-present variant must still be declared, got:\n{out_mirror_decl}"
        );

        let mut out_core = String::new();
        emit_from_impl_for_enum(&mut out_core, &en, "mylib", configured_features);
        assert!(
            !out_core.contains("_ => unreachable!"),
            "core->binding match is over the real core type, proven by configured_features to \
             lack the variant -- a catch-all here is an unreachable pattern:\n{out_core}"
        );

        let mut out_mirror = String::new();
        emit_from_mirror_to_core_enum(&mut out_mirror, &en, "mylib", configured_features);
        assert!(
            !out_mirror.contains("_ => unreachable!"),
            "binding->core match is over the Mirror enum, which -- with the SAME \
             configured_features just proven above to drop `Bar` from the declaration -- no \
             longer holds the variant either, so a catch-all here would be unreachable dead code \
             under -D warnings:\n{out_mirror}"
        );
    }

    /// A host-owned cfg-gated variant (`rust_path` rooted in the host crate) keeps its arm in
    /// both directions and its `#[cfg(...)]` guard, since the feature is safely forwardable via
    /// this crate's own `[features]` table.
    #[test]
    fn host_cfg_variant_keeps_its_arm_and_gate_in_both_directions() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            rust_path: "mylib::ImageOutputFormat".to_string(),
            has_serde: true,
            variants: vec![
                make_unit_variant("Jpeg", None),
                make_unit_variant("Heif", Some(r#"feature = "heic""#)),
            ],
            ..Default::default()
        };

        let mut out_core = String::new();
        emit_from_impl_for_enum(&mut out_core, &en, "mylib", None);
        assert_eq!(
            out_core.matches("#[cfg(feature = \"heic\")]").count(),
            2,
            "the host-owned variant's From<CoreType> and fallible decoder arms must keep their #[cfg] guards, got:\n{out_core}"
        );
        // alef #547: a host-owned cfg-gated variant's own arm carries the identical #[cfg(...)]
        // guard as the variant itself, so the two always compile in or out together and the match
        // stays exhaustive either way -- unlike the foreign case, no catch-all is ever needed
        // here, regardless of `configured_features`.
        assert!(
            !out_core.contains("_ => unreachable!"),
            "a host-owned cfg-gated variant alone must not trigger a catch-all, got:\n{out_core}"
        );

        let mut out_mirror = String::new();
        emit_from_mirror_to_core_enum(&mut out_mirror, &en, "mylib", None);
        assert!(
            !out_mirror.contains("_ => unreachable!"),
            "a host-owned cfg-gated variant alone must not trigger a catch-all, got:\n{out_mirror}"
        );
        assert_eq!(
            out_mirror.matches("#[cfg(feature = \"heic\")]").count(),
            1,
            "the host-owned variant's From<Mirror> arm must keep its #[cfg] guard, got:\n{out_mirror}"
        );
    }
}
