use crate::backends::rustler::template_env;
use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::{
    VariantDeclaration, enum_conversion_needs_catch_all_for_features, enum_variant_declaration,
};
use crate::codegen::shared::binding_fields;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use ahash::AHashSet;

/// Whether `variant`'s `#[cfg(...)]` is safe to re-emit verbatim on its match arm.
///
/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate; this Rustler crate never declares a Cargo feature for it (see
/// `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim as `#[cfg(feature = "...")]` is
/// an `unexpected cfg condition value` error. Such a variant is dropped entirely instead --
/// named and counted via `tracing::debug!`, not silently; `codegen::foreign_cfg_variants` raises
/// the same fact to WARN once for the whole run -- mirroring
/// `codegen::conversions::enums::emit_cfg_gated_arm`. A host-owned cfg keeps its arm and its
/// `#[cfg(...)]`: forwarding already declared that feature, so the gate is valid. ~keep
fn rustler_flat_variant_kept(enum_def: &EnumDef, variant: &EnumVariant, is_host_enum: bool, direction: &str) -> bool {
    if variant.cfg.is_none() || is_host_enum {
        return true;
    }
    tracing::debug!(
        enum_name = %enum_def.name,
        enum_rust_path = %enum_def.rust_path,
        variant_name = %variant.name,
        cfg = variant.cfg.as_deref().unwrap_or_default(),
        direction = direction,
        "dropping Rustler flat-data-enum conversion match arm for a foreign-crate variant behind \
         a #[cfg(...)] this crate cannot declare as a Cargo feature; the variant is unreachable \
         from this conversion"
    );
    false
}

/// Generate a Rustler opaque resource wrapper for a type.
pub(super) fn gen_opaque_resource(typ: &TypeDef, core_import: &str, _opaque_types: &AHashSet<String>) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("#[derive(Clone)]\n");
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    out.push_str(&template_env::render(
        "rust_opaque_struct.jinja",
        minijinja::context! {
            struct_name => &typ.name,
            core_path => &core_path,
        },
    ));
    out
}

/// Generate a Rustler NIF struct definition using the shared TypeMapper.
/// Rustler 0.37: NifStruct is a derive macro with #[module = "..."] attribute.
///
/// Fields listed in `exclude_fields` are omitted from the generated struct —
/// used to skip bridge fields (e.g. visitor) that are handled at the Elixir layer
/// and cannot implement Rustler's Encoder/Decoder traits.
///
/// `delegatable_deserialize_types` is the "core→binding convertible" set already used to gate
/// `gen_from_core_to_binding_cfg` in `native.rs` (`core_to_binding_for_deserialize`) -- reusing
/// it here guarantees the `From<core::Type>` a delegating `Deserialize` needs is always emitted
/// for any type it names. See `struct_wants_deserialize_delegation`'s doc comment. ~keep
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &crate::backends::rustler::type_map::RustlerMapper,
    module_prefix: &str,
    exclude_fields: &AHashSet<String>,
    core_import: &str,
    opaque_type_names: &[String],
    delegatable_deserialize_types: &AHashSet<String>,
) -> String {
    let mut out = String::with_capacity(512);
    let delegate_deserialize = delegatable_deserialize_types.contains(&typ.name)
        && crate::codegen::generators::struct_wants_deserialize_delegation(typ, opaque_type_names, &[]);
    let nif_derive = if typ.has_default {
        "rustler::NifMap"
    } else {
        "rustler::NifStruct"
    };
    let visible_fields: Vec<&FieldDef> = binding_fields(&typ.fields)
        .filter(|field| !exclude_fields.contains(&field.name))
        .collect();
    out.push_str(&template_env::render(
        "rust_struct_derive_line.jinja",
        minijinja::context! {
            delegate_deserialize => delegate_deserialize,
            nif_derive => nif_derive,
            has_sensitive => visible_fields.iter().any(|field| field.sensitive),
        },
    ));
    if !typ.has_default {
        out.push_str(&template_env::render(
            "rust_module_attr.jinja",
            minijinja::context! {
                module_prefix => module_prefix,
                struct_name => &typ.name,
            },
        ));
    }
    out.push_str(&template_env::render(
        "rust_struct_header.jinja",
        minijinja::context! {
            struct_name => &typ.name,
        },
    ));

    for field in &visible_fields {
        let field_type = if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        out.push_str(&template_env::render(
            "rust_struct_field.jinja",
            minijinja::context! {
                name => crate::codegen::naming::internal_rust_identifier(&field.name),
                type => &field_type,
            },
        ));
    }

    out.push_str("}\n");
    out.push_str(&crate::codegen::generators::gen_redacted_struct_debug_impl(
        &typ.name,
        None,
        visible_fields.iter().copied(),
    ));
    if delegate_deserialize {
        out.push_str(&crate::codegen::generators::gen_delegating_deserialize_impl(
            typ,
            core_import,
            "",
            &[],
        ));
    }
    out
}

