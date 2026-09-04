use crate::codegen::conversions::ConversionConfig;
use crate::codegen::conversions::helpers::{
    clippy_allow_attr_line, core_prim_str, core_type_path_remapped, field_references_excluded_type, is_newtype,
    is_tuple_type_name, needs_clippy_allow, needs_f64_cast, needs_i32_cast, needs_i64_cast,
};
use crate::core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};

use super::fields::field_conversion_to_core_cfg;
use super::wrappers::apply_core_wrapper_to_core;

/// Generate `impl From<BindingType> for core::Type` (binding -> core).
/// Sanitized fields use `Default::default()` unless the sanitizer only removed a
/// core wrapper that can be reconstructed losslessly from the binding value.
pub fn gen_from_binding_to_core(typ: &TypeDef, core_import: &str) -> String {
    gen_from_binding_to_core_cfg(typ, core_import, &ConversionConfig::default())
}

/// Generate `impl From<BindingType> for core::Type` with backend-specific config.
pub fn gen_from_binding_to_core_cfg(typ: &TypeDef, core_import: &str, config: &ConversionConfig) -> String {
    let core_path = core_type_path_remapped(typ, core_import, config.source_crate_remaps);
    let binding_name = format!("{}{}", config.type_name_prefix, typ.name);

    let has_explicit_static_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");

    if is_newtype(typ) {
        let field = &typ.fields[0];
        let newtype_inner_expr = match &field.ty {
            // An identity/tuple-passthrough Named type is never actually converted (no
            // separate binding-side type exists for it), so `.into()` would be a no-op.
            TypeRef::Named(name) if is_tuple_type_name(name) => "val._0".to_string(),
            TypeRef::Named(_) => "val._0.into()".to_string(),
            TypeRef::Path => "val._0.into()".to_string(),
            TypeRef::Duration => "std::time::Duration::from_millis(val._0)".to_string(),
            _ => "val._0".to_string(),
        };
        let (needs_redundant_closure_allow, needs_useless_conversion_allow) =
            needs_clippy_allow(std::iter::once(newtype_inner_expr.as_str()));
        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => core_path,
                binding_name => binding_name,
                has_lifetime_params => typ.has_lifetime_params,
                is_newtype => true,
                has_explicit_static_new => false,
                newtype_inner_expr => newtype_inner_expr,
                builder_mode => false,
                uses_builder_pattern => false,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => vec![] as Vec<String>,
                fields => vec![] as Vec<String>,
                needs_redundant_closure_allow => needs_redundant_closure_allow,
                needs_useless_conversion_allow => needs_useless_conversion_allow,
            },
        );
    }

    if typ.has_lifetime_params {
        if let Some(constructor_call) =
            gen_from_lifetime_type_constructor(typ, &core_path, &binding_name, core_import, config)
        {
            return constructor_call;
        }
        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => &core_path,
                binding_name => &binding_name,
                has_lifetime_params => true,
                is_newtype => false,
                has_explicit_static_new => true,
                newtype_inner_expr => String::new(),
                builder_mode => false,
                uses_builder_pattern => false,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => vec![] as Vec<String>,
                fields => vec![] as Vec<String>,
                needs_redundant_closure_allow => false,
                needs_useless_conversion_allow => false,
            },
        );
    }

    if has_explicit_static_new {
        if let Some(call) = gen_from_explicit_new_constructor(typ, &core_path, &binding_name, config) {
            return call;
        }
        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => &core_path,
                binding_name => &binding_name,
                has_lifetime_params => false,
                is_newtype => false,
                has_explicit_static_new => true,
                newtype_inner_expr => String::new(),
                builder_mode => false,
                uses_builder_pattern => false,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => vec![] as Vec<String>,
                fields => vec![] as Vec<String>,
                needs_redundant_closure_allow => false,
                needs_useless_conversion_allow => false,
            },
        );
    }

    let uses_builder_pattern = (config.option_duration_on_defaults
        && typ.has_default
        && typ
            .fields
            .iter()
            .any(|f| !f.optional && matches!(f.ty, TypeRef::Duration)))
        || (config.optionalize_defaults && typ.has_default);

    let has_optionalized_fields = config.option_duration_on_defaults
        && typ.has_default
        && typ
            .fields
            .iter()
            .any(|f| !f.optional && matches!(f.ty, TypeRef::Duration));

    if has_optionalized_fields {
        let optionalized = config.optionalize_defaults && typ.has_default;
        let mut statements = Vec::new();
        for field in &typ.fields {
            if field.binding_excluded {
                continue;
            }
            if field.sanitized && field.core_wrapper != CoreWrapper::Cow {
                continue;
            }
            if !config.exclude_types.is_empty() && field_references_excluded_type(&field.ty, config.exclude_types) {
                continue;
            }
            let binding_name_field = config.binding_field_name_owned(&typ.name, &field.name);
            if !field.optional && matches!(field.ty, TypeRef::Duration) {
                let cast = if config.cast_large_ints_to_i64 { " as u64" } else { "" };
                statements.push(format!(
                    "if let Some(__v) = val.{binding_name_field} {{ __result.{} = std::time::Duration::from_millis(__v{cast}); }}",
                    field.name
                ));
                continue;
            }
            let field_is_optionalized_by_duration = config.option_duration_on_defaults
                && typ.has_default
                && !field.optional
                && matches!(field.ty, TypeRef::Duration);
            let field_is_config_optionalized = (optionalized && !field.optional) || field_is_optionalized_by_duration;

            let _field_is_genuinely_optional = config.option_duration_on_defaults && typ.has_default && field.optional;

            let conversion = if field_is_config_optionalized {
                field_conversion_to_core_cfg(&field.name, &field.ty, false, config)
            } else {
                field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config)
            };
            let conversion = if binding_name_field != field.name {
                conversion.replace(&format!("val.{}", field.name), &format!("val.{binding_name_field}"))
            } else {
                conversion
            };
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                if field_is_config_optionalized {
                    statements.push(format!(
                        "if let Some(__v) = val.{binding_name_field} {{ __result.{} = {}; }}",
                        field.name,
                        expr.replace(&format!("val.{binding_name_field}"), "__v")
                    ));
                } else {
                    statements.push(format!("__result.{} = {};", field.name, expr));
                }
            }
        }

        let (needs_redundant_closure_allow, needs_useless_conversion_allow) =
            needs_clippy_allow(statements.iter().map(String::as_str));
        return crate::codegen::template_env::render(
            "conversions/binding_to_core_impl",
            minijinja::context! {
                core_path => core_path,
                binding_name => binding_name,
                has_lifetime_params => typ.has_lifetime_params,
                is_newtype => false,
                has_explicit_static_new => false,
                newtype_inner_expr => "",
                builder_mode => true,
                uses_builder_pattern => uses_builder_pattern,
                has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
                statements => statements,
                fields => vec![] as Vec<String>,
                needs_redundant_closure_allow => needs_redundant_closure_allow,
                needs_useless_conversion_allow => needs_useless_conversion_allow,
            },
        );
    }

    let optionalized = config.optionalize_defaults && typ.has_default;

    if typ.has_private_fields && !optionalized {
        return gen_private_field_construction(typ, &core_path, &binding_name, config);
    }

    let mut fields = Vec::new();
    let mut statements = Vec::new();
    let core_has_default = typ.has_default;

    for field in &typ.fields {
        if field.binding_excluded {
            if !core_has_default {
                fields.push(format!("{}: Default::default()", field.name));
                continue;
            }
            continue;
        }
        let references_excluded =
            !config.exclude_types.is_empty() && field_references_excluded_type(&field.ty, config.exclude_types);
        if references_excluded && typ.has_stripped_cfg_fields {
            continue;
        }
        if field.cfg.is_some()
            && !config.never_skip_cfg_field_names.contains(&field.name)
            && config.strip_cfg_fields_from_binding_struct
        {
            continue;
        }
        if optionalized && ((field.sanitized && field.core_wrapper != CoreWrapper::Cow) || references_excluded) {
            continue;
        }
        let field_was_optionalized = optionalized && !field.optional;
        let binding_name_field = config.binding_field_name_owned(&typ.name, &field.name);
        let conversion = field_core_conversion(field, typ, config, field_was_optionalized, references_excluded);
        // A cfg-gated field (`field.cfg`) that was NOT stripped from the binding struct (the
        // `strip_cfg_fields_from_binding_struct` branch above) still needs its own gate repeated
        // on every reference to it here -- `val.{binding_name_field}` reads a binding-struct
        // field that only exists under the same feature, and the assignment target on the core
        // side may be gated too (an `E0609`/`E0425` pair otherwise, symmetric with the
        // core-to-binding direction). ~keep
        let gate = field.cfg.as_deref().filter(|gate| !gate.is_empty());
        if optionalized {
            if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
                let statement = if field_was_optionalized {
                    format!(
                        "if let Some(__v) = val.{binding_name_field} {{ __result.{} = {}; }}",
                        field.name,
                        expr.replace(&format!("val.{binding_name_field}"), "__v")
                    )
                } else {
                    format!("__result.{} = {};", field.name, expr)
                };
                statements.push(match gate {
                    // ~keep The gate goes on a BLOCK, not on the statement itself. `__result.f =
                    // v;` is an expression statement, and an attribute on an expression is
                    // `E0658` -- unstable. A block is a stable place for one, so the assignment
                    // moves inside it. Only this branch needs it: every struct-literal branch
                    // attaches its gate to a field initialiser, which is already legal.
                    Some(g) => format!("#[cfg({g})]\n        {{\n            {statement}\n        }}"),
                    None => statement,
                });
            }
        } else {
            let conversion = match gate {
                Some(g) => format!("#[cfg({g})]\n            {conversion}"),
                None => conversion,
            };
            fields.push(conversion);
        }
    }

    let emit_trailer = typ.has_stripped_cfg_fields || core_has_default;

    let (needs_redundant_closure_allow, needs_useless_conversion_allow) = needs_clippy_allow(
        fields
            .iter()
            .map(String::as_str)
            .chain(statements.iter().map(String::as_str)),
    );

    crate::codegen::template_env::render(
        "conversions/binding_to_core_impl",
        minijinja::context! {
            core_path => core_path,
            binding_name => binding_name,
            has_lifetime_params => typ.has_lifetime_params,
            is_newtype => false,
            has_explicit_static_new => has_explicit_static_new,
            newtype_inner_expr => "",
            builder_mode => optionalized,
            uses_builder_pattern => uses_builder_pattern,
            has_stripped_cfg_fields => emit_trailer,
            statements => statements,
            fields => fields,
            needs_redundant_closure_allow => needs_redundant_closure_allow,
            needs_useless_conversion_allow => needs_useless_conversion_allow,
        },
    )
}

