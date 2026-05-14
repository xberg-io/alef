use alef_core::ir::{FunctionDef, ParamDef, TypeRef};

use super::conversions::{dart_call_arg, frb_rust_type_inner, primitive_name};
use super::helpers::emit_cleaned_dartdoc;

pub(crate) fn emit_bridge_fn(
    out: &mut String,
    f: &FunctionDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
    types_needing_from_conversion: &std::collections::HashSet<String>,
    opaque_type_names: &std::collections::HashSet<String>,
    stub_methods: &[String],
) {
    emit_cleaned_dartdoc(out, &f.doc, "");

    let fn_name = &f.name;
    let async_kw = if f.is_async { "async " } else { "" };

    // Bridge function parameters use the LOCAL mirror type names (no source-crate prefix).
    // FRB's generated wire code decodes arguments into local mirror types (`crate::T`)
    // and passes them to bridge functions. Using source-crate `T` in signatures would
    // create a type mismatch: `crate::T` != source-crate `T` in Rust's type system even
    // though they are layout-identical via `#[frb(mirror(T))]`.
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let rust_ty = frb_rust_type_mirror(&p.ty, p.optional);
            format!("{}: {rust_ty}", p.name)
        })
        .collect();

    let has_error = f.error_type.is_some();
    // Return type uses local mirror names so FRB's generated SseEncode impls match.
    let return_ty = if has_error {
        format!("Result<{}, String>", frb_rust_type_mirror(&f.return_type, false))
    } else {
        frb_rust_type_mirror(&f.return_type, false)
    };

    out.push_str(&crate::template_env::render(
        "rust_bridge_fn_open.jinja",
        minijinja::context! {
            async_kw => async_kw,
            fn_name => fn_name.as_str(),
            params => params.join(", "),
            return_ty => return_ty.as_str(),
        },
    ));

    // Resolve the call target.
    let resolved_path = if f.rust_path.is_empty() {
        format!("{source_crate_name}::{fn_name}")
    } else {
        f.rust_path.replace('-', "_")
    };

    // When the function is listed in `stub_methods`, emit an `unimplemented!()` body
    // immediately. These functions have FFI signatures (e.g. nested tuples containing
    // `Vec<u8>`) that cannot be reconstructed from the FRB wire format. Rather than
    // emitting partially-broken argument conversions, replace the entire body with a
    // clear unimplemented marker so the crate still compiles for code-gen consumers.
    if stub_methods.contains(fn_name) {
        out.push_str("    ::std::unimplemented!(\"this method is listed in dart.stub_methods and cannot be bridged through FRB\")\n");
        out.push_str("}\n");
        return;
    }

    // Build call-site arguments. Named types (structs/enums declared with
    // `#[frb(mirror(T))]`) are received as the local mirror type but the core fn
    // expects source-crate `T`. For types without sanitized fields, transmute is sound
    // because the layouts are identical. For types with sanitized fields (e.g.
    // ExtractionConfig which has cancel_token and concurrency as sanitized Option<String>
    // fields that differ in size from the core types), we use From<MirrorT> for CoreT
    // to avoid undefined behavior from layout mismatches.
    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            dart_call_arg_with_mirror_transmute(
                p,
                source_crate_name,
                type_paths,
                types_needing_from_conversion,
                opaque_type_names,
            )
        })
        .collect();

    let call = format!("{resolved_path}({})", call_args.join(", "));

    // Determine if the return type needs a mirror-transmute (Named or Vec<Named> or Option<Named>).
    let ret_transmute = return_transmute_expr(&f.return_type, source_crate_name, type_paths, opaque_type_names);

    // Build suffix cast for primitives / Strings.
    let result_cast = if ret_transmute.is_empty() {
        build_primitive_result_cast(&f.return_type, f.returns_ref)
    } else {
        String::new()
    };

    let body = build_body(&call, &result_cast, &ret_transmute, has_error, f.is_async);

    out.push_str(&body);
    out.push_str("}\n");
}

/// Build the FRB-friendly parameter type using **local mirror names** (no source-crate prefix).
/// FRB decodes wire bytes into local `crate::T` mirror types and passes them to bridge fns.
pub(crate) fn frb_rust_type_mirror(ty: &TypeRef, optional: bool) -> String {
    let inner = frb_rust_type_mirror_inner(ty);
    if optional { format!("Option<{inner}>") } else { inner }
}

