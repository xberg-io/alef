//! WASM struct and opaque type code generation.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::builder::ImplBuilder;
use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
use crate::codegen::type_mapper::TypeMapper;
use crate::codegen::{generators, naming::to_node_name, shared};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{EnumDef, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToPascalCase;

use super::functions::{
    emit_rustdoc, format_param_unused, gen_wasm_unimplemented_body, wasm_call_args, wasm_mapped_param_type,
    wasm_mapped_return_type, wasm_newtype_param_bindings, wasm_wrap_return, wrap_jsvalue_mapped_return,
};
use super::methods::gen_method_with_type_paths;

#[path = "types_accessors.rs"]
mod types_accessors;
#[path = "types_helpers.rs"]
pub(in crate::backends::wasm::gen_bindings) mod types_helpers;
#[path = "types_unit_enum.rs"]
mod types_unit_enum;

#[cfg(test)]
#[path = "types_tests.rs"]
mod types_tests;

use types_accessors::{AccessorEnv, gen_clear_method, gen_getter, gen_setter};
pub(in crate::backends::wasm::gen_bindings) use types_helpers::filter_cfg_fields_for_features;
use types_helpers::{
    complex_newtype_field_uses_jsvalue, is_bare_tagged_data_enum, is_option_of_tagged_data_enum,
    is_vec_of_tagged_data_enum,
};

mod opaque;
pub(super) use opaque::{gen_opaque_struct, gen_opaque_struct_methods};

/// Generate a wasm-bindgen struct definition with private fields.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    core_import: &str,
    prefix: &str,
    tagged_data_enum_names: &AHashSet<String>,
    source_crate_remaps: &[(&str, &str)],
    is_core_to_binding_convertible: bool,
) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut out = String::with_capacity(512);
    out.push_str(&emit_rustdoc(&typ.doc));

    let mut fields = Vec::new();
    for field in shared::binding_fields(&typ.fields) {
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        let force_optional = typ.has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
        let is_vec_tagged_enum = is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
        let is_option_tagged_enum =
            !is_vec_tagged_enum && is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
        let is_bare_tagged_enum = !is_vec_tagged_enum
            && !is_option_tagged_enum
            && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
        let field_type = if complex_newtype_field_uses_jsvalue(field) {
            if field.optional || matches!(field.ty, TypeRef::Optional(_)) {
                "Option<JsValue>".to_string()
            } else {
                "JsValue".to_string()
            }
        } else if force_optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else if is_vec_tagged_enum {
            "JsValue".to_string()
        } else if is_option_tagged_enum || (is_bare_tagged_enum && field.optional) {
            "Option<JsValue>".to_string()
        } else if is_bare_tagged_enum {
            "JsValue".to_string()
        } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
            mapper.map_type(&field.ty)
        } else if field.optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        fields.push((field.name.clone(), field_type));
    }

    // When the type IS convertible, suppress #[derive(Default)] and emit the delegating
    let derives_default = !typ.has_default || !is_core_to_binding_convertible;
    out.push_str(&crate::backends::wasm::template_env::render(
        "gen_struct",
        crate::alef_context! {
            struct_name => js_name,
            unprefixed_name => typ.name,
            derives_default => derives_default,
            fields => fields.iter().map(|(name, ty)| {
                crate::alef_context! {
                    name => name,
                    field_type => ty,
                }
            }).collect::<Vec<_>>(),
        },
    ));
    if typ.has_default && is_core_to_binding_convertible {
        out.push_str(&generators::gen_delegating_default_impl(
            typ,
            core_import,
            prefix,
            source_crate_remaps,
        ));
    }
    out
}

