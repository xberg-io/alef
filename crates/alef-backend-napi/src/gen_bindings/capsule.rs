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
    capsule_types: &HashMap<String, NodeCapsuleTypeConfig>,
    core_import: &str,
) -> String {
    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{js_name}\")")
    } else {
        String::new()
    };

    // Build parameter list: (env: &napi::Env, <user params...>)
    // napi-rs v3 passes `&Env` as a special injected parameter — the macro recognizes it
    // as NapiArgType::Env and injects &__wrapped_env without consuming a JS argument slot.
    let mut sig_params: Vec<String> = vec!["env: &napi::Env".to_string()];
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

    // Look up the capsule config for this function's return type to read
    // `property_name` and optional `type_tag`. Fall back to defaults when missing
    // so the codegen never panics on a malformed config (validation lives in
    // alef-core::config).
    let capsule_name = return_type_name(func, capsule_types).unwrap_or("");
    let cfg = capsule_types.get(capsule_name);
    let property_name = cfg
        .map(|c| c.property_name.clone())
        .unwrap_or_else(|| "__parser".to_string());
    let type_tag_const = cfg
        .and_then(|c| c.type_tag.as_ref())
        .map(|_| format!("__ALEF_CAPSULE_TAG_{}", capsule_name.to_ascii_uppercase()));

    // node-tree-sitter's `Napi::Value::As<External<TSLanguage>>` only recognises
    // values produced by raw `napi_create_external`. napi-rs's
    // `bindgen_prelude::External::new()` wraps the value differently and fails
    // the C++-side `IsExternal()` check at runtime. We therefore call
    // `napi_create_external` directly through a hand-declared FFI extern.
    let tag_block = if let Some(const_name) = &type_tag_const {
        format!(
            r#"    let status = unsafe {{
        napi_type_tag_object(env_raw, external_value, &{const_name})
    }};
    if status != napi::sys::Status::napi_ok {{
        return Err(napi::Error::new(
            napi::Status::GenericFailure,
            format!("napi_type_tag_object failed: status={{status}}"),
        ));
    }}
"#
        )
    } else {
        String::new()
    };

    let body = format!(
        r#"    let value = {core_fn_path}({args}){err_conv};
    // SAFETY: `into_raw()` transfers ownership of the raw pointer. The downstream JS
    // runtime keeps the External alive as long as the returned object is reachable;
    // dropping the pointer prematurely would be a use-after-free.
    let ptr = value.into_raw() as *mut std::ffi::c_void;
    let mut obj = napi::bindgen_prelude::Object::new(env)?;
    let env_raw = env.raw();
    let mut external_value: napi::sys::napi_value = std::ptr::null_mut();
    let create_status = unsafe {{
        napi_create_external(env_raw, ptr, None, std::ptr::null_mut(), &mut external_value)
    }};
    if create_status != napi::sys::Status::napi_ok {{
        return Err(napi::Error::new(
            napi::Status::GenericFailure,
            format!("napi_create_external failed: status={{create_status}}"),
        ));
    }}
{tag_block}    // SAFETY: external_value was just created by napi_create_external on env_raw.
    let unknown = unsafe {{
        napi::bindgen_prelude::Unknown::from_raw_unchecked(env_raw, external_value)
    }};
    obj.set_named_property("{property_name}", unknown)?;
    Ok(obj)"#,
        core_fn_path = core_fn_path,
        args = call_args.join(", "),
        err_conv = err_conv,
        tag_block = tag_block,
        property_name = property_name,
    );

    format!(
        "#[napi{js_name_attr}]\npub fn {fn_name}({params}) -> napi::Result<napi::bindgen_prelude::Object<'_>> {{\n{body}\n}}\n\n",
        js_name_attr = js_name_attr,
        fn_name = func.name,
        params = sig_params.join(", "),
        body = body,
    )
}

/// Emit the FFI extern declarations used by every capsule-returning shim.
/// Emitted once per crate when any capsule_types are configured.
///
/// The `#[cfg_attr(target_os = "windows", link(name = "node", kind = "raw-dylib"))]`
/// is required for Windows MSVC: the linker needs an import-library entry for
/// every imported symbol. `napi-build`'s generated `.def` covers only symbols in
/// `napi-sys`'s `generate!` allowlist — `napi_create_external` and
/// `napi_type_tag_object` are not in that list, so MSVC fails with `LNK2019:
/// unresolved external symbol` without `kind = "raw-dylib"` (Rust 1.71+).
/// On Linux/macOS the attribute is a no-op; Node's dynamic loader resolves the
/// symbols at module load.
pub(super) fn gen_ffi_declarations() -> String {
    r#"#[repr(C)]
struct NapiTypeTag {
    lower: u64,
    upper: u64,
}

#[cfg_attr(target_os = "windows", link(name = "node", kind = "raw-dylib"))]
unsafe extern "C" {
    fn napi_create_external(
        env: napi::sys::napi_env,
        data: *mut std::ffi::c_void,
        finalize_cb: Option<
            unsafe extern "C" fn(
                env: napi::sys::napi_env,
                data: *mut std::ffi::c_void,
                hint: *mut std::ffi::c_void,
            ),
        >,
        finalize_hint: *mut std::ffi::c_void,
        result: *mut napi::sys::napi_value,
    ) -> napi::sys::napi_status;
    fn napi_type_tag_object(
        env: napi::sys::napi_env,
        value: napi::sys::napi_value,
        type_tag: *const NapiTypeTag,
    ) -> napi::sys::napi_status;
}
"#
    .to_string()
}

