//! NAPI-RS tagged-enum From-impl code generation (binding ↔ core conversions).

use crate::{
    codegen::{
        cfg::is_host_owned_rust_path,
        conversions::{
            enum_conversion_needs_catch_all_for_features,
            helpers::{
                sanitized_field_to_binding_expr, sanitized_map_field_to_core_expr, sanitized_vec_field_to_core_expr,
            },
        },
        naming::wire_variant_value,
    },
    core::ir::{EnumDef, EnumVariant, TypeRef},
};

use super::enums::{
    tagged_enum_binding_field_name, tagged_enum_binding_struct_fields, tagged_enum_field_is_tuple,
    tagged_enum_mixed_named_fields, variant_data_field_names,
};
use super::functions::{core_prim_str, needs_napi_cast};

/// Build the binding→core conversion expression for a sanitized tagged-enum field, gated to
/// the specific shapes this backend can invert (`Vec<Vec<String>>` and `Map<String, String>`,
/// matching what `sanitized_vec_field_to_core_expr`/`sanitized_map_field_to_core_expr`
/// support). Every other sanitized shape keeps the pre-#218 `Default::default()` fallback,
/// which always compiles. The binding-side struct field is always `Option<T>` regardless of
/// the core field's own optionality (see `tagged_enum_binding_struct_fields` field emission in
/// `enums.rs`), so `optional` only changes whether the *result* is re-wrapped in `Option<_>`.
fn sanitized_binding_to_core_expr(binding_field_name: &str, ty: &TypeRef, optional: bool) -> String {
    let is_vec_vec_string = matches!(
        ty,
        TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    );
    if is_vec_vec_string {
        return if optional {
            format!(
                "val.{binding_field_name}.map(|v| {})",
                sanitized_vec_field_to_core_expr("v", ty)
            )
        } else {
            sanitized_vec_field_to_core_expr(&format!("val.{binding_field_name}.as_deref().unwrap_or_default()"), ty)
        };
    }
    if optional {
        if let Some(inner) = sanitized_map_field_to_core_expr("m", ty) {
            return format!("val.{binding_field_name}.map(|m| {inner})");
        }
    } else if let Some(expr) =
        sanitized_map_field_to_core_expr(&format!("val.{binding_field_name}.unwrap_or_default()"), ty)
    {
        return expr;
    }
    "Default::default()".to_string()
}

/// Build the core→binding field-init expression for a sanitized tagged-enum field, gated to
/// the same shapes as [`sanitized_binding_to_core_expr`] via
/// `sanitized_field_to_binding_expr`. `f` is the already-destructured core-side variable
/// name; unsupported shapes fall back to `None`, which always compiles (and matches the
/// `destructured` pattern binding that field with an ignored `_`-prefixed name).
fn sanitized_core_to_binding_expr(f: &str, ty: &TypeRef, optional: bool) -> String {
    if optional {
        return match sanitized_field_to_binding_expr("v", ty) {
            Some(inner) => format!("{f}: {f}.map(|v| {inner})"),
            None => format!("{f}: None"),
        };
    }
    match sanitized_field_to_binding_expr(f, ty) {
        Some(expr) => format!("{f}: Some({expr})"),
        None => format!("{f}: None"),
    }
}