/// Generate wasm-bindgen methods for a struct.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    core_import: &str,
    opaque_types: &AHashSet<String>,
    api_enums: &[EnumDef],
    prefix: &str,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    untagged_ts_value_types: &AHashMap<String, String>,
    source_crate_remaps: &[(&str, &str)],
    api_types: &[TypeDef],
) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);
    impl_builder.add_attr("wasm_bindgen");

    let enum_names: AHashSet<String> = api_enums.iter().map(|e| e.name.clone()).collect();
    let mut type_paths = AHashMap::new();
    for api_type in api_types.iter().filter(|api_type| !api_type.is_trait) {
        type_paths.insert(
            api_type.name.clone(),
            crate::codegen::conversions::core_type_path_remapped(api_type, core_import, source_crate_remaps),
        );
    }
    for enum_def in api_enums {
        type_paths.insert(
            enum_def.name.clone(),
            crate::codegen::conversions::core_enum_path_remapped(enum_def, core_import, source_crate_remaps),
        );
    }
    // Mirrors the emission loop in `mod.rs`: every non-trait, non-excluded type becomes a
    // `#[wasm_bindgen]` class, whether it is an opaque wrapper or a field-carrying struct. ~keep
    let class_type_names: AHashSet<String> = api_types
        .iter()
        .filter(|t| !t.is_trait && !exclude_types.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();
    let tagged_data_enum_names: AHashSet<String> = api_enums
        .iter()
        .filter(|e| super::enums::is_tagged_data_enum(e, api_types))
        .map(|e| e.name.clone())
        .collect();
    // Every identifier this impl block already mints from the consumer's own API: an inherent
    // method name, and a field name (whose getter is `pub fn {field}`). `gen_clear_method`
    // stands down rather than collide with one as `E0592`. ~keep
    let reserved_idents: AHashSet<String> = typ
        .methods
        .iter()
        .map(|m| m.name.clone())
        .chain(typ.fields.iter().map(|f| f.name.clone()))
        .collect();

    if !typ.fields.is_empty() {
        impl_builder.add_method(&gen_new_method(
            typ,
            mapper,
            exclude_types,
            prefix,
            &tagged_data_enum_names,
            &class_type_names,
        ));
        // The wasm wrapper always has a Default impl — either #[derive(Default)] or the
        if !typ.methods.iter().any(|m| m.name == "default") {
            impl_builder.add_method(&gen_default_method(typ, prefix));
        }
    }

    impl_builder.add_method(&gen_transfer_copy_method(typ, prefix));

    let mut emitted_field_names: AHashSet<&str> = AHashSet::default();
    for field in shared::binding_fields(&typ.fields) {
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        emitted_field_names.insert(field.name.as_str());
        impl_builder.add_method(&gen_getter(
            field,
            &AccessorEnv {
                mapper,
                enum_names: &enum_names,
                tagged_data_enum_names: &tagged_data_enum_names,
                untagged_ts_value_types,
                class_type_names: &class_type_names,
            },
            typ.has_default,
        ));
        impl_builder.add_method(&gen_setter(
            field,
            &AccessorEnv {
                mapper,
                enum_names: &enum_names,
                tagged_data_enum_names: &tagged_data_enum_names,
                untagged_ts_value_types,
                class_type_names: &class_type_names,
            },
            typ.has_default,
        ));
        if let Some(clear) = gen_clear_method(field, mapper, &class_type_names, &reserved_idents) {
            impl_builder.add_method(&clear);
        }
    }

    if !exclude_types.contains(&typ.name) {
        for method in &typ.methods {
            // A field and an inherent method sharing the same name both mint a
            // `pub fn {name}` in this impl block — the getter above is emitted first and
            // wins, so the method wrapper is skipped to avoid `E0592: duplicate definitions`. ~keep
            if emitted_field_names.contains(method.name.as_str()) {
                continue;
            }
            let refs_excluded = method
                .params
                .iter()
                .any(|p| field_references_excluded_type(&p.ty, exclude_types))
                || field_references_excluded_type(&method.return_type, exclude_types);
            if refs_excluded {
                continue;
            }
            impl_builder.add_method(&gen_method_with_type_paths(
                method,
                mapper,
                &typ.name,
                core_import,
                opaque_types,
                prefix,
                typ,
                mutex_types,
                streaming_item_types,
                source_crate_remaps,
                &type_paths,
            ));
        }
    }

    impl_builder.build()
}

