use crate::core::ir::{CoreWrapper, EnumDef, TypeDef, TypeRef};
use ahash::AHashSet;
use minijinja::context;

use super::enum_defaults::{gen_string_to_enum_expr, get_direct_enum_named, get_vec_enum_named};
use super::primitives::{core_prim_str, needs_i64_cast};

/// Detect whether a field's `TypeRef` references an untagged data enum.
/// The PHP backend maps these to `serde_json::Value` in the binding struct, so
/// inline `Into::into` cannot be used at the binding→core boundary; callers must
/// roundtrip via `serde_json::from_value` to recover the typed core enum.
///
/// Returns a tuple describing the wrapping shape so the caller can emit the
/// correct conversion expression: `(direct, optional_named, vec_named, optional_vec_named)`.
fn untagged_data_enum_shape(ty: &TypeRef, untagged_data_enum_names: &AHashSet<String>) -> Option<UntaggedShape> {
    let is_untagged = |n: &str| untagged_data_enum_names.contains(n);
    match ty {
        TypeRef::Named(n) if is_untagged(n) => Some(UntaggedShape::Direct),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if is_untagged(n) => Some(UntaggedShape::OptionalNamed),
            TypeRef::Vec(vi) => match vi.as_ref() {
                TypeRef::Named(n) if is_untagged(n) => Some(UntaggedShape::OptionalVecNamed),
                _ => None,
            },
            _ => None,
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if is_untagged(n) => Some(UntaggedShape::VecNamed),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum UntaggedShape {
    /// `Named` directly — emit `serde_json::from_value(...).unwrap_or_default()`.
    Direct,
    /// `Option<Named>` — emit `.and_then(|v| serde_json::from_value(v).ok())`.
    OptionalNamed,
    /// `Vec<Named>` — emit `into_iter().filter_map(... from_value ...).collect()`.
    VecNamed,
    /// `Option<Vec<Named>>` — emit `.map(|v| v.into_iter().filter_map(...).collect())`.
    OptionalVecNamed,
}

/// Render the binding→core expression for an untagged-data-enum field, mirroring
/// the dedicated `From` impl emitted by `gen_from_binding_to_core_cfg` for PHP.
/// The expressions use `self.<name>.clone()` (rather than `val.<name>`) because
/// accessor delegation runs on `&self` and must not move the wrapper fields.
fn untagged_data_enum_expr(field_name: &str, shape: UntaggedShape, optional: bool) -> String {
    match shape {
        UntaggedShape::Direct => {
            if optional {
                format!("self.{field_name}.clone().and_then(|v| serde_json::from_value(v).ok())")
            } else {
                format!("serde_json::from_value(self.{field_name}.clone()).unwrap_or_default()")
            }
        }
        UntaggedShape::OptionalNamed => {
            format!("self.{field_name}.clone().and_then(|v| serde_json::from_value(v).ok())")
        }
        UntaggedShape::VecNamed => {
            if optional {
                format!(
                    "self.{field_name}.clone().map(|v| v.into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect())"
                )
            } else {
                format!(
                    "self.{field_name}.clone().into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect()"
                )
            }
        }
        UntaggedShape::OptionalVecNamed => {
            format!(
                "self.{field_name}.clone().map(|v| v.into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect())"
            )
        }
    }
}

/// PHP-specific lossy binding->core struct literal.
pub(crate) fn gen_php_lossy_binding_to_core_fields(
    typ: &TypeDef,
    core_import: &str,
    enum_names: &AHashSet<String>,
    untagged_data_enum_names: &AHashSet<String>,
    enums: &[EnumDef],
) -> String {
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);

    if typ.has_lifetime_params {
        return format!("let core_self = {core_path}::from(self.clone());\n        ");
    }

    let mut out = crate::backends::php::template_env::render(
        "php_lossy_binding_struct_begin.jinja",
        context! {
            core_type => &core_path,
            emit_spread => typ.has_default,
        },
    );
    for field in &typ.fields {
        if field.binding_excluded {
            if !typ.has_default {
                out.push_str(&crate::backends::php::template_env::render(
                    "php_struct_field_assignment.jinja",
                    context! {
                        field_name => field.name.as_str(),
                        field_expr => "Default::default()",
                    },
                ));
                continue;
            }
            continue;
        }
        if field.cfg.is_some() {
            continue;
        }
        let name = &field.name;
        if field.sanitized {
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => "Default::default()",
                },
            ));
        } else {
            let expr = if let Some(shape) = untagged_data_enum_shape(&field.ty, untagged_data_enum_names) {
                untagged_data_enum_expr(name, shape, field.optional)
            } else if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
                gen_string_to_enum_expr(&format!("self.{name}"), &enum_name, field.optional, enums, core_import)
            } else if let Some(enum_name) = get_vec_enum_named(&field.ty, enum_names) {
                let elem_conv = gen_string_to_enum_expr("s", &enum_name, false, enums, core_import);
                if field.optional {
                    format!("self.{name}.clone().map(|v| v.into_iter().map(|s| {elem_conv}).collect())")
                } else {
                    format!("self.{name}.clone().into_iter().map(|s| {elem_conv}).collect()")
                }
            } else {
                match &field.ty {
                    TypeRef::Primitive(p) if needs_i64_cast(p) => {
                        let core_ty = core_prim_str(p);
                        if field.optional {
                            format!("self.{name}.map(|v| v as {core_ty})")
                        } else {
                            format!("self.{name} as {core_ty}")
                        }
                    }
                    TypeRef::Primitive(_) => format!("self.{name}"),
                    TypeRef::Duration => {
                        if field.optional {
                            format!("self.{name}.map(|v| std::time::Duration::from_millis(v as u64))")
                        } else if typ.has_default {
                            crate::backends::php::template_env::render(
                                "php_duration_default_expr.jinja",
                                context! {
                                    value_expr => &format!("self.{name}"),
                                    cast => " as u64",
                                    core_type => &core_path,
                                    field_name => name.as_str(),
                                },
                            )
                        } else {
                            format!("std::time::Duration::from_millis(self.{name} as u64)")
                        }
                    }
                    TypeRef::String | TypeRef::Char => {
                        if matches!(field.core_wrapper, CoreWrapper::Cow | CoreWrapper::Box) {
                            if field.optional {
                                format!("self.{name}.clone().map(Into::into)")
                            } else {
                                format!("self.{name}.clone().into()")
                            }
                        } else {
                            format!("self.{name}.clone()")
                        }
                    }
                    TypeRef::Bytes => format!("self.{name}.clone().into()"),
                    TypeRef::Path => {
                        if field.optional {
                            format!("self.{name}.clone().map(Into::into)")
                        } else {
                            format!("self.{name}.clone().into()")
                        }
                    }
                    TypeRef::Named(_) => {
                        if field.optional {
                            format!("self.{name}.clone().map(Into::into)")
                        } else {
                            format!("self.{name}.clone().into()")
                        }
                    }
                    TypeRef::Vec(inner) => match inner.as_ref() {
                        TypeRef::Named(_) => {
                            if field.optional {
                                format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                            } else {
                                format!("self.{name}.clone().into_iter().map(Into::into).collect()")
                            }
                        }
                        TypeRef::Primitive(p) if needs_i64_cast(p) => {
                            let core_ty = core_prim_str(p);
                            if field.optional {
                                format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                            } else {
                                format!("self.{name}.clone().into_iter().map(|v| v as {core_ty}).collect()")
                            }
                        }
                        _ => format!("self.{name}.clone()"),
                    },
                    TypeRef::Optional(inner) => match inner.as_ref() {
                        TypeRef::Primitive(p) if needs_i64_cast(p) => {
                            let core_ty = core_prim_str(p);
                            format!("self.{name}.map(|v| v as {core_ty})")
                        }
                        TypeRef::Duration => {
                            format!("self.{name}.map(|v| std::time::Duration::from_millis(v as u64))")
                        }
                        TypeRef::Named(_) => {
                            format!("self.{name}.clone().map(Into::into)")
                        }
                        TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                            format!("self.{name}.clone().map(|v| v.into_iter().map(Into::into).collect())")
                        }
                        TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Primitive(p) if needs_i64_cast(p)) => {
                            if let TypeRef::Primitive(p) = vi.as_ref() {
                                let core_ty = core_prim_str(p);
                                format!("self.{name}.clone().map(|v| v.into_iter().map(|x| x as {core_ty}).collect())")
                            } else {
                                format!("self.{name}.clone()")
                            }
                        }
                        _ => format!("self.{name}.clone()"),
                    },
                    TypeRef::Map(_, v) if matches!(v.as_ref(), TypeRef::Json) => "Default::default()".to_string(),
                    TypeRef::Map(_, v) if matches!(v.as_ref(), TypeRef::Named(_)) => {
                        if field.optional {
                            format!("self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())")
                        } else {
                            format!("self.{name}.clone().into_iter().map(|(k, v)| (k, v.into())).collect()")
                        }
                    }
                    TypeRef::Map(_, _) => {
                        if field.optional {
                            format!("self.{name}.clone().map(|m| m.into_iter().collect())")
                        } else {
                            format!("self.{name}.clone().into_iter().collect()")
                        }
                    }
                    TypeRef::Unit => format!("self.{name}.clone()"),
                    TypeRef::Json => "Default::default()".to_string(),
                }
            };
            out.push_str(&crate::backends::php::template_env::render(
                "php_struct_field_assignment.jinja",
                context! {
                    field_name => name.as_str(),
                    field_expr => &expr,
                },
            ));
        }
    }
    if typ.has_default {
        out.push_str(&crate::backends::php::template_env::render(
            "php_default_update.jinja",
            minijinja::Value::default(),
        ));
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_lossy_binding_struct_end.jinja",
        minijinja::Value::default(),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::FieldDef;

    fn field(name: &str, binding_excluded: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            binding_excluded,
            ..Default::default()
        }
    }

    fn typ(name: &str, has_default: bool, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("crate::{name}"),
            fields,
            is_clone: true,
            has_default,
            ..Default::default()
        }
    }

    #[test]
    fn binding_excluded_with_default_uses_spread() {
        let typ = typ(
            "ConfigWithDefault",
            true,
            vec![field("name", false), field("internal", true)],
        );
        let out = gen_php_lossy_binding_to_core_fields(&typ, "crate", &AHashSet::new(), &AHashSet::new(), &[]);

        assert!(
            out.contains("..Default::default()"),
            "spread should be emitted when has_default is true; got:\n{out}"
        );
        assert!(
            !out.contains("internal: Default::default()"),
            "binding-excluded field should not be explicitly emitted when has_default is true; got:\n{out}"
        );
        assert!(out.contains("name:"), "non-excluded field should appear; got:\n{out}");
    }

    #[test]
    fn binding_excluded_without_default_emits_explicit_default() {
        let typ = typ(
            "NoDefaultStruct",
            false,
            vec![field("name", false), field("internal", true)],
        );
        let out = gen_php_lossy_binding_to_core_fields(&typ, "crate", &AHashSet::new(), &AHashSet::new(), &[]);

        assert!(
            !out.contains("..Default::default()"),
            "spread must NOT be emitted when has_default is false; got:\n{out}"
        );
        assert!(
            out.contains("internal: Default::default()"),
            "binding-excluded field must be explicitly defaulted when has_default is false; got:\n{out}"
        );
        assert!(
            out.contains("name:"),
            "non-excluded field should still appear; got:\n{out}"
        );
    }

    #[test]
    fn fully_mirrored_with_default_still_emits_spread() {
        let typ = typ("Plain", true, vec![field("name", false), field("value", false)]);
        let out = gen_php_lossy_binding_to_core_fields(&typ, "crate", &AHashSet::new(), &AHashSet::new(), &[]);

        assert!(
            out.contains("..Default::default()"),
            "has_default core type must always get the spread trailer; got:\n{out}"
        );
        assert!(
            out.contains("#[allow(clippy::needless_update)]"),
            "the spread over a fully-mirrored literal needs the needless_update allow; got:\n{out}"
        );
        assert!(out.contains("name:"), "name field should appear; got:\n{out}");
        assert!(out.contains("value:"), "value field should appear; got:\n{out}");
    }

    #[test]
    fn stripped_cfg_without_default_omits_spread() {
        let mut typ = typ("StrippedNoDefault", false, vec![field("kept", false)]);
        typ.has_stripped_cfg_fields = true;
        let out = gen_php_lossy_binding_to_core_fields(&typ, "crate", &AHashSet::new(), &AHashSet::new(), &[]);

        assert!(
            !out.contains("..Default::default()"),
            "spread must NOT be emitted when has_default is false even with stripped cfg fields; got:\n{out}"
        );
    }
}