/// Build a `From<Binding> for Core` impl for a type whose core struct has private
/// (non-`pub`) fields.
///
/// Such a type cannot be constructed with struct-literal syntax from a foreign crate, so
/// we seed the core type's `Default` — which fills the private fields with their defaults
/// inside the defining crate — and assign only the public binding fields onto it.
///
/// When the core type does not implement `Default`, there is no foreign-crate construction
/// path (a struct literal cannot set the private fields, and there is no base to seed), so
/// we emit a `compile_error!` guiding the core author to derive `Default` (or otherwise
/// expose a constructor / exclude the type from this backend). Per-field serde `Deserialize`
/// is deliberately not used as a fallback: it suffers from `into()` target-type ambiguity
/// and fragile placeholder generation, which would trade a clear compile error for a subtle
/// runtime one.
fn gen_private_field_construction(
    typ: &TypeDef,
    core_path: &str,
    binding_name: &str,
    config: &ConversionConfig,
) -> String {
    let mut assignments = Vec::new();
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        let references_excluded =
            !config.exclude_types.is_empty() && field_references_excluded_type(&field.ty, config.exclude_types);
        if references_excluded && typ.has_stripped_cfg_fields {
            continue;
        }
        if field.cfg.is_some()
            && !config.never_skip_cfg_field_names.contains(&field.name)
            && config.strip_cfg_fields_from_binding_struct
        {
            continue;
        }
        let conversion = field_core_conversion(field, typ, config, false, references_excluded);
        let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) else {
            continue;
        };
        if expr == "Default::default()" {
            continue;
        }
        assignments.push(crate::codegen::conversions::construction::FieldAssign {
            core_field: field.name.clone(),
            expr: expr.to_string(),
        });
    }

    // When the core type has no `Default`, the emitted impl is a bare `compile_error!` that
    // never references `assignments` at all, so neither lint can fire regardless of their
    // content.
    let (needs_redundant_closure_allow, needs_useless_conversion_allow) = if typ.has_default {
        needs_clippy_allow(assignments.iter().map(|a| a.expr.as_str()))
    } else {
        (false, false)
    };
    let mut allow_attrs: Vec<&str> = vec!["clippy::field_reassign_with_default, clippy::let_and_return"];
    match (needs_redundant_closure_allow, needs_useless_conversion_allow) {
        (true, true) => allow_attrs.push("clippy::redundant_closure, clippy::useless_conversion"),
        (true, false) => allow_attrs.push("clippy::redundant_closure"),
        (false, true) => allow_attrs.push("clippy::useless_conversion"),
        (false, false) => {}
    }

    crate::codegen::conversions::construction::gen_private_field_from_impl(
        &crate::codegen::conversions::construction::PrivateFieldImpl {
            core_path,
            binding_name,
            param: "val",
            has_default: typ.has_default,
            assignments: &assignments,
            allow_attrs: &allow_attrs,
        },
    )
}

