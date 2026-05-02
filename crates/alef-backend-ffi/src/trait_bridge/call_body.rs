//! `gen_vtable_call_body` — generates the body of sync vtable-forwarding methods.

use alef_codegen::generators::trait_bridge::{TraitBridgeSpec, format_type_ref};
use alef_core::ir::{MethodDef, PrimitiveType, TypeRef};
use std::fmt::Write;

use super::{FfiBridgeGenerator, helpers::default_for_type};

impl FfiBridgeGenerator {
    /// Generate the body of a sync method that calls through the vtable.
    pub(super) fn gen_vtable_call_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let mut out = String::with_capacity(512);
        let name = &method.name;
        let has_error = method.error_type.is_some();

        // Extract the vtable fn pointer — return an error / default if it's None.
        writeln!(out, "let Some(fp) = self.vtable.{name} else {{").ok();
        if has_error {
            writeln!(
                out,
                "    return Err(Box::from(\"vtable.{name} is null — bridge not initialised\"));"
            )
            .ok();
        } else {
            // For infallible methods, return the Rust default value
            let default_expr = default_for_type(&method.return_type);
            writeln!(out, "    return {default_expr};").ok();
        }
        writeln!(out, "}};").ok();

        // Marshal each parameter to its C representation.
        // When p.optional is true, the Rust type is Option<T>; treat it the same as
        // TypeRef::Optional(T) and generate a nullable-pointer pattern.
        for p in &method.params {
            let effective_optional = p.optional || matches!(&p.ty, TypeRef::Optional(_));
            let inner_ty: &TypeRef = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };

