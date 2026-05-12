//! NAPI-RS capsule-type codegen: External<T> + `__parser` property passthrough.
//!
//! When `[crates.node.capsule_types]` is configured, types listed there are NOT emitted
//! as `#[napi]` opaque wrappers. Instead, functions returning those types produce a
//! `JsObject` carrying a `Napi::External<T>` in a `__parser` property — the shape
//! consumed by the `tree-sitter` npm package's `Parser.setLanguage()`.
//!
//! Only the `"external_pointer"` construct variant is implemented. The emitted shim:
//!   1. Calls the core function to obtain the Rust value.
//!   2. Calls `value.into_raw()` to get a raw pointer (assumed available on the type).
//!   3. Creates a `JsObject`, sets `__parser` to `env.create_external(ptr, None)`.
//!   4. Returns the object.
//!
//! Assumption: the capsule type exposes `pub fn into_raw(self) -> *const <opaque>`.
//! If the method name differs in a future version, the generated Rust shim will fail
//! at compile time in the downstream crate (not silently at runtime).

use alef_codegen::naming::to_node_name;
use alef_core::config::NodeCapsuleTypeConfig;
use alef_core::ir::{FunctionDef, TypeRef};
use std::collections::HashMap;

/// Returns `true` when this function returns a capsule-configured type.
///
/// Only return-type capsule involvement is checked — NAPI capsule types are
/// pass-through values and are never accepted as input parameters in this design.
pub(super) fn function_involves_capsule(
    func: &FunctionDef,
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
) -> bool {
    return_type_name(func, capsule_types).is_some()
}

/// Returns the capsule return type name if the function returns a capsule-configured type.
pub(super) fn return_type_name<'a>(
    func: &'a FunctionDef,
    capsule_types: &'a HashMap<String, NodeCapsuleTypeConfig>,
) -> Option<&'a str> {
    fn named_from_ref(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => named_from_ref(inner),
            _ => None,
        }
    }
    let name = named_from_ref(&func.return_type)?;
    if capsule_types.contains_key(name) {
        Some(name)
    } else {
        None
    }
}

/// Generate a `#[napi]` shim for a function whose return type is a capsule type.
///
/// The shim:
/// - Takes `env: napi::Env` as its first parameter (how napi-rs exposes the JS env to
///   free functions that return `JsObject` directly).
/// - Calls the core function.
/// - Calls `value.into_raw()` to extract the raw pointer.
///   ASSUMPTION: the type's `into_raw()` method exists and returns a raw pointer.
///   This assumption is documented here and will surface as a compile error in the
///   downstream crate if the API changes.
/// - Wraps the pointer in `env.create_external(ptr, None)` and sets it as the
///   `__parser` property of a new `JsObject`.
pub(super) fn gen_capsule_function(
    func: &FunctionDef,
    _capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
    core_import: &str,
) -> String {
    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{js_name}\")")
    } else {
        String::new()
    };

    // Build parameter list: (env: napi::Env, <user params...>)
    // napi-rs passes `Env` as the first parameter for functions that need raw JS object creation.
    let mut sig_params: Vec<String> = vec!["env: napi::Env".to_string()];
    for param in &func.params {
        let ts = match &param.ty {
            TypeRef::String | TypeRef::Char => "String".to_string(),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::String | TypeRef::Char => "Option<String>".to_string(),
                TypeRef::Primitive(p) => format!("Option<{}>", prim_rust_str(p)),
                _ => "Option<String>".to_string(),
            },
            TypeRef::Primitive(p) => prim_rust_str(p).to_string(),
            _ => "String".to_string(),
        };
        sig_params.push(format!("{}: {ts}", param.name));
    }

    // Build core call args
    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.is_ref && matches!(p.ty, TypeRef::String | TypeRef::Char) {
                format!("&{}", p.name)
            } else {
                p.name.clone()
            }
        })
        .collect();

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?";

    // Emit the shim body.
    // SAFETY comment: into_raw() transfers ownership of the raw pointer to the External.
    // The tree-sitter npm package stores it in a JS External and calls back into the
    // C ABI. Dropping this pointer prematurely would be a use-after-free; we rely on
    // the downstream JS runtime keeping the External alive for as long as the parser runs.
    let body = format!(
        r#"    let value = {core_fn_path}({args}){err_conv};
    // ASSUMPTION: the capsule type exposes `into_raw()` returning a raw pointer.
    // This is a compile-time assumption — if the method name changes in a future
    // version of tree-sitter, this shim will fail to compile in the downstream crate.
    // SAFETY: `into_raw()` transfers ownership of the raw pointer. The external value
    // is kept alive by the JS runtime as long as the returned object is reachable.
    let ptr = value.into_raw() as *mut std::ffi::c_void;
    let mut obj = env.create_object()?;
    let external = env.create_external(ptr, None)?;
    obj.set_named_property("__parser", external)?;
    Ok(obj)"#,
        core_fn_path = core_fn_path,
        args = call_args.join(", "),
        err_conv = err_conv,
    );

    format!(
        "#[napi{js_name_attr}]\npub fn {fn_name}({params}) -> napi::Result<napi::JsObject> {{\n{body}\n}}\n\n",
        js_name_attr = js_name_attr,
        fn_name = func.name,
        params = sig_params.join(", "),
        body = body,
    )
}