/// Compute a single field's binding→core conversion as a `"name: expr"` fragment.
///
/// Applies, in order: the sanitized/excluded-type `Default::default()` fallback, the
/// per-type conversion (`field_conversion_to_core_cfg`), newtype re-wrapping, `Box`
/// wrapping, core-wrapper (Cow/Arc/Bytes) wrapping and opaque-handle special-casing, and
/// finally keyword-escaped binding field-name substitution. Shared by the struct-literal
/// loop and the private-field construction strategies so both stay byte-for-byte identical.
fn field_core_conversion(
    field: &FieldDef,
    typ: &TypeDef,
    config: &ConversionConfig,
    field_was_optionalized: bool,
    references_excluded: bool,
) -> String {
    let conversion = if (field.sanitized && field.core_wrapper != CoreWrapper::Cow) || references_excluded {
        format!("{}: Default::default()", field.name)
    } else if field_was_optionalized {
        field_conversion_to_core_cfg(&field.name, &field.ty, false, config)
    } else {
        field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config)
    };
    let conversion = if let Some(newtype_path) = &field.newtype_wrapper {
        if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
            match &field.ty {
                TypeRef::Optional(_) => format!("{}: ({expr}).map({newtype_path})", field.name),
                TypeRef::Vec(_) => {
                    let inner_expr = if let Some(prefix) = expr.strip_suffix(".collect()") {
                        format!("{prefix}.collect::<Vec<_>>()")
                    } else {
                        expr.to_string()
                    };
                    format!(
                        "{}: ({inner_expr}).into_iter().map({newtype_path}).collect()",
                        field.name
                    )
                }
                _ if field.optional => format!("{}: ({expr}).map({newtype_path})", field.name),
                _ => format!("{}: {newtype_path}({expr})", field.name),
            }
        } else {
            conversion
        }
    } else {
        conversion
    };
    let conversion = if field.is_boxed && matches!(&field.ty, TypeRef::Named(_)) {
        if let Some(expr) = conversion.strip_prefix(&format!("{}: ", field.name)) {
            if field.optional {
                format!("{}: {}.map(Box::new)", field.name, expr)
            } else {
                format!("{}: Box::new({})", field.name, expr)
            }
        } else {
            conversion
        }
    } else {
        conversion
    };
    let is_opaque_arc_field = field.core_wrapper == CoreWrapper::Arc
        && matches!(&field.ty, TypeRef::Named(n) if config
            .opaque_types
            .is_some_and(|opaque| opaque.contains(n.as_str())));
    let is_opaque_no_wrapper_field = field.core_wrapper == CoreWrapper::None
        && matches!(&field.ty, TypeRef::Named(n) if config
            .opaque_types
            .is_some_and(|opaque| opaque.contains(n.as_str())));
    let conversion = if is_opaque_arc_field {
        // The opaque wrapper's `inner` field is always `Arc<T>`. When the core field is
        // `Box<T>` instead, moving `.inner` out directly would produce `Arc<T>`, not `Box<T>`
        // — deref-clone the shared value and rebox it instead of moving the handle.
        if field.is_boxed {
            if field.optional {
                format!(
                    "{}: val.{}.map(|v| Box::new((*v.inner).clone()))",
                    field.name, field.name
                )
            } else {
                format!("{}: Box::new((*val.{}.inner).clone())", field.name, field.name)
            }
        } else if field.optional {
            format!("{}: val.{}.map(|v| v.inner)", field.name, field.name)
        } else {
            format!("{}: val.{}.inner", field.name, field.name)
        }
    } else if is_opaque_no_wrapper_field {
        if config.trait_bridge_field_is_arc_wrapper(&field.name) {
            if field.is_boxed {
                if field.optional {
                    format!(
                        "{}: val.{}.map(|v| Box::new((*v.inner).clone()))",
                        field.name, field.name
                    )
                } else {
                    format!("{}: Box::new((*val.{}.inner).clone())", field.name, field.name)
                }
            } else if field.optional {
                format!("{}: val.{}.map(|v| (*v.inner).clone())", field.name, field.name)
            } else {
                format!("{}: (*val.{}.inner).clone()", field.name, field.name)
            }
        } else {
            format!("{}: Default::default()", field.name)
        }
    } else {
        apply_core_wrapper_to_core(
            &conversion,
            &field.name,
            &field.core_wrapper,
            &field.vec_inner_core_wrapper,
            field.optional,
        )
    };
    let binding_name_field = config.binding_field_name_owned(&typ.name, &field.name);
    if binding_name_field != field.name {
        conversion.replace(&format!("val.{}", field.name), &format!("val.{binding_name_field}"))
    } else {
        conversion
    }
}

