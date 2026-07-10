//! `gen_vtable_call_body` — generates the body of sync vtable-forwarding methods.

use crate::codegen::generators::trait_bridge::{TraitBridgeSpec, format_type_ref};
use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};

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
    /// actual error type (e.g. `SampleCrateError::Plugin { ... }`).
    pub(super) fn gen_vtable_call_body(
        &self,
        method: &MethodDef,
        spec: &TraitBridgeSpec,
        inside_closure: bool,
    ) -> String {
        let name = &method.name;

        if method.returns_ref && matches!(&method.return_type, TypeRef::Vec(_)) {
            return format!("self.{name}_strs\n");
        }

        let mut out = String::with_capacity(512);
        let has_error = method.error_type.is_some();

        let make_err = |msg_literal: String| -> String {
            let msg_literal = msg_literal.trim_end().to_string();
            if inside_closure {
                format!("return Err(Box::from({msg_literal}));\n")
            } else {
                let coerced = if msg_literal.starts_with('"') {
                    format!("{msg_literal}.to_string()")
                } else {
                    msg_literal
                };
                format!("return Err({});\n", spec.make_error(&coerced))
            }
        };

        out.push_str(&crate::backends::ffi::template_env::render(
            "ffi_vtable_extract.jinja",
            minijinja::context! {
                name => name,
            },
        ));
        if has_error {
            let null_msg = crate::backends::ffi::template_env::render(
                "ffi_vtable_not_initialised_msg.jinja",
                minijinja::context! {
                    name => name,
                },
            );
            out.push_str(&make_err(null_msg));
        } else {
            let default_expr = default_for_type(&method.return_type);
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_return_default_4.jinja",
                minijinja::context! {
                    wrapper => spec.wrapper_name(),
                    method_name => name,
                    default_expr => &default_expr,
                },
            ));
        }
        out.push_str(
            "};
",
        );

        for p in &method.params {
            let effective_optional = p.optional || matches!(&p.ty, TypeRef::Optional(_));
            let inner_ty: &TypeRef = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };

            if effective_optional {
                match inner_ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => {
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "ffi_opt_str_storage_and_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                                is_ref => p.is_ref,
                            },
                        ));
                    }
                    TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        out.push_str(&crate::backends::ffi::template_env::render(
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
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "ffi_opt_nullable_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                    }
                    _ => {}
                }
            } else {
                match inner_ty {
                    TypeRef::String | TypeRef::Char | TypeRef::Path => {
                        let (val, needs_as_ref) = match inner_ty {
                            TypeRef::Path => {
                                let expr = format!("{}.to_string_lossy()", p.name);
                                (expr, true)
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
                        out.push_str(&crate::backends::ffi::template_env::render(
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
                            let param_err_msg = crate::backends::ffi::template_env::render(
                                "ffi_nul_byte_param_msg.jinja",
                                minijinja::context! {
                                    name => param_name,
                                },
                            );
                            out.push_str(&make_err(param_err_msg));
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            out.push_str(&crate::backends::ffi::template_env::render(
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
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "ffi_cs_as_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                    }
                    TypeRef::Json | TypeRef::Named(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "ffi_json_to_string.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                        out.push_str(&crate::backends::ffi::template_env::render(
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
                            let param_err_msg = crate::backends::ffi::template_env::render(
                                "ffi_nul_byte_json_param_msg.jinja",
                                minijinja::context! {
                                    name => param_name,
                                },
                            );
                            out.push_str(&make_err(param_err_msg));
                        } else {
                            let default_expr = default_for_type(&method.return_type);
                            out.push_str(&crate::backends::ffi::template_env::render(
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
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "ffi_cs_as_ptr.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        ));
                    }
                    _ => {}
                }
            }
        }

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
                    TypeRef::Primitive(PrimitiveType::Bool) => format!("{} as i32", p.name),
                    TypeRef::Bytes => format!("{}.as_ptr()", p.name),
                    _ => p.name.clone(),
                }
            };
            call_args.push(arg);
            if matches!(&p.ty, TypeRef::Bytes) {
                call_args.push(format!("{}.len()", p.name));
            }
        }

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
        let needs_out_error = has_error || needs_result_out;
        if needs_out_error {
            out.push_str(
                "let mut _out_error: *mut std::ffi::c_char = std::ptr::null_mut();
",
            );
            call_args.push("&mut _out_error".to_string());
        }

        let args_str = call_args.join(", ");

        out.push_str("// SAFETY: fp is a valid non-null function pointer; all temporaries outlive this call;\n");
        out.push_str("// user_data validity is the caller's responsibility (documented in the vtable API).\n");
        // tripping clippy::let_and_return. Skip the binding in that case and emit the
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
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_unsafe_fp_call.jinja",
                minijinja::context! {
                    args => &args_str,
                },
            ));
        }

        if has_error {
            let error_return = make_err("msg".to_string());
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_vtable_error_check.jinja",
                minijinja::context! {
                    name => name,
                    vtable_expr => "self.vtable",
                    error_return => &error_return,
                },
            ));

            match &method.return_type {
                TypeRef::Unit => {
                    out.push_str(
                        "Ok(())
",
                    );
                }
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    out.push_str(&crate::backends::ffi::template_env::render(
                        "ffi_decode_string_result.jinja",
                        minijinja::context! { vtable_expr => "self.vtable" },
                    ));
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    out.push_str(
                        "if _out_result.is_null() {
",
                    );
                    let null_result_msg = crate::backends::ffi::template_env::render(
                        "ffi_vtable_null_out_result_msg.jinja",
                        minijinja::context! {
                            name => name,
                        },
                    );
                    out.push_str(&make_err(null_result_msg));
                    out.push_str(
                        "}
",
                    );
                    out.push_str("// SAFETY: out_result was written by the callee as a valid NUL-terminated string.\n");
                    out.push_str(
                        "let json = unsafe { std::ffi::CStr::from_ptr(_out_result) }.to_string_lossy().into_owned();\n",
                    );
                    out.push_str("if let Some(free_fn) = self.vtable.free_string {\n");
                    out.push_str("    // SAFETY: free_fn is the vtable-provided destructor for callback strings.\n");
                    out.push_str("    unsafe { free_fn(_out_result) };\n");
                    out.push_str("}\n");
                    if inside_closure {
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "ffi_serde_from_str_err.jinja",
                            minijinja::context! {
                                ret_ty => &ret_ty,
                            },
                        ));
                    } else {
                        let err_constructor = spec.make_error("e.to_string()");
                        out.push_str(&crate::backends::ffi::template_env::render(
                            "ffi_sync_serde_from_str_err.jinja",
                            minijinja::context! {
                                ret_ty => &ret_ty,
                                err_constructor => &err_constructor,
                            },
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
                    out.push_str(&crate::backends::ffi::template_env::render(
                        "ffi_ok_rc_as.jinja",
                        minijinja::context! {
                            ret_ty => &ret_ty,
                        },
                    ));
                }
            }
        } else {
            match &method.return_type {
                TypeRef::Unit => {}
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    out.push_str(&crate::backends::ffi::template_env::render(
                        "ffi_decode_string_value.jinja",
                        minijinja::context! {
                            wrapper => spec.wrapper_name(),
                            method_name => name,
                            vtable_expr => "self.vtable",
                        },
                    ));
                }
                TypeRef::Named(_) | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                    let ret_ty = format_type_ref(&method.return_type, &spec.type_paths);
                    out.push_str(
                        "if _out_result.is_null() {
",
                    );
                    out.push_str(&format!(
                        "    eprintln!(\"[{wrapper}] host '{name}' wrote no result; returning default\");
",
                        wrapper = spec.wrapper_name(),
                    ));
                    out.push_str(
                        "    return Default::default();
",
                    );
                    out.push_str(
                        "}
",
                    );
                    out.push_str("// SAFETY: out_result was written by the callee as a valid NUL-terminated string.\n");
                    out.push_str(
                        "let json = unsafe { std::ffi::CStr::from_ptr(_out_result) }.to_string_lossy().into_owned();\n",
                    );
                    out.push_str("if let Some(free_fn) = self.vtable.free_string {\n");
                    out.push_str("    // SAFETY: free_fn is the vtable-provided destructor for callback strings.\n");
                    out.push_str("    unsafe { free_fn(_out_result) };\n");
                    out.push_str("}\n");
                    out.push_str(&crate::backends::ffi::template_env::render(
                        "ffi_serde_from_str_default.jinja",
                        minijinja::context! {
                            wrapper => spec.wrapper_name(),
                            method_name => name,
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
                    // (no preceding `let _rc = ...;`) to avoid clippy::let_and_return.
                    out.push_str(&crate::backends::ffi::template_env::render(
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
    use crate::codegen::generators::trait_bridge::TraitBridgeSpec;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
    use std::collections::HashMap;

    fn make_simple_trait_spec<'a>(
        trait_def: &'a crate::core::ir::TypeDef,
        bridge_cfg: &'a TraitBridgeConfig,
    ) -> TraitBridgeSpec<'a> {
        TraitBridgeSpec {
            trait_def,
            bridge_config: bridge_cfg,
            core_import: "my_lib",
            wrapper_prefix: "Ml",
            type_paths: HashMap::new(),
            lifetime_type_names: std::collections::HashSet::new(),
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
            lifetime_type_names: Default::default(),
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
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        }
    }

    fn make_trait_def(name: &str, methods: Vec<MethodDef>) -> crate::core::ir::TypeDef {
        crate::core::ir::TypeDef {
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
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
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
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

    /// Regression (#114): call body for a Bytes parameter must pass both `.as_ptr()` and
    /// `.len()` to the vtable call.  The vtable field now has a companion `{name}_len: usize`
    /// immediately after `{name}: *const u8`, and the call body must supply both arguments.
    /// A payload containing a 0x00 byte would be silently truncated if only the pointer were
    /// passed and the callee fell back to a strlen scan.
    #[test]
    fn call_body_bytes_param_passes_ptr_and_len() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg();
        let method = MethodDef {
            name: "ingest".to_string(),
            params: vec![ParamDef {
                name: "data".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let trait_def = make_trait_def("TestTrait", vec![method.clone()]);
        let spec = make_simple_trait_spec(&trait_def, &bridge_cfg);

        let body = generator.gen_vtable_call_body(&method, &spec, true);
        assert!(
            body.contains("data.as_ptr()"),
            "call body must pass `.as_ptr()` for Bytes param;\nactual:\n{body}"
        );
        assert!(
            body.contains("data.len()"),
            "call body must pass `.len()` companion for Bytes param;\nactual:\n{body}"
        );
    }

    /// Regression: sync method bodies (inside_closure=false) must not emit bare &'static str
    /// literals when the error constructor wraps a String (e.g. `MyError::Other(String)`).
    ///
    /// Before the fix, `make_err("\"some message\"")` with `inside_closure=false` produced
    /// `MyError::from("some message")` — a `&'static str` — which fails to compile when the
    /// error variant requires a `String`.  After the fix every static-string error path emits
    /// `"some message".to_string()`.
    #[test]
    fn bug_sync_static_error_literal_coerced_to_string() {
        let generator = make_generator();
        let bridge_cfg = make_bridge_cfg();
        let method = MethodDef {
            name: "submit".to_string(),
            params: vec![ParamDef {
                name: "doc".to_string(),
                ty: TypeRef::Named("MyDoc".to_string()),
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: true,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: false,
            error_type: Some("MyError".to_string()),
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let trait_def = make_trait_def("TestTrait", vec![method.clone()]);
        let spec = make_simple_trait_spec(&trait_def, &bridge_cfg);

        let sync_body = generator.gen_vtable_call_body(&method, &spec, false);

        assert!(
            sync_body.contains("\".to_string()"),
            "sync body must coerce string literals to String via .to_string();\n\
             actual body:\n{sync_body}"
        );

        for line in sync_body.lines() {
            if line.contains("MyError::from(\"") {
                assert!(
                    line.contains(".to_string()"),
                    "string literal passed to error constructor without .to_string():\n  {line}\n\
                     full body:\n{sync_body}"
                );
            }
        }
    }
}
