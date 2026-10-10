use crate::core::ir::{EnumDef, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};

pub(in crate::backends::rustler::gen_bindings) fn json_encode_param_indices(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> AHashSet<usize> {
    params
        .iter()
        .enumerate()
        .filter_map(|(idx, param)| match &param.ty {
            TypeRef::Named(name) if default_types.contains(name.as_str()) && !opaque_types.contains(name.as_str()) => {
                Some(idx)
            }
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(inner_name) if !opaque_types.contains(inner_name) => Some(idx),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub(in crate::backends::rustler::gen_bindings) fn has_fallible_deserialization_params(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> bool {
    params.iter().any(|param| match &param.ty {
        TypeRef::Named(name) => default_types.contains(name),
        TypeRef::Vec(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name))
        }
        TypeRef::Json => true,
        _ => false,
    })
}

pub(in crate::backends::rustler::gen_bindings) fn function_deserialization_introduces_result(
    function: &crate::core::ir::FunctionDef,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> bool {
    if function.is_async || function.error_type.is_some() {
        return false;
    }
    let has_default = function
        .params
        .iter()
        .any(|param| matches!(&param.ty, TypeRef::Named(name) if default_types.contains(name)));
    let has_named_vec = function.params.iter().any(|param| {
        matches!(&param.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name)))
    });
    let can_delegate =
        crate::codegen::shared::can_auto_delegate_function(function, opaque_types) || has_default || has_named_vec;
    can_delegate && has_fallible_deserialization_params(&function.params, opaque_types, default_types)
}

pub(in crate::backends::rustler::gen_bindings) fn method_deserialization_introduces_result(
    method: &crate::core::ir::MethodDef,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> bool {
    if method.is_async || method.error_type.is_some() {
        return false;
    }
    let has_default = method
        .params
        .iter()
        .any(|param| matches!(&param.ty, TypeRef::Named(name) if default_types.contains(name)));
    let can_delegate_refmut = is_opaque
        && matches!(method.receiver, Some(crate::core::ir::ReceiverKind::RefMut))
        && method.trait_source.is_none()
        && !method.sanitized
        && method.params.iter().all(|param| {
            !param.sanitized
                && crate::codegen::shared::is_delegatable_param(&param.ty, opaque_types)
                && !crate::codegen::shared::is_named_ref_param_pub(param, opaque_types)
        })
        && crate::codegen::shared::is_delegatable_return(&method.return_type);
    let can_delegate =
        crate::codegen::shared::can_auto_delegate(method, opaque_types) || has_default || can_delegate_refmut;
    can_delegate && has_fallible_deserialization_params(&method.params, opaque_types, default_types)
}

/// Map a param index → tagged-enum name when the param's type (or its `Vec<_>` element)
/// is a serde-tagged enum (`#[serde(tag = "...")]`). Used by the wrapper to insert a
/// per-enum `encode_<EnumName>/1` helper call before `Jason.encode!`, so callers can pass
/// idiomatic Elixir tuples (`{:click, %{...}}`) or bare atoms (`:scrape`) for unit variants.
///
/// The flag `is_vec` indicates whether the param is `Vec<T>` (true) or bare `T` (false).
pub(in crate::backends::rustler::gen_bindings) fn tagged_enum_param_map(
    params: &[ParamDef],
    enum_lookup: &AHashMap<String, &EnumDef>,
) -> AHashMap<usize, TaggedEnumParam> {
    params
        .iter()
        .enumerate()
        .filter_map(|(idx, param)| {
            let (inner_name, is_vec) = match &param.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => (n.as_str(), true),
                    _ => return None,
                },
                TypeRef::Named(n) => (n.as_str(), false),
                _ => return None,
            };
            let enum_def = enum_lookup.get(inner_name)?;
            if enum_def.serde_tag.is_some() {
                Some((
                    idx,
                    TaggedEnumParam {
                        enum_name: enum_def.name.clone(),
                        is_vec,
                    },
                ))
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub(in crate::backends::rustler::gen_bindings) struct TaggedEnumParam {
    pub enum_name: String,
    pub is_vec: bool,
}

pub(in crate::backends::rustler::gen_bindings) fn nif_arg(
    index: usize,
    param: &str,
    json_encode_params: &AHashSet<usize>,
    tagged_enum_params: &AHashMap<usize, TaggedEnumParam>,
) -> String {
    if let Some(te) = tagged_enum_params.get(&index) {
        let helper = encoder_fn_name(&te.enum_name);
        if te.is_vec {
            format!("Jason.encode!(Enum.map({param}, &{helper}/1))")
        } else {
            format!("Jason.encode!({helper}({param}))")
        }
    } else if json_encode_params.contains(&index) {
        format!("(cond do is_nil({param}) -> nil; is_binary({param}) -> {param}; true -> Jason.encode!({param}) end)")
    } else {
        param.to_string()
    }
}

pub(in crate::backends::rustler::gen_bindings) fn keyword_nif_arg(
    index: usize,
    param: &str,
    json_encode_params: &AHashSet<usize>,
    tagged_enum_params: &AHashMap<usize, TaggedEnumParam>,
) -> String {
    if let Some(te) = tagged_enum_params.get(&index) {
        let helper = encoder_fn_name(&te.enum_name);
        let mapped = if te.is_vec {
            format!("Jason.encode!(Enum.map(v, &{helper}/1))")
        } else {
            format!("Jason.encode!({helper}(v))")
        };
        format!("case Keyword.get(opts, :{param}) do nil -> nil; v -> {mapped} end")
    } else if json_encode_params.contains(&index) {
        format!("case Keyword.get(opts, :{param}) do nil -> nil; v -> Jason.encode!(v) end")
    } else {
        format!("Keyword.get(opts, :{param})")
    }
}

/// Returns the private encoder function name for a tagged enum, e.g.
/// `PageAction` → `encode_page_action`. Elixir function names must start with
/// a lowercase letter or underscore, so we snake_case the enum name.
pub(in crate::backends::rustler::gen_bindings) fn encoder_fn_name(enum_name: &str) -> String {
    format!("encode_{}", crate::codegen::naming::pascal_to_snake(enum_name))
}

/// Emit a private Elixir helper `defp encode_<snake_enum>(value)` that converts
/// idiomatic Elixir input shapes into the JSON wire shape that the NIF's serde
/// decoder expects for a serde-tagged enum:
///
///   * `:variant_atom` (unit variant) → `%{"<tag>" => "<wireName>"}`
///   * `{:variant_atom, %{field: ...}}` → `%{"<tag>" => "<wireName>", "<fieldWire>" => ...}`
///   * `%{}` (already a wire-shaped map) → passthrough
///
/// `enum_def.serde_tag` is required (caller filters); if absent this returns an empty string.
///
/// This function prepares data only — the Elixir atom spelling, the escaped wire strings, and
/// which shape each variant takes — and hands it to `elixir_tagged_enum_encoder.ex.jinja`, which
/// owns every line, brace and indent of the emitted module. It used to build the same text with
/// `push_str(&format!(...))`, against the repo's `jinja-templates` rule, and the split is not
/// cosmetic: the tag interpolated straight into `%{"{tag}" => ...}` with no escaping at all, and
/// the wire values got `\` and `"` but not `#`. Escaping now happens once, here, on values that
/// then travel into the template untouched — the environment applies no autoescaping to a
/// `.jinja` template (see `template_env`'s `rendering_a_text_template_does_not_autoescape`), so
/// what Rust escapes is exactly what lands in the file, with no second pass to double it. ~keep
pub(in crate::backends::rustler::gen_bindings) fn emit_tagged_enum_encoder(enum_def: &EnumDef) -> String {
    use crate::backends::rustler::elixir_escape::{elixir_atom_body, escape_elixir_string_literal};
    use crate::backends::rustler::template_env;
    use crate::codegen::naming::{pascal_to_snake, wire_field_name, wire_variant_value};

    let Some(tag) = enum_def.serde_tag.as_deref() else {
        return String::new();
    };
    if enum_def.serde_untagged {
        return String::new();
    }

    let rename_all = enum_def.serde_rename_all.as_deref();
    let variants: Vec<minijinja::Value> = enum_def
        .variants
        .iter()
        .filter(|variant| !variant.binding_excluded)
        .map(|variant| {
            let field_renames: Vec<minijinja::Value> = variant
                .fields
                .iter()
                .filter(|field| !field.binding_excluded)
                .filter_map(|field| {
                    let wire_field = wire_field_name(&field.name, field.serde_rename.as_deref(), None);
                    if wire_field == field.name {
                        return None;
                    }
                    Some(crate::alef_context! {
                        atom => elixir_atom_body(&field.name),
                        wire => escape_elixir_string_literal(&wire_field),
                    })
                })
                .collect();
            let wire = wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all);
            crate::alef_context! {
                atom => elixir_atom_body(&pascal_to_snake(&variant.name)),
                wire => escape_elixir_string_literal(&wire),
                is_unit => variant.fields.is_empty(),
                field_renames => field_renames,
            }
        })
        .collect();

    template_env::render(
        "elixir_tagged_enum_encoder.ex.jinja",
        crate::alef_context! {
            fn_name => encoder_fn_name(&enum_def.name),
            enum_name => escape_elixir_string_literal(&enum_def.name),
            tag => escape_elixir_string_literal(tag),
            variants => variants,
        },
    )
}