/// Generate `impl From<BindingType> for CoreType` using a static constructor method.
///
/// Used for types without lifetime params but with private fields (indicated by
/// `has_explicit_static_new`). Finds a static method (no receiver) whose parameters
/// cover all binding fields, then emits `Self::<method>(...)` using
/// `field_conversion_to_core_cfg` for each argument.
///
/// Returns `None` when no suitable constructor is found.
pub fn gen_from_explicit_new_constructor(
    typ: &TypeDef,
    core_path: &str,
    binding_name: &str,
    config: &ConversionConfig,
) -> Option<String> {
    let field_names: std::collections::HashSet<&str> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| f.name.as_str())
        .collect();

    let constructor = typ
        .methods
        .iter()
        .find(|m| {
            m.receiver.is_none()
                && !m.name.contains("borrowed")
                && field_names
                    .iter()
                    .all(|fname| m.params.iter().any(|p| p.name == *fname))
        })
        .or_else(|| {
            typ.methods.iter().find(|m| {
                m.receiver.is_none()
                    && field_names
                        .iter()
                        .all(|fname| m.params.iter().any(|p| p.name == *fname))
            })
        })?;

    let mut args: Vec<String> = Vec::new();
    for param in &constructor.params {
        if let Some(field) = typ.fields.iter().find(|f| f.name == param.name) {
            let binding_field = config.binding_field_name_owned(&typ.name, &field.name);
            let expr = field_conversion_to_core_cfg(&field.name, &field.ty, field.optional, config);
            let expr = if let Some(e) = expr.strip_prefix(&format!("{}: ", field.name)) {
                e.to_string()
            } else {
                expr
            };
            let expr = if binding_field != field.name {
                expr.replace(&format!("val.{}", field.name), &format!("val.{binding_field}"))
            } else {
                expr
            };
            args.push(expr);
        } else {
            match &param.ty {
                TypeRef::Map(_, _) => args.push("Default::default()".to_string()),
                _ => {
                    if param.is_ref {
                        args.push("&Default::default()".to_string());
                    } else {
                        args.push("Default::default()".to_string());
                    }
                }
            }
        }
    }

    let args_str = args.join(",\n        ");
    let (needs_redundant_closure_allow, needs_useless_conversion_allow) =
        needs_clippy_allow(args.iter().map(String::as_str));
    let allow_attr = clippy_allow_attr_line(needs_redundant_closure_allow, needs_useless_conversion_allow);
    Some(format!(
        "{allow_attr}impl From<{binding_name}> for {core_path} {{\n\
             fn from(val: {binding_name}) -> Self {{\n\
                 Self::{constructor_name}(\n\
                     {args_str},\n\
                 )\n\
             }}\n\
         }}\n",
        constructor_name = constructor.name,
    ))
}