fn frb_rust_type_mirror_inner(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => frb_rust_type_inner(&TypeRef::Primitive(p.clone())),
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", frb_rust_type_mirror_inner(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", frb_rust_type_mirror_inner(inner)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            frb_rust_type_mirror_inner(k),
            frb_rust_type_mirror_inner(v)
        ),
        // Named types: use bare name (resolves to local mirror `crate::T`)
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "i64".to_string(),
    }
}

/// Build call-site expression for one parameter, transmuting Named mirror types to core types.
///
/// For Named types without sanitized fields, the bridge fn receives the local mirror type
/// (`crate::T`) but the core function expects source-crate `T`. Since `#[frb(mirror(T))]`
/// guarantees identical layout for non-sanitized structs, `unsafe { std::mem::transmute }`
/// is sound and zero-cost for those.
///
/// For Named types with sanitized fields (e.g. ExtractionConfig, which has cancel_token
/// and concurrency as `Option<String>` in the mirror but different-sized types in core),
/// we use `SourceT::from(name)` instead to avoid UB from layout mismatches.
fn dart_call_arg_with_mirror_transmute(
    p: &ParamDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
    types_needing_from_conversion: &std::collections::HashSet<String>,
    opaque_type_names: &std::collections::HashSet<String>,
) -> String {
    let name = &p.name;
    let original = p.original_type.as_deref().unwrap_or("");
    let stripped_orig = original
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim();

    // Tuple parameters.
    if !stripped_orig.is_empty() && stripped_orig.starts_with("Vec(") && stripped_orig.contains("Named(\"(") {
        let tuple_inner = stripped_orig
            .find("Named(\"(")
            .and_then(|start| {
                let rest = &stripped_orig[start + 8..];
                rest.find(")\")")
                    .map(|end| rest[..end].trim_end_matches(')').to_string())
            })
            .unwrap_or_default();
        if tuple_inner.starts_with("PathBuf,") || tuple_inner.starts_with("PathBuf ,") {
            return format!("{name}.into_iter().map(|p| (std::path::PathBuf::from(p), None)).collect::<Vec<_>>()");
        }
        if tuple_inner.starts_with("Vec<u8>,") || tuple_inner.starts_with("Vec<u8> ,") {
            return format!(
                "{{ let _ = {name}; ::std::unimplemented!(\"batch_extract_bytes from Dart not yet bridged\") }}"
            );
        }
    }

    // Path.
    if matches!(p.ty, TypeRef::Path) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(std::path::Path::new)");
            }
            return format!("{name}.map(std::path::PathBuf::from)");
        }
        if p.is_ref {
            return format!("std::path::Path::new(&{name})");
        }
        return format!("std::path::PathBuf::from({name})");
    }

    // Primitives: cast back to original width.
    if let TypeRef::Primitive(prim) = &p.ty {
        let target = primitive_name(prim);
        if target != "i64" && target != "f64" && target != "bool" {
            if p.optional {
                return format!("{name}.map(|v| v as {target})");
            }
            return if p.is_ref {
                format!("&({name} as {target})")
            } else {
                format!("{name} as {target}")
            };
        }
    }

    // Inner-Vec primitive cast.
    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Primitive(prim) = inner.as_ref() {
            let target = primitive_name(prim);
            if target != "i64" && target != "f64" && target != "bool" {
                if p.optional {
                    if p.is_ref {
                        return format!(
                            "{name}.as_ref().map(|v| v.iter().map(|x| *x as {target}).collect::<Vec<_>>()).as_deref()"
                        );
                    }
                    return format!("{name}.map(|v| v.into_iter().map(|x| x as {target}).collect::<Vec<_>>())");
                }
                if p.is_ref {
                    return format!("{name}.iter().map(|x| *x as {target}).collect::<Vec<_>>().as_slice()");
                }
                return format!("{name}.into_iter().map(|x| x as {target}).collect::<Vec<_>>()");
            }
        }
    }

    // Opaque wrapper types: access .inner field directly (no transmute needed).
    // These use #[frb(opaque)] struct { inner: source::T } pattern, so the bridge fn
    // parameter is the wrapper and .inner gives the core type.
    if let TypeRef::Named(type_name) = &p.ty {
        if opaque_type_names.contains(type_name.as_str()) {
            if p.optional {
                if p.is_ref {
                    return format!("{name}.as_ref().map(|h| &h.inner)");
                }
                return format!("{name}.map(|h| h.inner)");
            }
            if p.is_ref {
                return format!("&{name}.inner");
            }
            return format!("{name}.inner");
        }
    }

    // Named type: use From conversion for types with sanitized fields (layout differs),
    // or transmute for types with identical mirror/core layout.
    if let TypeRef::Named(type_name) = &p.ty {
        let core_ty = resolve_core_type(type_name, source_crate_name, type_paths);
        if types_needing_from_conversion.contains(type_name.as_str()) {
            return build_named_in_from(name, type_name, &core_ty, p.is_ref, p.optional);
        }
        return build_named_in_transmute(name, type_name, &core_ty, p.is_ref, p.optional);
    }

    // Vec<Named>: use From or transmute depending on whether the element type has sanitized fields.
    if let TypeRef::Vec(inner) = &p.ty {
        if let TypeRef::Named(type_name) = inner.as_ref() {
            let core_ty = resolve_core_type(type_name, source_crate_name, type_paths);
            if types_needing_from_conversion.contains(type_name.as_str()) {
                // Use From conversion for each element.
                if p.optional {
                    return format!("{name}.map(|v| v.into_iter().map({core_ty}::from).collect::<Vec<_>>())");
                }
                return format!("{name}.into_iter().map({core_ty}::from).collect::<Vec<_>>()");
            }
            if p.optional {
                return format!(
                    "{name}.map(|v| unsafe {{ std::mem::transmute::<Vec<{type_name}>, Vec<{core_ty}>>(v) }})"
                );
            }
            if p.is_ref {
                // &[MirrorT] → &[CoreT] via transmute (same layout, same size)
                return format!(
                    "unsafe {{ std::mem::transmute::<*const {type_name}, *const {core_ty}>({name}.as_ptr()) }}"
                );
            }
            return format!("unsafe {{ std::mem::transmute::<Vec<{type_name}>, Vec<{core_ty}>>({name}) }}");
        }
    }

    // Option<Named>: use From or transmute depending on whether the type has sanitized fields.
    if let TypeRef::Optional(inner) = &p.ty {
        if let TypeRef::Named(type_name) = inner.as_ref() {
            let core_ty = resolve_core_type(type_name, source_crate_name, type_paths);
            if types_needing_from_conversion.contains(type_name.as_str()) {
                return format!("{name}.map({core_ty}::from)");
            }
            return format!("{name}.map(|v| unsafe {{ std::mem::transmute::<{type_name}, {core_ty}>(v) }})");
        }
    }

    // Default: fall back to original logic for non-Named cases.
    dart_call_arg(p)
}

