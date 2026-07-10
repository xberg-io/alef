use crate::backends::magnus::type_map::MagnusMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use ahash::AHashSet;

/// Check if the last parameter is a struct type with has_default (typically a config struct).
/// Used to determine if a function should use variadic arity for optional config handling.
pub(in crate::backends::magnus::gen_bindings::functions) fn last_param_is_default_struct(
    func: &FunctionDef,
    api: &ApiSurface,
) -> bool {
    func.params.last().is_some_and(|p| {
        if let TypeRef::Named(name) = &p.ty {
            api.types
                .iter()
                .find(|t| &t.name == name)
                .is_some_and(|t| t.has_default)
        } else {
            false
        }
    })
}

/// Returns true when the function has optional params (or promoted required params that follow
/// optional ones), meaning Magnus needs variadic arity (-1) with scan_args.
pub(in crate::backends::magnus::gen_bindings::functions) fn needs_variadic_arity(
    params: &[crate::core::ir::ParamDef],
) -> bool {
    params.iter().any(|p| p.optional) || {
        let mut seen_optional = false;
        params.iter().any(|p| {
            if p.optional {
                seen_optional = true;
                false
            } else {
                seen_optional && !p.optional
            }
        })
    }
}

/// Map a single parameter's type to its Magnus scan_args type string.
/// Optional and promoted params become `Option<T>`, required params become `T`.
/// When treat_as_optional is true, also wrap in Option (used for default-struct config params).
fn param_scan_args_type(
    p: &crate::core::ir::ParamDef,
    promoted: bool,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
) -> String {
    let inner = if let TypeRef::Named(name) = &p.ty {
        if !opaque_types.contains(name.as_str()) {
            "magnus::Value".to_string()
        } else {
            mapper.map_type(&p.ty)
        }
    } else {
        mapper.map_type(&p.ty)
    };
    if p.optional || promoted {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

/// Extended version that accepts treat_as_optional for default-struct config params.
/// For optional String types, use Option<magnus::Value> to handle nil properly via scan_args.
fn param_scan_args_type_extended(
    p: &crate::core::ir::ParamDef,
    promoted: bool,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    treat_as_optional: bool,
) -> String {
    let inner = if let TypeRef::Named(name) = &p.ty {
        if !opaque_types.contains(name.as_str()) {
            "magnus::Value".to_string()
        } else {
            mapper.map_type(&p.ty)
        }
    } else if matches!(p.ty, TypeRef::String) && (p.optional || promoted || treat_as_optional) {
        "magnus::Value".to_string()
    } else {
        mapper.map_type(&p.ty)
    };
    if p.optional || promoted || treat_as_optional {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

/// Generate the scan_args call + destructuring for variadic Magnus functions.
///
/// Returns a string of Rust code that:
/// 1. Calls `scan_args` with appropriate required/optional type params.
/// 2. Destructures `.required` and `.optional` to bind individual param names.
/// 3. If last_is_default_config is true, treats the last param as optional (for config defaults).
pub(in crate::backends::magnus::gen_bindings::functions) fn gen_scan_args_prologue_with_defaults(
    params: &[crate::core::ir::ParamDef],
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    last_is_default_config: bool,
) -> String {
    let mut seen_optional = false;
    let mut req_types: Vec<String> = Vec::new();
    let mut opt_types: Vec<String> = Vec::new();
    let mut req_names: Vec<String> = Vec::new();
    let mut opt_names: Vec<String> = Vec::new();

    for (idx, p) in params.iter().enumerate() {
        let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
        let is_last = idx == params.len() - 1;
        let treat_as_optional = (p.optional || promoted) || (is_last && last_is_default_config);

        if treat_as_optional {
            seen_optional = true;
            opt_types.push(param_scan_args_type_extended(
                p,
                promoted,
                mapper,
                opaque_types,
                is_last && last_is_default_config,
            ));
            opt_names.push(p.name.clone());
        } else {
            let _ = seen_optional;
            req_types.push(param_scan_args_type(p, false, mapper, opaque_types));
            req_names.push(p.name.clone());
        }
    }

    let req_type_str = req_types.join(", ");
    let opt_type_str = opt_types.join(", ");
    let _type_params = match (req_types.is_empty(), opt_types.is_empty()) {
        (true, true) => "()".to_string(),
        (false, true) => format!("({req_type_str},)"),
        (true, false) => format!("((), ({opt_type_str},))"),
        (false, false) => format!("(({req_type_str},), ({opt_type_str},))"),
    };

    let scan_args_line = crate::backends::magnus::template_env::render(
        "function_scan_args_call.rs.jinja",
        minijinja::context! {
            has_required => !req_types.is_empty(),
            has_optional => !opt_types.is_empty(),
            required_types => &req_type_str,
            optional_types => &opt_type_str,
        },
    );

    let mut lines = vec![scan_args_line];

    if !req_names.is_empty() {
        let pat = if req_names.len() == 1 {
            format!("({},)", req_names[0])
        } else {
            format!(
                "({})",
                req_names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")
            )
        };
        lines.push(crate::backends::magnus::template_env::render(
            "function_scan_args_destructure.rs.jinja",
            minijinja::context! {
                pattern => &pat,
                source => "required",
            },
        ));
    }

    if !opt_names.is_empty() {
        let pat = if opt_names.len() == 1 {
            format!("({},)", opt_names[0])
        } else {
            format!(
                "({})",
                opt_names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")
            )
        };
        lines.push(crate::backends::magnus::template_env::render(
            "function_scan_args_destructure.rs.jinja",
            minijinja::context! {
                pattern => &pat,
                source => "optional",
            },
        ));
    }

    for (idx, p) in params.iter().enumerate() {
        let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
        let is_last = idx == params.len() - 1;
        let treat_as_optional = (p.optional || promoted) || (is_last && last_is_default_config);

        if treat_as_optional && matches!(p.ty, TypeRef::String) {
            lines.push(crate::backends::magnus::template_env::render(
                "function_optional_string_scan_arg.rs.jinja",
                minijinja::context! {
                    name => &p.name,
                },
            ));
        }
    }

    lines.join("\n    ")
}
