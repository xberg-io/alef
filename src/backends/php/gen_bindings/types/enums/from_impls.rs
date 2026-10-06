//! Flat data-enum core/binding conversion impl generators, split out of enums.rs.

use super::*;

/// Generate `From<core::DataEnum> for PhpDataEnum` and `From<PhpDataEnum> for core::DataEnum`
/// for a tagged data enum lowered to a flat PHP class.
pub(crate) fn gen_flat_data_enum_from_impls(
    enum_def: &EnumDef,
    core_import: &str,
    configured_features: Option<&[String]>,
) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = &enum_def.name;
    // A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's
    // own cfg gate; this PHP crate never declares a Cargo feature for it (see
    // `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim as `#[cfg(...)]` produces an
    // `unexpected cfg condition value` error. Such a variant is dropped from both match arms
    // below instead -- named via `tracing::debug!`, with `codegen::foreign_cfg_variants` raising
    // the same fact to WARN once for the whole run -- mirroring
    // `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper` and
    // `codegen::conversions::enums::gen_enum_from_core_to_binding_cfg`. A host-owned cfg keeps
    // its arm and its `#[cfg(...)]`. ~keep
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let all_flat_fields: std::collections::BTreeSet<String> = {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for variant in &enum_def.variants {
            for (idx, _) in variant.fields.iter().enumerate() {
                seen.insert(flat_field_name(variant, idx));
            }
        }
        seen
    };

    let mut out = String::new();

    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_from_start.jinja",
        minijinja::context! {
            core_path => &core_path,
            binding_name => &binding_name,
        },
    ));
    for variant in &enum_def.variants {
        if variant.cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %enum_def.name,
                enum_rust_path = %enum_def.rust_path,
                variant_name = %variant.name,
                cfg = variant.cfg.as_deref().unwrap_or_default(),
                "dropping PHP flat-enum From<CoreType> arm for a foreign-crate enum variant \
                 behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the variant \
                 is unreachable from this conversion"
            );
            continue;
        }
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_variant_match_empty.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    variant_name => &variant.name,
                    tag_field => tag_field,
                    tag_val => &tag_val,
                    needs_default => !all_flat_fields.is_empty(),
                    cfg => variant.cfg.as_deref(),
                },
            ));
        } else {
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
            let pattern = if is_tuple {
                let names: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        if f.sanitized {
                            format!("_{}", f.name)
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect();
                names.join(", ")
            } else {
                let bindings: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        if f.sanitized {
                            format!("{}: _{}", f.name, f.name)
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect();
                bindings.join(", ")
            };
            let pattern_start = if is_tuple {
                format!("            {core_path}::{}({pattern}) => Self {{", variant.name)
            } else {
                format!("            {core_path}::{}{{ {pattern} }} => Self {{", variant.name)
            };
            // `pattern_start` is a pre-existing raw `format!` site (not this fix's doing --
            // see the `jinja-templates` rule note this leaves for a future pass); the cfg
            // guard itself is a plain attribute line, matching how every other backend
            // (dart, swift, ffi) emits a per-arm `#[cfg(...)]` without a dedicated template.
            if let Some(cfg) = variant.cfg.as_deref() {
                out.push_str("            #[cfg(");
                out.push_str(cfg);
                out.push_str(")]\n");
            }
            out.push_str(&pattern_start);
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_tag_assignment.jinja",
                minijinja::context! {
                    tag_field => tag_field,
                    tag_val => &tag_val,
                },
            ));
            for (idx, f) in variant.fields.iter().enumerate() {
                let flat_name = flat_field_name(variant, idx);
                let bound_var = if f.sanitized {
                    format!("_{}", f.name)
                } else {
                    f.name.clone()
                };
                let expr = flat_enum_core_to_binding_field_expr(f, &bound_var);
                out.push_str(&crate::backends::php::template_env::render(
                    "php_flat_enum_variant_field.jinja",
                    minijinja::context! {
                        flat_name => &flat_name,
                        expr => &expr,
                    },
                ));
            }
            // field — the struct update would have no effect and triggers `clippy::needless_update`.
            let variant_flat_names: std::collections::BTreeSet<String> =
                (0..variant.fields.len()).map(|i| flat_field_name(variant, i)).collect();
            if variant_flat_names == all_flat_fields {
                out.push_str(" },\n");
            } else {
                out.push_str(" ..Default::default() },\n");
            }
        }
    }
    // When the IR has excluded variants (e.g. cfg-gated variants with #[alef(skip)] or
    // #[doc(hidden)]), the Rust compiler sees those variants at compile time but the generated
    // match has no arm for them, so a catch-all is required. A foreign-crate cfg-gated variant
    // (dropped above, unconditionally) leaves the same kind of gap UNLESS this binding's own
    // configured feature set proves the variant unreachable, in which case the gap closes and no
    // catch-all is needed. A host-owned cfg-gated variant never needs one: its arm carries the
    // identical `#[cfg(...)]` as the variant itself, so they compile in or out together and the
    // match stays exhaustive either way -- see
    // `codegen::conversions::enum_conversion_needs_catch_all_for_features`, the same
    // configured-feature-aware resolver every `ConversionConfig`-driven enum conversion already
    // uses, which this defers to so the reimplementations here and in NAPI's
    // `gen_tagged_enum_core_to_binding` can't drift back out of sync with each other or with the
    // shared `gen_enum_from_core_to_binding_cfg` (alef #544). This match is over the real CORE
    // type (`core_path` above), a shape this crate does not declare and cannot influence, so
    // `configured_features`' proof about the dependency is already the complete answer -- `true`
    // here. (The sibling `From<PhpDataEnum> for core::DataEnum` impl below matches a runtime
    // string tag, never a real enum, so `unreachable_patterns` can never fire there and it needs
    // no call into this resolver at all.) See
    // `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    if enum_conversion_needs_catch_all_for_features(
        enum_def,
        is_host_enum,
        !enum_def.excluded_variants.is_empty(),
        configured_features,
        true,
    ) {
        out.push_str("            _ => Default::default(),\n");
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_match_end.jinja",
        minijinja::Value::default(),
    ));

    // (may have #[serde(rename = "camelCase")] on individual variant fields).
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_into_start.jinja",
        minijinja::context! {
            binding_name => &binding_name,
            core_path => &core_path,
            tag_field => tag_field,
        },
    ));
    for variant in &enum_def.variants {
        if variant.cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %enum_def.name,
                enum_rust_path = %enum_def.rust_path,
                variant_name = %variant.name,
                cfg = variant.cfg.as_deref().unwrap_or_default(),
                "dropping PHP flat-enum From<Binding> arm for a foreign-crate enum variant \
                 behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the tag \
                 falls through to the existing fallback arm instead"
            );
            continue;
        }
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_variant_match_into_empty.jinja",
                minijinja::context! {
                    tag_val => &tag_val,
                    core_path => &core_path,
                    variant_name => &variant.name,
                    cfg => variant.cfg.as_deref(),
                },
            ));
        } else {
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
            let pattern_start = if is_tuple {
                format!("            \"{tag_val}\" => {core_path}::{}(", variant.name)
            } else {
                format!("            \"{tag_val}\" => {core_path}::{}{{", variant.name)
            };
            // See the matching note in the core→binding loop above: `pattern_start` is a
            // pre-existing raw `format!` site; the cfg guard is a plain attribute line.
            if let Some(cfg) = variant.cfg.as_deref() {
                out.push_str("            #[cfg(");
                out.push_str(cfg);
                out.push_str(")]\n");
            }
            out.push_str(&pattern_start);
            if is_tuple {
                let exprs: Vec<String> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(idx, f)| flat_enum_binding_to_core_field_expr(f, &flat_field_name(variant, idx)))
                    .collect();
                out.push_str(&crate::backends::php::template_env::render(
                    "php_flat_enum_tuple_exprs.jinja",
                    minijinja::context! {
                        exprs_joined => exprs.join(", "),
                    },
                ));
                out.push_str(" ),\n");
            } else {
                for (idx, f) in variant.fields.iter().enumerate() {
                    let flat_name = flat_field_name(variant, idx);
                    let expr = flat_enum_binding_to_core_field_expr(f, &flat_name);
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_flat_enum_variant_field.jinja",
                        minijinja::context! {
                            flat_name => &flat_name,
                            expr => &expr,
                        },
                    ));
                }
                out.push_str(" },\n");
            }
        }
    }
    // enum has a visible `Default` impl (a variant with `#[default]`): delegate to
    // `impl Default` is marked `#[cfg_attr(alef, alef(skip))]` it is invisible to Alef's IR,
    let core_has_default = enum_def.variants.iter().any(|v| v.is_default);
    if core_has_default {
        out.push_str(&crate::backends::php::template_env::render(
            "php_flat_enum_default_fallback_match_arm.jinja",
            minijinja::context! {
                core_path => &core_path,
            },
        ));
    } else {
        out.push_str(
            "            _ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\"),\n",
        );
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_match_end.jinja",
        minijinja::Value::default(),
    ));

    let _ = TypeRef::Unit;
    let _ = PrimitiveType::Bool;

    out
}

