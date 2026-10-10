use crate::backends::ffi::type_map::c_return_type_with_paths;
use crate::codegen::conversions::{core_enum_path, core_type_path};
use crate::codegen::naming::{pascal_to_snake, wire_variant_value};
use crate::core::ir::{CoreWrapper, EnumDef, FieldDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

use super::field_ownership::field_accessor_ownership_lines;
use super::functions::{
    ParamConversionContext, gen_param_conversion_with_enums, optional_borrowed_param_call_arg,
    param_has_explicit_newtype,
};
use super::helpers::{gen_ffi_unimplemented_body, gen_value_to_c, null_return_value};

fn is_primitive_c_type_override(c_type: &str) -> bool {
    matches!(
        c_type,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "f32"
            | "f64"
            | "int"
            | "bool"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "size_t"
            | "ssize_t"
            | "intptr_t"
            | "uintptr_t"
            | "ptrdiff_t"
            | "float"
            | "double"
            | "char"
    )
}

fn c_symbol_component(name: &str) -> String {
    pascal_to_snake(name)
}

pub(super) fn gen_type_from_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let return_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };

    crate::backends::ffi::template_env::render(
        "type_from_json.jinja",
        crate::alef_context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => return_qualified,
            serialized_handle => typ.has_lifetime_params,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

pub(super) fn gen_type_to_json(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let ptr_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };

    crate::backends::ffi::template_env::render(
        "type_to_json.jinja",
        crate::alef_context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => ptr_qualified,
            serialized_handle => typ.has_lifetime_params,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

pub(super) fn gen_type_free(typ: &TypeDef, prefix: &str, core_import: &str) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let ptr_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };

    crate::backends::ffi::template_env::render(
        "type_free.jinja",
        crate::alef_context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => ptr_qualified,
            serialized_handle => typ.has_lifetime_params,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Emit the `extern "C"` accessor that reads one field of `typ` through its opaque handle.
///
/// `[crates.ffi] rename_fields` deliberately does not reach here, and this is the C backend's
/// whole answer to it: `field.name` is not a field identifier on this surface, it is one
/// component of a global C symbol (`{prefix}_{type_snake}_{field_name}`). A C consumer never
/// spells `value.field` — the type crosses the ABI as an opaque handle and every read goes
/// through a function like this one. Renaming the component would rename an exported symbol,
/// which is an ABI break wearing a naming preference's clothes. The C backend therefore has no
/// public DTO field surface for `rename_fields` to govern. ~keep
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_field_accessor(
    typ: &TypeDef,
    field: &FieldDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    clone_names: &AHashSet<String>,
    fields_c_types: &std::collections::HashMap<String, String>,
) -> anyhow::Result<String> {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified_base = core_type_path(typ, core_import);
    let qualified = if typ.has_lifetime_params {
        format!("{qualified_base}<'static>")
    } else {
        qualified_base
    };
    let field_name = &field.name;

    let effective_ty = if field.optional {
        TypeRef::Optional(Box::new(field.ty.clone()))
    } else {
        field.ty.clone()
    };

    let field_core_import = if let Some(ref rust_path) = field.type_rust_path {
        let rust_path_norm = rust_path.replace('-', "_");
        if let Some(pos) = rust_path_norm.rfind("::") {
            let module_prefix = &rust_path_norm[..pos];
            if module_prefix == core_import
                || module_prefix.starts_with(&format!("{core_import}::"))
                || module_prefix.starts_with(&format!("{core_import}_"))
            {
                module_prefix.to_string()
            } else {
                format!("{core_import}::{module_prefix}")
            }
        } else {
            core_import.to_string()
        }
    } else {
        core_import.to_string()
    };

    let lookup_key = format!("{}.{}", type_snake, field.name);
    let c_type_override = fields_c_types.get(&lookup_key).filter(|t| t.as_str() != "skip");
    let (mut ret_type, override_is_opaque_handle, override_type_name) = if let Some(override_type) = c_type_override {
        if !is_primitive_c_type_override(override_type) && override_type != "char*" {
            ("AlefHandle".to_string(), true, Some(override_type.clone()))
        } else {
            (
                c_return_type_with_paths(&effective_ty, &field_core_import, path_map).into_owned(),
                false,
                None,
            )
        }
    } else {
        (
            c_return_type_with_paths(&effective_ty, &field_core_import, path_map).into_owned(),
            false,
            None,
        )
    };
    if ret_type.contains("Self") {
        ret_type = ret_type.replace("Self", &qualified);
    }

    let null_ret = if override_is_opaque_handle {
        "0".to_string()
    } else {
        null_return_value(&effective_ty).to_string()
    };

    let needs_len_out = matches!(field.ty, TypeRef::Bytes);

    if let Some(named_type) = underlying_named_type(&effective_ty) {
        if let Some(override_type) = override_type_name.as_deref() {
            anyhow::ensure!(
                named_type == override_type,
                "cannot generate FFI field accessor `{type_name}.{field_name}`: configured handle type `{override_type}` does not match field type `{named_type}`"
            );
        }
        anyhow::ensure!(
            enum_names.contains(named_type) || clone_names.contains(named_type),
            "cannot generate owned FFI field accessor `{type_name}.{field_name}` for non-Copy, non-Clone type `{named_type}`"
        );
    }

    let body = gen_field_access_body(
        field,
        needs_len_out,
        enum_names,
        clone_names,
        override_is_opaque_handle,
        override_type_name.as_deref(),
    );
    let ownership_lines = field_accessor_ownership_lines(
        &effective_ty,
        prefix,
        enum_names,
        clone_names,
        override_type_name.as_deref(),
    );

    Ok(crate::backends::ffi::template_env::render(
        "field_accessor_header.jinja",
        crate::alef_context! {
            field_name => field_name,
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
            serialized_handle => typ.has_lifetime_params,
            ret_type => ret_type,
            needs_len_out => needs_len_out,
            null_return_value => null_ret,
            body => body,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
            ownership_lines => ownership_lines,
        },
    ))
}

/// Generate a companion `{prefix}_{type_snake}_has_{field_name}` presence accessor for an
/// optional scalar field, so callers can distinguish `None` from a zero-valued `Some` before
/// trusting the sibling getter's return value. Only call when
/// `optional_leaf_needs_presence_signal(&field.ty)` is true.
pub(super) fn gen_field_presence_accessor(typ: &TypeDef, field: &FieldDef, prefix: &str, core_import: &str) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified_base = core_type_path(typ, core_import);
    let qualified = if typ.has_lifetime_params {
        format!("{qualified_base}<'static>")
    } else {
        qualified_base
    };

    crate::backends::ffi::template_env::render(
        "field_presence_accessor.jinja",
        crate::alef_context! {
            field_name => &field.name,
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
            serialized_handle => typ.has_lifetime_params,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Unwrap a field type to its underlying `Named` type name, peeling an outer
/// `Optional`. Returns `None` for primitives, strings, collections, etc.
fn underlying_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => underlying_named_type(inner),
        _ => None,
    }
}

/// Generate the body of a field accessor that reads from `obj.{field_name}`.
fn gen_field_access_body(
    field: &FieldDef,
    needs_len_out: bool,
    enum_names: &AHashSet<String>,
    clone_names: &AHashSet<String>,
    override_is_opaque_handle: bool,
    override_type_name: Option<&str>,
) -> String {
    let field_name = &field.name;
    let mut out = String::with_capacity(2048);

    if override_is_opaque_handle && override_type_name.is_some() {
        let field_is_override_type = underlying_named_type(&field.ty)
            .zip(override_type_name)
            .is_some_and(|(field_named, ovr)| field_named == ovr);
        if !field_is_override_type {
            unreachable!("opaque field override compatibility is validated before body generation");
        }
    }

    if field.optional {
        if needs_len_out {
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_field_start.jinja",
                crate::alef_context! { field_name => field_name },
            ));
            out.push_str("        Some(val) => {\n");
            out.push_str("            if !out_len.is_null() {\n");
            out.push_str("                // SAFETY: null check above guarantees out_len is a valid pointer.\n");
            out.push_str("                unsafe { *out_len = val.len(); }\n");
            out.push_str("            }\n");
            out.push_str("            val.as_ptr() as *mut u8\n");
            out.push_str("        }\n");
            out.push_str("        None => {\n");
            out.push_str("            if !out_len.is_null() {\n");
            out.push_str("                // SAFETY: null check above guarantees out_len is a valid pointer.\n");
            out.push_str("                unsafe { *out_len = 0; }\n");
            out.push_str("            }\n");
            out.push_str("            std::ptr::null_mut()\n");
            out.push_str("        }\n");
            out.push_str("    }\n");
        } else if let TypeRef::Optional(inner) = &field.ty {
            let inner_null = null_return_value(&TypeRef::Optional(Box::new(*inner.clone())));
            let inner_val_expr = if let Some(wrapper) = field
                .newtype_wrapper
                .as_deref()
                .filter(|wrapper| crate::codegen::conversions::helpers::is_explicit_newtype(wrapper))
            {
                crate::codegen::conversions::helpers::apply_newtype_from_core_after_optionals(
                    "inner_val.clone()",
                    wrapper,
                    2,
                )
            } else {
                match inner.as_ref() {
                    TypeRef::Primitive(_) => "*inner_val".to_string(),
                    _ => "inner_val".to_string(),
                }
            };
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_field_start.jinja",
                crate::alef_context! { field_name => field_name },
            ));
            out.push_str("        Some(Some(inner_val)) => {\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                crate::alef_context! {
                    content => gen_value_to_c(&inner_val_expr, inner, "            ", enum_names, clone_names),
                },
            ));
            out.push_str("        }\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_arm_value.jinja",
                crate::alef_context! {
                    pattern => "Some(None)",
                    value => &inner_null.to_string(),
                },
            ));
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_arm_value.jinja",
                crate::alef_context! {
                    pattern => "None",
                    value => &null_return_value(&TypeRef::Optional(Box::new(field.ty.clone()))).to_string(),
                },
            ));
            out.push_str("    }\n");
        } else {
            let val_expr = if let Some(wrapper) = field
                .newtype_wrapper
                .as_deref()
                .filter(|wrapper| crate::codegen::conversions::helpers::is_explicit_newtype(wrapper))
            {
                crate::codegen::conversions::helpers::apply_newtype_from_core_after_optionals("val.clone()", wrapper, 1)
            } else if field.newtype_wrapper.is_some() && matches!(field.ty, TypeRef::Primitive(_)) {
                "val.0".to_string()
            } else if matches!(field.ty, TypeRef::Primitive(_)) {
                "*val".to_string()
            } else if field.is_boxed {
                "(**val)".to_string()
            } else {
                "val".to_string()
            };
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_field_start.jinja",
                crate::alef_context! { field_name => field_name },
            ));
            out.push_str("        Some(val) => {\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                crate::alef_context! {
                    content => gen_value_to_c(&val_expr, &field.ty, "            ", enum_names, clone_names),
                },
            ));
            out.push_str("        }\n");
            out.push_str(&crate::backends::ffi::template_env::render(
                "match_arm_value.jinja",
                crate::alef_context! {
                    pattern => "None",
                    value => &null_return_value(&TypeRef::Optional(Box::new(field.ty.clone()))).to_string(),
                },
            ));
            out.push_str("    }\n");
        }
    } else if needs_len_out {
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_field_access.jinja",
            crate::alef_context! { field_name => field_name },
        ));
        out.push_str("    if !out_len.is_null() {\n");
        out.push_str("// SAFETY: null check above guarantees out_len is a valid pointer.\n");
        out.push_str("        unsafe { *out_len = data.len(); }\n");
        out.push_str("    }\n");
        out.push_str("    data.as_ptr() as *mut u8\n");
    } else {
        let access_expr = if let Some(wrapper) = field
            .newtype_wrapper
            .as_deref()
            .filter(|wrapper| crate::codegen::conversions::helpers::is_explicit_newtype(wrapper))
        {
            crate::codegen::conversions::helpers::apply_newtype_from_core(&format!("obj.{field_name}.clone()"), wrapper)
        } else if field.newtype_wrapper.is_some() && matches!(field.ty, TypeRef::Primitive(_)) {
            format!("obj.{field_name}.0")
        } else if field.core_wrapper == CoreWrapper::Arc || field.is_boxed {
            format!("(*obj.{field_name})")
        } else {
            format!("obj.{field_name}")
        };
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            crate::alef_context! {
                content => gen_value_to_c(&access_expr, &field.ty, "    ", enum_names, clone_names),
            },
        ));
    }

    out
}

