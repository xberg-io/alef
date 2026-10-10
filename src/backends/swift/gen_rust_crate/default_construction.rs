//! Emits the mutable-default-construction path for struct wrapper `new()` methods.
//!
//! When a struct needs serde-based field assignment (has_serde=true, or has
//! `Vec<non-primitive>` fields, or non-serde String-like fields), `emit_type_wrapper`
//! delegates here. The emitted code creates a `Default` instance and assigns each
//! field individually via serde JSON round-trips and native unwrapping.

use crate::backends::swift::gen_rust_crate::feature_gate;
use crate::backends::swift::gen_rust_crate::type_bridge::{
    enum_from_string_fn_name, needs_json_bridge, swift_bridge_rust_type,
};
use crate::codegen::conversions::helpers::{
    apply_explicit_field_newtype_to_core, apply_field_newtype_to_core, is_explicit_newtype,
};
use crate::core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub(crate) struct EnumKinds<'sets, 'names> {
    all: &'sets HashSet<&'names str>,
    unit: &'sets HashSet<&'names str>,
    reconstructible: &'sets HashSet<&'names str>,
}

impl<'sets, 'names> EnumKinds<'sets, 'names> {
    pub(crate) fn new(all: &'sets HashSet<&'names str>, unit: &'sets HashSet<&'names str>) -> Self {
        Self {
            all,
            unit,
            reconstructible: unit,
        }
    }

    pub(crate) fn with_reconstructible(
        all: &'sets HashSet<&'names str>,
        unit: &'sets HashSet<&'names str>,
        reconstructible: &'sets HashSet<&'names str>,
    ) -> Self {
        Self {
            all,
            unit,
            reconstructible,
        }
    }

    pub(crate) fn all(self) -> &'sets HashSet<&'names str> {
        self.all
    }

    fn contains(self, name: &str) -> bool {
        self.all.contains(name)
    }

    fn is_unit(self, name: &str) -> bool {
        self.unit.contains(name)
    }

    pub(crate) fn supports_constructor_field(self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Named(name) if self.contains(name) => self.is_unit(name),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => !self.contains_enum(inner),
            TypeRef::Map(key, value) => !self.contains_enum(key) && !self.contains_enum(value),
            _ => true,
        }
    }

    pub(crate) fn supports_compatibility_constructor_field(self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Named(name) if self.contains(name) => self.reconstructible.contains(name.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => self.supports_compatibility_constructor_field(inner),
            TypeRef::Map(key, value) => {
                self.supports_compatibility_constructor_field(key)
                    && self.supports_compatibility_constructor_field(value)
            }
            _ => true,
        }
    }

    fn contains_enum(self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Named(name) => self.contains(name),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => self.contains_enum(inner),
            TypeRef::Map(key, value) => self.contains_enum(key) || self.contains_enum(value),
            _ => false,
        }
    }
}

fn is_explicitly_excluded(ty: &TypeDef, field: &FieldDef, exclude_fields: &HashSet<String>) -> bool {
    let field_key = format!("{}.{}", ty.name, field.name.to_snake_case());
    exclude_fields.contains(&field_key)
}