/// Build the expression for a single flat-enum variant field when converting core → binding.
/// The binding struct field is always `Option<MappedType>` for flat data enums.
///
/// - Sanitized fields cannot be converted (the core type is unknown/complex); emit `None`.
/// - `is_boxed` fields: unbox with `*` before converting.
/// - `TypeRef::Path`: convert via `to_string_lossy().into_owned()`.
/// - `TypeRef::Primitive(Usize | U64 | Isize)`: cast to `i64` (PHP's integer representation).
/// - Everything else: use `.into()` / `.map(Into::into)`.
fn flat_enum_core_to_binding_field_expr(f: &crate::core::ir::FieldDef, bound_var: &str) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};

    if f.sanitized {
        return "None".to_string();
    }

    let wrap_some = |inner: String| -> String { format!("Some({inner})") };

    match &f.ty {
        TypeRef::Path => {
            if f.optional {
                format!("{bound_var}.map(|p| p.to_string_lossy().into_owned())")
            } else {
                wrap_some(format!("{bound_var}.to_string_lossy().into_owned()"))
            }
        }
        TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::Isize) => {
            if f.optional {
                format!("{bound_var}.map(|v| v as i64)")
            } else {
                wrap_some(format!("{bound_var} as i64"))
            }
        }
        TypeRef::Named(_) if f.is_boxed => {
            if f.optional {
                format!("{bound_var}.map(|v| (*v).into())")
            } else {
                wrap_some(format!("(*{bound_var}).into()"))
            }
        }
        TypeRef::Primitive(_) | TypeRef::String => {
            if f.optional {
                bound_var.to_string()
            } else {
                wrap_some(bound_var.to_string())
            }
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if f.optional {
                format!("{bound_var}.map(|v| v.into_iter().map(Into::into).collect())")
            } else {
                wrap_some(format!("{bound_var}.into_iter().map(Into::into).collect()"))
            }
        }
        _ => {
            if f.optional {
                format!("{bound_var}.map(Into::into)")
            } else {
                wrap_some(format!("{bound_var}.into()"))
            }
        }
    }
}