/// Generate a Rustler config constructor impl for a type with `has_default`.
/// Fields in `exclude_fields` are skipped in both the struct and the constructor.
pub(super) fn gen_rustler_config_impl(
    typ: &TypeDef,
    mapper: &crate::backends::rustler::type_map::RustlerMapper,
    exclude_fields: &AHashSet<String>,
) -> String {
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "rust_impl_header.jinja",
        minijinja::context! {
            struct_name => &typ.name,
        },
    ));

    let excl_std: std::collections::HashSet<String> = exclude_fields.iter().cloned().collect();
    let map_fn = |ty: &TypeRef| mapper.map_type(ty);
    let config_method =
        crate::codegen::config_gen::gen_rustler_kwargs_constructor_with_exclude(typ, &map_fn, &excl_std);
    out.push_str(config_method.trim_start());

    out.push_str("}\n");
    out
}

/// Map a field's TypeRef to a Rust type string suitable for use in a Rustler NifTaggedEnum variant.
/// Named types are emitted by short name (the binding-layer mirror struct).
/// Vec<Named> recurses so JSON arrays round-trip as actual arrays.
/// Map and complex types collapse to String for JSON round-trip.
fn field_type_for_rustler(field: &FieldDef) -> String {
    let base = field_type_for_rustler_inner(&field.ty);
    if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
        format!("Option<{base}>")
    } else {
        base
    }
}

fn field_type_for_rustler_inner(ty: &TypeRef) -> String {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "u8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "u16".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "u32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "u64".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "usize".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "i8".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "i16".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "i32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "i64".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "isize".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "f32".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "f64".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Vec(inner) => format!("Vec<{}>", field_type_for_rustler_inner(inner)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            field_type_for_rustler_inner(k),
            field_type_for_rustler_inner(v)
        ),
        TypeRef::Optional(inner) => format!("Option<{}>", field_type_for_rustler_inner(inner)),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Return the serde wire name for a variant, applying serde_rename_all if set.
fn variant_wire_name(variant: &crate::core::ir::EnumVariant, enum_def: &EnumDef) -> String {
    crate::codegen::naming::wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    )
}