/// Emit the body of a `new()` constructor that routes through `Default` + field assignment.
///
/// Returns the lines that go inside the `fn new(…)` body, *not* including the opening/
/// closing braces of the `impl` block — the caller writes those.
pub(crate) fn emit_default_construction_body(
    ty: &TypeDef,
    source_path: &str,
    type_paths: &HashMap<String, String>,
    enum_kinds: EnumKinds<'_, '_>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&crate::backends::swift::template_env::render(
        "default_construction_let_mut.jinja",
        crate::alef_context! {
            source_path => source_path,
        },
    ));
    for f in &ty.fields {
        if !feature_gate::cfg_satisfied(f.cfg.as_deref(), configured_features) {
            continue;
        }
        let name = f.name.to_snake_case();
        let param = crate::core::keywords::swift_ident(&name);
        if f.binding_excluded {
            continue;
        }
        if is_explicitly_excluded(ty, f, exclude_fields) {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_excluded_comment.jinja",
                crate::alef_context! {
                    name => &name,
                },
            ));
            continue;
        }
        if let Some(wrapper) = f
            .newtype_wrapper
            .as_deref()
            .filter(|wrapper| is_explicit_newtype(wrapper))
        {
            let binding_expr = if needs_json_bridge(&f.ty) {
                let native_ty = swift_bridge_rust_type(&f.ty);
                let binding_ty = if f.optional {
                    format!("Option<{native_ty}>")
                } else {
                    native_ty
                };
                format!("::serde_json::from_str::<{binding_ty}>(&{param}).expect(\"valid JSON for {name}\")")
            } else {
                param.clone()
            };
            let converted = apply_field_newtype_to_core(&binding_expr, &f.ty, f.optional, wrapper);
            out.push_str(&format!("        __target.{name} = {converted};\n"));
            continue;
        }
        let excluded_inner: Option<&str> = if needs_json_bridge(&f.ty) {
            match &f.ty {
                TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n)
                        if !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str()) =>
                    {
                        Some(n.as_str())
                    }
                    _ => None,
                },
                TypeRef::Named(n) if !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str()) => {
                    Some(n.as_str())
                }
                _ => None,
            }
        } else {
            None
        };
        if excluded_inner.is_some() {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_inner_excluded.jinja",
                crate::alef_context! {
                    name => &name,
                },
            ));
        } else if needs_json_bridge(&f.ty) {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_json_bridge_read.jinja",
                crate::alef_context! {
                    param => &param,
                    name => &name,
                },
            ));
        } else if let TypeRef::Named(n) = &f.ty {
            let is_enum = enum_kinds.contains(n);
            if is_enum && enum_kinds.is_unit(n) {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_enum_assign.jinja",
                    crate::alef_context! {
                        name => &name,
                        param => &param,
                        type_name => n,
                        helper => enum_from_string_fn_name(n),
                        optional => f.optional,
                    },
                ));
            } else if is_enum {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_enum_assign.jinja",
                    crate::alef_context! {
                        name => &name,
                        param => &param,
                        type_name => n,
                        helper => enum_from_string_fn_name(n),
                        optional => f.optional,
                    },
                ));
            } else if f.optional {
                if f.is_boxed {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_optional_boxed_assign.jinja",
                        crate::alef_context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_optional_arc_assign.jinja",
                        crate::alef_context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_optional_plain_assign.jinja",
                        crate::alef_context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                }
            } else if f.is_boxed {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_boxed_assign.jinja",
                    crate::alef_context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_arc_assign.jinja",
                    crate::alef_context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_plain_assign.jinja",
                    crate::alef_context! {
                        param => &param,
                        name => &name,
                    },
                ));
            }
        } else if let TypeRef::Vec(inner) = &f.ty {
            if let TypeRef::Named(inner_n) = inner.as_ref() {
                let is_enum = enum_kinds.contains(inner_n);
                if is_enum {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_vec_enum_assign.jinja",
                        crate::alef_context! {
                            name => &name,
                            param => &param,
                            type_name => inner_n,
                            helper => enum_from_string_fn_name(inner_n),
                            optional => f.optional,
                        },
                    ));
                } else {
                    let unwrap_expr = match f.vec_inner_core_wrapper {
                        CoreWrapper::Arc => "std::sync::Arc::new(w.0)".to_string(),
                        _ => "w.0".to_string(),
                    };
                    if f.optional {
                        out.push_str(&crate::backends::swift::template_env::render(
                            "default_field_vec_named_unwrap.jinja",
                            crate::alef_context! {
                                param => &param,
                                name => &name,
                                unwrap_expr => &unwrap_expr,
                            },
                        ));
                    } else {
                        out.push_str(&crate::backends::swift::template_env::render(
                            "default_field_vec_named_unwrap_plain.jinja",
                            crate::alef_context! {
                                param => &param,
                                name => &name,
                                unwrap_expr => &unwrap_expr,
                            },
                        ));
                    }
                }
            } else if ty.has_serde && !f.sanitized {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_vec_serde_round_trip.jinja",
                    crate::alef_context! {
                        param => &param,
                        name => &name,
                    },
                ));
            } else if matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes) {
                if f.sanitized && ty.has_serde {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_vec_serde_round_trip.jinja",
                        crate::alef_context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                } else {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "default_field_vec_primitive_assign.jinja",
                        crate::alef_context! {
                            param => &param,
                            name => &name,
                        },
                    ));
                }
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_vec_non_primitive_comment.jinja",
                    crate::alef_context! {
                        name => &name,
                    },
                ));
            }
        } else if matches!(f.ty, TypeRef::Char) {
            if !ty.has_serde {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_non_serde_comment.jinja",
                    crate::alef_context! { name => &name },
                ));
            } else if f.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_optional_char_assign.jinja",
                    crate::alef_context! {
                        name => &name,
                        param => &param,
                    },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_char_assign.jinja",
                    crate::alef_context! {
                        name => &name,
                        param => &param,
                    },
                ));
            }
        } else if matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json) {
            if !ty.has_serde {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_non_serde_comment.jinja",
                    crate::alef_context! { name => &name },
                ));
            } else if f.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_optional_serde.jinja",
                    crate::alef_context! { param => &param, name => &name },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_string_like_serde.jinja",
                    crate::alef_context! { param => &param, name => &name },
                ));
            }
        } else if matches!(f.ty, TypeRef::Bytes) {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_bytes_assign.jinja",
                crate::alef_context! { name => &name },
            ));
        } else if matches!(f.ty, TypeRef::Duration) {
            if f.optional {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_optional_duration_assign.jinja",
                    crate::alef_context! { param => &param, name => &name },
                ));
            } else {
                out.push_str(&crate::backends::swift::template_env::render(
                    "default_field_duration_assign.jinja",
                    crate::alef_context! { param => &param, name => &name },
                ));
            }
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "default_field_generic_assign.jinja",
                crate::alef_context! { name => &name, param => &param },
            ));
        }
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "dc_construct_target.jinja",
        crate::alef_context! { ty_name => &ty.name },
    ));
    out
}