fn resolve_core_type(
    type_name: &str,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match type_paths.get(type_name) {
        Some(path) => path.clone(),
        None => format!("{source_crate_name}::{type_name}"),
    }
}

/// Emit a From-based conversion for a single Named parameter going INTO the core fn.
///
/// Used when the mirror type has sanitized fields that make transmute unsound.
/// Relies on the generated `From<MirrorT> for CoreT` impl from `emit_from_mirror_to_core_struct`.
fn build_named_in_from(name: &str, _mirror_name: &str, core_ty: &str, is_ref: bool, optional: bool) -> String {
    if optional {
        return format!("{name}.map({core_ty}::from)");
    }
    if is_ref {
        // Cannot take a reference to a temporary value — convert to owned then borrow.
        // The calling convention becomes by-value and is re-borrowed at the call site.
        return format!("&{core_ty}::from({name})");
    }
    format!("{core_ty}::from({name})")
}

/// Emit the transmute call for a single Named parameter going INTO the core fn.
fn build_named_in_transmute(name: &str, mirror_name: &str, core_ty: &str, is_ref: bool, optional: bool) -> String {
    if optional {
        return format!("{name}.map(|v| unsafe {{ std::mem::transmute::<{mirror_name}, {core_ty}>(v) }})");
    }
    if is_ref {
        return format!("unsafe {{ std::mem::transmute::<&{mirror_name}, &{core_ty}>(&{name}) }}");
    }
    format!("unsafe {{ std::mem::transmute::<{mirror_name}, {core_ty}>({name}) }}")
}