/// Generate a Rustler flat struct enum for data enums with tuple fields containing Named types.
///
/// Instead of NifTaggedEnum with `{:Variant, inner_data}` tuples, this generates:
/// - A flat NifStruct with all variant inner types as optional fields
/// - A discriminator string field for the variant type
/// - From impls that populate the appropriate field and set the discriminator
///
/// Example: Shape enum with Excel(ExcelMetadata) → struct with `type: String` and
/// `excel: Option<ExcelMetadata>`, with other optional fields (see
/// [`super::helpers::flat_data_enum_discriminator`] for the `"type"` fallback name).
/// `declared_variants` must be the SAME filtered list `gen_enum` (the caller) already computed
/// via `enum_variant_declaration` -- not re-derived here -- so the flat `NifStruct` this function
/// declares can never disagree with the `NifUnitEnum`/`NifTaggedEnum` declaration the sibling
/// branches emit for the same cfg-gated-variant question. Before this fix this function read
/// `enum_def.variants` directly and had no cfg awareness at all: a foreign cfg-gated variant this
/// binding's own configured feature set proves unreachable still got its own `pub {name}:
/// Option<T>` field on the struct -- an Elixir caller could set that key in the map handed to the
/// NIF and it would silently decode as a value the real conversion (`rustler_flat_variant_kept`
/// in this file's `From` impls) never produces, the same declaration/conversion mismatch the
/// unit/tagged branches had. ~keep
fn gen_rustler_flat_data_enum(enum_def: &EnumDef, module_prefix: &str, declared_variants: &[&EnumVariant]) -> String {
    let name = &enum_def.name;
    let mut out = String::with_capacity(1024);

    out.push_str(&template_env::render(
        "flat_enum_derive.jinja",
        minijinja::context! {
            has_sensitive => crate::codegen::generators::enum_has_sensitive_representation(enum_def),
        },
    ));
    out.push_str(&template_env::render(
        "flat_enum_struct_header.jinja",
        minijinja::context! {
            module_prefix => module_prefix,
            name => name,
        },
    ));

    let discriminator_field =
        crate::codegen::naming::internal_rust_identifier(super::helpers::flat_data_enum_discriminator(enum_def));
    out.push_str(&template_env::render(
        "flat_enum_discriminator_field.jinja",
        minijinja::context! {
            discriminator_field => discriminator_field,
        },
    ));

    for variant in declared_variants {
        if !variant.fields.is_empty()
            && variant.is_tuple
            && let Some(first_field) = variant.fields.first()
        {
            let field_name = crate::codegen::naming::pascal_to_snake(&variant.name);
            let field_type = field_type_for_rustler(first_field);
            out.push_str(&template_env::render(
                "flat_enum_variant_field.jinja",
                minijinja::context! {
                    field_name => &field_name,
                    field_type => &field_type,
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_struct_footer.jinja",
        minijinja::context! {},
    ));

    out.push_str(&template_env::render(
        "flat_enum_default_impl.jinja",
        minijinja::context! {
            name => name,
            discriminator_field => discriminator_field,
        },
    ));

    for variant in declared_variants {
        if !variant.fields.is_empty() && variant.is_tuple {
            let field_name = crate::codegen::naming::pascal_to_snake(&variant.name);
            out.push_str(&template_env::render(
                "flat_enum_default_variant_field.jinja",
                minijinja::context! {
                    field_name => &field_name,
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_default_impl_footer.jinja",
        minijinja::context! {},
    ));
    out.push_str(&crate::codegen::generators::gen_redacted_enum_debug_impl(enum_def));

    out
}

/// Generate a `From<core::EnumName> for FlatStruct` impl for flat data enums.
///
/// The generic `gen_enum_from_core_to_binding_cfg` generates enum→enum arm matching, which
/// does not apply to flat structs. This function generates the correct struct-init form:
///
/// ```text
/// impl From<core::FormatMetadata> for FormatMetadata {
///     fn from(val: core::FormatMetadata) -> Self {
///         match val {
///             core::FormatMetadata::Excel(_0) => Self {
///                 r#type: "Excel".to_string(), excel: Some(_0.into()), ..Default::default()
///             },
///             ...
///         }
///     }
/// }
/// ```
pub(super) fn gen_rustler_flat_data_enum_from_core(
    enum_def: &EnumDef,
    core_import: &str,
    configured_features: Option<&[String]>,
) -> String {
    let name = &enum_def.name;
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let discriminator =
        crate::codegen::naming::internal_rust_identifier(super::helpers::flat_data_enum_discriminator(enum_def));
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "flat_enum_from_core_impl.jinja",
        minijinja::context! {
            core_path => &core_path,
            name => name,
        },
    ));

    for variant in &enum_def.variants {
        if !rustler_flat_variant_kept(enum_def, variant, is_host_enum, "from_core") {
            continue;
        }
        let field_name = crate::codegen::naming::pascal_to_snake(&variant.name);
        let wire_name = variant_wire_name(variant, enum_def);

        if variant.fields.is_empty() {
            out.push_str(&template_env::render(
                "flat_enum_from_core_variant_unit.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                    cfg => variant.cfg.clone(),
                },
            ));
        } else if variant.is_tuple {
            let first_field = variant.fields.first().unwrap();
            let is_boxed = first_field.is_boxed;
            let is_sanitized_to_string = first_field.sanitized && matches!(first_field.ty, TypeRef::String);
            let is_vec_of_named =
                matches!(&first_field.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)));
            let is_string_field = matches!(first_field.ty, TypeRef::String) && !is_sanitized_to_string;
            let data_expr: String = if is_sanitized_to_string {
                if is_boxed {
                    "format!(\"{:?}\", *_0)".to_string()
                } else {
                    "format!(\"{:?}\", _0)".to_string()
                }
            } else if is_vec_of_named {
                if is_boxed {
                    "(*_0).into_iter().map(Into::into).collect()".to_string()
                } else {
                    "_0.into_iter().map(Into::into).collect()".to_string()
                }
            } else if is_string_field {
                if is_boxed {
                    "(*_0)".to_string()
                } else {
                    "_0".to_string()
                }
            } else if is_boxed {
                "(*_0).into()".to_string()
            } else {
                "_0.into()".to_string()
            };
            // `..Default::default()` triggers clippy::needless_update. When there are
            let tuple_variant_count = enum_def.variants.iter().filter(|v| v.is_tuple).count();
            let all_fields_specified = tuple_variant_count == 1;
            out.push_str(&template_env::render(
                "flat_enum_from_core_variant_tuple.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                    fname => &field_name,
                    expr => &data_expr,
                    all_fields_specified => all_fields_specified,
                    cfg => variant.cfg.clone(),
                },
            ));
        }
    }

    // Previously triggered on ANY cfg-gated variant, host-owned or not, and papered over the
    // resulting `unreachable_patterns` (rather than the genuinely non-exhaustive case this
    // catch-all exists for) with `#[allow(unreachable_patterns)]`. A host-owned variant's arm
    // carries the identical `#[cfg(...)]` as the variant itself (see `rustler_flat_variant_kept`
    // above), so the two always compile in or out together and the match stays exhaustive either
    // way. A FOREIGN cfg-gated variant's arm is unconditionally dropped instead (same function),
    // so whether a catch-all is still needed for it depends on whether this binding's own
    // configured feature set proves the variant unreachable -- delegated to
    // `codegen::conversions::enum_conversion_needs_catch_all_for_features`, the same resolver
    // every `ConversionConfig`-driven enum conversion already uses, so this bespoke flat-data
    // generator can't drift from that verdict (alef #544). ~keep
    // This match is over the real CORE type (`core_path` above) -- a shape this generator does
    // not declare and cannot influence, so `configured_features`' proof about the dependency is
    // already the complete answer regardless of any binding declaration. `true` here. See
    // `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    if enum_conversion_needs_catch_all_for_features(
        enum_def,
        is_host_enum,
        !enum_def.excluded_variants.is_empty(),
        configured_features,
        true,
    ) {
        out.push_str("            _ => Self::default(),\n");
    }

    out.push_str(&template_env::render(
        "flat_enum_from_core_impl_footer.jinja",
        minijinja::context! {},
    ));

    out
}