/// Convert snake_case parameter names to camelCase for JS-facing constructor signatures.
/// Also converts the assignments list to use explicit `field: param` syntax.
///
/// Assignment forms:
/// 1. Shorthand (required field): `"tool_call_id"` → `"tool_call_id: toolCallId"`
/// 2. Explicit passthrough: `"total_tokens: total_tokens"` → `"total_tokens: totalTokens"`
/// 3. Explicit with suffix: `"total_tokens: total_tokens.unwrap_or_default()"` →
///    `"total_tokens: totalTokens.unwrap_or_default()"` (leading ident renamed, suffix kept)
/// 4. Constant expressions (e.g. `"field: Default::default()"`): kept as-is.
fn convert_constructor_params_to_camel_case(
    param_list: &str,
    assignments: &str,
    field_names: &[String],
    borrowed: &AHashSet<String>,
) -> (String, String) {
    let field_to_camel: std::collections::HashMap<String, String> = field_names
        .iter()
        .map(|name| (name.clone(), to_node_name(name)))
        .collect();

    let is_multiline = param_list.contains('\n');
    let raw_camel_params: Vec<String> = param_list
        .split(',')
        .filter_map(|param| {
            let trimmed = param.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some((name, ty)) = trimmed.split_once(':') {
                let camel_name = to_node_name(name.trim());
                let borrow = if borrowed.contains(name.trim()) { "&" } else { "" };
                Some(format!("{}: {}{}", camel_name, borrow, ty.trim()))
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect();
    let camel_params = if is_multiline {
        format!("\n        {},\n    ", raw_camel_params.join(",\n        "))
    } else {
        raw_camel_params.join(", ")
    };

    let camel_assignments = assignments
        .split(", ")
        .map(|assignment| {
            if assignment.contains(':') {
                if let Some((field_name, rhs)) = assignment.split_once(':') {
                    let field_trimmed = field_name.trim();
                    let rhs_trimmed = rhs.trim();
                    let (leading_ident, suffix) = split_leading_ident(rhs_trimmed);
                    // A borrowed parameter is `&Wasm{Type}`, so the struct literal has to store
                    // a clone -- mirroring what `gen_class_field_setter.jinja` does. ~keep
                    let clone = if borrowed.contains(field_trimmed) && suffix.is_empty() {
                        ".clone()"
                    } else {
                        ""
                    };
                    if let Some(camel_rhs) = field_to_camel.get(leading_ident) {
                        format!("{}: {}{}{}", field_trimmed, camel_rhs, suffix, clone)
                    } else {
                        format!("{}: {}{}", field_trimmed, rhs_trimmed, clone)
                    }
                } else {
                    assignment.to_string()
                }
            } else {
                let field_name = assignment.trim();
                if borrowed.contains(field_name) {
                    let camel_name = field_to_camel.get(field_name).map_or(field_name, String::as_str);
                    return format!("{field_name}: {camel_name}.clone()");
                }
                match field_to_camel.get(field_name) {
                    // Only emit `field: camelVar` when the rename actually changes the
                    // identifier; single-word fields (snake == camel) stay shorthand. ~keep
                    Some(camel_name) if camel_name != field_name => format!("{}: {}", field_name, camel_name),
                    _ => assignment.to_string(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    (camel_params, camel_assignments)
}

/// Split a Rust expression into `(leading_identifier, rest_of_expression)`.
fn split_leading_ident(expr: &str) -> (&str, &str) {
    let end = expr
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(expr.len());
    (&expr[..end], &expr[end..])
}

fn preserve_optional_constructor_defaults(assignments: &str, fields: &[FieldDef]) -> (String, bool) {
    let mut rendered = assignments.to_string();
    let mut use_defaults = false;
    for field in fields
        .iter()
        .filter(|field| field.cfg.is_none() && (field.optional || matches!(field.ty, TypeRef::Optional(_))))
    {
        let direct = format!("{}: {}", field.name, field.name);
        let preserved = format!("{}: {}.or(defaults.{})", field.name, field.name, field.name);
        if rendered.contains(&direct) {
            rendered = rendered.replace(&direct, &preserved);
            use_defaults = true;
        } else {
            let mut replaced = false;
            rendered = rendered
                .split(", ")
                .map(|assignment| {
                    if assignment == field.name {
                        replaced = true;
                        preserved.as_str()
                    } else {
                        assignment
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            use_defaults |= replaced;
        }
    }
    (rendered, use_defaults)
}

/// Generate a constructor method with camelCase parameter names for JS consumers.
fn gen_new_method(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    prefix: &str,
    tagged_data_enum_names: &AHashSet<String>,
    class_type_names: &AHashSet<String>,
) -> String {
    use super::field_references_excluded_type;
    use crate::codegen::shared::constructor_parts;

    let map_fn = |ty: &crate::core::ir::TypeRef| {
        if is_vec_of_tagged_data_enum(ty, tagged_data_enum_names)
            || is_bare_tagged_data_enum(ty, tagged_data_enum_names)
        {
            "JsValue".to_string()
        } else if is_option_of_tagged_data_enum(ty, tagged_data_enum_names) {
            "Option<JsValue>".to_string()
        } else {
            mapper.map_type(ty)
        }
    };

    let filtered_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| !field_references_excluded_type(&f.ty, exclude_types))
        .cloned()
        .collect();

    let field_names: Vec<String> = filtered_fields.iter().map(|f| f.name.clone()).collect();

    let has_field_defaults = filtered_fields.iter().any(|field| field.has_bare_serde_enum_default());
    let (param_list, _, assignments) = if typ.has_default {
        crate::codegen::shared::config_constructor_parts_with_options(&filtered_fields, &map_fn, true, typ)
    } else if has_field_defaults {
        crate::codegen::shared::constructor_parts_with_field_defaults(&filtered_fields, &map_fn, typ)
    } else {
        constructor_parts(&filtered_fields, &map_fn)
    };
    let param_list = rewrite_complex_newtype_constructor_params(&param_list, &filtered_fields, typ.has_default);

    let (assignments, use_defaults) = if typ.has_default {
        preserve_optional_constructor_defaults(&assignments, &filtered_fields)
    } else {
        (assignments, false)
    };
    let borrowed = borrowable_constructor_fields(typ, &filtered_fields, mapper, class_type_names);
    let (param_list_camel, assignments_camel) =
        convert_constructor_params_to_camel_case(&param_list, &assignments, &field_names, &borrowed);

    let field_count = filtered_fields.iter().filter(|f| f.cfg.is_none()).count();
    let mut attrs = Vec::new();
    if field_count > 7 {
        attrs.push("#[allow(clippy::too_many_arguments)]");
    }
    attrs.push("#[allow(non_snake_case)]");

    crate::backends::wasm::template_env::render(
        "gen_struct_constructor",
        crate::alef_context! {
            doc_lines => consumed_argument_doc(&filtered_fields, mapper, class_type_names, &borrowed),
            attrs => attrs,
            param_list => param_list_camel,
            assignments => assignments_camel,
            struct_name => format!("{prefix}{}", typ.name),
            use_defaults => use_defaults,
        },
    )
    .trim_end()
    .to_string()
}

/// Replace constructor parameter types for the specific fields whose explicit container path
/// crosses the WASM boundary as `JsValue`.
///
/// A `TypeMapper` receives only `&TypeRef`, so using it to recognize these fields also changes an
/// ordinary sibling with an equal `TypeRef`, and an excluded complex field can poison an included
/// sibling. Shared constructor generation gets the ordinary mapper; this pass then rewrites only
/// included fields carrying the explicit complex-wrapper marker. ~keep
fn rewrite_complex_newtype_constructor_params(
    param_list: &str,
    fields: &[FieldDef],
    optionalize_all_fields: bool,
) -> String {
    let mut rewritten = param_list.to_string();
    for field in fields.iter().filter(|field| complex_newtype_field_uses_jsvalue(field)) {
        let optional_parameter = optionalize_all_fields || field.optional || field.has_bare_serde_enum_default();
        let jsvalue_type = if matches!(field.ty, TypeRef::Optional(_)) || optional_parameter {
            "Option<JsValue>".to_string()
        } else {
            "JsValue".to_string()
        };
        let marker = format!("{}: ", field.name);
        let Some(marker_start) = rewritten.find(&marker) else {
            continue;
        };
        let type_start = marker_start + marker.len();
        let type_end = rewritten[type_start..]
            .find(',')
            .map_or(rewritten.len(), |offset| type_start + offset);
        rewritten.replace_range(type_start..type_end, &jsvalue_type);
    }
    rewritten
}

/// The fields whose constructor parameter can be taken as `&Wasm{Type}` instead of by value.
///
/// wasm-bindgen lowers a by-value exported-struct argument through `__destroy_into_raw()`, so
/// `new Options(palette, ...)` leaves the caller's `palette` handle dead (alef#472). A borrow is
/// passed as a plain `__wbg_ptr` read and leaves it alive.
///
/// Only a *required* parameter of a struct that does not derive `Default` qualifies, and that is
/// a wasm-bindgen limit rather than a conservative choice: `Option<&T>` has no
/// `OptionFromWasmAbi` impl, so an optional parameter cannot be borrowed at all -- and
/// `shared::config_constructor_parts_inner`, the arm taken when `typ.has_default`, types *every*
/// parameter as `Option<T>`, required ones included. A `#[derive(Default)]` options struct
/// therefore has no borrowable constructor parameter whatsoever. That remaining half of alef#472
/// is tracked separately; `consumed_argument_doc` is what tells the consumer about it. ~keep
fn borrowable_constructor_fields(
    typ: &TypeDef,
    fields: &[FieldDef],
    mapper: &WasmMapper,
    class_type_names: &AHashSet<String>,
) -> AHashSet<String> {
    if typ.has_default {
        return AHashSet::default();
    }
    fields
        .iter()
        .filter(|f| {
            !f.optional && !f.has_bare_serde_enum_default() && f.cfg.is_none() && !matches!(f.ty, TypeRef::Optional(_))
        })
        .filter(|f| types_helpers::class_backed_field_type(f, mapper, class_type_names).is_some())
        .map(|f| f.name.clone())
        .collect()
}

/// Rustdoc stating the ownership contract for each class-typed argument this constructor takes
/// by value, or nothing when it takes none.
///
/// These are exactly the class-typed parameters `borrowable_constructor_fields` could not take
/// by reference, and taking ownership of them is wasm-bindgen's ordinary lowering for a by-value
/// exported-struct argument, not a defect: passing a freshly built value is correct and is what
/// a constructor argument is normally for. It becomes a trap only for a handle the caller still
/// holds, where the failure is a `null pointer passed to rust` at the *next* use, far from this
/// call -- so the doc names the safe alternative rather than only the hazard.
///
/// This text is the resolution of alef#479: there is no non-breaking way to remove the
/// ownership transfer (`Option<&T>` has no `OptionFromWasmAbi` impl, and dropping the parameter
/// changes a published constructor signature), and `default()` plus the borrowed setters #470
/// added is already a complete construction path. Keep it accurate rather than deleting it. ~keep
fn consumed_argument_doc(
    fields: &[FieldDef],
    mapper: &WasmMapper,
    class_type_names: &AHashSet<String>,
    borrowed: &AHashSet<String>,
) -> Vec<String> {
    let consumed: Vec<String> = fields
        .iter()
        .filter(|f| !borrowed.contains(&f.name))
        .filter(|f| types_helpers::class_backed_field_type(f, mapper, class_type_names).is_some())
        .map(|f| format!("`{}`", to_node_name(&f.name)))
        .collect();
    let consumed_vectors: Vec<String> = fields
        .iter()
        .filter(|f| types_helpers::class_backed_vec_element_type(f, mapper, class_type_names).is_some())
        .map(|f| format!("`{}`", to_node_name(&f.name)))
        .collect();
    let mut lines = Vec::new();
    if !consumed.is_empty() {
        lines.extend([
            format!("Takes ownership of {}.", consumed.join(", ")),
            "wasm-bindgen lowers a by-value class argument through `__destroy_into_raw()`, so each".to_string(),
            "of those handles is dead once this returns. Passing a freshly built value is fine. To".to_string(),
            "keep a handle you already hold, build with `default()` and assign the property instead:".to_string(),
            "the generated setter borrows and leaves your handle alive.".to_string(),
        ]);
    }
    if !consumed_vectors.is_empty() {
        lines.extend([
            format!("Takes ownership of every element in {}.", consumed_vectors.join(", ")),
            "wasm-bindgen cannot borrow class values nested in an array. To keep those handles,".to_string(),
            "copy each retained element with `copyForTransfer()` before passing the array.".to_string(),
        ]);
    }
    lines
}

/// Generate a `default()` static factory method.
///
/// Provides an arg-free way to obtain a fresh instance for types whose constructor
/// requires positional arguments. Every wasm struct derives `Default`.
fn gen_default_method(typ: &TypeDef, prefix: &str) -> String {
    format!(
        "#[wasm_bindgen]\n#[allow(clippy::should_implement_trait)]\npub fn default() -> {prefix}{} {{\n    <{prefix}{} as ::core::default::Default>::default()\n}}",
        typ.name, typ.name
    )
}

/// Emit an independent JS handle that is safe to give to an ABI position which takes ownership.
/// wasm-bindgen cannot borrow class values nested in `Vec<T>`, so callers retain the original and
/// transfer this clone instead. The generated name is reserved for Alef's ownership adapter. ~keep
fn gen_transfer_copy_method(typ: &TypeDef, prefix: &str) -> String {
    format!(
        "/// Return an independent handle for an ownership-taking API.\n\
         ///\n\
         /// Use `copyForTransfer()` for each class value placed in an array when the original\n\
         /// handle must remain usable.\n\
         #[wasm_bindgen(js_name = \"copyForTransfer\")]\n\
         pub fn copy_for_transfer(&self) -> {prefix}{} {{\n    self.clone()\n}}",
        typ.name
    )
}