/// Which of `enum_def`'s variants a `_from_i32`/`_from_str` C validation function may still
/// advertise, paired with each kept variant's ORIGINAL position in `enum_def.variants`.
///
/// The original position matters: [`gen_enum_from_i32_rs_helper`] reserves a discriminant per
/// variant by enumeration position and never re-numbers when it drops an arm ("the discriminant
/// is still reserved, so numbering stays stable across feature subsets" -- see that function's
/// doc comment), so a validation function built from a re-sequenced (post-filter) index would
/// disagree with the reconstruction helper about what a given integer means.
fn declared_variant_indices<'a>(
    enum_def: &'a EnumDef,
    is_host_enum: bool,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> Vec<(usize, &'a crate::core::ir::EnumVariant)> {
    enum_def
        .variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| {
            !matches!(
                crate::codegen::conversions::enum_variant_declaration(variant, is_host_enum, configured_features),
                crate::codegen::conversions::VariantDeclaration::Drop
            )
        })
        .collect()
}

/// Generate the `{prefix}_{enum_snake}_from_i32` bounds-check function: accepts an i32
/// discriminant iff it names a variant [`gen_enum_from_i32_rs_helper`] can actually reconstruct.
///
/// Filtering by [`crate::codegen::conversions::enum_variant_declaration`] -- the same authority
/// napi's `gen_enum` consults for its wrapper declaration -- keeps this validation surface in
/// agreement with that reconstruction helper: before this fix, every discriminant in
/// `0..variants.len()` validated successfully here regardless of cfg, even though the helper
/// already drops the reconstruction arm for any foreign cfg-gated variant unconditionally -- so a
/// C caller could be told a discriminant was valid, then have the paired reconstruction silently
/// return `None` for it. A host-owned cfg-gated variant, or a foreign one this binding's
/// configured feature set cannot prove absent, is unaffected -- existing behavior, unchanged.
///
/// `host_crate_name`, not `core_import`, decides ownership -- the identical split
/// [`gen_enum_from_i32_rs_helper`] already makes, and for the identical reason: `core_import` can
/// be a configured re-export alias (`[crate] core_import`) that differs from the literal crate
/// name a `rust_path`'s leading `::`-segment is stamped with, so comparing against it here would
/// let this validation function disagree with the reconstruction helper about the exact same
/// enum. ~keep
pub(super) fn gen_enum_from_i32(
    enum_def: &EnumDef,
    prefix: &str,
    host_crate_name: &str,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(host_crate_name, &enum_def.rust_path);
    let variants: Vec<minijinja::Value> = declared_variant_indices(enum_def, is_host_enum, configured_features)
        .into_iter()
        .map(|(index, variant)| crate::alef_context! { index => index, name => variant.name.clone() })
        .collect();

    crate::backends::ffi::template_env::render(
        "enum_from_i32.jinja",
        crate::alef_context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            variants => variants,
        },
    )
}