/// Generate the binding-to-core direction for a flat data enum.
///
/// Local representation is a struct with a discriminator field plus one optional payload
/// field per variant. Dispatches on the discriminator string to produce the matching core
/// enum variant, threading per-payload conversions (`.into()`, or iter-map for `Vec<Named>`).
pub(super) fn gen_rustler_flat_data_enum_to_core(enum_def: &EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let discriminator =
        crate::codegen::naming::internal_rust_identifier(super::helpers::flat_data_enum_discriminator(enum_def));
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "flat_enum_to_core_impl_header.jinja",
        minijinja::context! {
            name => name,
            core_path => &core_path,
            discriminator => discriminator,
        },
    ));

    for variant in &enum_def.variants {
        if !rustler_flat_variant_kept(enum_def, variant, is_host_enum, "to_core") {
            continue;
        }
        let field_name = crate::codegen::naming::pascal_to_snake(&variant.name);
        let wire_name = variant_wire_name(variant, enum_def);

        if variant.fields.is_empty() {
            out.push_str(&template_env::render(
                "flat_enum_to_core_variant_unit.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    core_path => &core_path,
                    variant_name => &variant.name,
                    cfg => variant.cfg.clone(),
                },
            ));
        } else if variant.is_tuple {
            let first_field = variant.fields.first().unwrap();
            let is_boxed = first_field.is_boxed;
            let is_sanitized_to_string = first_field.sanitized && matches!(first_field.ty, TypeRef::String);
            let is_vec_of_named =
                matches!(&first_field.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)));
            let is_string_field = matches!(first_field.ty, TypeRef::String) && !is_sanitized_to_string;
            let payload_expr = if is_sanitized_to_string {
                "Default::default()".to_string()
            } else if is_vec_of_named {
                format!("val.{field_name}.unwrap_or_default().into_iter().map(Into::into).collect()")
            } else if is_string_field {
                format!("val.{field_name}.unwrap_or_default()")
            } else {
                format!("val.{field_name}.unwrap_or_default().into()")
            };
            let payload_expr = if is_boxed {
                format!("Box::new({payload_expr})")
            } else {
                payload_expr
            };
            out.push_str(&template_env::render(
                "flat_enum_to_core_variant_tuple.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    core_path => &core_path,
                    variant_name => &variant.name,
                    payload_expr => &payload_expr,
                    cfg => variant.cfg.clone(),
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_to_core_impl_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Generate a Rustler NIF enum definition.
///
/// Unit enums (all variants have no fields) use `NifUnitEnum` — they encode as atoms.
/// Data enums where all tuple variants contain Named types (structs) use flat NifStruct
/// with optional fields. Other data enums use `NifTaggedEnum`.
///
/// `api` is consulted only to decide whether the emitted `NifTaggedEnum` needs a narrow
/// `#[expect(clippy::large_enum_variant, ...)]` -- see
/// [`crate::codegen::enum_variant_size::enum_should_expect_large_variant_lint`] for the
/// heuristic and why it must stay conservative about emitting the attribute (alef #545).
///
/// `core_import`/`configured_features` decide which variants this NIF declaration keeps, via
/// the same [`enum_variant_declaration`] authority every conversion arm below (and every other
/// Rust-emitting backend's own wrapper declaration) already consults: a FOREIGN cfg-gated
/// variant this binding's own configured feature set proves unreachable is dropped from the
/// declaration itself, not just from the `From` impls. Before this fix the declaration always
/// kept every variant regardless of `configured_features` while the conversions already dropped
/// the unreachable ones -- an Elixir caller could pass an atom the NIF decoder never actually
/// matches, decoding as `badarg` instead of failing to compile. A host-owned cfg-gated variant is
/// still always kept (`enum_variant_declaration` never resolves a host-owned gate to `Drop`),
/// matching every other backend's declaration surface. ~keep
pub(super) fn gen_enum(
    enum_def: &EnumDef,
    module_prefix: &str,
    api: &ApiSurface,
    core_import: &str,
    configured_features: Option<&[String]>,
) -> String {
    let name = &enum_def.name;
    let mut out = String::with_capacity(512);

    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let configured_features_set: Option<std::collections::HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    let declared_variants: Vec<&EnumVariant> = enum_def
        .variants
        .iter()
        .filter(|variant| {
            !matches!(
                enum_variant_declaration(variant, is_host_enum, configured_features_set.as_ref()),
                VariantDeclaration::Drop
            )
        })
        .collect();

    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    let use_flat_struct = super::helpers::is_flat_data_enum(enum_def);

    if use_flat_struct {
        return gen_rustler_flat_data_enum(enum_def, module_prefix, &declared_variants);
    }

    if has_data {
        if crate::codegen::enum_variant_size::enum_should_expect_large_variant_lint(enum_def, api) {
            out.push_str(&template_env::render(
                "nif_tagged_enum_large_variant_expect.jinja",
                minijinja::context! {
                    reason => "this enum's data variants differ substantially in estimated size; \
                               boxing the largest field would change the generated public variant \
                               shape, so the size difference is accepted here instead",
                },
            ));
        }
        if crate::codegen::generators::enum_has_sensitive_representation(enum_def) {
            out.push_str("#[derive(Clone, serde::Serialize, serde::Deserialize, rustler::NifTaggedEnum)]\n");
        } else {
            out.push_str("#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rustler::NifTaggedEnum)]\n");
        }
        if let Some(tag) = &enum_def.serde_tag {
            out.push_str(&template_env::render(
                "nif_tagged_enum_serde_tag.jinja",
                minijinja::context! {
                    tag => tag,
                    serde_content => &enum_def.serde_content,
                    serde_rename_all => &enum_def.serde_rename_all,
                },
            ));
        }
        out.push_str(&template_env::render(
            "nif_tagged_enum_header.jinja",
            minijinja::context! {
                name => name,
            },
        ));
        for variant in &declared_variants {
            if variant.fields.is_empty() {
                out.push_str(&template_env::render(
                    "nif_tagged_enum_variant_unit.jinja",
                    minijinja::context! {
                        variant_name => &variant.name,
                    },
                ));
            } else {
                out.push_str(&template_env::render(
                    "nif_tagged_enum_variant_struct_header.jinja",
                    minijinja::context! {
                        variant_name => &variant.name,
                    },
                ));
                for field in &variant.fields {
                    // Optional fields need #[serde(default)] so Rustler's decoder doesn't fail
                    if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
                        out.push_str(&template_env::render(
                            "nif_tagged_enum_variant_field_attr.jinja",
                            minijinja::context! {
                                attr => "serde(default)",
                            },
                        ));
                    } else if matches!(field.ty, TypeRef::Optional(_)) && field.optional {
                        out.push_str(&template_env::render(
                            "nif_tagged_enum_variant_field_attr.jinja",
                            minijinja::context! {
                                attr => "serde(default)",
                            },
                        ));
                    }
                    let field_type = field_type_for_rustler(field);
                    out.push_str(&template_env::render(
                        "nif_tagged_enum_variant_field_line.jinja",
                        minijinja::context! {
                            field_line => format!(
                                "{}: {},",
                                crate::codegen::naming::internal_rust_identifier(&field.name),
                                field_type,
                            ),
                        },
                    ));
                }
                out.push_str(&template_env::render(
                    "nif_tagged_enum_variant_struct_footer.jinja",
                    minijinja::context! {},
                ));
            }
        }
        out.push_str("}\n");
    } else {
        out.push_str(&template_env::render(
            "nif_unit_enum_header.jinja",
            minijinja::context! {
                name => name,
                has_sensitive => crate::codegen::generators::enum_has_sensitive_representation(enum_def),
            },
        ));
        for variant in &declared_variants {
            out.push_str(&template_env::render(
                "nif_enum_variant.jinja",
                minijinja::context! {
                    variant_name => &variant.name,
                },
            ));
        }
        out.push_str("}\n");
    }
    out.push_str(&crate::codegen::generators::gen_redacted_enum_debug_impl(enum_def));

    let default_variant = declared_variants
        .iter()
        .find(|v| v.is_default)
        .or_else(|| declared_variants.first())
        .copied();
    if let Some(dv) = default_variant {
        out.push_str(&template_env::render(
            "nif_enum_default_header.jinja",
            minijinja::context! {
                name => name,
            },
        ));
        if has_data && !dv.fields.is_empty() {
            let field_defaults: Vec<String> = dv
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            out.push_str(&template_env::render(
                "nif_enum_default_with_fields.jinja",
                minijinja::context! {
                    variant_name => &dv.name,
                    field_defaults => field_defaults.join(", "),
                },
            ));
        } else {
            out.push_str(&template_env::render(
                "nif_enum_default_value.jinja",
                minijinja::context! {
                    variant_name => &dv.name,
                },
            ));
        }
        out.push_str(&template_env::render(
            "nif_enum_default_footer.jinja",
            minijinja::context! {},
        ));
    }

    out
}

/// Wrap a return expression for Rustler (opaque types get ResourceArc wrapping).
pub(super) fn gen_rustler_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    _type_name: &str,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
) -> String {
    match return_type {
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if returns_ref {
                format!("ResourceArc::new({n} {{ inner: Arc::new(std::sync::RwLock::new({expr}.clone())) }})")
            } else {
                format!("ResourceArc::new({n} {{ inner: Arc::new(std::sync::RwLock::new({expr})) }})")
            }
        }
        TypeRef::Named(_) => {
            if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char => {
            if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Bytes => format!("{expr}.to_vec()"),
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Duration => format!("{expr}.as_millis() as u64"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if returns_ref {
                    format!(
                        "{expr}.iter().cloned().map(|v| ResourceArc::new({n} {{ inner: Arc::new(std::sync::RwLock::new(v)) }})).collect()"
                    )
                } else {
                    format!(
                        "{expr}.into_iter().map(|v| ResourceArc::new({n} {{ inner: Arc::new(std::sync::RwLock::new(v)) }})).collect()"
                    )
                }
            }
            TypeRef::Named(_) if returns_ref => {
                format!("{expr}.iter().cloned().map(Into::into).collect()")
            }
            TypeRef::Named(_) => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String | TypeRef::Char if returns_ref => {
                format!("{expr}.iter().map(|s| s.to_string()).collect()")
            }
            _ => expr.to_string(),
        },
        TypeRef::Map(_, _) => {
            if returns_ref {
                format!("{expr}.iter().map(|(k, v)| (k.clone(), v.clone())).collect()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Bytes if returns_ref => {
                format!("{expr}.map(|v| v.into())")
            }
            TypeRef::Path => format!("{expr}.map(|v| v.to_string_lossy().to_string())"),
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if returns_ref {
                    format!(
                        "{expr}.map(|v| ResourceArc::new({n} {{ inner: Arc::new(std::sync::RwLock::new(v.clone())) }}))"
                    )
                } else {
                    format!("{expr}.map(|v| ResourceArc::new({n} {{ inner: Arc::new(std::sync::RwLock::new(v)) }}))")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(|v| v.into())")
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

#[cfg(test)]
mod cfg_gate_tests;
#[cfg(test)]
mod gen_enum_declaration_cfg_tests;
#[cfg(test)]
mod tests;