/// Build the field initializer list used in the direct struct literal construction path.
///
/// Only called when `needs_default_construction` is false: all fields can be constructed
/// directly from the bridge parameter without going through a Default instance.
pub(crate) fn emit_direct_field_inits(
    ty: &TypeDef,
    type_paths: &HashMap<String, String>,
    enum_kinds: EnumKinds<'_, '_>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &std::collections::HashSet<&str>,
) -> Vec<String> {
    ty.fields
        .iter()
        .map(|f| {
            let name = f.name.to_snake_case();
            if !feature_gate::cfg_satisfied(f.cfg.as_deref(), configured_features) {
                return format!("            {name}: ::std::default::Default::default()");
            }
            if f.binding_excluded {
                return format!("            {name}: ::std::default::Default::default()");
            }
            if is_explicitly_excluded(ty, f, exclude_fields) {
                return format!("            {name}: ::std::default::Default::default()");
            }
            let is_excluded_inner = needs_json_bridge(&f.ty) && {
                match &f.ty {
                    TypeRef::Optional(inner) | TypeRef::Vec(inner) => matches!(inner.as_ref(),
                        TypeRef::Named(n) if !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str())
                    ),
                    TypeRef::Named(n) => !type_paths.contains_key(n.as_str()) || no_serde_names.contains(n.as_str()),
                    _ => false,
                }
            };
            if is_excluded_inner {
                format!("            {name}: ::std::default::Default::default()")
            } else if let Some(converted) = apply_explicit_field_newtype_to_core(
                &if needs_json_bridge(&f.ty) {
                    let native_ty = swift_bridge_rust_type(&f.ty);
                    let binding_ty = if f.optional {
                        format!("Option<{native_ty}>")
                    } else {
                        native_ty
                    };
                    format!(
                        "::serde_json::from_str::<{binding_ty}>(&{name}).expect(\"valid JSON for {name}\")"
                    )
                } else {
                    name.clone()
                },
                f,
            ) {
                format!("            {name}: {converted}")
            } else if needs_json_bridge(&f.ty) {
                let native_ty = swift_bridge_rust_type(&f.ty);
                let opt_ty = if f.optional { format!("Option<{native_ty}>") } else { native_ty };
                format!(
                    "            {name}: serde_json::from_str::<{opt_ty}>(&{name}).expect(\"valid JSON for {name}\")"
                )
            } else if let TypeRef::Named(n) = &f.ty {
                let is_enum = enum_kinds.contains(n);
                if is_enum && enum_kinds.is_unit(n) {
                    crate::backends::swift::template_env::render(
                        "default_field_enum_direct.jinja",
                        crate::alef_context! {
                            name => &name,
                            type_name => n,
                            helper => enum_from_string_fn_name(n),
                            optional => f.optional,
                        },
                    )
                } else if is_enum {
                    crate::backends::swift::template_env::render(
                        "default_field_enum_direct.jinja",
                        crate::alef_context! {
                            name => &name,
                            type_name => n,
                            helper => enum_from_string_fn_name(n),
                            optional => f.optional,
                        },
                    )
                } else if f.optional {
                    if matches!(f.core_wrapper, CoreWrapper::Arc) {
                        format!("            {name}: {name}.map(|w| std::sync::Arc::new(w.0))")
                    } else {
                        format!("            {name}: {name}.map(|w| w.0)")
                    }
                } else if matches!(f.core_wrapper, CoreWrapper::Arc) {
                    format!("            {name}: std::sync::Arc::new({name}.0)")
                } else {
                    format!("            {name}: {name}.0")
                }
            } else if let TypeRef::Vec(inner) = &f.ty {
                if let TypeRef::Named(inner_n) = inner.as_ref() {
                    let is_enum = enum_kinds.contains(inner_n);
                    if is_enum {
                        crate::backends::swift::template_env::render(
                            "default_field_vec_enum_direct.jinja",
                            crate::alef_context! {
                                name => &name,
                                type_name => inner_n,
                                helper => enum_from_string_fn_name(inner_n),
                                optional => f.optional,
                            },
                        )
                    } else {
                        let unwrap_expr = match f.vec_inner_core_wrapper {
                            CoreWrapper::Arc => "std::sync::Arc::new(w.0)".to_string(),
                            _ => "w.0".to_string(),
                        };
                        if f.optional {
                            format!("            {name}: {name}.map(|v| v.into_iter().map(|w| {unwrap_expr}).collect())")
                        } else {
                            format!("            {name}: {name}.into_iter().map(|w| {unwrap_expr}).collect()")
                        }
                    }
                } else if f.sanitized && ty.has_serde && matches!(inner.as_ref(), TypeRef::Primitive(_)) {
                    if f.optional {
                        format!(
                            "            {name}: {name}.and_then(|v| ::serde_json::to_value(v).ok()).and_then(|j| ::serde_json::from_value(j).ok())"
                        )
                    } else {
                        format!(
                            "            {name}: ::serde_json::to_value({name}).ok().and_then(|j| ::serde_json::from_value(j).ok()).unwrap_or_default()"
                        )
                    }
                } else {
                    format!("            {name}")
                }
            } else if matches!(f.ty, TypeRef::Char) {
                if !ty.has_serde {
                    format!("            {name}: ::std::default::Default::default()")
                } else if f.optional {
                    format!("            {name}: {name}.as_ref().and_then(|s| s.chars().next())")
                } else {
                    format!("            {name}: {name}.chars().next().unwrap_or('\\0')")
                }
            } else if matches!(f.ty, TypeRef::String | TypeRef::Path | TypeRef::Json) {
                if !ty.has_serde {
                    format!("            {name}: ::std::default::Default::default()")
                } else if f.optional {
                    format!(
                        "            {name}: {name}.and_then(|s| serde_json::from_str(&s).ok().or_else(|| serde_json::from_value(::serde_json::Value::String(s)).ok()))"
                    )
                } else {
                    format!(
                        "            {name}: serde_json::from_str(&{name}).ok().or_else(|| serde_json::from_value::<_>(::serde_json::Value::String({name}.clone())).ok()).unwrap_or_else(|| panic!(\"failed to deserialize {name}\"))"
                    )
                }
            } else if matches!(f.ty, TypeRef::Bytes) {
                format!("            {name}: {name}.into()")
            } else if matches!(f.ty, TypeRef::Duration) {
                if f.optional {
                    format!("            {name}: {name}.map(std::time::Duration::from_millis)")
                } else {
                    format!("            {name}: std::time::Duration::from_millis({name})")
                }
            } else {
                format!("            {name}")
            }
        })
        .collect()
}

