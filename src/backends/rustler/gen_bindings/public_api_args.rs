use crate::core::ir::{EnumDef, ParamDef, TypeRef};
use ahash::{AHashMap, AHashSet};

pub(in crate::backends::rustler::gen_bindings) fn json_encode_param_indices(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> AHashSet<usize> {
    // The NIF side (`sync_functions.rs::gen_nif_function`) marshals every
    // default-typed `Named` param and every `Vec<Named>` whose inner is a
    // *non-opaque* struct as `Option<String>` JSON. The wrapper must mirror
    // that exact predicate, otherwise structured DTOs reach the NIF as raw
    // Erlang terms and Rustler raises `ArgumentError`.
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
        format!("Jason.encode!({param})")
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
pub(in crate::backends::rustler::gen_bindings) fn emit_tagged_enum_encoder(enum_def: &EnumDef) -> String {
    use crate::codegen::naming::{pascal_to_snake, wire_field_name, wire_variant_value};

    let Some(tag) = enum_def.serde_tag.as_deref() else {
        return String::new();
    };
    if enum_def.serde_untagged {
        // Untagged enums have no discriminator — skip.
        return String::new();
    }

    let fn_name = encoder_fn_name(&enum_def.name);
    let rename_all = enum_def.serde_rename_all.as_deref();

    let mut out = String::with_capacity(1024);
    let mut first_clause = true;

    // No `@doc` on `defp` — Elixir warns on @doc attached to private functions.
    // The clauses below collectively define the encoder; no function head is needed
    // because none of the clauses use default arguments.

    for variant in &enum_def.variants {
        if variant.binding_excluded {
            continue;
        }
        let atom = pascal_to_snake(&variant.name);
        let wire = wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all);
        // Escape any quotes in the wire name (defensive — serde wire names are usually safe).
        let wire_escaped = wire.replace('\\', "\\\\").replace('"', "\\\"");

        if variant.fields.is_empty() {
            // Unit variant: accept both bare atom and tuple form with an empty/any map.
            if !first_clause {
                out.push('\n');
            }
            out.push_str(&format!(
                "  defp {fn_name}(:{atom}), do: %{{\"{tag}\" => \"{wire_escaped}\"}}\n"
            ));
            // Blank line between the two unit variant clauses (atom form vs tuple form).
            out.push('\n');
            out.push_str(&format!(
                "  defp {fn_name}({{:{atom}, _}}), do: %{{\"{tag}\" => \"{wire_escaped}\"}}\n"
            ));
            first_clause = false;
            continue;
        }

        // Struct variant: build a per-variant field-rename branch.
        // Map each known Rust snake_case key to its serde wire name.
        // Field-level rename_all on variants is not currently captured in the IR — only
        // explicit `#[serde(rename = "...")]` per field is honored. Unknown keys are
        // passed through as their string form, preserving forwards compatibility.
        if !first_clause {
            out.push('\n');
        }
        out.push_str(&format!("  defp {fn_name}({{:{atom}, %{{}} = data}}) do\n"));
        out.push_str("    data\n");
        out.push_str("    |> Enum.reduce(%{}, fn {k, v}, acc ->\n");
        out.push_str("      key =\n");
        out.push_str("        case k do\n");
        for field in &variant.fields {
            if field.binding_excluded {
                continue;
            }
            let wire_field = wire_field_name(&field.name, field.serde_rename.as_deref(), None);
            // Only emit a rename arm when the Rust ident differs from the wire form;
            // otherwise the catch-all `Atom.to_string` already does the right thing.
            if wire_field != field.name {
                let wire_field_escaped = wire_field.replace('\\', "\\\\").replace('"', "\\\"");
                out.push_str(&format!("          :{} -> \"{}\"\n", field.name, wire_field_escaped));
            }
        }
        out.push_str("          k when is_atom(k) -> Atom.to_string(k)\n");
        out.push_str("          k when is_binary(k) -> k\n");
        out.push_str("        end\n\n");
        out.push_str("      Map.put(acc, key, v)\n");
        out.push_str("    end)\n");
        out.push_str(&format!("    |> Map.put(\"{tag}\", \"{wire_escaped}\")\n"));
        out.push_str("  end\n");
        first_clause = false;
    }

    // Map passthrough: caller already produced a wire-shaped map.
    if !first_clause {
        out.push('\n');
    }
    out.push_str(&format!("  defp {fn_name}(%{{}} = m), do: m\n"));
    // Error path: anything else is a programming error — be loud about it.
    out.push('\n');
    out.push_str(&format!(
        "  defp {fn_name}(other),\n    do: raise(ArgumentError, \"expected {} (atom, {{atom, map}}, or map), got: \" <> inspect(other))\n\n",
        enum_def.name
    ));

    out
}