/// Build the expression for a single flat-enum variant field when converting binding → core.
/// The binding struct field is always `Option<MappedType>`; the core field may be non-optional.
///
/// - Sanitized fields: emit `Default::default()` (cannot round-trip through PHP).
/// - `is_boxed` fields: wrap the result in `Box::new(...)`.
/// - `TypeRef::Path`: convert `String → PathBuf` via `PathBuf::from`.
/// - `TypeRef::Primitive(Usize | U64 | Isize)`: cast `i64 → usize/u64/isize`.
/// - Everything else: `.into()` / `.map(Into::into)`.
fn flat_enum_binding_to_core_field_expr(f: &crate::core::ir::FieldDef, flat_name: &str) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};

    if f.sanitized {
        return if f.is_boxed {
            "Box::default()".to_string()
        } else {
            "Default::default()".to_string()
        };
    }

    let expr = match &f.ty {
        TypeRef::Path => {
            if f.optional {
                format!("val.{flat_name}.map(std::path::PathBuf::from)")
            } else {
                format!("val.{flat_name}.map(std::path::PathBuf::from).unwrap_or_default()")
            }
        }
        TypeRef::Primitive(p @ (PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::Isize)) => {
            let core_ty = match p {
                PrimitiveType::Usize => "usize",
                PrimitiveType::U64 => "u64",
                PrimitiveType::Isize => "isize",
                _ => unreachable!(),
            };
            if f.optional {
                format!("val.{flat_name}.map(|v| v as {core_ty})")
            } else {
                format!("val.{flat_name}.map(|v| v as {core_ty}).unwrap_or_default()")
            }
        }
        TypeRef::Primitive(_) | TypeRef::String => {
            if f.optional {
                format!("val.{flat_name}")
            } else {
                format!("val.{flat_name}.unwrap_or_default()")
            }
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if f.optional {
                format!("val.{flat_name}.map(|v| v.into_iter().map(Into::into).collect())")
            } else {
                format!("val.{flat_name}.map(|v| v.into_iter().map(Into::into).collect()).unwrap_or_default()")
            }
        }
        _ => {
            if f.optional {
                format!("val.{flat_name}.map(Into::into)")
            } else {
                format!("val.{flat_name}.map(Into::into).unwrap_or_default()")
            }
        }
    };

    if f.is_boxed { format!("Box::new({expr})") } else { expr }
}
