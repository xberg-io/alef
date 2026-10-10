use heck::ToSnakeCase;

use crate::backends::ffi::template_env::render;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

/// Generate `{prefix}_convert` — the real no-visitor implementation of the core `convert`
/// function.
///
/// When `visitor_callbacks = true`, the core `convert` function has a visitor parameter
/// that causes the IR to sanitize the function (marking it as unimplementable via the normal
/// codegen path). Instead of emitting a stub, the FFI generator calls this function to produce
/// a proper implementation that passes `None` for the visitor.
///
/// The generated function takes `html` and `options` (no visitor param) and returns a
/// heap-allocated result that the caller must free with the matching result free function.
pub fn gen_convert_no_visitor(
    prefix: &str,
    core_import: &str,
    bridge_cfg: Option<&TraitBridgeConfig>,
    function: Option<&FunctionDef>,
) -> String {
    let Some(function) = function else {
        tracing::warn!(
            "gen_convert_no_visitor(ffi): visitor callbacks require a matching public function, skipping no-visitor wrapper"
        );
        return String::new();
    };
    let visitor_function = no_visitor_function_spec(prefix, function, core_import, bridge_cfg);
    render(
        "ffi_visitor_no_callback_function.jinja",
        crate::alef_context! {
            prefix,
            fn_name => visitor_function.fn_name,
            params => visitor_function.ffi_params,
            return_type => visitor_function.return_type,
            param_conversions => visitor_function.param_conversions,
            call => visitor_function.call,
        },
    )
}

pub(super) struct LegacyVisitorFunctionSpec {
    pub(super) fn_name: String,
    pub(super) ffi_params: Vec<String>,
    pub(super) param_conversions: String,
    pub(super) return_type: String,
    pub(super) call: String,
}

struct LegacyNoVisitorFunctionSpec {
    pub(super) fn_name: String,
    pub(super) ffi_params: Vec<String>,
    pub(super) param_conversions: String,
    pub(super) return_type: String,
    pub(super) call: String,
}

pub(super) fn visitor_function_spec(
    prefix: &str,
    func: &FunctionDef,
    core_import: &str,
    bridge_cfg: Option<&TraitBridgeConfig>,
    embed_visitor_in_options: bool,
    options_field: &str,
) -> Option<LegacyVisitorFunctionSpec> {
    let mut param_conversions = String::new();
    let mut call_args = Vec::new();
    let mut ffi_params = Vec::new();
    let options_param_name = visitor_options_param(func, bridge_cfg).map(|param| param.name.as_str());

    for param in &func.params {
        if is_bridge_param(param, bridge_cfg) {
            call_args.push("visitor_handle".to_string());
            continue;
        }
        ffi_params.push(ffi_param_decl(param, core_import));
        param_conversions.push_str(&param_conversion(param, core_import));
        call_args.push(rust_call_arg(param));
    }

    let call = if embed_visitor_in_options {
        if let Some(options_param_name) = options_param_name {
            let options_local = format!("{options_param_name}_rs");
            let Some(options_path) = visitor_options_param(func, bridge_cfg)
                .and_then(|param| named_type_ref(&param.ty))
                .map(|name| rust_named_path(core_import, name))
            else {
                tracing::warn!(
                    "gen_visitor_bindings(ffi): options-field visitor wrapper requires an options parameter, skipping with-visitor wrapper"
                );
                return None;
            };
            for arg in &mut call_args {
                if arg == &options_local {
                    *arg = "options_with_visitor".to_string();
                }
            }
            format!(
                "    let mut options_with_visitor: Option<{options_path}> = {options_local};\n\
                 if visitor_handle.is_some() {{\n\
                     let opts = options_with_visitor.get_or_insert_with({options_path}::default);\n\
                     opts.{options_field} = visitor_handle;\n\
                 }}\n\
                 match {core_import}::{function_name}({call_args}) {{",
                function_name = func.name,
                call_args = call_args.join(", "),
            )
        } else {
            format!(
                "    match {core_import}::{function_name}({call_args}) {{",
                function_name = func.name,
                call_args = call_args.join(", "),
            )
        }
    } else {
        format!(
            "    match {core_import}::{function_name}({call_args}) {{",
            function_name = func.name,
            call_args = call_args.join(", "),
        )
    };

    Some(LegacyVisitorFunctionSpec {
        fn_name: format!("{}_{}_with_visitor", prefix, func.name.to_snake_case()),
        ffi_params,
        param_conversions,
        return_type: return_type_path(&func.return_type, core_import),
        call,
    })
}