/// Generate a `From<Binding> for CoreType<'_>` impl using a static constructor method.
///
/// For types with `has_lifetime_params=true`, struct-literal construction is forbidden
/// (private fields). This function locates a static method (no receiver) in `typ.methods`
/// whose parameters are a superset of the type's binding fields, then emits a call to that
/// constructor, using field conversion expressions for params that match a binding field and
/// `Default::default()` for any extra params not present in the binding struct.
///
/// Returns `None` when no suitable constructor is found.
pub fn gen_from_lifetime_type_constructor(
    typ: &TypeDef,
    core_path: &str,
    binding_name: &str,
    core_import: &str,
    config: &ConversionConfig,
) -> Option<String> {
    let field_names: std::collections::HashSet<&str> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .map(|f| f.name.as_str())
        .collect();

    let constructor = typ
        .methods
        .iter()
        .find(|m| {
            m.receiver.is_none()
                && field_names
                    .iter()
                    .all(|fname| m.params.iter().any(|p| p.name == *fname))
                && !m.name.contains("borrowed")
        })
        .or_else(|| {
            typ.methods.iter().find(|m| {
                m.receiver.is_none()
                    && field_names
                        .iter()
                        .all(|fname| m.params.iter().any(|p| p.name == *fname))
            })
        })?;

    let mut args: Vec<String> = Vec::new();
    for param in &constructor.params {
        if let Some(field) = typ.fields.iter().find(|f| f.name == param.name) {
            let binding_field = config.binding_field_name_owned(&typ.name, &field.name);
            let expr = match &field.ty {
                TypeRef::String if matches!(field.core_wrapper, CoreWrapper::Cow | CoreWrapper::Box) => {
                    if field.optional {
                        format!("val.{binding_field}.map(Into::into)")
                    } else {
                        format!("val.{binding_field}.into()")
                    }
                }
                TypeRef::Map(_k, _v) => {
                    format!(
                        "val.{binding_field}.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<std::collections::BTreeMap<_, _>>()"
                    )
                }
                TypeRef::Named(type_name) => {
                    // A PHP-facing enum-as-String field can hold anything a script assigned
                    // before this conversion runs -- it is a public property, not something
                    // this generator controls -- so a bad value is a genuine runtime
                    // possibility, not just an internal-bug signal. `.expect()` panicking here
                    // unwinds across the FFI boundary into Zend's C frames: undefined
                    // behaviour and a process crash, not a catchable PHP exception. A parse
                    // failure is now reported via `PhpException::throw()` (which sets the
                    // pending Zend exception directly, without needing this `From` impl to
                    // return early) and the conversion still yields a valid `Self` so the
                    // surrounding constructor call type-checks -- a fieldless fallback variant
                    // supplied by the caller via `enum_string_fallback_variant`, when one is
                    // known for this enum. ~keep
                    let is_enum_string = config
                        .enum_string_names
                        .is_some_and(|names| names.contains(type_name.as_str()));
                    if is_enum_string {
                        let fallback_variant = config
                            .enum_string_fallback_variant
                            .and_then(|variants| variants.get(type_name.as_str()));
                        if field.optional {
                            format!(
                                "val.{binding_field}.and_then(|s| serde_json::from_value(serde_json::Value::String(s)).map_or_else(|e| {{ let _ = ext_php_rs::exception::PhpException::default(format!(\"invalid {type_name}: {{e}}\")).throw(); None }}, Some))"
                            )
                        } else if let Some(variant) = fallback_variant {
                            format!(
                                "serde_json::from_value(serde_json::Value::String(val.{binding_field}.clone())).unwrap_or_else(|e| {{ let _ = ext_php_rs::exception::PhpException::default(format!(\"invalid {type_name}: {{e}}\")).throw(); {core_import}::{type_name}::{variant} }})"
                            )
                        } else {
                            // No known fallback variant (should not happen -- populated 1:1
                            // with `enum_string_names`); fabricating an unsound placeholder
                            // would be worse than the original panic, so this keeps it. ~keep
                            format!(
                                "serde_json::from_value(serde_json::Value::String(val.{binding_field}.clone())).expect(\"valid {type_name}\")"
                            )
                        }
                    } else if field.optional {
                        format!("val.{binding_field}.map(Into::into)")
                    } else {
                        format!("val.{binding_field}.into()")
                    }
                }
                TypeRef::Primitive(p) => {
                    let needs_cast = (config.cast_large_ints_to_i64 && needs_i64_cast(p))
                        || (config.cast_large_ints_to_f64 && needs_f64_cast(p))
                        || (config.cast_uints_to_i32 && needs_i32_cast(p));
                    if needs_cast {
                        let core_ty = core_prim_str(p);
                        format!("val.{binding_field} as {core_ty}")
                    } else {
                        format!("val.{binding_field}")
                    }
                }
                TypeRef::String | TypeRef::Unit => {
                    format!("val.{binding_field}")
                }
                TypeRef::Optional(_) => format!("val.{binding_field}.map(Into::into)"),
                _ => format!("val.{binding_field}.into()"),
            };
            args.push(expr);
        } else {
            match &param.ty {
                TypeRef::Map(_, _) => {
                    args.push("std::collections::BTreeMap::new()".to_string());
                }
                _ => {
                    if param.is_ref {
                        args.push("&Default::default()".to_string());
                    } else {
                        args.push("Default::default()".to_string());
                    }
                }
            }
        }
    }

    let args_str = args.join(",\n        ");
    let (needs_redundant_closure_allow, needs_useless_conversion_allow) =
        needs_clippy_allow(args.iter().map(String::as_str));
    let allow_attr = clippy_allow_attr_line(needs_redundant_closure_allow, needs_useless_conversion_allow);
    let code = format!(
        "{allow_attr}impl From<{binding_name}> for {core_path}<'_> {{\n\
             fn from(val: {binding_name}) -> Self {{\n\
                 {core_path}::{constructor_name}(\n\
                     {args_str},\n\
                 )\n\
             }}\n\
         }}\n",
        constructor_name = constructor.name,
    );
    Some(code)
}