/// Emit a `const __ALEF_CAPSULE_TAG_<NAME>: NapiTypeTag = ...;` for each capsule
/// type that has a configured type_tag. Skipped for types without a tag.
pub(super) fn gen_type_tag_constants(capsule_types: &HashMap<String, NodeCapsuleTypeConfig>) -> String {
    let mut entries: Vec<(&String, &NodeCapsuleTypeConfig)> = capsule_types.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::new();
    for (name, cfg) in entries {
        if let Some(tag) = &cfg.type_tag {
            let lower = tag.lower.trim_start_matches("0x");
            let upper = tag.upper.trim_start_matches("0x");
            out.push_str(&format!(
                "const __ALEF_CAPSULE_TAG_{}: NapiTypeTag = NapiTypeTag {{\n    lower: 0x{},\n    upper: 0x{},\n}};\n",
                name.to_ascii_uppercase(),
                lower,
                upper,
            ));
        }
    }
    out
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
            property_name: "__parser".to_string(),
            type_tag: None,
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

    /// gen_capsule_function emits a napi shim using raw napi_create_external + property name.
    #[test]
    fn gen_capsule_function_emits_external_and_parser_property() {
        let func = make_get_language_fn();
        let capsules = capsule_map(&[("Language", make_capsule_config("Language", "tree-sitter"))]);
        let out = gen_capsule_function(&func, &capsules, "ts_pack");
        assert!(out.contains("#[napi"), "must have #[napi] attr: {out}");
        assert!(out.contains("napi::Env"), "must accept env: {out}");
        assert!(
            out.contains("bindgen_prelude::Object"),
            "must return bindgen_prelude::Object: {out}"
        );
        assert!(out.contains("into_raw"), "must call into_raw(): {out}");
        assert!(
            out.contains("napi_create_external"),
            "must call raw napi_create_external (not bindgen_prelude::External::new): {out}"
        );
        assert!(
            !out.contains("bindgen_prelude::External::new"),
            "must NOT use bindgen_prelude::External::new (rejected by node-tree-sitter): {out}"
        );
        assert!(
            out.contains("__parser"),
            "must default property name to __parser: {out}"
        );
        assert!(
            !out.contains("napi_type_tag_object"),
            "must NOT emit type-tag call when type_tag is unset: {out}"
        );
    }

    /// When a type_tag is configured, the shim emits napi_type_tag_object with the tag constant.
    #[test]
    fn gen_capsule_function_emits_type_tag_when_configured() {
        let func = make_get_language_fn();
        let mut cfg = make_capsule_config("Language", "tree-sitter");
        cfg.property_name = "language".to_string();
        cfg.type_tag = Some(alef_core::config::NapiTypeTagConfig {
            lower: "0x8AF2E5212AD58ABF".to_string(),
            upper: "0xD5006CAD83ABBA16".to_string(),
        });
        let capsules = capsule_map(&[("Language", cfg)]);
        let out = gen_capsule_function(&func, &capsules, "ts_pack");
        assert!(
            out.contains("napi_type_tag_object"),
            "must call napi_type_tag_object when type_tag set: {out}"
        );
        assert!(
            out.contains("__ALEF_CAPSULE_TAG_LANGUAGE"),
            "must reference the per-capsule tag constant: {out}"
        );
        assert!(
            out.contains(r#"set_named_property("language""#),
            "must honour configured property_name: {out}"
        );
    }

    /// gen_type_tag_constants emits one const per tagged capsule type.
    #[test]
    fn gen_type_tag_constants_emits_only_tagged_entries() {
        let mut tagged = make_capsule_config("Language", "tree-sitter");
        tagged.type_tag = Some(alef_core::config::NapiTypeTagConfig {
            lower: "0x8AF2E5212AD58ABF".to_string(),
            upper: "0xD5006CAD83ABBA16".to_string(),
        });
        let untagged = make_capsule_config("Parser", "tree-sitter");
        let capsules = capsule_map(&[("Language", tagged), ("Parser", untagged)]);
        let out = gen_type_tag_constants(&capsules);
        assert!(
            out.contains("__ALEF_CAPSULE_TAG_LANGUAGE"),
            "must emit constant for tagged entry: {out}"
        );
        assert!(out.contains("0x8AF2E5212AD58ABF"), "must inline lower hex: {out}");
        assert!(out.contains("0xD5006CAD83ABBA16"), "must inline upper hex: {out}");
        assert!(
            !out.contains("__ALEF_CAPSULE_TAG_PARSER"),
            "must skip untagged entries: {out}"
        );
    }

    /// gen_ffi_declarations exposes the two raw N-API entry points the shims need.
    #[test]
    fn gen_ffi_declarations_exposes_required_n_api_entry_points() {
        let out = gen_ffi_declarations();
        assert!(out.contains("fn napi_create_external"));
        assert!(out.contains("fn napi_type_tag_object"));
        assert!(out.contains("struct NapiTypeTag"));
    }

    /// gen_ffi_declarations emits a `kind = "raw-dylib"` link attribute scoped
    /// to Windows so MSVC can synthesize import-library entries for symbols
    /// outside napi-sys's `generate!` allowlist. Linux/macOS rely on Node's
    /// dynamic loader and need no attribute — the cfg_attr is a no-op there.
    #[test]
    fn gen_ffi_declarations_emits_windows_raw_dylib_link() {
        let out = gen_ffi_declarations();
        assert!(
            out.contains(r#"#[cfg_attr(target_os = "windows", link(name = "node", kind = "raw-dylib"))]"#),
            "must gate the raw napi extern block with a Windows raw-dylib link attribute so MSVC can \
             link against napi_create_external + napi_type_tag_object (symbols missing from napi-sys's \
             generate! allowlist). Got: {out}"
        );
    }
}
