use crate::core::ir::{DefaultValue, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};
use ahash::AHashSet;
use std::collections::{HashMap, HashSet};

/// Recursively replace `Named(n)` references where `n` is excluded from the binding's public surface
/// (e.g. `InternalDocument`) with `TypeRef::Json`. An excluded type is never emitted as a binding
/// declaration, so a trait-bridge interface/stub method referencing it would be an undefined name;
/// the runtime bridge marshals such values as JSON, so `Json` is the faithful stand-in. Shared by
/// the go/pyo3/napi/magnus excluded-type handling so the substitution stays identical across them.
pub fn substitute_excluded_types(ty: &TypeRef, excluded: &HashSet<&str>) -> TypeRef {
    match ty {
        TypeRef::Named(name) if excluded.contains(name.as_str()) => TypeRef::Json,
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(substitute_excluded_types(inner, excluded))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(substitute_excluded_types(inner, excluded))),
        TypeRef::Map(k, v) => TypeRef::Map(
            Box::new(substitute_excluded_types(k, excluded)),
            Box::new(substitute_excluded_types(v, excluded)),
        ),
        other => other.clone(),
    }
}

/// Fields that should be emitted in generated binding structs.
///
/// Source-level binding exclusions (`#[doc(hidden)]` / `#[cfg_attr(alef, alef(skip))]`)
/// keep the field in IR so conversion code can still default the core field, but public
/// language DTOs must not expose it.
pub fn binding_fields(fields: &[FieldDef]) -> impl Iterator<Item = &FieldDef> {
    fields.iter().filter(|field| !field.binding_excluded)
}

/// Returns true if this parameter is required but must be promoted to optional
/// because it follows an optional parameter in the list.
/// PyO3 requires that required params come before all optional params.
pub fn is_promoted_optional(params: &[ParamDef], idx: usize) -> bool {
    if params[idx].optional {
        return false;
    }
    params[..idx].iter().any(|p| p.optional)
}

/// Check if a free function can be auto-delegated to the core crate.
/// Opaque Named params are allowed (unwrapped via Arc). Non-opaque Named params are not
/// (require From impls that may not exist for types with sanitized fields).
///
/// For extendr R backend: slice params `&[T]` (represented as `Vec<T>` with `is_ref=true`)
/// are delegatable because extendr can convert them to `Vec<T>` at the boundary.
pub fn can_auto_delegate_function(func: &crate::core::ir::FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    !func.sanitized
        && func.params.iter().all(|p| {
            !p.sanitized
                && is_delegatable_param_with_slices(&p.ty, opaque_types)
                && !is_named_ref_param(p, opaque_types)
        })
        && is_delegatable_return(&func.return_type)
}

/// Check if all params and return type are delegatable.
/// For opaque types, skip methods with RefMut receiver (cannot borrow Arc mutably).
///
/// For extendr R backend: slice params `&[T]` (represented as `Vec<T>` with `is_ref=true`)
/// are delegatable because extendr can convert them to `Vec<T>` at the boundary.
pub fn can_auto_delegate(method: &MethodDef, opaque_types: &AHashSet<String>) -> bool {
    if matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none() {
        return false;
    }
    !method.sanitized
        && method.params.iter().all(|p| {
            !p.sanitized
                && is_delegatable_param_with_slices(&p.ty, opaque_types)
                && !is_named_ref_param(p, opaque_types)
        })
        && is_delegatable_return(&method.return_type)
}

/// Like [`can_auto_delegate`] but permits non-opaque Named `&T` params (and `&[T]` / `&[&str]`
/// slices), because the caller emits owned `_core` let-bindings and passes a borrow of them.
///
/// This is the method-level analogue of the free-function `can_delegate_with_named_let_bindings`
/// check. It must only be used by generators whose delegation body wires up
/// `gen_named_let_bindings_pub` + `gen_call_args_with_let_bindings` (e.g. the shared static
/// method generator); generators that emit inline `.into()` call args must keep using
/// [`can_auto_delegate`].
pub fn can_auto_delegate_with_named_let_bindings(method: &MethodDef, opaque_types: &AHashSet<String>) -> bool {
    if matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none() {
        return false;
    }
    !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && is_delegatable_param_with_slices(&p.ty, opaque_types))
        && is_delegatable_return(&method.return_type)
}