/// Build a conversion expression for the return value FROM the core fn to the local mirror type.
/// Returns an empty string if no conversion is needed.
/// Returns a closure/expression string that wraps the raw call value.
///
/// Uses `From<SourceT> for T` rather than transmute, because mirror types may differ
/// in layout (e.g. `Cow<'static, str>` in core vs `String` in mirror).
fn return_transmute_expr(
    ty: &TypeRef,
    _source_crate_name: &str,
    _type_paths: &std::collections::HashMap<String, String>,
    opaque_type_names: &std::collections::HashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(mirror_name) => {
            if opaque_type_names.contains(mirror_name.as_str()) {
                // Opaque wrapper: construct the wrapper struct from the core value.
                format!("|inner| {mirror_name} {{ inner }}")
            } else {
                // Use From conversion: core type implements Into<MirrorType> via the generated From impl.
                format!("|v| {mirror_name}::from(v)")
            }
        }
        TypeRef::Vec(inner) => {
            if let TypeRef::Named(mirror_name) = inner.as_ref() {
                if opaque_type_names.contains(mirror_name.as_str()) {
                    format!("|v| v.into_iter().map(|inner| {mirror_name} {{ inner }}).collect()")
                } else {
                    format!("|v| v.into_iter().map({mirror_name}::from).collect()")
                }
            } else {
                String::new()
            }
        }
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(mirror_name) = inner.as_ref() {
                if opaque_type_names.contains(mirror_name.as_str()) {
                    format!("|v: Option<_>| v.map(|inner| {mirror_name} {{ inner }})")
                } else {
                    // Add explicit type annotation to avoid E0282 type inference failure
                    // when the core return type is ambiguous (e.g. returned via trait object).
                    format!("|v: Option<_>| v.map({mirror_name}::from)")
                }
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Build primitive/string result cast (suffix after the raw value).
fn build_primitive_result_cast(ty: &TypeRef, returns_ref: bool) -> String {
    match ty {
        TypeRef::Primitive(_) => {
            let target = frb_rust_type_inner(ty);
            format!(" as {target}")
        }
        TypeRef::String | TypeRef::Path | TypeRef::Char => ".to_string()".to_string(),
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path | TypeRef::Char) && returns_ref =>
        {
            ".map(|v| format!(\"{:?}\", v))".to_string()
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(prim) => {
                let target = primitive_name(prim);
                if target == "f64" || target == "i64" || target == "bool" {
                    String::new()
                } else {
                    format!(
                        ".into_iter().map(|x| x as {}).collect::<Vec<_>>()",
                        frb_rust_type_inner(inner)
                    )
                }
            }
            TypeRef::String | TypeRef::Path | TypeRef::Char => {
                ".into_iter().map(|s| s.to_string()).collect::<Vec<_>>()".to_string()
            }
            TypeRef::Vec(inner2) => {
                if let TypeRef::Primitive(prim) = inner2.as_ref() {
                    let target = primitive_name(prim);
                    let frb_target = frb_rust_type_inner(inner2);
                    if target != frb_target.as_str() {
                        format!(
                            ".into_iter().map(|row| row.into_iter().map(|x| x as {frb_target}).collect::<Vec<_>>()).collect::<Vec<_>>()"
                        )
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        },
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Path | TypeRef::Char => ".map(|s| s.to_string())".to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// Build the function body string.
///
/// `ret_transmute` is a closure expression `|v| unsafe { transmute(v) }` that wraps
/// the raw Ok value when returning a mirror type. If empty, `result_cast` is used as
/// a suffix instead (for primitives / String).
fn build_body(call: &str, result_cast: &str, ret_transmute: &str, has_error: bool, is_async: bool) -> String {
    if !ret_transmute.is_empty() {
        // Named return type: wrap in transmute closure.
        let transmute_fn = ret_transmute;
        if has_error {
            if is_async {
                return format!("    {call}.await.map({transmute_fn}).map_err(|e| e.to_string())\n");
            }
            return format!("    {call}.map({transmute_fn}).map_err(|e| e.to_string())\n");
        }
        if is_async {
            return format!("    ({transmute_fn})({call}.await)\n");
        }
        return format!("    ({transmute_fn})({call})\n");
    }

    // Non-Named return type: use result_cast suffix.
    if has_error {
        if is_async {
            return format!("    {call}.await.map(|v| v{result_cast}).map_err(|e| e.to_string())\n");
        }
        return format!("    {call}.map(|v| v{result_cast}).map_err(|e| e.to_string())\n");
    }
    if is_async {
        if result_cast.is_empty() {
            return format!("    {call}.await\n");
        }
        return format!("    {call}.await{result_cast}\n");
    }
    if result_cast.is_empty() {
        format!("    {call}\n")
    } else {
        format!("    {call}{result_cast}\n")
    }
}