/// Whether `variant`'s match arm should be emitted, and with which `#[cfg(...)]` guard.
///
/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate; this NAPI crate never declares a Cargo feature for it (see
/// `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim as `#[cfg(feature = "...")]` is
/// an `unexpected cfg condition value` error. Such an arm is dropped entirely instead --
/// named and counted via `tracing::debug!`, not silently; `codegen::foreign_cfg_variants` raises
/// the same fact to WARN once for the whole run -- mirroring
/// `codegen::conversions::enums::emit_cfg_gated_arm` and
/// `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper`. A host-owned cfg keeps its
/// arm and its `#[cfg(...)]`: forwarding already declared that feature, so the gate is valid. ~keep
fn napi_variant_cfg(enum_def: &EnumDef, variant: &EnumVariant, is_host_enum: bool, direction: &str) -> Option<String> {
    let cfg = variant.cfg.as_deref()?;
    if !is_host_enum {
        tracing::debug!(
            enum_name = %enum_def.name,
            enum_rust_path = %enum_def.rust_path,
            variant_name = %variant.name,
            cfg = cfg,
            direction = direction,
            "dropping NAPI tagged-enum conversion match arm for a foreign-crate variant behind a \
             #[cfg(...)] this crate cannot declare as a Cargo feature; the variant is unreachable \
             from this conversion"
        );
        return None;
    }
    Some(cfg.to_string())
}

/// Generate `From<JsTaggedEnum> for core::TaggedEnum` for a flattened struct representation.
pub(super) fn gen_tagged_enum_binding_to_core(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    struct_names: &ahash::AHashSet<String>,
) -> String {
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    let variants = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let kept = variant.cfg.is_none() || is_host_enum;
            let cfg = napi_variant_cfg(enum_def, variant, is_host_enum, "binding_to_core");
            if !kept {
                return None;
            }
            let tag_value = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
            let is_empty = variant.fields.is_empty();

            Some(if is_empty {
                minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value.to_string(),
                    is_empty => true,
                    is_tuple => false,
                    cfg => cfg,
                }
            } else {
                let field_exprs: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        let binding_field_name = tagged_enum_binding_field_name(enum_def, variant, f);
                        let has_binding = fields_with_binding_struct.contains(f.name.as_str());
                        let is_single_tuple_named = variant.fields.len() == 1
                            && tagged_enum_field_is_tuple(f)
                            && matches!(&f.ty, TypeRef::Named(_));
                        let is_mixed = !is_single_tuple_named && mixed_named_fields.contains(&f.name);
                        if f.sanitized {
                            let expr = sanitized_binding_to_core_expr(&binding_field_name, &f.ty, f.optional);
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        } else if f.optional {
                            match &f.ty {
                                TypeRef::Path => {
                                    format!("val.{binding_field_name}.map(std::path::PathBuf::from)")
                                }
                                TypeRef::Named(n) if is_mixed => {
                                    let core_type = format!("{core_import}::{n}");
                                    format!(
                                        "val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok())",
                                        binding_field_name
                                    )
                                }
                                TypeRef::Named(_) if has_binding => {
                                    format!("val.{binding_field_name}.map(|v| v.into())")
                                }
                                TypeRef::Named(_) => {
                                    format!("val.{binding_field_name}.map(|v| v.into())")
                                }
                                TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                    let core_ty = core_prim_str(p);
                                    format!("val.{binding_field_name}.map(|v| v as {core_ty})")
                                }
                                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                    format!(
                                        "val.{binding_field_name}.map(|v| v.into_iter().map(Into::into).collect())"
                                    )
                                }
                                _ => {
                                    format!("val.{binding_field_name}")
                                }
                            }
                        } else {
                            let expr = match &f.ty {
                                TypeRef::Named(n) if is_mixed => {
                                    let core_type = format!("{core_import}::{n}");
                                    format!(
                                        "val.{}.and_then(|s| serde_json::from_str::<{core_type}>(&s).ok()).unwrap_or_default()",
                                        binding_field_name
                                    )
                                }
                                TypeRef::Named(_) if has_binding => {
                                    format!("val.{binding_field_name}.map(|v| v.into()).unwrap_or_default()")
                                }
                                TypeRef::Named(_) => {
                                    format!("val.{binding_field_name}.map(|v| v.into()).unwrap_or_default()")
                                }
                                TypeRef::Path => {
                                    format!(
                                        "val.{binding_field_name}.map(std::path::PathBuf::from).unwrap_or_default()"
                                    )
                                }
                                TypeRef::Primitive(p) if needs_napi_cast(p) => {
                                    let core_ty = core_prim_str(p);
                                    format!("val.{binding_field_name}.map(|v| v as {core_ty}).unwrap_or_default()")
                                }
                                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                    format!(
                                        "val.{binding_field_name}.map(|v| v.into_iter().map(Into::into).collect()).unwrap_or_default()"
                                    )
                                }
                                // ~keep A byte payload is `napi::bindgen_prelude::Buffer` on the
                                // binding side (see `NapiMapper::bytes`), never `Vec<u8>`, so it
                                // needs an explicit copy back out. The catch-all below produced
                                // `val.f.unwrap_or_default()`, which is a `Buffer` where the core
                                // variant wants a `Vec<u8>` -- E0308 on every data-carrying enum
                                // with a bytes variant.
                                TypeRef::Bytes => {
                                    format!("val.{binding_field_name}.map(|b| b.to_vec()).unwrap_or_default()")
                                }
                                _ => {
                                    format!("val.{binding_field_name}.unwrap_or_default()")
                                }
                            };
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        }
                    })
                    .collect();

                let field_inits: Vec<String> = variant
                    .fields
                    .iter()
                    .zip(field_exprs.iter())
                    .map(|(f, expr)| format!("{}: {expr}", f.name))
                    .collect();

                minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value,
                    is_empty => false,
                    is_tuple => is_tuple,
                    field_exprs => field_exprs,
                    field_inits => field_inits,
                    cfg => cfg,
                }
            })
        })
        .collect::<Vec<_>>();

    // Prefer the first variant with no cfg gate as the unconditional `_ =>` fallback: a cfg-gated
    // variant (host-owned or foreign) may not exist in every build, so it cannot safely stand in
    // as the always-available default. Falls back to the very first variant only when every
    // variant carries a cfg -- matching the pre-existing behavior for an all-gated enum, which
    // was already relying on at least one feature subset making it available. ~keep
    let default_variant = enum_def
        .variants
        .iter()
        .find(|v| v.cfg.is_none())
        .or_else(|| enum_def.variants.first())
        .map(|first| {
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&first.fields);
            let is_empty = first.fields.is_empty();

            if is_empty {
                minijinja::context! {
                    name => first.name.clone(),
                    is_empty => true,
                    is_tuple => false,
                }
            } else if is_tuple {
                let defaults: Vec<&str> = first.fields.iter().map(|_| "Default::default()").collect();
                minijinja::context! {
                    name => first.name.clone(),
                    is_empty => false,
                    is_tuple => true,
                    defaults => defaults,
                }
            } else {
                let default_fields: Vec<String> = first
                    .fields
                    .iter()
                    .map(|f| format!("{}: Default::default()", f.name))
                    .collect();
                minijinja::context! {
                    name => first.name.clone(),
                    is_empty => false,
                    is_tuple => false,
                    default_fields => default_fields,
                }
            }
        });

    crate::backends::napi::template_env::render(
        "gen_tagged_enum_binding_to_core.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            tag_field => tag_field,
            variants => variants,
            default_variant => default_variant,
        },
    )
}

