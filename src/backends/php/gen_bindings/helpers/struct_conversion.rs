use crate::core::ir::{CoreWrapper, EnumDef, TypeDef, TypeRef};
use ahash::AHashSet;
use minijinja::context;

use super::enum_defaults::{gen_string_to_enum_expr, get_direct_enum_named, get_vec_enum_named};
use super::primitives::{core_prim_str, needs_i64_cast};

/// PHP-specific lossy binding->core struct literal.
pub(crate) fn gen_php_lossy_binding_to_core_fields(
    typ: &TypeDef,
    core_import: &str,
    enum_names: &AHashSet<String>,
    enums: &[EnumDef],
) -> String {
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);

    // Types with lifetime parameters (e.g. `NodeContext<'a>`) have private fields that
    // make struct-literal construction impossible. Delegate to the `From` impl (generated
    // separately via `gen_from_binding_to_core_cfg`) which uses the appropriate constructor.
    if typ.has_lifetime_params {
        return format!("let core_self = {core_path}::from(self.clone());\n        ");
    }

    let mut out = crate::backends::php::template_env::render(
        "php_lossy_binding_struct_begin.jinja",
        context! {
            core_type => &core_path,
            has_stripped_cfg_fields => typ.has_stripped_cfg_fields,
        },
    );
    let has_binding_excluded_fields = typ.fields.iter().any(|f| f.binding_excluded);
    for field in &typ.fields {
        if field.binding_excluded {
            // Skip binding_excluded fields entirely; the trailing `..Default::default()`
            // spread fills them with the CORE type's Default impl. Emitting
            // `<field>: Default::default()` would override that — and is wrong when
            // the core's Default calls a custom function (e.g. `CrawlConfig::default()`
            // sets `ssrf: SsrfPolicy::from_env()`, whereas `<SsrfPolicy as Default>`
            // is the static `deny_private = true` policy).
            continue;
        }
        // Skip cfg-gated fields — they are absent from the binding struct.
        // The ..Default::default() spread below fills them when the feature is enabled.
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
            // Check if this Named field is an enum (PHP maps enums to String).
            // If so, use string->enum parsing instead of .into().
            let expr = if let Some(enum_name) = get_direct_enum_named(&field.ty, enum_names) {
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
                            // Duration stored as Option<i64> (option_duration_on_defaults).
                            // Use the core type's default rather than Duration::default() (0s)
                            // so that e.g. BrowserConfig.timeout preserves its 30s default.
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
                    // Map with Json values: PHP stores String but core expects serde_json::Value.
                    // Can't recover original Values, so fall back to an empty map.
                    TypeRef::Map(_, v) if matches!(v.as_ref(), TypeRef::Json) => "Default::default()".to_string(),
                    // Map<K, Named>: each value needs Into conversion to bridge the binding wrapper
                    // type into the core type (e.g. PhpExtractionPattern → ExtractionPattern).
                    TypeRef::Map(_, v) if matches!(v.as_ref(), TypeRef::Named(_)) => {
                        if field.optional {
                            format!("self.{name}.clone().map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())")
                        } else {
                            format!("self.{name}.clone().into_iter().map(|(k, v)| (k, v.into())).collect()")
                        }
                    }
                    // Map<K, V> where V is not Json/Named: PHP uses HashMap but core may use BTreeMap.
                    // Use into_iter().collect() to allow coercion to the target map type.
                    TypeRef::Map(_, _) => {
                        if field.optional {
                            format!("self.{name}.clone().map(|m| m.into_iter().collect())")
                        } else {
                            format!("self.{name}.clone().into_iter().collect()")
                        }
                    }
                    TypeRef::Unit => format!("self.{name}.clone()"),
                    // Json maps to String in PHP -- can't directly assign to serde_json::Value
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
    // Use ..Default::default() to fill cfg-gated fields stripped from the IR,
    // and binding-excluded fields (alef(skip)) so they pick up the core's Default,
    // including custom Default impls that depend on runtime configuration.
    if typ.has_stripped_cfg_fields || has_binding_excluded_fields {
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
