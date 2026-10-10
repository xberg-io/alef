//! Tagged-enum binding/core conversion generators, split out of enums.rs.

use super::*;

/// Whether `variant`'s `#[cfg(...)]` is safe to re-emit verbatim on its match arm.
///
/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate; this WASM crate never declares a Cargo feature for it (see
/// `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim as `#[cfg(feature = "...")]` is
/// an `unexpected cfg condition value` error. Such a variant is dropped entirely instead --
/// named and counted via `tracing::debug!`, not silently; `codegen::foreign_cfg_variants` raises
/// the same fact to WARN once for the whole run -- mirroring
/// `codegen::conversions::enums::emit_cfg_gated_arm`. A host-owned cfg keeps its arm and its
/// `#[cfg(...)]`: forwarding already declared that feature, so the gate is valid. ~keep
fn wasm_tagged_variant_kept(enum_def: &EnumDef, variant: &EnumVariant, is_host_enum: bool, direction: &str) -> bool {
    if variant.cfg.is_none() || is_host_enum {
        return true;
    }
    tracing::debug!(
        enum_name = %enum_def.name,
        enum_rust_path = %enum_def.rust_path,
        variant_name = %variant.name,
        cfg = variant.cfg.as_deref().unwrap_or_default(),
        direction = direction,
        "dropping WASM tagged-enum conversion match arm for a foreign-crate variant behind a \
         #[cfg(...)] this crate cannot declare as a Cargo feature; the variant is unreachable \
         from this conversion"
    );
    false
}

#[cfg(test)]
pub(in crate::backends::wasm::gen_bindings) fn gen_tagged_enum_binding_to_core(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
) -> String {
    gen_tagged_enum_binding_to_core_with_paths(enum_def, core_import, prefix, &[], &ahash::AHashMap::new())
}

pub(in crate::backends::wasm::gen_bindings) fn gen_tagged_enum_binding_to_core_with_paths(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &ahash::AHashMap<String, String>,
) -> String {
    let core_path = crate::codegen::conversions::core_enum_path_remapped(enum_def, core_import, source_crate_remaps);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let tag_field_ident = escape_rust_keyword(tag_field);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let mixed = mixed_type_fields(enum_def);
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();

    let mut lines = vec![];
    lines.push(format!("impl From<{binding_name}> for {core_path} {{"));
    lines.push(format!("    fn from(val: {binding_name}) -> Self {{"));
    lines.push(format!("        match val.{tag_field_ident}.as_str() {{"));
    for variant in &enum_def.variants {
        if !wasm_tagged_variant_kept(enum_def, variant, is_host_enum, "binding_to_core") {
            continue;
        }
        let tag_value = variant_tag_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if let Some(cfg) = variant.cfg.as_deref() {
            lines.push(format!("            #[cfg({})]", cfg));
        }
        if variant.fields.is_empty() {
            lines.push(format!("            \"{tag_value}\" => Self::{},", variant.name));
        } else if variant.is_tuple {
            let args: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if mixed.contains(&f.name) {
                        mixed_field_binding_to_core_expr(f, &f_ident, core_import)
                    } else if tuple_vec_fields.contains(&f.name) {
                        let orig = f.original_type.as_deref().unwrap_or("Vec<(String, String)>");
                        format!(
                            "val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{orig}>(v.clone()).ok()).unwrap_or_default()"
                        )
                    } else {
                        tagged_enum_binding_to_core_expr(f, &f_ident, core_import, source_crate_remaps, type_paths)
                    }
                })
                .collect();
            lines.push(format!(
                "            \"{tag_value}\" => Self::{}({}),",
                variant.name,
                args.join(", ")
            ));
        } else {
            let inits: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if mixed.contains(&f.name) {
                        format!("{}: {}", f.name, mixed_field_binding_to_core_expr(f, &f_ident, core_import))
                    } else if tuple_vec_fields.contains(&f.name) {
                        let orig = f.original_type.as_deref().unwrap_or("Vec<(String, String)>");
                        format!(
                            "{}: val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{orig}>(v.clone()).ok()).unwrap_or_default()",
                            f.name
                        )
                    } else {
                        format!(
                            "{}: {}",
                            f.name,
                            tagged_enum_binding_to_core_expr(f, &f_ident, core_import, source_crate_remaps, type_paths)
                        )
                    }
                })
                .collect();
            lines.push(format!(
                "            \"{tag_value}\" => Self::{} {{ {} }},",
                variant.name,
                inits.join(", ")
            ));
        }
    }
    // Prefer the first variant with no cfg gate as the unconditional `_ =>` fallback: a
    // cfg-gated variant (host-owned or foreign) may not exist in every build, so it cannot
    // safely stand in as the always-available default. Falls back to the very first variant
    // only when every variant carries a cfg. ~keep
    let default_variant = enum_def
        .variants
        .iter()
        .find(|v| v.cfg.is_none())
        .or_else(|| enum_def.variants.first());
    if let Some(first) = default_variant {
        if enum_def
            .variants
            .iter()
            .flat_map(|variant| variant.fields.iter())
            .any(|field| field.newtype_wrapper.as_deref().is_some_and(is_explicit_newtype))
        {
            lines.push(format!(
                "            _ => wasm_bindgen::throw_str(\"unknown {} variant\"),",
                enum_def.name
            ));
        } else if first.fields.is_empty() {
            lines.push(format!("            _ => Self::{},", first.name));
        } else if first.is_tuple {
            let args: Vec<String> = first.fields.iter().map(|_| "Default::default()".to_string()).collect();
            lines.push(format!("            _ => Self::{}({}),", first.name, args.join(", ")));
        } else {
            let defaults: Vec<String> = first
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            lines.push(format!(
                "            _ => Self::{} {{ {} }},",
                first.name,
                defaults.join(", ")
            ));
        }
    }
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Generate `From<core::{Enum}> for Wasm{Enum}` for a tagged-struct enum representation.
#[cfg(test)]
pub(in crate::backends::wasm::gen_bindings) fn gen_tagged_enum_core_to_binding(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
) -> String {
    gen_tagged_enum_core_to_binding_with_remaps(enum_def, core_import, prefix, &[])
}