fn prim_rust_str(p: &alef_core::ir::PrimitiveType) -> &'static str {
    use alef_core::ir::PrimitiveType;
    match p {
        PrimitiveType::Bool => "bool",
        PrimitiveType::U8 => "u8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::U64 => "i64", // NAPI maps u64 → i64
        PrimitiveType::I8 => "i8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::F32 => "f64", // NAPI maps f32 → f64
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "i64",
        PrimitiveType::Isize => "i64",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::NodeCapsuleTypeConfig;
    use alef_core::ir::{FunctionDef, ParamDef, TypeRef};
    use std::collections::HashMap;

    fn make_capsule_config(type_name: &str, from_module: &str) -> NodeCapsuleTypeConfig {
        NodeCapsuleTypeConfig {
            type_name: type_name.to_string(),
            from_module: from_module.to_string(),
            construct: "external_pointer".to_string(),
        }
    }

    fn capsule_map(entries: &[(&str, NodeCapsuleTypeConfig)]) -> HashMap<String, NodeCapsuleTypeConfig> {
        entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn make_get_language_fn() -> FunctionDef {
        FunctionDef {
            name: "get_language".to_string(),
            rust_path: "ts_pack::get_language".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::Named("Language".to_string()),
            is_async: false,
            error_type: Some("ts_pack::Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }
    }

    /// function_involves_capsule returns true for a function returning a capsule type.
    #[test]
    fn function_involves_capsule_detects_capsule_return() {
        let func = make_get_language_fn();
        let capsules = capsule_map(&[("Language", make_capsule_config("Language", "tree-sitter"))]);
        assert!(function_involves_capsule(&func, &capsules));
    }

    /// function_involves_capsule returns false for a non-capsule return.
    #[test]
    fn function_involves_capsule_returns_false_for_non_capsule() {
        let func = FunctionDef {
            name: "get_name".to_string(),
            rust_path: "ts_pack::get_name".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        };
        let capsules = capsule_map(&[("Language", make_capsule_config("Language", "tree-sitter"))]);
        assert!(!function_involves_capsule(&func, &capsules));
    }

    /// return_type_name returns the capsule type name for a capsule-returning function.
    #[test]
    fn return_type_name_detects_capsule_return() {
        let func = make_get_language_fn();
        let capsules = capsule_map(&[("Language", make_capsule_config("Language", "tree-sitter"))]);
        assert_eq!(return_type_name(&func, &capsules), Some("Language"));
    }

    /// gen_capsule_function emits a napi shim with __parser + External.
    #[test]
    fn gen_capsule_function_emits_external_and_parser_property() {
        let func = make_get_language_fn();
        let capsules = capsule_map(&[("Language", make_capsule_config("Language", "tree-sitter"))]);
        let out = gen_capsule_function(&func, &capsules, "ts_pack");
        assert!(out.contains("#[napi"), "must have #[napi] attr: {out}");
        assert!(out.contains("napi::Env"), "must accept env: {out}");
        assert!(out.contains("JsObject"), "must return JsObject: {out}");
        assert!(out.contains("into_raw"), "must call into_raw(): {out}");
        assert!(out.contains("create_external"), "must call create_external: {out}");
        assert!(out.contains("__parser"), "must set __parser property: {out}");
    }
}
