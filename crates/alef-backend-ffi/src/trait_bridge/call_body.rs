//! `gen_vtable_call_body` — generates the body of sync vtable-forwarding methods.

use alef_codegen::generators::trait_bridge::{TraitBridgeSpec, format_type_ref};
use alef_core::ir::{MethodDef, PrimitiveType, TypeRef};

use super::{FfiBridgeGenerator, helpers::default_for_type};

impl FfiBridgeGenerator {
    /// Generate the body of a vtable-forwarding method call.
    ///
    /// When `inside_closure` is `true` the body will be inlined inside a
    /// `_SendFn` closure whose return type is
    /// `Box<dyn std::error::Error + Send + Sync>`.  Error construction then uses
    /// `Box::from(msg)` which satisfies that type.
    ///
    /// When `inside_closure` is `false` the body is emitted directly as the
    /// trait method body, whose return type is `Result<T, ErrorType>`.  Error
    /// construction then uses `spec.make_error(...)` to construct the trait's
    /// actual error type (e.g. `KreuzbergError::Plugin { ... }`).
    pub(super) fn gen_vtable_call_body(
        &self,
        method: &MethodDef,
        spec: &TraitBridgeSpec,
        inside_closure: bool,
    ) -> String {
        let name = &method.name;

        // Short-circuit: methods that return `&[T]` (Vec(T) + returns_ref) are pre-cached
        // at construction time.  The body simply returns the cached field directly,
        // bypassing the vtable call entirely.
        if method.returns_ref && matches!(&method.return_type, TypeRef::Vec(_)) {
            return format!("self.{name}_strs\n");
        }

        let mut out = String::with_capacity(512);
        let has_error = method.error_type.is_some();

        // Helper: emit an error expression appropriate for the calling context.
        // Inside the async _SendFn closure the return type is Box<dyn Error + Send + Sync>;
        // outside (sync method body) it is Result<T, TraitErrorType>.
        let make_err = |msg_literal: String| -> String {
            if inside_closure {
                format!("return Err(Box::from({msg_literal}));\n")
            } else {
                format!("return Err({});\n", spec.make_error(&msg_literal))
            }
        };

        // Extract the vtable fn pointer — return an error / default if it's None.
        out.push_str(&crate::template_env::render(
            "ffi_vtable_extract.jinja",
            minijinja::context! {
                name => name,
            },
        ));
        if has_error {
            out.push_str(&make_err(format!("\"vtable.{name} is null — bridge not initialised\"")));
        } else {
            // For infallible methods, return the Rust default value
            let default_expr = default_for_type(&method.return_type);
            out.push_str(&crate::template_env::render(
                "ffi_return_default_4.jinja",
                minijinja::context! {
                    default_expr => &default_expr,
                },
            ));
        }
        out.push_str(
            "};
",
        );

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
                        out.push_str(&crate::template_env::render(
                            "ffi_opt_str_storage_and_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                                is_ref => p.is_ref,
                            },
                        ));
                    }
                    TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        out.push_str(&crate::template_env::render(
                            "ffi_opt_json_storage_open.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                        out.push_str(
                            "    let s = serde_json::to_string(v).unwrap_or_default();
",
                        );
                        out.push_str(
                            "    std::ffi::CString::new(s).ok()
",
                        );
                        out.push_str(
                            "});
",
                        );
                        out.push_str(&crate::template_env::render(
                            "ffi_opt_nullable_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
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
                        out.push_str(&crate::template_env::render(
                            "ffi_cs_match_open.jinja",
                            minijinja::context! {
                                name => &p.name,
                                arg => &arg,
                            },
                        ));
                        out.push_str(
                            "    Ok(s) => s,
",
                        );
                        out.push_str(
                            "    Err(_) => {
",
                        );
                        if has_error {
                            let param_name = &p.name;
                            out.push_str(&make_err(format!("\"nul byte in param {param_name}\"")));
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            out.push_str(&crate::template_env::render(
                                "ffi_return_default_8.jinja",
                                minijinja::context! {
                                    default_expr => &default_expr,
                                },
                            ));
                        }
                        out.push_str(
                            "    }
",
                        );
                        out.push_str(
                            "};
",
                        );
                        out.push_str(&crate::template_env::render(
                            "ffi_cs_as_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                    }
                    TypeRef::Json | TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        out.push_str(&crate::template_env::render(
                            "ffi_json_to_string.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                        out.push_str(&crate::template_env::render(
                            "ffi_json_cs_match_open.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                        out.push_str(
                            "    Ok(s) => s,
",
                        );
                        out.push_str(
                            "    Err(_) => {
",
                        );
                        if has_error {
                            let param_name = &p.name;
                            out.push_str(&make_err(format!("\"nul byte in serialized param {param_name}\"")));
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            out.push_str(&crate::template_env::render(
                                "ffi_return_default_8.jinja",
                                minijinja::context! {
                                    default_expr => &default_expr,
                                },
                            ));
                        }
                        out.push_str(
                            "    }
",
                        );
                        out.push_str(
                            "};
",
                        );
                        out.push_str(&crate::template_env::render(
                            "ffi_cs_as_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
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
            out.push_str("let mut _out_result: *mut std::ffi::c_char = std::ptr::null_mut();\n");
            call_args.push("&mut _out_result".to_string());
        }
        if has_error {
            out.push_str(
                "let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();
",
            );
            call_args.push("&mut _out_error".to_string());
        }

        let args_str = call_args.join(", ");

        out.push_str("// SAFETY: fp is a valid non-null function pointer; all temporaries outlive this call;\n");
        out.push_str("// user_data validity is the caller's responsibility (documented in the vtable API).\n");
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
            out.push_str(&crate::template_env::render(
                "ffi_unsafe_fp_call.jinja",
                minijinja::context! {
                    args => &args_str,
                },
            ));
        }

        // Handle the return
        if has_error {
            out.push_str(
                "if _rc != 0 {
",
            );
            out.push_str(
                "    let msg = if _out_error.is_null() {
",
            );
            out.push_str(&format!(
                "        format!(\"vtable.{name} returned error code {{}}\", _rc)\n"
            ));
            out.push_str(
                "    } else {
",
            );
            out.push_str("// SAFETY: out_error was written by the callee as a valid CString.\n");
            out.push_str("let cs = unsafe { std::ffi::CString::from_raw(_out_error) };\n");
            out.push_str(
                "        cs.to_string_lossy().into_owned()
",
            );
            out.push_str(
                "    };
",
            );
            out.push_str(&make_err("msg".to_string()));
            out.push_str(
                "}
",
            );

            // Decode successful return
            match &method.return_type {
                TypeRef::Unit => {
                    out.push_str(
                        "Ok(())
",
                    );
                }
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    out.push_str(
                        "if _out_result.is_null() {
",
                    );
                    out.push_str(
                        "    return Ok(String::new());
",
                    );
                    out.push_str(
                        "}
",
                    );
                    out.push_str("// SAFETY: out_result was written by the callee as a valid CString.\n");
                    out.push_str(
                        "let cs = unsafe { std::ffi::CString::from_raw(_out_result) };
",
                    );
                    out.push_str(
                        "Ok(cs.to_string_lossy().into_owned())
",
                    );
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    out.push_str(
                        "if _out_result.is_null() {
",
                    );
                    out.push_str(&make_err(format!("\"vtable.{name} returned null out_result\"")));
                    out.push_str(
                        "}
",
                    );
                    out.push_str("// SAFETY: out_result was written by the callee as a valid CString.\n");
                    out.push_str(
                        "let cs = unsafe { std::ffi::CString::from_raw(_out_result) };
",
                    );
                    out.push_str(
                        "let json = cs.to_string_lossy();
",
                    );
                    if inside_closure {
                        // Inside the _SendFn closure the return type is Box<dyn Error>
                        out.push_str(&crate::template_env::render(
                            "ffi_serde_from_str_err.jinja",
                            minijinja::context! {
                                ret_ty => &ret_ty,
                            },
                        ));
                    } else {
                        // Sync method body — error type is the trait's ErrorType
                        let err_constructor = spec.make_error("e.to_string()");
                        out.push_str(&format!(
                            "serde_json::from_str::<{ret_ty}>(&json).map_err(|e| {err_constructor})\n"
                        ));
                    }
                }
                TypeRef::Primitive(PrimitiveType::Bool) => {
                    out.push_str(
                        "Ok(_rc != 0)
",
                    );
                }
                other => {
                    let ret_ty = format_type_ref(other, &spec.type_paths);
                    out.push_str(&crate::template_env::render(
                        "ffi_ok_rc_as.jinja",
                        minijinja::context! {
                            ret_ty => &ret_ty,
                        },
                    ));
                }
            }
        } else {
            // Infallible — decode return value directly
            match &method.return_type {
                TypeRef::Unit => {}
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    out.push_str(
                        "if _out_result.is_null() {
",
                    );
                    out.push_str(
                        "    return String::new();
",
                    );
                    out.push_str(
                        "}
",
                    );
                    out.push_str("// SAFETY: out_result was written by the callee as a valid CString.\n");
                    out.push_str(
                        "let cs = unsafe { std::ffi::CString::from_raw(_out_result) };
",
                    );
                    out.push_str(
                        "cs.to_string_lossy().into_owned()
",
                    );
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    out.push_str(
                        "if _out_result.is_null() {
",
                    );
                    out.push_str(
                        "    return Default::default();
",
                    );
                    out.push_str(
                        "}
",
                    );
                    out.push_str("// SAFETY: out_result was written by the callee as a valid CString.\n");
                    out.push_str(
                        "let cs = unsafe { std::ffi::CString::from_raw(_out_result) };
",
                    );
                    out.push_str(
                        "let json = cs.to_string_lossy();
",
                    );
                    out.push_str(&crate::template_env::render(
                        "ffi_serde_from_str_default.jinja",
                        minijinja::context! {
                            ret_ty => &ret_ty,
                        },
                    ));
                }
                TypeRef::Primitive(PrimitiveType::Bool) => {
                    out.push_str(
                        "_rc != 0
",
                    );
                }
                TypeRef::Primitive(_) | TypeRef::Duration => {
                    // tail_returns_rc_only path: emit the unsafe call as the tail expression
                    // (no preceding `let _rc = ...;`) to avoid clippy::let_and_return.
                    out.push_str(&crate::template_env::render(
                        "ffi_unsafe_fp_tail.jinja",
                        minijinja::context! {
                            args => &args_str,
                        },
                    ));
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
            plugin_error_constructor: None,
        }
    }

    fn make_bridge_cfg() -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: None,
            registry_getter: None,
            register_fn: None,

            unregister_fn: None,

            clear_fn: None,
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

        let body = generator.gen_vtable_call_body(&method, &spec, true);
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

        let body = generator.gen_vtable_call_body(&method, &spec, true);
        assert!(
            body.contains("Err(Box::from("),
            "fallible method must return Err on failure (inside_closure=true)"
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

        let body = generator.gen_vtable_call_body(&method, &spec, true);
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

        let body = generator.gen_vtable_call_body(&method, &spec, true);
        assert!(body.contains("_rc != 0"), "bool return must compare rc to 0");
    }
}
