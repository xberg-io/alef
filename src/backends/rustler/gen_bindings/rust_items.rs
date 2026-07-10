use crate::backends::rustler::template_env;
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use ahash::AHashSet;
use heck::ToSnakeCase;

pub(super) fn rustler_default_for_type(ty: &crate::core::ir::TypeRef) -> &'static str {
    use crate::core::ir::{PrimitiveType, TypeRef};
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false",
        TypeRef::Primitive(_) => "0",
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "String::new()",
        _ => "Default::default()",
    }
}

/// Generate a from_json NIF shim for one serde-capable struct type.
pub(super) fn gen_from_json_nif(typ: &crate::core::ir::TypeDef, core_import: &str) -> String {
    let type_name = &typ.name;
    let snake = type_name.to_snake_case();
    let fn_name = format!("{snake}_from_json");
    let core_ty = if typ.rust_path.is_empty() {
        format!("{core_import}::{type_name}")
    } else {
        typ.rust_path.replace('-', "_")
    };
    format!(
        "#[rustler::nif]\npub fn {fn_name}(json: String) -> Result<{type_name}, String> {{\n    \
        serde_json::from_str::<{core_ty}>(&json)\n        \
        .map({type_name}::from)\n        \
        .map_err(|e| e.to_string())\n}}\n"
    )
}

/// Generate the rustler::init! macro invocation.
pub(super) fn gen_nif_init(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_functions: &AHashSet<String>,
    exclude_types: &AHashSet<&str>,
) -> String {
    let mut exports = vec![];

    if let Some(reg) = config.custom_registrations.for_language(Language::Elixir) {
        for func in &reg.functions {
            exports.push(func.clone());
        }
    }

    for func in api
        .functions
        .iter()
        .filter(|f| !exclude_functions.contains(f.name.as_str()))
    {
        let func_name = if func.is_async {
            let n = func.name.as_str();
            if n.ends_with("_async") {
                n.to_string()
            } else {
                format!("{n}_async")
            }
        } else {
            func.name.clone()
        };
        exports.push(func_name);
    }

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(typ.name.as_str()))
    {
        for method in typ
            .methods
            .iter()
            .filter(|m| !exclude_functions.contains(m.name.as_str()))
        {
            let method_name = if method.is_async {
                format!("{}_{}_async", typ.name.to_lowercase(), method.name)
            } else {
                format!("{}_{}", typ.name.to_lowercase(), method.name)
            };
            exports.push(method_name);
        }
    }

    let has_trait_bridges = config
        .trait_bridges
        .iter()
        .any(|b| !b.exclude_languages.iter().any(|l| l == "elixir" || l == "rustler"));
    if has_trait_bridges {
        exports.push("complete_trait_call".to_string());
        exports.push("fail_trait_call".to_string());
    }

    if !api.services.is_empty() {
        if !has_trait_bridges {
            exports.push("complete_trait_call".to_string());
        }
        exports.push("app_run".to_string());
        exports.push("app_into_router".to_string());
        for http_method in &[
            "get", "post", "put", "patch", "delete", "head", "options", "connect", "trace",
        ] {
            exports.push(format!("app_{}", http_method));
        }
    }

    exports.sort();
    exports.dedup();
    let module = config
        .elixir
        .as_ref()
        .map(|e| {
            use heck::ToUpperCamelCase;
            format!(
                "Elixir.{}.Native",
                e.app_name.as_deref().unwrap_or("NativeModule").to_upper_camel_case()
            )
        })
        .unwrap_or_else(|| "Elixir.NativeModule.Native".to_string());
    let opaque_types: Vec<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && !exclude_types.contains(t.name.as_str()))
        .map(|t| t.name.as_str())
        .collect();

    let streaming_handle_types: Vec<String> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, crate::core::config::AdapterPattern::Streaming))
        .filter_map(|a| {
            let owner = a.owner_type.as_deref()?;
            Some(format!(
                "{}{}Handle",
                streaming_handle_type_component(owner),
                streaming_handle_type_component(&a.name)
            ))
        })
        .collect();

    if !opaque_types.is_empty() || !streaming_handle_types.is_empty() {
        let mut registrations: Vec<String> = opaque_types
            .iter()
            .map(|name| {
                template_env::render(
                    "rustler_resource_registration.rs.jinja",
                    minijinja::context! {
                        type_name => name,
                    },
                )
                .trim_end()
                .to_string()
            })
            .collect();
        for name in &streaming_handle_types {
            registrations.push(
                template_env::render(
                    "rustler_resource_registration.rs.jinja",
                    minijinja::context! {
                        type_name => name,
                    },
                )
                .trim_end()
                .to_string(),
            );
        }
        let reg_body = registrations.join("\n");
        template_env::render(
            "rustler_init_with_load.rs.jinja",
            minijinja::context! {
                registrations => &reg_body,
                module => &module,
                nifs => &exports,
            },
        )
        .trim_end()
        .to_string()
    } else {
        template_env::render(
            "rustler_init.rs.jinja",
            minijinja::context! {
                module => &module,
                nifs => &exports,
            },
        )
        .trim_end()
        .to_string()
    }
}

/// Return the public type-name component used in generated Rustler resource structs.
fn streaming_handle_type_component(name: &str) -> String {
    public_host_identifier(Language::Elixir, PublicIdentifierKind::Type, name)
}

/// Patch a generated streaming `_start` NIF so its first parameter — when typed as
/// a default-typed (has_default) core type — is taken as `Option<String>` JSON and
/// deserialized to the core type before the inner method call.
///
/// Mirrors the approach used in `gen_nif_function` / `gen_nif_method` for non-streaming
/// methods. Without this patch, the generated `_start` function would expect a
/// fully-populated `NifMap` from Elixir, which fails for any partial map.
pub(super) fn patch_streaming_default_param(
    code: &str,
    adapter: &crate::core::config::AdapterConfig,
    default_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let Some(first_param) = adapter.params.first() else {
        return code.to_string();
    };
    let core_ty = first_param.ty.as_str();
    if !default_types.contains(core_ty) {
        return code.to_string();
    }
    let param_name = first_param.name.as_str();

    let typed_param = format!("{param_name}: {core_ty},");
    let json_param = format!("{param_name}: Option<String>,");
    let mut patched = code.replace(&typed_param, &json_param);

    let old_binding = format!("let core_{param_name}: {core_import}::{core_ty} = {param_name}.into();");
    let new_binding = template_env::render(
        "streaming_default_deser_binding.rs.jinja",
        minijinja::context! {
            param_name => param_name,
            core_import => core_import,
            core_ty => core_ty,
        },
    )
    .trim_end()
    .to_string();
    patched = patched.replace(&old_binding, &new_binding);

    patched
}