            if effective_optional {
                match inner_ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => {
                        // Option<&str> → nullable *const c_char via CString storage
                        let map_expr = if p.is_ref {
                            format!(
                                "let _{name}_storage: Option<std::ffi::CString> = {name}.and_then(|v| std::ffi::CString::new(v).ok());",
                                name = p.name
                            )
                        } else {
                            format!(
                                "let _{name}_storage: Option<std::ffi::CString> = {name}.as_deref().and_then(|v| std::ffi::CString::new(v).ok());",
                                name = p.name
                            )
                        };
                        writeln!(out, "{map_expr}").ok();
                        writeln!(
                            out,
                            "let {name}_ptr: *const std::ffi::c_char = _{name}_storage.as_ref().map_or(std::ptr::null(), |cs| cs.as_ptr());",
                            name = p.name
                        )
                        .ok();
                    }
                    TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        writeln!(
                            out,
                            "let _{name}_storage: Option<std::ffi::CString> = {name}.as_ref().and_then(|v| {{",
                            name = p.name
                        )
                        .ok();
                        writeln!(out, "    let s = serde_json::to_string(v).unwrap_or_default();").ok();
                        writeln!(out, "    std::ffi::CString::new(s).ok()").ok();
                        writeln!(out, "}});").ok();
                        writeln!(
                            out,
                            "let {name}_ptr: *const std::ffi::c_char = _{name}_storage.as_ref().map_or(std::ptr::null(), |cs| cs.as_ptr());",
                            name = p.name
                        )
                        .ok();
                    }
                    _ => {} // optional primitives: pass directly by name (0 = None sentinel on C side)
                }
            } else {
                match inner_ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => {
                        // Path params are &Path / PathBuf — convert to string via to_string_lossy().
                        // String/Char params are &str / String — use as-is or .as_str().
                        let (val, needs_as_ref) = match inner_ty {
                            TypeRef::Path => {
                                let expr = format!("{}.to_string_lossy()", p.name);
                                (expr, true) // Cow<str> needs .as_ref() for CString::new
                            }
                            _ => {
                                let expr = if p.is_ref {
                                    p.name.clone()
                                } else {
                                    format!("{}.as_str()", p.name)
                                };
                                (expr, false)
                            }
                        };
                        let arg = if needs_as_ref { format!("{val}.as_ref()") } else { val };
                        writeln!(
                            out,
                            "let _{name}_cs = match std::ffi::CString::new({arg}) {{",
                            name = p.name
                        )
                        .ok();
                        writeln!(out, "    Ok(s) => s,").ok();
                        writeln!(out, "    Err(_) => {{").ok();
                        if has_error {
                            writeln!(out, "        return Err(Box::from(\"nul byte in param {}\"));", p.name).ok();
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            writeln!(out, "        return {default_expr};").ok();
                        }
                        writeln!(out, "    }}").ok();
                        writeln!(out, "}};").ok();
                        writeln!(out, "let {name}_ptr = _{name}_cs.as_ptr();", name = p.name).ok();
                    }
                    TypeRef::Json | TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        writeln!(
                            out,
                            "let _{name}_json = serde_json::to_string(&{name}).unwrap_or_default();",
                            name = p.name
                        )
                        .ok();
                        writeln!(
                            out,
                            "let _{name}_cs = match std::ffi::CString::new(_{name}_json) {{",
                            name = p.name
                        )
                        .ok();
                        writeln!(out, "    Ok(s) => s,").ok();
                        writeln!(out, "    Err(_) => {{").ok();
                        if has_error {
                            writeln!(
                                out,
                                "        return Err(Box::from(\"nul byte in serialized param {}\"));",
                                p.name
                            )
                            .ok();
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            writeln!(out, "        return {default_expr};").ok();
                        }
                        writeln!(out, "    }}").ok();
                        writeln!(out, "}};").ok();
                        writeln!(out, "let {name}_ptr = _{name}_cs.as_ptr();", name = p.name).ok();
                    }
                    _ => {} // primitives, bytes, duration: pass directly
                }
            }
        }

        // Build the argument list for the fn pointer call
        let mut call_args = vec!["self.user_data".to_string()];
        for p in &method.params {
            let effective_optional = p.optional || matches!(&p.ty, TypeRef::Optional(_));
            let inner_ty: &TypeRef = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            let arg = if effective_optional {
                match inner_ty {
                    TypeRef::Primitive(_) => p.name.clone(),
                    _ => format!("{}_ptr", p.name),
                }
            } else {
                match inner_ty {
                    TypeRef::String
                    | TypeRef::Char
                    | TypeRef::Path
                    | TypeRef::Json
                    | TypeRef::Named(_)
                    | TypeRef::Vec(_)
                    | TypeRef::Map(_, _) => format!("{}_ptr", p.name),
                    // Bool is represented as i32 in the C ABI; cast explicitly.
                    TypeRef::Primitive(PrimitiveType::Bool) => format!("{} as i32", p.name),
                    // Bytes params are &[u8]; the vtable expects *const u8.
                    TypeRef::Bytes => format!("{}.as_ptr()", p.name),
                    _ => p.name.clone(),
                }
            };
            call_args.push(arg);
        }

        // Prepare out-params
        let needs_result_out = matches!(
            &method.return_type,
            TypeRef::String
                | TypeRef::Char
                | TypeRef::Path
                | TypeRef::Json
                | TypeRef::Named(_)
                | TypeRef::Vec(_)
                | TypeRef::Map(_, _)
        );
        if needs_result_out {
            writeln!(
                out,
                "let mut _out_result: *mut std::ffi::c_char = std::ptr::null_mut();"
            )
            .ok();
            call_args.push("&mut _out_result".to_string());
        }
        if has_error {
            writeln!(out, "let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();").ok();
            call_args.push("&mut _out_error".to_string());
        }

        let args_str = call_args.join(", ");

        writeln!(
            out,
            "// SAFETY: fp is a valid non-null function pointer; all temporaries outlive this call;"
        )
        .ok();
        writeln!(
            out,
            "// user_data validity is the caller's responsibility (documented in the vtable API)."
        )
        .ok();
        // For infallible primitive/Duration returns the body would tail with `_rc`,
        // tripping clippy::let_and_return. Skip the binding in that case and emit the
        // unsafe call inline as the tail expression below.
        let tail_returns_rc_only = !has_error
            && matches!(
                method.return_type,
                TypeRef::Primitive(
                    PrimitiveType::U8
                        | PrimitiveType::U16
                        | PrimitiveType::U32
                        | PrimitiveType::U64
                        | PrimitiveType::I8
                        | PrimitiveType::I16
                        | PrimitiveType::I32
                        | PrimitiveType::I64
                        | PrimitiveType::F32
                        | PrimitiveType::F64
                        | PrimitiveType::Usize
                        | PrimitiveType::Isize,
                ) | TypeRef::Duration
            );
        if !tail_returns_rc_only {
            writeln!(out, "let _rc = unsafe {{ fp({args_str}) }};").ok();
        }

        // Handle the return
        if has_error {
            writeln!(out, "if _rc != 0 {{").ok();
            writeln!(out, "    let msg = if _out_error.is_null() {{").ok();
            writeln!(out, "        format!(\"vtable.{name} returned error code {{}}\", _rc)").ok();
            writeln!(out, "    }} else {{").ok();
            writeln!(
                out,
                "        // SAFETY: out_error was written by the callee as a valid CString."
            )
            .ok();
            writeln!(
                out,
                "        let cs = unsafe {{ std::ffi::CString::from_raw(_out_error) }};"
            )
            .ok();
            writeln!(out, "        cs.to_string_lossy().into_owned()").ok();
            writeln!(out, "    }};").ok();
            writeln!(out, "    return Err(Box::from(msg));").ok();
            writeln!(out, "}}").ok();

            // Decode successful return
            match &method.return_type {
                TypeRef::Unit => {
                    writeln!(out, "Ok(())").ok();
                }
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(out, "    return Ok(String::new());").ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "Ok(cs.to_string_lossy().into_owned())").ok();
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(
                        out,
                        "    return Err(Box::from(\"vtable.{name} returned null out_result\"));"
                    )
                    .ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "let json = cs.to_string_lossy();").ok();
                    writeln!(
                        out,
                        "serde_json::from_str::<{ret_ty}>(&json).map_err(|e| Box::from(e.to_string()) as Box<dyn std::error::Error + Send + Sync>)"
                    )
                    .ok();
                }
                TypeRef::Primitive(PrimitiveType::Bool) => {
                    writeln!(out, "Ok(_rc != 0)").ok();
                }
                other => {
                    let ret_ty = format_type_ref(other, &spec.type_paths);
                    writeln!(out, "Ok(_rc as {ret_ty})").ok();
                }
            }
        } else {
            // Infallible — decode return value directly
            match &method.return_type {
                TypeRef::Unit => {}
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(out, "    return String::new();").ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "cs.to_string_lossy().into_owned()").ok();
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    writeln!(out, "if _out_result.is_null() {{").ok();
                    writeln!(out, "    return Default::default();").ok();
                    writeln!(out, "}}").ok();
                    writeln!(
                        out,
                        "// SAFETY: out_result was written by the callee as a valid CString."
                    )
                    .ok();
                    writeln!(out, "let cs = unsafe {{ std::ffi::CString::from_raw(_out_result) }};").ok();
                    writeln!(out, "let json = cs.to_string_lossy();").ok();
                    writeln!(out, "serde_json::from_str::<{ret_ty}>(&json).unwrap_or_default()").ok();
                }
                TypeRef::Primitive(PrimitiveType::Bool) => {
                    writeln!(out, "_rc != 0").ok();
                }
                TypeRef::Primitive(_) | TypeRef::Duration => {
                    // tail_returns_rc_only path: emit the unsafe call as the tail expression
                    // (no preceding `let _rc = ...;`) to avoid clippy::let_and_return.
                    writeln!(out, "unsafe {{ fp({args_str}) }}").ok();
                }
                _ => {}
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_codegen::generators::trait_bridge::TraitBridgeSpec;
    use alef_core::config::TraitBridgeConfig;
    use alef_core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
    use std::collections::HashMap;

    fn make_simple_trait_spec<'a>(
        trait_def: &'a alef_core::ir::TypeDef,
        bridge_cfg: &'a TraitBridgeConfig,
    ) -> TraitBridgeSpec<'a> {
        TraitBridgeSpec {
            trait_def,
            bridge_config: bridge_cfg,
            core_import: "my_lib",
            wrapper_prefix: "Ml",
            type_paths: HashMap::new(),
            error_type: "MyError".to_string(),
            error_constructor: "MyError::from({msg})".to_string(),
        }
    }

    fn make_generator() -> FfiBridgeGenerator {
        FfiBridgeGenerator {
            prefix: "ml".to_string(),
            core_import: "my_lib".to_string(),
            type_paths: HashMap::new(),
            error_type: "MyError".to_string(),
        }
    }

    fn make_bridge_cfg() -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: Vec::new(),
            bind_via: alef_core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
        }
    }

    fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> alef_core::ir::TypeDef {
        alef_core::ir::TypeDef {
            name: name.to_string(),
            rust_path: format!("my_lib::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods,
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }
    }

    fn make_method(name: &str, return_type: TypeRef, has_error: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type,
            is_async: false,
            is_static: false,
            error_type: if has_error {
                Some("Box<dyn std::error::Error + Send + Sync>".to_string())
            } else {
                None
            },
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }
    }

    #[test]
    fn call_body_checks_fp_not_null() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg();
        let method = make_method("run", TypeRef::Unit, false);
        let trait_def = make_trait_def("TestTrait", vec![method.clone()]);
        let spec = make_simple_trait_spec(&trait_def, &bridge_cfg);

        let body = generator.gen_vtable_call_body(&method, &spec);
        assert!(body.contains("self.vtable.run"), "must access vtable fn ptr");
        assert!(body.contains("else {"), "must check for None fn ptr");
    }

    #[test]
    fn call_body_fallible_method_returns_err_on_rc_nonzero() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg();
        let method = make_method("process", TypeRef::String, true);
        let trait_def = make_trait_def("TestTrait", vec![method.clone()]);
        let spec = make_simple_trait_spec(&trait_def, &bridge_cfg);

        let body = generator.gen_vtable_call_body(&method, &spec);
        assert!(
            body.contains("Err(Box::from("),
            "fallible method must return Err on failure"
        );
        assert!(body.contains("_out_error"), "must use out_error param");
    }

    #[test]
    fn call_body_string_param_uses_cstring() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg();
        let method = MethodDef {
            name: "greet".to_string(),
            params: vec![ParamDef {
                name: "msg".to_string(),
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
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        };
        let trait_def = make_trait_def("TestTrait", vec![method.clone()]);
        let spec = make_simple_trait_spec(&trait_def, &bridge_cfg);

        let body = generator.gen_vtable_call_body(&method, &spec);
        assert!(body.contains("CString::new"), "string param must convert to CString");
        assert!(body.contains("msg_ptr"), "must create _ptr binding for string param");
    }

    #[test]
    fn call_body_infallible_bool_return() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg();
        let method = make_method("ping", TypeRef::Primitive(PrimitiveType::Bool), false);
        let trait_def = make_trait_def("TestTrait", vec![method.clone()]);
        let spec = make_simple_trait_spec(&trait_def, &bridge_cfg);

        let body = generator.gen_vtable_call_body(&method, &spec);
        assert!(body.contains("_rc != 0"), "bool return must compare rc to 0");
    }
}
