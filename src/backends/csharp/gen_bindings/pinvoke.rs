//! C# `[DllImport]` declaration emission for free functions and methods.
//!
//! Split out of `functions.rs` (which is over the file-size cap and owns the *walk* over the API
//! surface) so the declaration of a primary export and the declaration of its
//! `{fn}_has_result` presence companion are produced side by side. A companion emitted from a
//! different walk than its primary is how the two drift into disagreeing about arity or width;
//! here the companion literally reuses the primary's own rendered parameter block. ~keep

use super::marshalling::{
    FfiEmitter, is_bridge_param, pinvoke_param_type_with_scalars, pinvoke_return_type_with_capsules,
};
use super::result_presence::presence_declaration;
use crate::backends::csharp::template_env::render;
use crate::codegen::naming::to_csharp_name;
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::{HashMap, HashSet};

/// The `out` parameters a bytes-returning export appends to its declared parameter list.
const BYTES_RESULT_OUT_PARAMS: &str = concat!(
    "        out IntPtr outPtr,\n",
    "        out UIntPtr outLen,\n",
    "        out UIntPtr outCap\n",
);

/// Returns true when a function returns bytes — uses the owned out-param convention:
/// `(args..., out IntPtr, out UIntPtr, out UIntPtr) -> int`.
pub(super) fn is_bytes_result_func(func: &FunctionDef) -> bool {
    matches!(func.return_type, TypeRef::Bytes)
}

/// Same check for MethodDef.
pub(super) fn is_bytes_result_method(method: &MethodDef) -> bool {
    matches!(method.return_type, TypeRef::Bytes)
}

/// The declared parameter list of one `[DllImport]`, rendered as the text that goes between the
/// stub name's parentheses: either empty, or a leading newline, one indented parameter per line,
/// and the closing indent the `);` sits on.
///
/// `leading` is the receiver declaration (`ulong handle`) for an instance method and `None`
/// otherwise. Free functions and methods share this so a declaration emitted for one can never
/// spell a parameter differently from the other. ~keep
fn param_declarations(
    params: &[&ParamDef],
    leading: Option<&str>,
    is_bytes_result: bool,
    scalar_named_types: &ahash::AHashSet<String>,
) -> String {
    if params.is_empty() && leading.is_none() && !is_bytes_result {
        return String::new();
    }

    let mut out = String::from("\n");
    if let Some(receiver) = leading {
        out.push_str("        ");
        out.push_str(receiver);
        out.push_str(",\n");
    }
    for param in params {
        out.push_str("        ");
        let pinvoke_ty = pinvoke_param_type_with_scalars(&param.ty, scalar_named_types);
        if pinvoke_ty == "string" {
            out.push_str("[MarshalAs(UnmanagedType.LPUTF8Str)] ");
        }
        let param_name = param.name.to_lower_camel_case();
        out.push_str(
            render("pinvoke_param.jinja", minijinja::context! { pinvoke_ty, param_name }).trim_end_matches('\n'),
        );
        out.push_str(",\n");
        if matches!(param.ty, TypeRef::Bytes) {
            let len_param_name = format!("{param_name}Len");
            out.push_str(&render(
                "pinvoke_bytes_len_param.jinja",
                minijinja::context! { len_param_name },
            ));
        }
    }

    if is_bytes_result {
        out.push_str(BYTES_RESULT_OUT_PARAMS);
    } else {
        out.truncate(out.len() - ",\n".len());
        out.push('\n');
    }
    out.push_str("    ");
    out
}

/// One `[DllImport]` declaration, plus the presence companion's when the FFI crate exports one.
fn declaration(entry_point: &str, cs_name: &str, return_type: &str, params: &str) -> String {
    let mut out = render("dll_import_attr.jinja", minijinja::context! { entry_point });
    out.push_str(&render(
        "pinvoke_declaration.jinja",
        minijinja::context! { return_type, cs_name, params },
    ));
    out
}

pub(super) fn gen_pinvoke_for_func(
    c_name: &str,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    capsule_types: &HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    scalar_named_types: &ahash::AHashSet<String>,
) -> String {
    let cs_name = to_csharp_name(&func.name);
    let is_bytes_result = is_bytes_result_func(func);
    let return_type = if is_bytes_result || (func.return_type == TypeRef::Unit && func.error_type.is_some()) {
        "int"
    } else {
        pinvoke_return_type_with_capsules(&func.return_type, capsule_types, FfiEmitter::FreeFunction)
    };

    let visible_params: Vec<&ParamDef> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .collect();
    let params = param_declarations(&visible_params, None, is_bytes_result, scalar_named_types);

    let mut out = declaration(c_name, &cs_name, return_type, &params);
    if let Some(companion) = presence_declaration(&func.return_type, None, c_name, &cs_name, &params) {
        out.push_str(&companion);
    }
    out
}

/// `capsule_types` is threaded in so this emitter derives its return type from the same
/// authority as [`gen_pinvoke_for_func`] instead of from a map that cannot see capsules at all.
/// It resolves to [`FfiEmitter::Method`], which for a capsule return deliberately keeps the
/// `AlefHandle` declaration — see [`FfiEmitter`] for the FFI-side reason. ~keep
pub(super) fn gen_pinvoke_for_method(
    c_name: &str,
    cs_name: &str,
    method: &MethodDef,
    capsule_types: &HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    scalar_named_types: &ahash::AHashSet<String>,
) -> String {
    let is_bytes_result = is_bytes_result_method(method);
    let return_type = if is_bytes_result || (method.return_type == TypeRef::Unit && method.error_type.is_some()) {
        "int"
    } else {
        pinvoke_return_type_with_capsules(&method.return_type, capsule_types, FfiEmitter::Method)
    };

    // The receiver crosses the C ABI as `AlefHandle` (`uint64_t`), not a pointer —
    // `ReceiverKind::{Ref,RefMut,Owned}` all map to `AlefHandle` in the FFI backend. ~keep
    let has_receiver = !method.is_static && method.receiver.is_some();
    let leading = has_receiver.then_some("ulong handle");

    let visible_params: Vec<&ParamDef> = method.params.iter().collect();
    let params = param_declarations(&visible_params, leading, is_bytes_result, scalar_named_types);

    let mut out = declaration(c_name, cs_name, return_type, &params);
    if let Some(companion) =
        presence_declaration(&method.return_type, method.receiver.as_ref(), c_name, cs_name, &params)
    {
        out.push_str(&companion);
    }
    out
}