fn no_visitor_function_spec(
    prefix: &str,
    func: &FunctionDef,
    core_import: &str,
    bridge_cfg: Option<&TraitBridgeConfig>,
) -> LegacyNoVisitorFunctionSpec {
    let mut param_conversions = String::new();
    let mut call_args = Vec::new();
    let mut ffi_params = Vec::new();

    for param in &func.params {
        if is_bridge_param(param, bridge_cfg) {
            call_args.push("None".to_string());
            continue;
        }
        ffi_params.push(ffi_param_decl(param, core_import));
        param_conversions.push_str(&param_conversion(param, core_import));
        call_args.push(rust_call_arg(param));
    }

    LegacyNoVisitorFunctionSpec {
        fn_name: format!("{}_{}", prefix, func.name.to_snake_case()),
        ffi_params,
        param_conversions,
        return_type: return_type_path(&func.return_type, core_import),
        call: format!(
            "    match {core_import}::{function_name}({call_args}) {{",
            function_name = func.name,
            call_args = call_args.join(", "),
        ),
    }
}

pub(super) fn named_type_ref(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => named_type_ref(inner),
        _ => None,
    }
}

fn rust_named_path(core_import: &str, name: &str) -> String {
    format!("{core_import}::{name}")
}

fn return_type_path(ty: &TypeRef, core_import: &str) -> String {
    named_type_ref(ty)
        .map(|name| rust_named_path(core_import, name))
        .unwrap_or_else(|| "()".to_string())
}

fn is_bridge_param(param: &ParamDef, bridge_cfg: Option<&TraitBridgeConfig>) -> bool {
    let Some(bridge_cfg) = bridge_cfg else {
        return false;
    };
    bridge_cfg.param_name.as_deref() == Some(param.name.as_str())
        || bridge_cfg.type_alias.as_deref() == named_type_ref(&param.ty)
}

pub(super) fn visitor_options_param<'a>(
    func: &'a FunctionDef,
    bridge_cfg: Option<&TraitBridgeConfig>,
) -> Option<&'a ParamDef> {
    if let Some(options_type) = bridge_cfg.and_then(|cfg| cfg.options_type.as_deref()) {
        return func
            .params
            .iter()
            .find(|param| named_type_ref(&param.ty) == Some(options_type));
    }
    func.params
        .iter()
        .find(|param| !is_bridge_param(param, bridge_cfg) && named_type_ref(&param.ty).is_some())
}

fn ffi_param_decl(param: &ParamDef, core_import: &str) -> String {
    match &param.ty {
        TypeRef::String | TypeRef::Path => format!("{}: *const std::ffi::c_char", param.name),
        TypeRef::Named(name) => {
            format!("{}: *const {}", param.name, rust_named_path(core_import, name))
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => {
                format!("{}: *const {}", param.name, rust_named_path(core_import, name))
            }
            _ => format!("{}: *const std::ffi::c_void", param.name),
        },
        _ => format!("{}: *const std::ffi::c_void", param.name),
    }
}

fn param_conversion(param: &ParamDef, core_import: &str) -> String {
    match &param.ty {
        TypeRef::String | TypeRef::Path => format!(
            r#"    if {name}.is_null() {{
        set_last_error(1, "Null pointer passed for {name}");
        return std::ptr::null_mut();
    }}
    // SAFETY: null check above guarantees {name} is a valid pointer.
    let {name}_rs = match unsafe {{ std::ffi::CStr::from_ptr({name}) }}.to_str() {{
        Ok(s) => s,
        Err(_) => {{
            set_last_error(1, "Invalid UTF-8 in {name} parameter");
            return std::ptr::null_mut();
        }}
    }};
"#,
            name = param.name,
        ),
        TypeRef::Named(name) => named_param_conversion(&param.name, core_import, name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => named_param_conversion(&param.name, core_import, name),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn named_param_conversion(param_name: &str, core_import: &str, type_name: &str) -> String {
    let path = rust_named_path(core_import, type_name);
    format!(
        r#"    let {name}_rs: Option<{path}> = if {name}.is_null() {{
        None
    }} else {{
        // SAFETY: {name} is a valid pointer guaranteed by the caller.
        Some(unsafe {{ &*{name} }}.clone())
    }};
"#,
        name = param_name,
        path = path,
    )
}

fn rust_call_arg(param: &ParamDef) -> String {
    match &param.ty {
        TypeRef::String | TypeRef::Path if param.is_ref => format!("&{}_rs", param.name),
        TypeRef::String | TypeRef::Path => format!("{}_rs", param.name),
        TypeRef::Named(_) | TypeRef::Optional(_) => format!("{}_rs", param.name),
        _ => param.name.clone(),
    }
}