/// Generate `From<core::TaggedEnum> for JsTaggedEnum` for a flattened struct representation.
pub(super) fn gen_tagged_enum_core_to_binding(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    struct_names: &ahash::AHashSet<String>,
    configured_features: Option<&[String]>,
) -> String {
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let fields_with_binding_struct = tagged_enum_binding_struct_fields(enum_def, struct_names);
    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    let all_fields: Vec<String> = {
        let mut fields = std::collections::BTreeSet::new();
        for v in &enum_def.variants {
            for f in &v.fields {
                if tagged_enum_field_is_tuple(f) && matches!(&f.ty, crate::core::ir::TypeRef::Named(_)) {
                    continue;
                }
                fields.insert(tagged_enum_binding_field_name(enum_def, v, f));
            }
        }
        fields.into_iter().collect()
    };

    let synth_field_names = variant_data_field_names(enum_def);

    let variants = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            let kept = variant.cfg.is_none() || is_host_enum;
            let cfg = napi_variant_cfg(enum_def, variant, is_host_enum, "core_to_binding");
            if !kept {
                return None;
            }
            let tag_value = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            let this_synth_field = if variant.fields.len() == 1 {
                let field = &variant.fields[0];
                if tagged_enum_field_is_tuple(field) && matches!(&field.ty, crate::core::ir::TypeRef::Named(_)) {
                    Some(tagged_enum_binding_field_name(enum_def, variant, field))
                } else {
                    None
                }
            } else {
                None
            };

            if variant.fields.is_empty() {
                let mut all_fields_none: Vec<String> = all_fields.iter().map(|f| format!("{f}: None")).collect();
                for sf in &synth_field_names {
                    all_fields_none.push(format!("{sf}: None"));
                }
                Some(minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value.to_string(),
                    is_empty => true,
                    is_tuple => false,
                    all_fields_none => all_fields_none,
                    cfg => cfg,
                })
            } else {
                use crate::core::ir::TypeRef;
                let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
                let variant_field_map: std::collections::BTreeMap<String, &crate::core::ir::FieldDef> = variant
                    .fields
                    .iter()
                    .map(|f| (tagged_enum_binding_field_name(enum_def, variant, f), f))
                    .collect();
                let destructured: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        let binding_field_name = tagged_enum_binding_field_name(enum_def, variant, f);
                        if f.sanitized && sanitized_field_to_binding_expr("_", &f.ty).is_none() {
                            if is_tuple {
                                format!("_{binding_field_name}")
                            } else {
                                format!("{}: _{}", f.name, f.name)
                            }
                        } else {
                            binding_field_name
                        }
                    })
                    .collect();
                let mut field_inits: Vec<String> = all_fields
                    .iter()
                    .map(|f| {
                        if let Some(field) = variant_field_map.get(f) {
                            let has_binding = fields_with_binding_struct.contains(f.as_str());
                            let is_mixed = mixed_named_fields.contains(field.name.as_str());
                            let boxed_deref = if field.is_boxed { "*" } else { "" };
                            if field.sanitized {
                                sanitized_core_to_binding_expr(f, &field.ty, field.optional)
                            } else if field.optional {
                                match &field.ty {
                                    TypeRef::Path => format!("{f}: {f}.map(|p| p.to_string_lossy().to_string())"),
                                    TypeRef::Named(_) if is_mixed => {
                                        format!("{f}: {f}.and_then(|v| serde_json::to_string(&v).ok())")
                                    }
                                    TypeRef::Named(_) if has_binding => {
                                        format!("{f}: {f}.map(|v| (*v).into())",)
                                    }
                                    TypeRef::Named(_) => {
                                        format!("{f}: {f}.map(|v| v.into())")
                                    }
                                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                        format!("{f}: {f}.map(|v| v.into_iter().map(Into::into).collect())")
                                    }
                                    // ~keep See the required-field arm: the binding side is a
                                    // `Buffer`, so a byte payload needs a conversion here too.
                                    TypeRef::Bytes => format!("{f}: {f}.map(Into::into)"),
                                    // No cast or wrap needed: the destructured binding is
                                    // already named `f`, identical to the field it fills, so
                                    // this is true field-init shorthand, not `f: f`.
                                    _ => f.clone(),
                                }
                            } else {
                                match &field.ty {
                                    TypeRef::Named(_) if is_mixed => {
                                        format!("{f}: serde_json::to_string(&{f}).ok()")
                                    }
                                    TypeRef::Named(_) if has_binding => {
                                        format!("{f}: Some(({boxed_deref}{f}).into())")
                                    }
                                    TypeRef::Named(_) => format!("{f}: Some(({boxed_deref}{f}).into())"),
                                    TypeRef::Path => format!("{f}: Some({f}.to_string_lossy().to_string())"),
                                    TypeRef::Primitive(p) if needs_napi_cast(p) => match p {
                                        crate::core::ir::PrimitiveType::F32 => format!("{f}: Some({f} as f64)"),
                                        crate::core::ir::PrimitiveType::U64
                                        | crate::core::ir::PrimitiveType::Usize
                                        | crate::core::ir::PrimitiveType::Isize => format!("{f}: Some({f} as i64)"),
                                        _ => format!("{f}: Some({f})"),
                                    },
                                    TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                                        format!("{f}: Some({f}.into_iter().map(Into::into).collect())")
                                    }
                                    // ~keep The binding field is `Option<Buffer>`, not
                                    // `Option<Vec<u8>>` (see `NapiMapper::bytes`), so the core
                                    // `Vec<u8>` has to be converted. `Some({f})` typechecked only
                                    // while byte payloads were being dropped entirely.
                                    TypeRef::Bytes => format!("{f}: Some({f}.into())"),
                                    _ => format!("{f}: Some({f})"),
                                }
                            }
                        } else {
                            format!("{f}: None")
                        }
                    })
                    .collect();
                for sf in &synth_field_names {
                    if this_synth_field.as_deref() == Some(sf.as_str()) {
                        let field = &variant.fields[0];
                        let var_name = tagged_enum_binding_field_name(enum_def, variant, field);
                        let is_boxed = field.is_boxed;
                        if is_boxed {
                            field_inits.push(format!("{sf}: Some((*{var_name}).into())"));
                        } else {
                            field_inits.push(format!("{sf}: Some({var_name}.into())"));
                        }
                    } else {
                        field_inits.push(format!("{sf}: None"));
                    }
                }

                Some(minijinja::context! {
                    name => variant.name.clone(),
                    tag_value => tag_value,
                    is_empty => false,
                    is_tuple => is_tuple,
                    destructured => destructured,
                    field_inits => field_inits,
                    cfg => cfg,
                })
            }
        })
        .collect::<Vec<_>>();

    // A foreign cfg-gated variant's arm is dropped unconditionally (see `napi_variant_cfg`
    // above), so whether a catch-all is still needed for it depends on whether this binding's own
    // configured feature set proves the variant unreachable -- delegated to
    // `codegen::conversions::enum_conversion_needs_catch_all_for_features`, the same resolver
    // every `ConversionConfig`-driven enum conversion already uses, so this bespoke
    // tagged-data-enum generator can't drift from that verdict (alef #547). This match is over
    // the real CORE type (`core_path` above) -- the flattened `#[napi(object)]` struct this
    // generator's sibling `gen_tagged_enum_as_object` declares is never matched over here, so
    // `configured_features`' proof about the dependency is already the complete answer. `true`
    // here. See `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    let has_excluded_variants = enum_conversion_needs_catch_all_for_features(
        enum_def,
        is_host_enum,
        !enum_def.excluded_variants.is_empty(),
        configured_features,
        true,
    );

    crate::backends::napi::template_env::render(
        "gen_tagged_enum_core_to_binding.jinja",
        minijinja::context! {
            binding_name => binding_name,
            core_path => core_path,
            tag_field => tag_field,
            variants => variants,
            has_excluded_variants => has_excluded_variants,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
    use crate::core::ir::{EnumDef, EnumVariant};

    fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    fn tagged_enum(rust_path: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: "VisitorResult".to_string(),
            rust_path: rust_path.to_string(),
            variants,
            serde_tag: Some("type".to_string()),
            ..Default::default()
        }
    }

    /// The regression this task fixes: `gen_tagged_enum_binding_to_core` and
    /// `gen_tagged_enum_core_to_binding` referenced every variant unconditionally, regardless of
    /// `EnumVariant::cfg` -- E0599 in a build excluding a gated variant's feature. A host-owned
    /// cfg-gated variant must now keep its arm, gated with `#[cfg(...)]`, in both directions.
    ///
    /// A second, later regression lived right next to this one: `gen_tagged_enum_core_to_binding`
    /// used to add the `_ => Default::default()` catch-all whenever ANY variant carried a cfg,
    /// host-owned or not. A host-owned variant's arm carries the identical `#[cfg(...)]` as the
    /// variant itself, so the two always compile in or out together and the match stays
    /// exhaustive either way -- the catch-all is unreachable and trips `-D warnings`'
    /// `unreachable_patterns` the moment the gating feature is active (the default once cfg
    /// features are forwarded, alef #464). `gen_tagged_enum_binding_to_core` matches on a string
    /// tag, not a Rust enum, so it always needs (and always emits, via its own
    /// `default_variant` mechanism) a fallback regardless of cfg -- only the core_to_binding
    /// (real Rust enum match) direction is asserted here.
    #[test]
    fn host_owned_cfg_variant_keeps_its_arm_and_gate_in_both_directions() {
        let en = tagged_enum(
            "mylib::VisitorResult",
            vec![
                unit_variant("Continue", None),
                unit_variant("Thumbnail", Some(r#"feature = "thumbnails""#)),
            ],
        );
        let struct_names = ahash::AHashSet::new();

        let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Js", &struct_names);
        assert!(
            binding_to_core.contains("Self::Thumbnail"),
            "the host-owned variant's arm must still be emitted, got:\n{binding_to_core}"
        );
        assert_eq!(
            binding_to_core.matches("#[cfg(feature = \"thumbnails\")]").count(),
            1,
            "the host-owned variant's arm must carry its #[cfg] guard exactly once, got:\n{binding_to_core}"
        );

        let core_to_binding = gen_tagged_enum_core_to_binding(&en, "mylib", "Js", &struct_names, None);
        assert!(
            core_to_binding.contains("mylib::VisitorResult::Thumbnail"),
            "the host-owned variant's arm must still be emitted, got:\n{core_to_binding}"
        );
        assert_eq!(
            core_to_binding.matches("#[cfg(feature = \"thumbnails\")]").count(),
            1,
            "the host-owned variant's arm must carry its #[cfg] guard exactly once, got:\n{core_to_binding}"
        );
        assert!(
            !core_to_binding.contains("_ => Default::default()"),
            "a host-owned cfg-gated variant must not trigger a catch-all (unreachable pattern \
             under -D warnings), got:\n{core_to_binding}"
        );
    }

    /// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's
    /// own cfg gate. Forwarding it as `#[cfg(...)]` names a feature this NAPI crate never
    /// declares -- an `unexpected cfg condition value` warning -- so the arm must be dropped
    /// entirely instead, mirroring `codegen::conversions::enums::emit_cfg_gated_arm`.
    #[test]
    fn foreign_owned_cfg_variant_arm_is_dropped_not_gated_in_both_directions() {
        let en = tagged_enum(
            "dep_crate::VisitorResult",
            vec![
                unit_variant("Continue", None),
                unit_variant("Testkit", Some(r#"feature = "testkit""#)),
            ],
        );
        let struct_names = ahash::AHashSet::new();

        let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Js", &struct_names);
        assert!(
            !binding_to_core.contains("#[cfg(feature = \"testkit\")]"),
            "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{binding_to_core}"
        );
        assert!(
            !binding_to_core.contains("Self::Testkit"),
            "a foreign-crate cfg-gated variant must not be referenced, got:\n{binding_to_core}"
        );

        let core_to_binding = gen_tagged_enum_core_to_binding(&en, "mylib", "Js", &struct_names, None);
        assert!(
            !core_to_binding.contains("#[cfg(feature = \"testkit\")]"),
            "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{core_to_binding}"
        );
        assert!(
            !core_to_binding.contains("::Testkit"),
            "a foreign-crate cfg-gated variant must not be referenced, got:\n{core_to_binding}"
        );
        assert!(
            core_to_binding.contains("_ => Default::default()"),
            "dropping the arm must still leave the match exhaustive via the catch-all, got:\n{core_to_binding}"
        );
    }

    /// Negative control: an ungated enum emits no `#[cfg(...)]` at all.
    #[test]
    fn ungated_enum_emits_no_cfg_in_either_direction() {
        let en = tagged_enum(
            "mylib::VisitorResult",
            vec![unit_variant("Continue", None), unit_variant("Skip", None)],
        );
        let struct_names = ahash::AHashSet::new();

        let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Js", &struct_names);
        assert!(
            !binding_to_core.contains("#[cfg("),
            "ungated enum must not emit #[cfg(...)], got:\n{binding_to_core}"
        );

        let core_to_binding = gen_tagged_enum_core_to_binding(&en, "mylib", "Js", &struct_names, None);
        assert!(
            !core_to_binding.contains("#[cfg("),
            "ungated enum must not emit #[cfg(...)], got:\n{core_to_binding}"
        );
    }

    /// A cfg-gated first variant must not be chosen as the unconditional `_ =>` default in
    /// `gen_tagged_enum_binding_to_core` -- the fallback must skip to the next ungated variant.
    #[test]
    fn default_variant_skips_a_cfg_gated_first_variant() {
        let en = tagged_enum(
            "mylib::VisitorResult",
            vec![
                unit_variant("Thumbnail", Some(r#"feature = "thumbnails""#)),
                unit_variant("Continue", None),
            ],
        );
        let struct_names = ahash::AHashSet::new();

        let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Js", &struct_names);
        assert!(
            binding_to_core.contains("_ => Self::Continue,"),
            "the unconditional default must fall back to the ungated variant, got:\n{binding_to_core}"
        );
    }
}