pub(in crate::backends::wasm::gen_bindings) fn gen_tagged_enum_core_to_binding_with_remaps(
    enum_def: &EnumDef,
    core_import: &str,
    prefix: &str,
    source_crate_remaps: &[(&str, &str)],
) -> String {
    let core_path = crate::codegen::conversions::core_enum_path_remapped(enum_def, core_import, source_crate_remaps);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let tag_field_ident = escape_rust_keyword(tag_field);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let mixed = mixed_type_fields(enum_def);
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();

    let mut all_field_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            all_field_names.insert(field.name.clone());
        }
    }

    let mut lines = vec![];
    lines.push(format!("impl From<{core_path}> for {binding_name} {{"));
    lines.push(format!("    fn from(val: {core_path}) -> Self {{"));
    lines.push("        match val {".to_string());
    for variant in &enum_def.variants {
        if !wasm_tagged_variant_kept(enum_def, variant, is_host_enum, "core_to_binding") {
            continue;
        }
        let tag_value = variant_tag_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        let variant_field_names: std::collections::BTreeSet<String> =
            variant.fields.iter().map(|f| f.name.clone()).collect();
        if let Some(cfg) = variant.cfg.as_deref() {
            lines.push(format!("            #[cfg({})]", cfg));
        }
        if variant.fields.is_empty() {
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                inits.push(format!("                {n_ident}: None"));
            }
            lines.push(format!("            {core_path}::{} => Self {{", variant.name));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        } else if variant.is_tuple {
            let local_names: Vec<String> = variant
                .fields
                .iter()
                .enumerate()
                .map(|(i, _)| format!("field{i}"))
                .collect();
            let destructure = local_names.join(", ");
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    let pos = variant.fields.iter().position(|f| &f.name == name).unwrap();
                    let local = &local_names[pos];
                    let init = if mixed.contains(name) {
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{local}).ok()")
                    } else if tuple_vec_fields.contains(name) {
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{local}).ok()")
                    } else if let Some(field) = variant.fields.iter().find(|f| &f.name == name) {
                        tagged_enum_core_to_binding_expr(field, &n_ident, local)
                    } else {
                        format!("                {n_ident}: None")
                    };
                    inits.push(init);
                } else {
                    inits.push(format!("                {n_ident}: None"));
                }
            }
            lines.push(format!(
                "            {core_path}::{}({}) => Self {{",
                variant.name, destructure
            ));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        } else {
            let destructure_names: Vec<String> = variant.fields.iter().map(|f| escape_rust_keyword(&f.name)).collect();
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    // `mixed` degrades the struct field to `Option<JsValue>` for named-field
                    // variants exactly as it does for tuple variants, so this arm must take the
                    // same serde bridge — `tagged_enum_core_to_binding_expr` would write
                    // `Some({local}.into())`, an E0277 against `JsValue`. ~keep
                    let init = if mixed.contains(name) || tuple_vec_fields.contains(name) {
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{n_ident}).ok()")
                    } else if let Some(field) = variant.fields.iter().find(|f| &f.name == name) {
                        tagged_enum_core_to_binding_expr(field, &n_ident, &n_ident)
                    } else {
                        format!("                {n_ident}: None")
                    };
                    inits.push(init);
                } else {
                    inits.push(format!("                {n_ident}: None"));
                }
            }
            lines.push(format!(
                "            {core_path}::{} {{ {} }} => Self {{",
                variant.name,
                destructure_names.join(", ")
            ));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        }
    }
    lines.push(
        crate::backends::wasm::template_env::render("tagged_enum_unmapped_core_arm", crate::alef_context! {})
            .trim_end()
            .to_string(),
    );
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}
