fn render_param_decl(name: &str, type_name: &str) -> String {
    template_env::render(
        "param_decl.rs.jinja",
        context! {
            name => name,
            type_name => type_name,
        },
    )
}

fn render_string_unmarshal(name: &str, ret_null: &str) -> String {
    template_env::render(
        "string_unmarshal.rs.jinja",
        context! {
            name => name,
            ret_null => ret_null,
        },
    )
}

fn render_byte_array_unmarshal(name: &str, ret_null: &str, is_optional: bool) -> String {
    template_env::render(
        "byte_array_unmarshal.rs.jinja",
        context! {
            name => name,
            ret_null => ret_null,
            is_optional => is_optional,
        },
    )
}

fn render_complex_unmarshal(name: &str, type_path: &str, ret_null: &str, is_optional: bool) -> String {
    template_env::render(
        "complex_unmarshal.rs.jinja",
        context! {
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
        context! {
            ret_null => ret_null,
            error_prefix => error_prefix,
        },
    )
}

fn render_vec_string_refs(refs_name: &str, source_name: &str) -> String {
    template_env::render(
        "vec_string_refs.rs.jinja",
        context! {
            refs_name => refs_name,
            source_name => source_name,
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
fn emit_single_param_unmarshal(out: &mut String, rust_name: &str, ty: &TypeRef, ret_null: &str, is_optional: bool) {
    match ty {
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            // jbyteArray → Vec<u8> via env.convert_byte_array.
            // SAFETY: `source` is a valid jbyteArray produced by the JNI caller.
            out.push_str(&render_byte_array_unmarshal(rust_name, ret_null, is_optional));
        }
        TypeRef::Bytes => {
            // jbyteArray → Vec<u8> via env.convert_byte_array.
            // The caller uses is_ref=true which will pass &<name> (coerces &Vec<u8> → &[u8]).
            // No bytes crate dependency needed.
            // SAFETY: `source` is a valid jbyteArray produced by the JNI caller.
            out.push_str(&render_byte_array_unmarshal(rust_name, ret_null, is_optional));
        }
        TypeRef::Path => {
            // JString → PathBuf via raw string (no JSON decode).
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            out.push_str(&template_env::render(
                "path_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                },
            ));
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => {
            // Vec<String> — deserialize into `<name>_vec` so the caller can optionally
            // produce `<name>_refs: Vec<&str>` for `&[&str]` call sites.
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            out.push_str(&template_env::render(
                "vec_string_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                    ret_null => ret_null,
                },
            ));
        }
        TypeRef::String => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            // A JSON-encoded string from Kotlin: `MAPPER.writeValueAsString(strParam)` → `"\"hello\""`
            out.push_str(&template_env::render(
                "request_string_value_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                },
            ));
        }
        _ => {
            out.push_str(&render_request_string_unmarshal(ret_null, ""));
            let type_path = type_ref_to_core_path(ty, "core_crate");
            // Kotlin passes "" as the sentinel for None (so we don't have to
            // round-trip a JSON `null` and the wire stays clean for the Some case).
            out.push_str(&template_env::render(
                "json_value_unmarshal.rs.jinja",
                context! {
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
        TypeRef::Unit => {
            // No return value.
        }
        TypeRef::Primitive(PrimitiveType::Bool) => {
            // jni 0.22 + jni-sys 0.4 made `jboolean` a `bool` (it was `u8` in
            // 0.21), so a `bool as bool` cast is a Rust compile error. Return
            // the value as-is.
            out.push_str(&template_env::render(
                "return_bool.rs.jinja",
                context! {
                    indent => indent,
                },
            ));
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            // Vec<u8> → jbyteArray
            out.push_str(&template_env::render(
                "return_byte_array.rs.jinja",
                context! {
                    indent => indent,
                    bytes_expr => "&v",
                },
            ));
        }
        TypeRef::Bytes => {
            // bytes::Bytes → jbyteArray (same as Vec<u8>)
            out.push_str(&template_env::render(
                "return_byte_array.rs.jinja",
                context! {
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
                context! {
                    indent => indent,
                    bytes_expr => bytes_expr,
                },
            ));
        }
        TypeRef::Primitive(p) => {
            // Cast the Rust primitive to the corresponding JNI numeric type.
            // This handles mismatches like u16 → jshort (i16), usize → jlong (i64).
            let jni_ty = jni_primitive_type(p);
            out.push_str(&template_env::render(
                "return_primitive.rs.jinja",
                context! {
                    indent => indent,
                    jni_ty => jni_ty,
                },
            ));
        }
        // Return raw `String` as a jstring without JSON marshalling. JSON-encoding
        // a `String` wraps the value in literal `"…"` (e.g. `Some("python")` →
        // `"\"python\""`), which the Kotlin layer surfaces verbatim because the
        // bridge signature is `external fun foo(...): String?`. Tests then see
        // `"python"` instead of `python` and fail with `expected: <python> but
        // was: <"python">`.
        TypeRef::String => {
            out.push_str(&template_env::render(
                "return_string.rs.jinja",
                context! { indent => indent },
            ));
        }
        // Same fix for `Option<String>`: emit a raw jstring on `Some`, null on
        // `None`. Without this arm the JSON path encodes `None` as the literal
        // string `"null"`, which Kotlin sees as a non-null `String` containing
        // the four characters `n`, `u`, `l`, `l`.
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
            out.push_str(&template_env::render(
                "return_optional_string.rs.jinja",
                context! { indent => indent },
            ));
        }
        _ => {
            out.push_str(&template_env::render(
                "return_json.rs.jinja",
                context! {
                    indent => indent,
                    ret_null => ret_null,
                },
            ));
        }
    }
}
