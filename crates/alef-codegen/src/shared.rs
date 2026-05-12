use ahash::AHashSet;
use alef_core::ir::{DefaultValue, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};
use std::collections::HashMap;

/// Returns true if this parameter is required but must be promoted to optional
/// because it follows an optional parameter in the list.
/// PyO3 requires that required params come before all optional params.
pub fn is_promoted_optional(params: &[ParamDef], idx: usize) -> bool {
    if params[idx].optional {
        return false; // naturally optional
    }
    // Check if any earlier param is optional
    params[..idx].iter().any(|p| p.optional)
}

/// Check if a free function can be auto-delegated to the core crate.
/// Opaque Named params are allowed (unwrapped via Arc). Non-opaque Named params are not
/// (require From impls that may not exist for types with sanitized fields).
pub fn can_auto_delegate_function(func: &alef_core::ir::FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    !func.sanitized
        && func
            .params
            .iter()
            .all(|p| !p.sanitized && is_delegatable_param(&p.ty, opaque_types) && !is_named_ref_param(p, opaque_types))
        && is_delegatable_return(&func.return_type)
}

/// Check if all params and return type are delegatable.
/// For opaque types, skip methods with RefMut receiver (cannot borrow Arc mutably).
pub fn can_auto_delegate(method: &MethodDef, opaque_types: &AHashSet<String>) -> bool {
    // Skip RefMut methods on opaque types (Arc doesn't allow mutable access)
    if matches!(method.receiver, Some(ReceiverKind::RefMut)) && method.trait_source.is_none() {
        return false;
    }
    !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && is_delegatable_param(&p.ty, opaque_types) && !is_named_ref_param(p, opaque_types))
        && is_delegatable_return(&method.return_type)
}

/// A Named param with is_ref=true needs a let-binding (can't inline .into() + borrow).
/// A Vec<String> param with is_ref=true needs conversion to Vec<&str>.
fn is_named_ref_param(p: &alef_core::ir::ParamDef, opaque_types: &AHashSet<String>) -> bool {
    if !p.is_ref {
        return false;
    }
    match &p.ty {
        TypeRef::Named(name) => !opaque_types.contains(name.as_str()),
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::String | TypeRef::Char),
        _ => false,
    }
}

/// A param type is delegatable if it's simple, or a Named type (opaque → Arc unwrap, non-opaque → .into()).
pub fn is_delegatable_param(ty: &TypeRef, _opaque_types: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Named(_) => true, // Opaque: &*param.inner; non-opaque: .into()
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_delegatable_param(inner, _opaque_types),
        TypeRef::Map(k, v) => is_delegatable_param(k, _opaque_types) && is_delegatable_param(v, _opaque_types),
        TypeRef::Json => false,
    }
}

/// Return types are more permissive — Named types work via .into() (core→binding From exists).
pub fn is_delegatable_return(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Named(_) => true, // core→binding From impl generated for all convertible types
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_delegatable_return(inner),
        TypeRef::Map(k, v) => is_delegatable_return(k) && is_delegatable_return(v),
        TypeRef::Json => false,
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
        TypeRef::Named(_) => false, // Requires From impl which may not exist
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_delegatable_type(inner),
        TypeRef::Map(k, v) => is_delegatable_type(k) && is_delegatable_type(v),
        TypeRef::Json => false,
    }
}

/// Check if a type is delegatable in the opaque method context.
/// Opaque methods can handle Named params via Arc unwrap and Named returns via Arc wrap.
pub fn is_opaque_delegatable_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration => true,
        TypeRef::Named(_) => true, // Opaque: Arc unwrap/wrap. Non-opaque: .into()
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_opaque_delegatable_type(inner),
        TypeRef::Map(k, v) => is_opaque_delegatable_type(k) && is_opaque_delegatable_type(v),
        TypeRef::Json => false,
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
    constructor_parts_with_renames(fields, type_mapper, None)
}

/// Like `constructor_parts` but with optional field renames for keyword escaping.
/// `field_renames` maps original field name → binding field name (e.g. "class" → "class_").
/// Parameters keep the original name (valid in Rust), struct literal uses the renamed field.
pub fn constructor_parts_with_renames(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    field_renames: Option<&HashMap<String, String>>,
) -> (String, String, String) {
    // Sort fields: required first, then optional.
    // Many FFI frameworks (PyO3, NAPI) require required params before optional ones.
    // Skip cfg-gated fields — they depend on features that may not be enabled.
    let mut sorted_fields: Vec<&FieldDef> = fields.iter().filter(|f| f.cfg.is_none()).collect();
    sorted_fields.sort_by_key(|f| f.optional as u8);

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            let ty = if f.optional {
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
            if f.optional {
                format!("{}=None", f.name)
            } else {
                f.name.clone()
            }
        })
        .collect();

    // Assignments keep original field order (for struct literal). Cfg-gated fields
    // remain present on the binding struct, so emit a default expression for them
    // (the caller cannot supply the trait-bridge wrapper value through the params).
    // When a field is renamed, emit `renamed: original` instead of just `original`.
    let assignments: Vec<String> = fields
        .iter()
        .map(|f| {
            let binding_name = field_renames
                .and_then(|r| r.get(&f.name))
                .map_or_else(|| f.name.as_str(), |s| s.as_str());
            if f.cfg.is_some() {
                let default_expr = match &f.ty {
                    TypeRef::Optional(_) => "None",
                    _ => "Default::default()",
                };
                return format!("{binding_name}: {default_expr}");
            }
            if binding_name != f.name {
                return format!("{binding_name}: {}", f.name);
            }
            f.name.clone()
        })
        .collect();

    // Format param_list with line wrapping if needed
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
    // After the first optional param, all subsequent params must also be optional
    // to satisfy PyO3's signature constraint (required params can't follow optional ones).
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
        .join(", ")
}