#[cfg(test)]
mod enum_option_tests {
    use super::*;

    fn options_type(optional: bool) -> TypeDef {
        TypeDef {
            name: "Options".to_string(),
            rust_path: "sample::Options".to_string(),
            has_default: true,
            fields: vec![FieldDef {
                name: "heading_style".to_string(),
                ty: TypeRef::Named("HeadingStyle".to_string()),
                optional,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn names() -> HashSet<&'static str> {
        ["HeadingStyle"].into_iter().collect()
    }

    #[test]
    fn constructor_supports_only_scalar_unit_enum_fields() {
        let all: HashSet<&str> = ["HeadingStyle", "Routing"].into_iter().collect();
        let unit: HashSet<&str> = ["HeadingStyle"].into_iter().collect();
        let enum_kinds = EnumKinds::new(&all, &unit);

        assert!(enum_kinds.supports_constructor_field(&TypeRef::Named("HeadingStyle".to_string())));
        assert!(!enum_kinds.supports_constructor_field(&TypeRef::Named("Routing".to_string())));
        assert!(
            !enum_kinds
                .supports_constructor_field(&TypeRef::Vec(Box::new(TypeRef::Named("HeadingStyle".to_string(),))))
        );
    }

    #[test]
    fn compatibility_constructor_requires_reconstructible_enum_payloads() {
        let all: HashSet<&str> = ["Routing", "External"].into_iter().collect();
        let unit = HashSet::new();
        let reconstructible: HashSet<&str> = ["Routing"].into_iter().collect();
        let enum_kinds = EnumKinds::with_reconstructible(&all, &unit, &reconstructible);

        assert!(enum_kinds.supports_compatibility_constructor_field(&TypeRef::Named("Routing".to_string())));
        assert!(
            enum_kinds.supports_compatibility_constructor_field(&TypeRef::Vec(Box::new(TypeRef::Named(
                "Routing".to_string(),
            ))))
        );
        assert!(!enum_kinds.supports_compatibility_constructor_field(&TypeRef::Named("External".to_string())));
    }

    #[test]
    fn default_construction_converts_required_enum_option_to_core() {
        let output = emit_default_construction_body(
            &options_type(false),
            "sample::Options",
            &HashMap::new(),
            EnumKinds::new(&names(), &names()),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(
            output.contains(
                "__target.heading_style = \
                 __alef_heading_style_from_swift_string(&heading_style.to_string())\
                 .expect(\"valid HeadingStyle bridge value\");"
            ),
            "the caller's enum option must replace the core default:\n{output}"
        );
        assert!(!output.contains("reverse From not generated"), "{output}");
    }

    #[test]
    fn default_construction_converts_optional_enum_option_to_core() {
        let output = emit_default_construction_body(
            &options_type(true),
            "sample::Options",
            &HashMap::new(),
            EnumKinds::new(&names(), &names()),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(
            output.contains(
                "__target.heading_style = heading_style.map(|value| \
                 __alef_heading_style_from_swift_string(&value.to_string())\
                 .expect(\"valid HeadingStyle bridge value\"));"
            ),
            "an optional enum option must preserve Some and None:\n{output}"
        );
    }

    #[test]
    fn direct_construction_converts_enum_option_to_core() {
        let output = emit_direct_field_inits(
            &options_type(false),
            &HashMap::new(),
            EnumKinds::new(&names(), &names()),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(
            output,
            vec![
                "            heading_style: \
                 __alef_heading_style_from_swift_string(&heading_style.to_string())\
                 .expect(\"valid HeadingStyle bridge value\")"
                    .to_string()
            ]
        );
    }

    #[test]
    fn direct_construction_converts_optional_enum_option_to_core() {
        let output = emit_direct_field_inits(
            &options_type(true),
            &HashMap::new(),
            EnumKinds::new(&names(), &names()),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(
            output,
            vec![
                "            heading_style: heading_style.map(|value| \
                 __alef_heading_style_from_swift_string(&value.to_string())\
                 .expect(\"valid HeadingStyle bridge value\"))"
                    .to_string()
            ]
        );
    }
}