/// A Named param with is_ref=true needs a let-binding (can't inline .into() + borrow).
/// A `Vec<String>` param with is_ref=true needs conversion to `Vec<&str>`.
/// A `Vec<NonOpaqueNamed>` param with is_ref=true needs a let-binding (gen_php_call_args emits
/// `&{name}_core[..]` which is only valid when a let binding for `{name}_core` exists).
/// Public alias for use by backend-specific codegen (e.g. napi types.rs opaque delegate check).
pub fn is_named_ref_param_pub(p: &crate::core::ir::ParamDef, opaque_types: &AHashSet<String>) -> bool {
    is_named_ref_param(p, opaque_types)
}

fn is_named_ref_param(p: &crate::core::ir::ParamDef, opaque_types: &AHashSet<String>) -> bool {
    if !p.is_ref {
        return false;
    }
    match &p.ty {
        TypeRef::Named(name) => !opaque_types.contains(name.as_str()),
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char => true,
            TypeRef::Named(name) => !opaque_types.contains(name.as_str()),
            _ => false,
        },
        _ => false,
    }
}

/// A param type is delegatable if it's simple, or a Named type (opaque → Arc unwrap, non-opaque → .into()).
///
/// `Json` is delegatable: the binding takes a JSON string and `gen_call_args` emits
/// `serde_json::from_str(...)` to bridge it into the core `serde_json::Value` parameter.
/// All Rust-based bindings already depend on serde_json (Json field round-tripping uses it).
pub fn is_delegatable_param(ty: &TypeRef, _opaque_types: &AHashSet<String>) -> bool {
    is_delegatable_param_with_slices(ty, _opaque_types)
}

/// Like `is_delegatable_param` but aware of slice parameters `&[T]` (represented as `Vec<T>` with `is_ref=true`).
/// Extendr R backend can auto-delegate slices by converting them to owned `Vec<T>` at the boundary.
fn is_delegatable_param_with_slices(ty: &TypeRef, _opaque_types: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) => is_delegatable_param_with_slices(inner, _opaque_types),
        TypeRef::Vec(inner) => is_delegatable_param_with_slices(inner, _opaque_types),
        TypeRef::Map(k, v) => {
            is_delegatable_param_with_slices(k, _opaque_types) && is_delegatable_param_with_slices(v, _opaque_types)
        }
    }
}

/// Return types are more permissive — Named types work via .into() (core→binding From exists).
///
/// `Json` is delegatable: the binding returns a JSON string and the core `serde_json::Value`
/// is serialized via `.to_string()` by `wrap_return_with_mutex_mapped`.
pub fn is_delegatable_return(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_delegatable_return(inner),
        TypeRef::Map(k, v) => is_delegatable_return(k) && is_delegatable_return(v),
    }
}

/// A type is delegatable if it can cross the binding boundary without From impls.
/// Named types are NOT delegatable as function params (may lack From impls).
/// For opaque methods, Named types are handled separately via Arc wrap/unwrap.
pub fn is_delegatable_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Named(_) => false,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_delegatable_type(inner),
        TypeRef::Map(k, v) => is_delegatable_type(k) && is_delegatable_type(v),
        TypeRef::Json => false,
    }
}

/// Check if a type is delegatable in the opaque method context.
/// Opaque methods can handle Named params via Arc unwrap and Named returns via Arc wrap.
///
/// `Json` is delegatable: for params, `gen_call_args` emits `serde_json::from_str(&name)` to
/// bridge the binding's `String` into the core's `serde_json::Value`; for return types,
/// `wrap_return_with_mutex_mapped` serializes the `Value` back to a `String` via `.to_string()`.
/// All Rust-based bindings already depend on serde_json (Json field round-tripping uses it).
pub fn is_opaque_delegatable_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_opaque_delegatable_type(inner),
        TypeRef::Map(k, v) => is_opaque_delegatable_type(k) && is_opaque_delegatable_type(v),
    }
}

/// Check if a type is "simple" — can be passed without any conversion.
pub fn is_simple_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_simple_type(inner),
        TypeRef::Map(k, v) => is_simple_type(k) && is_simple_type(v),
        TypeRef::Named(_) | TypeRef::Json => false,
    }
}