/// Build a function signature defaults string (for pyo3 signature etc.).
pub fn function_sig_defaults(params: &[ParamDef]) -> String {
    // After the first optional param, all subsequent params must also carry a default
    // to satisfy PyO3's signature constraint (required params can't follow optional ones).
    // For optional params and Named/non-primitive promoted params: use `=None`.
    // For promoted non-optional primitive params: use a type-appropriate zero/false default
    // so PyO3 does not wrap the Rust type in Option<T> (which would cause a `?` unwrap error).
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
                // Promoted non-optional param: emit a type-appropriate default instead of None
                // so PyO3 keeps the Rust parameter type as T (not Option<T>).
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
            // Ensure the literal is a valid Rust float (must contain '.' or 'e'/'E')
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
/// All fields become Option<T> with None defaults for optional fields,
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
    config_constructor_parts_inner(fields, type_mapper, option_duration_on_defaults, None)
}

/// Like `config_constructor_parts_with_options` but with field renames for keyword escaping.
pub fn config_constructor_parts_with_renames(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    field_renames: Option<&HashMap<String, String>>,
) -> (String, String, String) {
    config_constructor_parts_inner(fields, type_mapper, option_duration_on_defaults, field_renames)
}

pub fn config_constructor_parts(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
) -> (String, String, String) {
    config_constructor_parts_inner(fields, type_mapper, false, None)
}

fn config_constructor_parts_inner(
    fields: &[FieldDef],
    type_mapper: &dyn Fn(&TypeRef) -> String,
    option_duration_on_defaults: bool,
    field_renames: Option<&HashMap<String, String>>,
) -> (String, String, String) {
    let mut sorted_fields: Vec<&FieldDef> = fields.iter().filter(|f| f.cfg.is_none()).collect();
    sorted_fields.sort_by_key(|f| f.optional as u8);

    let params: Vec<String> = sorted_fields
        .iter()
        .map(|f| {
            let ty = type_mapper(&f.ty);
            // All fields become Option<T>, but avoid Option<Option<T>> for already-optional fields.
            // When f.ty is TypeRef::Optional(X), type_mapper already returns "Option<X>".
            // Wrapping it again would yield Option<Option<X>>, making `None` ambiguous in PyO3
            // signatures (E0283: type annotations needed).
            if matches!(f.ty, TypeRef::Optional(_)) {
                format!("{}: {}", f.name, ty)
            } else {
                format!("{}: Option<{}>", f.name, ty)
            }
        })
        .collect();

    // All fields have None default in signature
    let defaults = sorted_fields
        .iter()
        .map(|f| format!("{}=None", f.name))
        .collect::<Vec<_>>()
        .join(", ");

    // Assignments use unwrap_or_else with the typed default.
    // `binding_name` is the struct field name (possibly renamed for keyword escaping),
    // `f.name` is the original name used as the constructor parameter.
    // Cfg-gated fields remain on the binding struct but cannot be supplied by the
    // caller (their wrapper types aren't decodable from the host language); emit a
    // default expression for them so the struct literal stays complete.
    let assignments: Vec<String> = fields
        .iter()
        .map(|f| {
            let binding_name = field_renames
                .and_then(|r| r.get(&f.name))
                .map_or_else(|| f.name.as_str(), |s| s.as_str());
            if f.cfg.is_some() {
                let default_expr = match &f.ty {
                    TypeRef::Optional(_) => "None",
                    _ => "Default::default()",
                };
                return format!("{binding_name}: {default_expr}");
            }
            // Duration fields on has_default types are stored as Option<u64> when
            // option_duration_on_defaults is set — treat them as passthrough.
            if option_duration_on_defaults && matches!(f.ty, TypeRef::Duration) {
                return format!("{}: {}", binding_name, f.name);
            }
            if f.optional || matches!(&f.ty, TypeRef::Optional(_)) {
                // Optional fields: passthrough (both param and field are Option<T>)
                format!("{}: {}", binding_name, f.name)
            } else if let Some(ref typed_default) = f.typed_default {
                // For EnumVariant and Empty defaults, use unwrap_or_default()
                // because we can't generate qualified Rust paths here.
                match typed_default {
                    DefaultValue::EnumVariant(_) | DefaultValue::Empty => {
                        format!("{}: {}.unwrap_or_default()", binding_name, f.name)
                    }
                    _ => {
                        let default_val = format_default_value(typed_default);
                        // Use unwrap_or() for Copy literals (bool, int, float) to avoid
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
                // All binding types should impl Default (enums default to first variant,
                // structs default via From<CoreType::default()>). unwrap_or_default() works.
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
