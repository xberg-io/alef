//! Emits the swift-bridge wrapper newtype structs for IR struct types.
//!
//! `emit_type_wrapper` produces:
//!   - `pub struct T(pub SourceT)` newtype
//!   - `impl T { pub fn new(…) → T }` constructor
//!   - `impl T { pub fn field(&self) → BridgeType }` getters
//!
//! Enum wrappers live in `enums.rs`.

use crate::backends::swift::gen_rust_crate::default_construction::{
    emit_default_construction_body, emit_direct_field_inits,
};
use crate::backends::swift::gen_rust_crate::type_bridge::{bridge_type, needs_json_bridge};
use crate::codegen::generators::type_paths::resolve_type_path;
use crate::core::ir::{TypeDef, TypeRef};
use crate::core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

use super::getters::emit_getters;

pub(crate) fn emit_type_wrapper(
    ty: &TypeDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
) -> String {
    let mut out = String::new();
    let source_path = resolve_type_path(&ty.name, source_crate, type_paths);
    if let Some(cfg) = ty.cfg.as_deref() {
        out.push_str(&format!("#[cfg({cfg})]\n"));
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "struct_newtype.jinja",
        minijinja::context! {
            name => &ty.name,
            source_path => &source_path,
            has_lifetime_params => ty.has_lifetime_params,
        },
    ));

    if !ty.fields.is_empty() {
        if let Some(cfg) = ty.cfg.as_deref() {
            out.push_str(&format!("#[cfg({cfg})]\n"));
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "impl_header.jinja",
            minijinja::context! {
                name => &ty.name,
            },
        ));

        // Constructor — params use bridge types (String for JSON-bridged fields)
        // and Option<bridge_ty> when the field is optional.
        // Excluded fields (via exclude_fields config) are omitted from params
        // and left at Default::default() in the field initializers.
        let constructor_fields: Vec<_> = ty
            .fields
            .iter()
            .filter(|f| {
                let field_key = format!("{}.{}", ty.name, f.name.to_snake_case());
                !f.binding_excluded && !exclude_fields.contains(&field_key)
            })
            .collect();
        let params: Vec<String> = constructor_fields
            .iter()
            .map(|f| {
                let bridge_ty = bridge_type(&f.ty);
                let bridge_ty = if f.optional && !needs_json_bridge(&f.ty) {
                    // Optional fields are JSON-bridged so this branch is rarely hit;
                    // when it is (a primitive Option), wrap in Option<>.
                    format!("Option<{bridge_ty}>")
                } else {
                    bridge_ty
                };
                // Escape Swift keywords so the param name in `pub fn new()` matches
                // the extern declaration (which also escapes via swift_ident).
                let name = swift_ident(&f.name.to_snake_case());
                format!("{name}: {bridge_ty}")
            })
            .collect();

        // Determine construction strategy (see default_construction.rs for details):
        // when any field requires Default-based assignment, we cannot emit a direct struct literal.
        // Primitive-only DTOs always get a direct struct-literal constructor regardless
        // of `Default` impl or serde derive — must stay in lockstep with
        // `extern_block::has_constructor_extern`'s matching fast path.
        let all_primitive_fields = constructor_fields.iter().all(|f| matches!(f.ty, TypeRef::Primitive(_)));
        let has_vec_non_primitive = constructor_fields.iter().any(|f| {
            matches!(&f.ty, TypeRef::Vec(inner) if !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes))
        });
        let has_non_serde_string_field = !ty.has_serde
            && constructor_fields
                .iter()
                .any(|f| matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Char));
        let needs_default_construction = !all_primitive_fields
            && (ty.has_serde
                || has_vec_non_primitive
                || has_non_serde_string_field
                || ty.has_stripped_cfg_fields
                || constructor_fields
                    .iter()
                    .any(|f| needs_json_bridge(&f.ty) || matches!(f.ty, TypeRef::Named(_))));

        if needs_default_construction && !ty.has_default {
            // The struct needs mutable-default construction but doesn't impl Default.
            // Omit the constructor entirely — swift-bridge will not expose `init()` for
            // this type, which is correct: the host language can't construct it anyway.
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "fn_new_signature.jinja",
                minijinja::context! {
                    params => params.join(", "),
                    name => &ty.name,
                },
            ));

            if needs_default_construction && ty.has_default {
                let body = emit_default_construction_body(
                    ty,
                    &source_path,
                    type_paths,
                    enum_names,
                    no_serde_names,
                    exclude_fields,
                );
                out.push_str(&body);
            } else {
                let field_inits = emit_direct_field_inits(ty, type_paths, enum_names, no_serde_names, exclude_fields);
                out.push_str(&crate::backends::swift::template_env::render(
                    "struct_literal_open.jinja",
                    minijinja::context! {
                        name => &ty.name,
                        source_path => &source_path,
                    },
                ));
                for init in &field_inits {
                    out.push_str(init);
                    out.push_str(",\n");
                }
                out.push_str("        })\n");
            }
            out.push_str("    }\n");
        } // end else (constructor emitted)

        // Getters — return bridge types (String for JSON-bridged, wrappers for Named).
        emit_getters(ty, type_paths, enum_names, no_serde_names, exclude_fields, &mut out);

        out.push_str("}\n");
    }

    out
}

/// Emit a `pub fn create_<type_name>(api_key: String, base_url: Option<String>) -> Result<TypeName, String>`
/// constructor shim for an opaque type that exposes methods.
///
/// The source crate must provide `<TypeName>::new(api_key, base_url)` or a compatible constructor.
/// This mirrors the common stateful-client constructor pattern.
///
/// When the source crate's constructor signature differs
/// `DefaultClient::new(ClientConfig, Option<&str>)`), the caller can supply a
/// custom body via `[crates.<crate>.swift] client_constructor_body."TypeName" = "..."`
/// in alef.toml. The custom body is interpolated verbatim, with `{type_name}` and
/// `{source_path}` placeholders available.
pub(crate) fn emit_type_constructor_shim(
    ty: &TypeDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    custom_body: Option<&str>,
) -> String {
    let type_snake = ty.name.to_snake_case();
    let fn_name = format!("create_{type_snake}");
    let type_name = &ty.name;
    let source_path = resolve_type_path(type_name, source_crate, type_paths);

    let cfg_prefix = ty.cfg.as_deref().map(|c| format!("#[cfg({c})]\n")).unwrap_or_default();

    if let Some(body) = custom_body {
        let interpolated = body
            .replace("{type_name}", type_name)
            .replace("{source_path}", &source_path);
        return format!(
            concat!(
                "{cfg_prefix}pub fn {fn_name}(api_key: String, base_url: Option<String>)",
                " -> Result<{type_name}, String> {{\n",
                "{interpolated}\n",
                "}}\n"
            ),
            cfg_prefix = cfg_prefix,
            fn_name = fn_name,
            type_name = type_name,
            interpolated = interpolated,
        );
    }

    format!(
        "{cfg_prefix}pub fn {fn_name}(api_key: String, base_url: Option<String>) -> Result<{type_name}, String> {{\n    \
         {source_path}::new(api_key, base_url)\n        \
         .map_err(|e| e.to_string())\n        \
         .map({type_name})\n}}\n",
        cfg_prefix = cfg_prefix,
    )
}