/// Partition methods into (instance, static).
pub fn partition_methods(methods: &[MethodDef]) -> (Vec<&MethodDef>, Vec<&MethodDef>) {
    let instance: Vec<_> = methods.iter().filter(|m| m.receiver.is_some()).collect();
    let statics: Vec<_> = methods.iter().filter(|m| m.receiver.is_none()).collect();
    (instance, statics)
}

/// Build a constructor parameter list string.
/// Returns (param_list, signature_with_defaults, field_assignments).
/// If param_list exceeds 100 chars, uses multiline format with trailing commas.
pub fn constructor_parts(fields: &[FieldDef], type_mapper: &dyn Fn(&TypeRef) -> String) -> (String, String, String) {
    constructor_parts_with_renames_and_cfg_restore(fields, type_mapper, None, &[])
}

/// Like `constructor_parts` but with optional field renames for keyword escaping.
/// `field_renames` maps original field name → binding field name (e.g. "class" → "class_").
/// Parameters keep the original name (valid in Rust), struct literal uses the renamed field.
pub fn constructor_parts_with_renames(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    field_renames: Option<&HashMap<String, String>>,
) -> (String, String, String) {
    constructor_parts_with_renames_and_cfg_restore(fields, type_mapper, field_renames, &[])
}

/// Like `constructor_parts_with_renames` but also includes assignments for cfg-gated fields
/// that have been force-restored via trait-bridge `bind_via = "options_field"`. Such fields
/// are absent from the constructor parameter list but must be present in the `Self { ... }`
/// struct literal — emitted as `field: Default::default()` so the binding struct compiles.
pub fn constructor_parts_with_renames_and_cfg_restore(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    field_renames: Option<&HashMap<String, String>>,
    never_skip_cfg_field_names: &[String],
) -> (String, String, String) {
    let mut sorted_fields: Vec<&FieldDef> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| f.cfg.is_none() || never_skip_cfg_field_names.contains(&f.name))
        .collect();
    sorted_fields.sort_by_key(|f| (f.optional || f.cfg.is_some()) as u8);

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            let is_optional = f.optional || f.cfg.is_some();
            let ty = if is_optional {
                match &f.ty {
                    TypeRef::Optional(_) => type_mapper(&f.ty),
                    _ => format!("Option<{}>", type_mapper(&f.ty)),
                }
            } else {
                type_mapper(&f.ty)
            };
            format!("{}: {}", f.name, ty)
        })
        .collect();

    let defaults: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            if f.optional || f.cfg.is_some() {
                format!("{}=None", f.name)
            } else {
                f.name.clone()
            }
        })
        .collect();

    let assignments: Vec<String> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| {
            let binding_name = field_renames
                .and_then(|r| r.get(&f.name))
                .map_or_else(|| f.name.as_str(), |s| s.as_str());
            if f.cfg.is_some() && !never_skip_cfg_field_names.contains(&f.name) {
                return format!("{}: Default::default()", binding_name);
            }
            if binding_name != f.name {
                return binding_name.to_string();
            }
            f.name.clone()
        })
        .collect();

    let single_line = params.join(", ");
    let param_list = if single_line.len() > 100 {
        format!("\n        {},\n    ", params.join(",\n        "))
    } else {
        single_line
    };

    (param_list, defaults.join(", "), assignments.join(", "))
}

/// Build a function parameter list.
pub fn function_params(params: &[ParamDef], type_mapper: &dyn Fn(&TypeRef) -> String) -> String {
    function_params_vec(params, type_mapper).join(", ")
}

/// Per-parameter `name: type` strings, before joining. Callers that render the signature
/// across multiple lines (long-signature wrapping) must reuse this exact list rather than
/// recomputing types — a separate recomputation diverges from the backend-aware mapping used
/// for the single-line form (e.g. extendr's `Nullable<&T>`), producing a signature whose types
/// disagree with the generated body and fail to compile.
pub fn function_params_vec(params: &[ParamDef], type_mapper: &dyn Fn(&TypeRef) -> String) -> Vec<String> {
    let mut seen_optional = false;
    params
        .iter()
        .map(|p| {
            if p.optional {
                seen_optional = true;
            }
            let ty = if p.optional || seen_optional {
                format!("Option<{}>", type_mapper(&p.ty))
            } else {
                type_mapper(&p.ty)
            };
            format!("{}: {}", p.name, ty)
        })
        .collect::<Vec<_>>()
}

