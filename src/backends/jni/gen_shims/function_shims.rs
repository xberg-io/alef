/// Emit a shim for a top-level API function.
///
/// When the return type is an opaque named type the function returns `jlong`
/// (a raw `Box::into_raw` pointer) rather than a JSON-encoded `jstring`.
/// When a parameter is an opaque named type it is received as `jlong` and
/// dereferenced via an unsafe pointer cast — the Kotlin caller holds the
/// handle as a `Long` that was previously obtained from the constructor shim.
#[allow(clippy::too_many_arguments)]
fn emit_function_shim(
    out: &mut String,
    symbol: &str,
    rust_path: &str,
    params: &[ParamDef],
    return_type: &TypeRef,
    is_async: bool,
    has_error: bool,
    opaque_type_names: &std::collections::HashSet<&str>,
    core_crate_prefix: &str,
) {
    // The rust_path from the IR already includes the crate prefix (e.g., "{name}::extract_bytes").
    // Replace the core crate name with "core_crate::" since the generated code imports the crate as core_crate.
    // Both sides are normalised dash→underscore: rust_path uses underscores (Rust path syntax),
    // while core_crate_prefix is the cargo crate name which may contain dashes (for example
    // "sample-parser-core"). Without normalisation, starts_with would miss and the
    // emitter would prepend "core_crate::" on top of the already-qualified path, producing
    // double-qualified call sites that fail E0433 ("cannot find X in core_crate").
    let path = rust_path.replace('-', "_");
    let from_prefix = format!("{}::", core_crate_prefix.replace('-', "_"));
    let core_fn = if path.starts_with(&from_prefix) {
        path.replacen(&from_prefix, "core_crate::", 1)
    } else {
        // Path doesn't start with the crate name, prepend core_crate for bare function names
        format!("core_crate::{path}")
    };

    // Determine whether the return type is an opaque handle up-front so we can
    // use the correct null/zero sentinel in unmarshal error paths.
    let is_opaque_return = matches!(return_type, TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));
    let ret_decl = if is_opaque_return {
        " -> jlong".to_string()
    } else {
        method_return_type_decl(return_type)
    };
    let err_null = if is_opaque_return {
        "0"
    } else {
        method_return_null(return_type)
    };

    // Collect param signatures and unmarshal logic.
    let mut param_sigs = String::new();
    let mut unmarshal = String::new();
    let mut call_args = String::new();

    for p in params {
        let rust_name = p.name.replace('-', "_");
        // The base type (unwrap Optional to its inner type for JNI marshaling decisions).
        let base_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        match base_ty {
            TypeRef::String => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_string_unmarshal(&rust_name, err_null));
                // Build call-site expression.  Optional Strings: the Kotlin
                // facade passes "" (empty string) as the null-sentinel for
                // String? params via `value ?: ""`, because JNI primitive
                // signatures cannot express nullability.  Treat empty as
                // None so the Rust callee receives the correct Option<_>.
                if p.optional {
                    call_args.push_str("if ");
                    call_args.push_str(&rust_name);
                    call_args.push_str(".is_empty() { None } else { Some(");
                    call_args.push_str(&rust_name);
                    call_args.push_str(") }");
                } else if p.is_ref {
                    call_args.push('&');
                    call_args.push_str(&rust_name);
                } else {
                    call_args.push_str(&rust_name);
                }
            }
            TypeRef::Primitive(prim) => {
                let jni_ty = jni_primitive_type(prim);
                param_sigs.push_str(&render_param_decl(&rust_name, jni_ty));
                let cast = primitive_cast(prim);
                let cast_expr = if cast.is_empty() {
                    rust_name.clone()
                } else {
                    format!("{rust_name} as {cast}")
                };
                if p.optional {
                    // Optional numeric primitives: the Kotlin facade passes
                    // 0 / 0L / 0.0 / false as the null-sentinel for nullable
                    // primitives via `value ?: 0`, because JNI primitive
                    // signatures cannot express nullability.  Treat the
                    // default value as None so the Rust callee receives the
                    // correct Option<_>.
                    let zero_lit = primitive_zero_literal(prim);
                    if let Some(zero) = zero_lit {
                        call_args.push_str("if ");
                        call_args.push_str(&rust_name);
                        call_args.push_str(" != ");
                        call_args.push_str(zero);
                        call_args.push_str(" { Some(");
                        call_args.push_str(&cast_expr);
                        call_args.push_str(") } else { None }");
                    } else {
                        call_args.push_str("Some(");
                        call_args.push_str(&cast_expr);
                        call_args.push(')');
                    }
                } else {
                    call_args.push_str(&cast_expr);
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_base64_bytes_unmarshal(&rust_name, err_null, p.optional));
                if p.optional {
                    call_args.push_str(&rust_name);
                } else {
                    if p.is_ref {
                        call_args.push('&');
                        call_args.push_str(&rust_name);
                    } else {
                        call_args.push_str(&rust_name);
                    }
                }
            }
            TypeRef::Bytes => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_base64_bytes_unmarshal(&rust_name, err_null, p.optional));
                if p.optional {
                    call_args.push_str(&rust_name);
                } else {
                    if p.is_ref {
                        call_args.push('&');
                        call_args.push_str(&rust_name);
                    } else {
                        call_args.push_str(&rust_name);
                    }
                }
            }
            TypeRef::Path => {
                // Path params: receive a JString, unmarshal directly to PathBuf without
                // a JSON parse step.  Without this arm, the fallback `_` branch wraps
                // the raw string as a `serde_json::Value`, which doesn't impl
                // `AsRef<Path>` and fails with E0277 at the call site.
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_string_unmarshal(&rust_name, err_null));
                // Convert the unmarshalled String into a PathBuf so the call site
                // receives a real path value rather than a generic JSON Value.
                unmarshal.push_str(&format!(
                    "    let {rust_name} = std::path::PathBuf::from({rust_name});\n"
                ));
                if p.optional {
                    call_args.push_str("if ");
                    call_args.push_str(&rust_name);
                    call_args.push_str(".as_os_str().is_empty() { None } else { Some(");
                    call_args.push_str(&rust_name);
                    call_args.push_str(") }");
                } else if p.is_ref {
                    call_args.push('&');
                    call_args.push_str(&rust_name);
                } else {
                    call_args.push_str(&rust_name);
                }
            }
            TypeRef::Named(type_name) if opaque_type_names.contains(type_name.as_str()) => {
                // Opaque handle param: receive as jlong, dereference via raw pointer.
                // SAFETY: the Kotlin caller holds a Long obtained from the matching
                // constructor shim and guarantees the handle is live for this call.
                param_sigs.push_str(&render_param_decl(&rust_name, "jlong"));
                let type_path = format!("core_crate::{type_name}");
                unmarshal.push_str(&template_env::render(
                    "opaque_handle_unmarshal.rs.jinja",
                    context! {
                        name => rust_name,
                        type_path => type_path,
                    },
                ));
                // Pass as reference (already &T via deref).
                call_args.push_str(&rust_name);
            }
            _ => {
                // Complex types passed as JSON string from Kotlin side.
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                let type_path = type_ref_to_core_path(base_ty, "core_crate");
                // Optional complex params: the Kotlin/Java caller passes an empty
                // string (`""`) when the host-language value is null, the legacy
                // sentinel for "no payload" that pairs with `?.let { ... } ?: ""`.
                // Accept that sentinel as `None` instead of attempting to parse
                // it as JSON (which fails with `EOF while parsing a value`).
                if p.optional {
                    unmarshal.push_str(&render_complex_unmarshal(&rust_name, &type_path, err_null, true));
                    call_args.push_str(&rust_name);
                } else {
                    unmarshal.push_str(&render_complex_unmarshal(&rust_name, &type_path, err_null, false));
                    // Special case: Vec<String> with is_ref means the core expects `&[&str]`.
                    let is_vec_string_ref =
                        p.is_ref && matches!(base_ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));
                    if is_vec_string_ref {
                        let refs_name = format!("{rust_name}_refs");
                        unmarshal.push_str(&render_vec_string_refs(&refs_name, &rust_name));
                        call_args.push('&');
                        call_args.push_str(&refs_name);
                    } else if p.is_ref {
                        call_args.push('&');
                        call_args.push_str(&rust_name);
                    } else {
                        call_args.push_str(&rust_name);
                    }
                }
            }
        }
        call_args.push_str(", ");
    }
    // Remove trailing ", "
    if call_args.ends_with(", ") {
        call_args.truncate(call_args.len() - 2);
    }

    // Open the extern shim and upgrade EnvUnowned -> &mut Env<'_> via an
    // AttachGuard so the body can call get_string / new_string / throw_new etc.
    // We don't use `EnvUnowned::with_env` because it requires the closure to
    // return `Result<T, E>` and to call `.resolve::<P>()` on the outcome — a
    // significant refactor that would lose the existing early-return + sentinel
    // pattern. AttachGuard upgrades inline; panics inside the body are still
    // caught by `run_or_throw` (the existing per-call wrapper).
    out.push_str(&template_env::render(
        "function_shim_open.rs.jinja",
        context! {
            symbol => symbol,
            param_sigs => param_sigs,
            ret_decl => ret_decl,
        },
    ));

    out.push_str(&unmarshal);

    // Build the raw call expression (without async wrapping yet).
    let raw_call = if call_args.is_empty() {
        format!("{core_fn}()")
    } else {
        format!("{core_fn}({call_args})")
    };

    if has_error {
        let mut ok_body = String::new();
        if is_opaque_return {
            ok_body.push_str("            Box::into_raw(Box::new(v)) as jlong\n");
        } else {
            emit_return_marshal_with_indent(&mut ok_body, return_type, "            ", err_null);
        }
        render_call_result_body(out, &raw_call, is_async, true, err_null, &ok_body, "");
    } else {
        let mut value_body = String::new();
        if is_opaque_return {
            value_body.push_str("    Box::into_raw(Box::new(v)) as jlong\n");
        } else {
            emit_return_marshal_with_indent(&mut value_body, return_type, "    ", err_null);
        }
        render_call_result_body(out, &raw_call, is_async, false, err_null, "", &value_body);
    }
}
