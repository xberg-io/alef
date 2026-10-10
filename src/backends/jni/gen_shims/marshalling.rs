fn render_param_decl(name: &str, type_name: &str) -> String {
    template_env::render(
        "param_decl.rs.jinja",
        crate::alef_context! {
            name => name,
            type_name => type_name,
        },
    )
}

fn render_string_unmarshal(name: &str, ret_null: &str) -> String {
    template_env::render(
        "string_unmarshal.rs.jinja",
        crate::alef_context! {
            name => name,
            ret_null => ret_null,
        },
    )
}

fn render_byte_array_unmarshal(name: &str, ret_null: &str, is_optional: bool) -> String {
    template_env::render(
        "byte_array_unmarshal.rs.jinja",
        crate::alef_context! {
            name => name,
            ret_null => ret_null,
            is_optional => is_optional,
        },
    )
}

fn render_base64_bytes_unmarshal(name: &str, ret_null: &str, is_optional: bool) -> String {
    template_env::render(
        "base64_bytes_unmarshal.rs.jinja",
        crate::alef_context! {
            name => name,
            ret_null => ret_null,
            is_optional => is_optional,
        },
    )
}

fn render_complex_unmarshal(name: &str, type_path: &str, ret_null: &str, is_optional: bool) -> String {
    template_env::render(
        "complex_unmarshal.rs.jinja",
        crate::alef_context! {
            name => name,
            type_path => type_path,
            ret_null => ret_null,
            is_optional => is_optional,
        },
    )
}

fn render_request_string_unmarshal(ret_null: &str, error_prefix: &str) -> String {
    template_env::render(
        "request_string_unmarshal.rs.jinja",
        crate::alef_context! {
            ret_null => ret_null,
            error_prefix => error_prefix,
        },
    )
}

/// Emit unmarshal code for a single param.
///
/// Special cases:
/// - `Vec<u8>` / `Bytes`: the JNI param is `<rust_name>: jbyteArray`; use
///   `env.convert_byte_array` — no JSON round-trip.
/// - `Path` (`PathBuf`): the JNI param is `request_json: JString`; construct
///   `std::path::PathBuf::from(string)` instead of JSON-deserializing.
/// - Everything else: JSON-deserialize from `request_json: JString`.
///
/// When `is_optional` is true, the emitted binding has type `Option<T>` and an
/// empty-string sentinel (from Kotlin's `obj?.let { writeValueAsString(it) } ?: ""`)
/// is decoded as `None` rather than failing with `EOF while parsing`.
fn emit_single_param_unmarshal(
    out: &mut String,
    rust_name: &str,
    ty: &TypeRef,
    ret_null: &str,
    is_optional: bool,
    map_is_btree: bool,
) {
    match ty {
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            // SAFETY: `source` is a valid jbyteArray produced by the JNI caller.
            out.push_str(&render_byte_array_unmarshal(rust_name, ret_null, is_optional));
        }
        TypeRef::Bytes => {
            // SAFETY: `source` is a valid jbyteArray produced by the JNI caller.
            out.push_str(&render_byte_array_unmarshal(rust_name, ret_null, is_optional));
        }
        TypeRef::Path => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            out.push_str(&template_env::render(
                "path_unmarshal.rs.jinja",
                crate::alef_context! {
                    name => rust_name,
                },
            ));
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            out.push_str(&template_env::render(
                "vec_string_unmarshal.rs.jinja",
                crate::alef_context! {
                    name => rust_name,
                    ret_null => ret_null,
                },
            ));
        }
        TypeRef::String => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            out.push_str(&template_env::render(
                "request_string_value_unmarshal.rs.jinja",
                crate::alef_context! {
                    name => rust_name,
                },
            ));
        }
        _ => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            let type_path = type_ref_to_core_path_with_btree(ty, "core_crate", map_is_btree);
            out.push_str(&template_env::render(
                "json_value_unmarshal.rs.jinja",
                crate::alef_context! {
                    name => rust_name,
                    type_path => type_path,
                    ret_null => ret_null,
                    is_optional => is_optional,
                },
            ));
        }
    }
}

/// Emit the return marshalling code inside the `Ok(v) =>` arm.
fn emit_return_marshal(out: &mut String, return_type: &TypeRef, ret_null: &str) {
    emit_return_marshal_with_indent(out, return_type, "            ", ret_null);
}