/// Build a function signature defaults string (for pyo3 signature etc.).
pub fn function_sig_defaults(params: &[ParamDef]) -> String {
    let mut seen_optional = false;
    params
        .iter()
        .map(|p| {
            if p.optional {
                seen_optional = true;
            }
            if p.optional {
                format!("{}=None", p.name)
            } else if seen_optional {
                let default = match &p.ty {
                    TypeRef::Primitive(PrimitiveType::Bool) => "false",
                    TypeRef::Primitive(_) => "0",
                    _ => "None",
                };
                format!("{}={}", p.name, default)
            } else {
                p.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Format a DefaultValue as Rust code for the target language.
/// Used by backends generating config constructors with defaults.
pub fn format_default_value(default: &DefaultValue) -> String {
    match default {
        DefaultValue::BoolLiteral(b) => format!("{}", b),
        DefaultValue::StringLiteral(s) => format!("\"{}\".to_string()", s.escape_default()),
        DefaultValue::IntLiteral(i) => format!("{}", i),
        DefaultValue::FloatLiteral(f) => {
            let s = format!("{}", f);
            if s.contains('.') || s.contains('e') || s.contains('E') {
                s
            } else {
                format!("{s}.0")
            }
        }
        DefaultValue::EnumVariant(v) => v.clone(),
        DefaultValue::Empty => "Default::default()".to_string(),
        DefaultValue::None => "None".to_string(),
    }
}

/// Generate constructor parameter and assignment lists for types with has_default.
/// All fields become `Option<T>` with None defaults for optional fields,
/// or unwrap_or_else with actual defaults for required fields.
///
/// Returns (param_list, signature_defaults, assignments).
/// This is used by PyO3 and similar backends that need signature annotations.
/// Like `config_constructor_parts` but with extra options.
/// When `option_duration_on_defaults` is true, non-optional Duration fields are stored
/// as `Option<u64>` in the binding struct, so the constructor assignment is a passthrough
/// (the From conversion will handle the None → core default mapping).
pub fn config_constructor_parts_with_options(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
) -> (String, String, String) {
    config_constructor_parts_with_options_cfg(fields, type_mapper, option_duration_on_defaults, false)
}

pub fn config_constructor_parts_with_options_cfg(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    optionalize_all_defaults: bool,
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        option_duration_on_defaults,
        optionalize_all_defaults,
        None,
        &[],
    )
}

/// Like `config_constructor_parts_with_options` but with field renames for keyword escaping.
pub fn config_constructor_parts_with_renames(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    field_renames: Option<&HashMap<String, String>>,
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        option_duration_on_defaults,
        false,
        field_renames,
        &[],
    )
}

/// Like `config_constructor_parts_with_renames` but includes assignments for cfg-gated fields
/// force-restored via `never_skip_cfg_field_names` (emitted as `field: Default::default()`).
pub fn config_constructor_parts_with_renames_and_cfg_restore(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    field_renames: Option<&HashMap<String, String>>,
    never_skip_cfg_field_names: &[String],
) -> (String, String, String) {
    config_constructor_parts_inner(
        fields,
        type_mapper,
        option_duration_on_defaults,
        false,
        field_renames,
        never_skip_cfg_field_names,
    )
}

pub fn config_constructor_parts(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
) -> (String, String, String) {
    config_constructor_parts_inner(fields, type_mapper, false, false, None, &[])
}

fn config_constructor_parts_inner(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    optionalize_all_defaults: bool,
    field_renames: Option<&HashMap<String, String>>,
    never_skip_cfg_field_names: &[String],
) -> (String, String, String) {
    let mut sorted_fields: Vec<&FieldDef> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| f.cfg.is_none() || never_skip_cfg_field_names.contains(&f.name))
        .collect();
    sorted_fields.sort_by_key(|f| f.optional as u8);

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            let ty = type_mapper(&f.ty);
            if matches!(f.ty, TypeRef::Optional(_)) {
                format!("{}: {}", f.name, ty)
            } else {
                format!("{}: Option<{}>", f.name, ty)
            }
        })
        .collect();

    let defaults = sorted_fields
        .iter()
        .map(|f| format!("{}=None", f.name))
        .collect::<Vec<_>>()
        .join(", ");

    let assignments: Vec<String> = fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| {
            let binding_name = field_renames
                .and_then(|r| r.get(&f.name))
                .map_or_else(|| f.name.as_str(), |s| s.as_str());
            if f.cfg.is_some() {
                if never_skip_cfg_field_names.contains(&f.name) {
                    if f.optional || matches!(&f.ty, TypeRef::Optional(_)) {
                        return format!("{}: {}", binding_name, f.name);
                    }
                    return format!("{}: {}.unwrap_or_default()", binding_name, f.name);
                }
                return format!("{}: Default::default()", binding_name);
            }
            if (option_duration_on_defaults && matches!(f.ty, TypeRef::Duration)) || optionalize_all_defaults {
                return format!("{}: {}", binding_name, f.name);
            }
            if f.optional || matches!(&f.ty, TypeRef::Optional(_)) {
                format!("{}: {}", binding_name, f.name)
            } else if let Some(ref typed_default) = f.typed_default {
                match typed_default {
                    DefaultValue::EnumVariant(_) | DefaultValue::Empty => {
                        format!("{}: {}.unwrap_or_default()", binding_name, f.name)
                    }
                    _ => {
                        let default_val = format_default_value(typed_default);
                        // clippy::unnecessary_lazy_evaluations; use unwrap_or_else for heap types.
                        match typed_default {
                            DefaultValue::BoolLiteral(_)
                            | DefaultValue::IntLiteral(_)
                            | DefaultValue::FloatLiteral(_) => {
                                format!("{}: {}.unwrap_or({})", binding_name, f.name, default_val)
                            }
                            _ => {
                                format!("{}: {}.unwrap_or_else(|| {})", binding_name, f.name, default_val)
                            }
                        }
                    }
                }
            } else {
                format!("{}: {}.unwrap_or_default()", binding_name, f.name)
            }
        })
        .collect();

    let single_line = params.join(", ");
    let param_list = if single_line.len() > 100 {
        format!("\n        {},\n    ", params.join(",\n        "))
    } else {
        single_line
    };

    (param_list, defaults, assignments.join(", "))
}