/// Generate the `{prefix}_{enum_snake}_from_str` function: maps a serde wire-value string to its
/// integer discriminant, accepting only a wire value [`gen_enum_from_i32_rs_helper`] can actually
/// reconstruct from that same discriminant. See [`gen_enum_from_i32`]'s doc comment -- the
/// identical declaration/reconstruction agreement (and `host_crate_name` vs `core_import` split),
/// for the string-keyed sibling function. ~keep
pub(super) fn gen_enum_to_i32(
    enum_def: &EnumDef,
    prefix: &str,
    host_crate_name: &str,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(host_crate_name, &enum_def.rust_path);
    let variants: Vec<minijinja::Value> = declared_variant_indices(enum_def, is_host_enum, configured_features)
        .into_iter()
        .map(|(index, variant)| {
            let serde_rename_all = enum_def.serde_rename_all.as_deref();
            let wire_value = wire_variant_value(&variant.name, variant.serde_rename.as_deref(), serde_rename_all);
            crate::alef_context! { index => index, wire_value => wire_value }
        })
        .collect();

    crate::backends::ffi::template_env::render(
        "enum_to_i32.jinja",
        crate::alef_context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            variants => variants,
        },
    )
}

/// Generate a private Rust helper `fn {enum_snake}_from_i32_rs(v: i32) -> Option<{qualified}>`.
///
/// This helper is used by generated FFI function bodies to reconstruct an enum value from its
/// `i32` discriminant. It is `pub(crate)` to avoid unused-item warnings and is not exported
/// to C. All FFI parameter-crossing enums need this helper regardless of their `Copy` status.
///
/// A variant's `#[cfg(...)]` is only safe to re-emit verbatim when `enum_def.rust_path` is
/// owned by `host_crate_name` (see [`crate::codegen::cfg::is_host_owned_rust_path`], the same
/// authority [`crate::codegen::cfg::collect_cfg_gates`] uses to decide which cfgs get a Cargo
/// feature forwarded for them): a variant merged in from a foreign
/// `[[crates.source_crates]]` crate carries that crate's own cfg gate, and this FFI crate's
/// `Cargo.toml` never declares a feature for it, so an emitted `#[cfg(feature = "...")]`
/// referencing it is an `unexpected cfg condition value` error. Such an arm is dropped
/// entirely -- not silently, a `tracing::debug!` names the enum, variant, and cfg, and
/// `codegen::foreign_cfg_variants` raises the same fact to WARN once for the whole run -- rather than
/// emitting either an invalid attribute or, worse, an ungated reference to a variant that may
/// not even exist in the linked build. The discriminant is still reserved, so numbering stays
/// stable across feature subsets. A host-owned cfg keeps its arm and its `#[cfg(...)]`
/// unchanged: forwarding already declared that feature, so the gate is valid. ~keep
pub(super) fn gen_enum_from_i32_rs_helper(enum_def: &EnumDef, core_import: &str, host_crate_name: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let qualified = core_enum_path(enum_def, core_import);
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(host_crate_name, &enum_def.rust_path);

    let mut arms = String::new();
    for (i, variant) in enum_def.variants.iter().enumerate() {
        if variant.cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %enum_def.name,
                enum_rust_path = %enum_def.rust_path,
                variant_name = %variant.name,
                cfg = variant.cfg.as_deref().unwrap_or_default(),
                "dropping from_i32 reconstruction arm for a foreign-crate enum variant behind a \
                 #[cfg(...)] this FFI crate cannot declare as a Cargo feature; the discriminant \
                 stays reserved but the variant is unreachable from this helper"
            );
            continue;
        }
        arms.push_str(&crate::backends::ffi::template_env::render(
            "ffi_enum_from_i32_rs_arm.jinja",
            crate::alef_context! {
                index => i,
                qualified => qualified.clone(),
                variant_name => variant.name.clone(),
                // A variant behind `#[cfg(feature = "...")]` does not exist in a build without
                // that feature; an ungated arm naming it is a hard compile error in the
                // consumer's crate. The discriminant is still reserved, so numbering stays
                // stable across feature subsets. ~keep
                cfg => variant.cfg.clone(),
            },
        ));
    }

    crate::backends::ffi::template_env::render(
        "ffi_enum_from_i32_rs_helper.jinja",
        crate::alef_context! {
            enum_snake => enum_snake,
            qualified => qualified,
            arms => arms,
            // The enum's own item-level cfg, not just its variants': `qualified` names the core
            // type directly, so a host-owned enum declared behind e.g. `#[cfg(feature = "x")]`
            // makes this whole helper -- not only individual arms -- reference a type that does
            // not exist without that feature. See `gen_enum_free`/`gen_enum_to_json` below, which
            // apply the same `source_cfg` to their sibling struct helpers via `typ.cfg`. ~keep
            source_cfg => enum_def.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Generate a `_free` function for an enum type stored behind a scalar `AlefHandle`.
///
/// These are needed whenever an enum value crosses the FFI boundary as a return value,
/// method return, or field value: every producer in `gen_bindings` hands out such values
/// via `insert_handle` (see `value_to_c_conversion.jinja`'s `named_enum`/`named_clone`
/// arms and `gen_type_to_json`/`gen_type_free`), never a raw pointer, so the matching
/// `_free` must consume the same scalar handle and release it via `remove_handle`.
///
/// Forwards `enum_def.cfg` as `source_cfg`, mirroring `gen_type_free`'s `typ.cfg`: an enum
/// defined inside a `#[cfg(feature = "x")]`-gated module does not exist in a build without
/// that feature, so an ungated `remove_handle::<core::x::TheEnum>(...)` here is a hard
/// `E0433` for every consumer that declares `x` (e.g. via `[crates.ffi].extra_features`, kept
/// out of `default` on purpose) without also enabling it -- this function used to emit the
/// handle-accessor body unconditionally while `gen_type_free` already gated the struct
/// equivalent, so identically-gated enums and structs diverged. ~keep
pub(super) fn gen_enum_free(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_free.jinja",
        crate::alef_context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
            source_cfg => enum_def.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Generate a `_to_json` function for an enum type stored behind a scalar `AlefHandle`.
///
/// Serializes the enum to a JSON string using serde. Only generated for enums that
/// derive `Serialize` (i.e. `has_serde` is true).
pub(super) fn gen_enum_to_json(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_to_json.jinja",
        crate::alef_context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
            source_cfg => enum_def.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Generate a `_to_string` function for an enum type stored behind a scalar `AlefHandle`.
///
/// Renders the unit-variant name as serde would serialize it (e.g.
/// `BatchStatus::Completed` → `"completed"`), but stripped of the surrounding
/// JSON quotes so plain C string-comparison works. Only generated for enums
/// whose runtime serialization yields a string (`has_serde`); compound enums
/// would JSON-encode as objects and `as_str()` returns `None`.
pub(super) fn gen_enum_to_string(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_to_string.jinja",
        crate::alef_context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
            source_cfg => enum_def.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Generate a `_from_json` function for an enum type (for parameter passing from Java).
///
/// Deserializes the enum from a JSON string and returns a scalar `AlefHandle`, mirroring
/// `gen_type_from_json`. Only generated for enums that derive `Deserialize` (i.e.
/// `has_serde` is true).
pub(super) fn gen_enum_from_json(enum_def: &EnumDef, prefix: &str, core_import: &str) -> String {
    let enum_snake = c_symbol_component(&enum_def.name);
    let enum_name = &enum_def.name;
    let qualified = core_enum_path(enum_def, core_import);

    crate::backends::ffi::template_env::render(
        "enum_from_json.jinja",
        crate::alef_context! {
            enum_name => enum_name,
            enum_snake => enum_snake,
            prefix => prefix,
            qualified => qualified,
            source_cfg => enum_def.cfg.as_deref().unwrap_or(""),
        },
    )
}

pub(super) fn gen_type_new(
    typ: &TypeDef,
    prefix: &str,
    core_import: &str,
    params_str: &str,
    body: &str,
    err_ty: &str,
) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);

    crate::backends::ffi::template_env::render(
        "type_new.jinja",
        crate::alef_context! {
            type_name => type_name,
            type_snake => type_snake,
            prefix => prefix,
            qualified => qualified,
            params => params_str,
            body => body,
            err_ty => err_ty,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    )
}

/// Generate an opaque static constructor from a method definition.
///
/// For an opaque type with a static method that returns `Self` or `Result<Self, E>`,
/// emits an `#[no_mangle] pub unsafe extern "C" fn {prefix}_{type_snake}_{method_name}(...) -> *mut {TypeOpaque}`
/// that wraps the core call and returns a heap-allocated opaque handle.
///
/// The FFI symbol name is derived from the method name (e.g. `compile` → `_compile`,
/// `new` → `_new`), NOT hardcoded to `_new`. This allows named constructors like
/// `MetaSchema::compile` to be exported alongside or instead of `new`.
///
/// Parameters are marshalled from FFI types (enum params as i32, strings as *const c_char, etc.)
/// to core types via param conversion helpers. If the method signature is sanitized,
/// an unimplemented stub is generated instead.
pub(super) fn gen_opaque_static_constructor(
    typ: &TypeDef,
    method: &crate::core::ir::MethodDef,
    prefix: &str,
    core_import: &str,
    path_map: &ahash::AHashMap<String, String>,
    enum_names: &ahash::AHashSet<String>,
) -> String {
    use crate::core::ir::TypeRef;

    let type_name = &typ.name;
    let qualified = core_type_path(typ, core_import);
    let return_qualified = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };
    let ffi_fn_name = crate::codegen::c_consumer::method_symbol(prefix, &typ.name, &method.name);
    let will_be_unimplemented = method.sanitized;

    let mut out = String::with_capacity(4096);

    let mut ffi_params = Vec::new();
    for p in &method.params {
        let param_name = if will_be_unimplemented {
            format!("_{}", p.name)
        } else {
            p.name.clone()
        };
        let c_type = crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
            &p.ty,
            core_import,
            path_map,
            enum_names,
            p.is_mut,
        );
        ffi_params.push(format!("    {}: {}", param_name, c_type));

        if matches!(p.ty, TypeRef::Bytes) {
            let len_param_name = if will_be_unimplemented {
                format!("_{}_len", p.name)
            } else {
                format!("{}_len", p.name)
            };
            ffi_params.push(format!("    {}: usize", len_param_name));
        }
    }

    let allow_clippy = if ffi_params.len() > 7 {
        "#[allow(clippy::too_many_arguments)]\n"
    } else {
        ""
    };

    out.push_str(&crate::backends::ffi::template_env::render(
        "ffi_opaque_constructor_header.jinja",
        crate::alef_context! {
            allow_clippy => allow_clippy,
            ffi_fn_name => ffi_fn_name.clone(),
            ffi_params => ffi_params.join(",\n"),
            return_qualified => return_qualified.clone(),
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    ));

    if method.error_type.is_some() {
        out.push_str("    clear_last_error();\n");
    }

    if will_be_unimplemented {
        let unsupported_return = TypeRef::Named(type_name.to_string());
        out.push_str(&gen_ffi_unimplemented_body(
            &unsupported_return,
            &format!("{type_name}::new"),
            method.error_type.is_some(),
        ));
        out.push('\n');
        out.push_str("    })\n}\n");
        return out;
    }

    let conversion = ParamConversionContext {
        has_error: method.error_type.is_some(),
        is_bytes_result: false,
        return_type: &method.return_type,
        ffi_return_type: Some("AlefHandle"),
        core_import,
        path_map,
        enum_names,
    };
    for p in &method.params {
        match &p.ty {
            TypeRef::String if !p.optional && !param_has_explicit_newtype(p) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_string_param.jinja",
                    crate::alef_context! { name => p.name.clone() },
                ));
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                out.push_str(&gen_param_conversion_with_enums(p, &conversion));
            }
            TypeRef::Named(n) if enum_names.contains(n.as_str()) => {
                let enum_snake = c_symbol_component(n);
                let param_name = &p.name;
                let rs_name = format!("{param_name}_rs");
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_enum_discriminant_match.jinja",
                    crate::alef_context! {
                        rs_name => rs_name,
                        enum_snake => enum_snake,
                        name => param_name,
                        error_message => format!("invalid discriminant for {n}"),
                        fail_ret => "return 0;",
                    },
                ));
            }
            TypeRef::Named(_) => {
                // SAFETY: the pointer is generated by the caller from a valid T allocation;
                let param_name = &p.name;
                let rs_name = format!("{param_name}_rs");
                let clone_suffix = if p.is_ref { "" } else { ".clone()" };
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_named_param.jinja",
                    crate::alef_context! {
                        rs_name => rs_name,
                        param_name => param_name,
                        clone_suffix => clone_suffix,
                    },
                ));
            }
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_bool_param.jinja",
                    crate::alef_context! { name => p.name.clone() },
                ));
            }
            _ => {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "ffi_opaque_constructor_passthrough_param.jinja",
                    crate::alef_context! { name => p.name.clone() },
                ));
            }
        }
    }

    let call_args = method
        .params
        .iter()
        .map(|p| {
            let rs_name = format!("{}_rs", p.name);
            if p.optional && p.is_ref && matches!(p.ty, TypeRef::String | TypeRef::Char | TypeRef::Bytes) {
                optional_borrowed_param_call_arg(p, &rs_name)
            } else if p.is_ref {
                format!("&{rs_name}")
            } else {
                rs_name
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let _ = type_name;
    out.push_str(&crate::backends::ffi::template_env::render(
        "ffi_opaque_constructor_call.jinja",
        crate::alef_context! {
            qualified => qualified,
            method_name => method.name.clone(),
            call_args => call_args,
        },
    ));

    if method.error_type.is_some() {
        out.push_str("    match result {\n");
        out.push_str("        Ok(value) => match insert_handle(value) {\n");
        out.push_str("            Ok(handle) => handle,\n");
        out.push_str("            Err(error) => { set_handle_error(&error); 0 },\n");
        out.push_str("        },\n");
        out.push_str("        Err(e) => {\n");
        out.push_str("            set_last_error(1, &e.to_string());\n");
        out.push_str("            0\n");
        out.push_str("        }\n");
        out.push_str("    }\n");
    } else {
        out.push_str("    match insert_handle(result) {\n");
        out.push_str("        Ok(handle) => handle,\n");
        out.push_str("        Err(error) => { set_handle_error(&error); 0 },\n");
        out.push_str("    }\n");
    }
    out.push_str("    })\n}\n");

    out
}

/// Check if a method is an opaque static constructor eligible for C export.
///
/// A static constructor must:
/// - be marked `is_static` (no `self` receiver)
/// - return the owner type by value — either `TypeRef::Named(type_name)` (infallible)
///   or unwrapped through a `Result<Self, _>` return (the error type is tracked in
///   `method.error_type`, so the IR `return_type` is still `Named(type_name)` even for
///   fallible constructors)
///
/// There is NO name restriction: `new`, `compile`, `from_config`, etc. are all eligible.
/// This enables named constructors like `Schema::compile` to be exported as
/// `{prefix}_schema_compile` rather than being silently dropped.
///
/// Excluded:
/// - `default` — the `_default` C symbol collides with the auto-emitted `_new` when
///   both `new` and `default` exist on a type. `default` is almost always an
///   infallible no-arg factory; callers use the `_new` path.
/// - `to_json` / `from_json` — lifecycle helpers emitted by a separate path.
/// - `clone` — not a constructor; clones an existing instance.
pub(super) fn is_static_constructor(method: &crate::core::ir::MethodDef, type_name: &str) -> bool {
    if !method.is_static {
        return false;
    }
    if matches!(method.name.as_str(), "default" | "to_json" | "from_json" | "clone") {
        return false;
    }
    if method.returns_ref {
        return false;
    }
    match &method.return_type {
        crate::core::ir::TypeRef::Named(n) => n == type_name,
        _ => false,
    }
}