/// Emit the return marshalling code with a configurable leading indent.
///
/// Use the 12-space variant from inside an `Ok(v) =>` match arm; pass a
/// 4-space indent for the no-error code path that binds `v` directly.
///
/// `ret_null` is the sentinel value emitted on serialization failure so the
/// caller can distinguish an error return from a legitimate zero/null result.
fn emit_return_marshal_with_indent(out: &mut String, return_type: &TypeRef, indent: &str, ret_null: &str) {
    match return_type {
        TypeRef::Unit => {}
        TypeRef::Primitive(PrimitiveType::Bool) => {
            out.push_str(&template_env::render(
                "return_bool.rs.jinja",
                crate::alef_context! {
                    indent => indent,
                },
            ));
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            out.push_str(&template_env::render(
                "return_byte_array.rs.jinja",
                crate::alef_context! {
                    indent => indent,
                    bytes_expr => "&v",
                },
            ));
        }
        TypeRef::Bytes => {
            out.push_str(&template_env::render(
                "return_byte_array.rs.jinja",
                crate::alef_context! {
                    indent => indent,
                    bytes_expr => "v.as_ref()",
                },
            ));
        }
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Bytes)
                || matches!(inner.as_ref(), TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8))) =>
        {
            let bytes_expr = match inner.as_ref() {
                TypeRef::Bytes => "bytes.as_ref()",
                _ => "&bytes",
            };
            out.push_str(&template_env::render(
                "return_optional_byte_array.rs.jinja",
                crate::alef_context! {
                    indent => indent,
                    bytes_expr => bytes_expr,
                },
            ));
        }
        TypeRef::Primitive(p) => {
            // Only pass a cast target when the JNI wire type actually differs from `v`'s own
            // Rust type (see `jni_primitive_needs_cast`) -- otherwise `v as <wire type>` is a
            // no-op cast that trips `clippy::unnecessary_cast`.
            let jni_ty = if jni_primitive_needs_cast(p) {
                jni_primitive_type(p)
            } else {
                ""
            };
            out.push_str(&template_env::render(
                "return_primitive.rs.jinja",
                crate::alef_context! {
                    indent => indent,
                    jni_ty => jni_ty,
                },
            ));
        }
        TypeRef::String => {
            out.push_str(&template_env::render(
                "return_string.rs.jinja",
                crate::alef_context! { indent => indent },
            ));
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
            out.push_str(&template_env::render(
                "return_optional_string.rs.jinja",
                crate::alef_context! { indent => indent },
            ));
        }
        TypeRef::Optional(_) => {
            out.push_str(&format!("{indent}match v {{\n"));
            out.push_str(&format!("{indent}    None => std::ptr::null_mut(),\n"));
            out.push_str(&format!("{indent}    Some(inner) => {{\n"));
            out.push_str(&format!(
                "{indent}        let s = match serde_json::to_string(&inner) {{\n"
            ));
            out.push_str(&format!("{indent}            Ok(s) => s,\n"));
            out.push_str(&format!(
                "{indent}            Err(e) => {{ throw_jni_error(env, &format!(\"serialize: {{e}}\")); return {ret_null}; }}\n"
            ));
            out.push_str(&format!("{indent}        }};\n"));
            out.push_str(&format!("{indent}        string_to_jstring(env, s)\n"));
            out.push_str(&format!("{indent}    }}\n"));
            out.push_str(&format!("{indent}}}\n"));
        }
        _ => {
            out.push_str(&template_env::render(
                "return_json.rs.jinja",
                crate::alef_context! {
                    indent => indent,
                    ret_null => ret_null,
                },
            ));
        }
    }
}

#[cfg(test)]
mod marshalling_tests {
    use super::*;

    /// Return-side counterpart of the liter-llm sighting: a method returning `f64` must not
    /// cast its own return value to its own JNI wire type (`jni::sys::jdouble`, an alias of
    /// `f64`). This test was red before the fix (`out` contained `"v as jni::sys::jdouble"`).
    #[test]
    fn f64_return_has_no_cast() {
        let mut out = String::new();
        emit_return_marshal_with_indent(&mut out, &TypeRef::Primitive(PrimitiveType::F64), "    ", "0.0f64");
        assert_eq!(out, "    v\n");
    }

    /// Sibling positive control: `u64`'s JNI wire type (`jlong`) genuinely differs from `u64`,
    /// so the return cast must still be emitted.
    #[test]
    fn u64_return_still_casts() {
        let mut out = String::new();
        emit_return_marshal_with_indent(&mut out, &TypeRef::Primitive(PrimitiveType::U64), "    ", "0");
        assert_eq!(out, "    v as jlong\n");
    }
}