/// Format extra clippy allows for insertion into generated Rust binding files.
///
/// Accepts bare lint names (`"single_match"`) or `clippy::`-prefixed names
/// (`"clippy::single_match"`); both forms are normalised to the `clippy::` prefix.
///
/// Returns `None` when `extras` is empty — callers must skip emission entirely in
/// that case so output is byte-identical to the no-config baseline.
///
/// Returns `Some(attr)` where `attr` is the inner content of an `allow(...)` call,
/// e.g. `"allow(clippy::single_match, clippy::collapsible_match)"`.  Pass this
/// directly to [`crate::codegen::builder::RustFileBuilder::add_inner_attribute`]
/// or format it into a raw `#![allow(...)]` attribute string.
///
/// `already_emitted` is the attribute text emitted above this call (each backend's
/// default allow block). Lints already present there are filtered out so the extra
/// block never re-allows a lint — a duplicate would trip clippy's
/// `duplicated_attributes` lint under `-D warnings`. Returns `None` when nothing new
/// remains, so callers skip emission entirely.
pub fn format_extra_clippy_allows(extras: &[String], already_emitted: &str) -> Option<String> {
    if extras.is_empty() {
        return None;
    }
    let already = collect_clippy_lints(already_emitted);
    let mut seen = HashSet::new();
    let normalized: Vec<String> = extras
        .iter()
        .map(|s| {
            let trimmed = s.trim();
            if trimmed.starts_with("clippy::") {
                trimmed.to_string()
            } else {
                format!("clippy::{trimmed}")
            }
        })
        .filter(|s| !already.contains(s.as_str()))
        .filter(|s| seen.insert(s.clone()))
        .collect();
    if normalized.is_empty() {
        return None;
    }
    Some(format!("allow({})", normalized.join(", ")))
}

/// Collect the `clippy::<lint>` tokens present in already-emitted attribute text,
/// used to de-duplicate extra clippy allows against a backend's default allow block.
fn collect_clippy_lints(text: &str) -> HashSet<&str> {
    const PREFIX: &str = "clippy::";
    let mut lints = HashSet::new();
    let mut rest = text;
    while let Some(idx) = rest.find(PREFIX) {
        let after = &rest[idx + PREFIX.len()..];
        let name_len = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let token_end = idx + PREFIX.len() + name_len;
        if name_len > 0 {
            lints.insert(&rest[idx..token_end]);
        }
        rest = &rest[token_end..];
    }
    lints
}
