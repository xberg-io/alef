use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

use super::conversions::{dart_call_arg, frb_rust_type_inner, primitive_name};
use super::helpers::emit_cleaned_dartdoc;

pub(crate) fn emit_bridge_fn(
    out: &mut String,
    f: &FunctionDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
    types_needing_from_conversion: &std::collections::HashSet<String>,
    opaque_type_names: &std::collections::HashSet<String>,
    _stub_methods: &[String],
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
            // Opaque handle params with is_mut=true require `mut` binding so the body
            // can borrow `&mut name.inner`. This arises when the core function takes
            // `&mut OpaqueType` — the bridge receives an owned handle and must mutably
            // borrow its inner field.
            let is_opaque_handle = if let TypeRef::Named(type_name) = &p.ty {
                opaque_type_names.contains(type_name.as_str())
            } else {
                false
            };
            let mut_prefix = if p.is_mut && is_opaque_handle { "mut " } else { "" };
            format!("{mut_prefix}{}: {rust_ty}", p.name)
        })
        .collect();

    let has_error = f.error_type.is_some();
    // Return type uses local mirror names so FRB's generated SseEncode impls match.
    let return_ty = if has_error {
        format!("Result<{}, String>", frb_rust_type_mirror(&f.return_type, false))
    } else {
        frb_rust_type_mirror(&f.return_type, false)
    };

    out.push_str(&crate::backends::dart::template_env::render(
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

    // When the return type was sanitized (a Named core type collapsed to String because
    // the type is not exported through the Dart binding surface), the core fn still
    // returns the real type which is not `.to_string()`-able. Calling the core fn and
    // then trying to coerce its result into the sanitized binding type would fail to
    // compile (e.g. `Option<EmbeddingPreset>.map(|s| s.to_string())` where
    // `EmbeddingPreset: !Display`). Emit a default-value stub instead — the function is
    // unreachable through the binding surface anyway because the input/output types are
    // not visible to Dart callers.
    if f.return_sanitized {
        let suppress = if f.params.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = f.params.iter().map(|p| p.name.as_str()).collect();
            if names.len() == 1 {
                format!("    let _ = {};\n", names[0])
            } else {
                format!("    let _ = ({});\n", names.join(", "))
            }
        };
        let default_value = sanitized_return_default(&f.return_type);
        let body = if has_error {
            format!("{suppress}    Ok({default_value})\n")
        } else {
            format!("{suppress}    {default_value}\n")
        };
        out.push_str(&body);
        out.push_str("}\n");
        return;
    }

    // Collect pre-call `let` bindings for params that require a two-step conversion
    // (e.g. AHashMap<Cow, Value> params received from FRB as HashMap<String, String>).
    // These must be bound before the call so the reference in the call arg can borrow
    // the owned value rather than a temporary that would drop immediately.
    let mut pre_call_bindings: Vec<String> = Vec::new();

    // Build call-site arguments. Named types (structs/enums declared with
    // `#[frb(mirror(T))]`) are received as the local mirror type but the core fn
    // expects source-crate `T`. For types without sanitized fields, transmute is sound
    // because the layouts are identical. For types with sanitized fields (e.g.
    // a config DTO with sanitized Option<String>
    // fields that differ in size from the core types), we use From<MirrorT> for CoreT
    // to avoid undefined behavior from layout mismatches.
    let call_args: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            // AHashMap<Cow<'static, str>, Value> params: FRB bridges these as
            // HashMap<String, String> (user-friendly types). We need a two-step conversion:
            // (1) bind an owned AHashMap to a named `let` before the call so we can borrow it,
            // (2) pass the reference in the call arg.
            if let TypeRef::Map(_, _) = &p.ty {
                if p.map_is_ahash && p.map_key_is_cow {
                    let bound_name = format!("__{}_ahash", p.name);
                    pre_call_bindings.push(format!(
                        "    let {bound_name} = {}.map(|m| m.into_iter().map(|(k, v)| (std::borrow::Cow::Owned(k), serde_json::Value::String(v))).collect::<ahash::AHashMap<std::borrow::Cow<'static, str>, serde_json::Value>>());",
                        p.name
                    ));
                    return if p.optional && p.is_ref {
                        format!("{bound_name}.as_ref()")
                    } else if p.is_ref {
                        format!("{bound_name}.as_ref().unwrap()")
                    } else {
                        bound_name
                    };
                }
            }
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
    let ret_transmute = return_transmute_expr(
        &f.return_type,
        source_crate_name,
        type_paths,
        opaque_type_names,
        f.returns_ref,
    );

    // Build suffix cast for primitives / Strings.
    let result_cast = if ret_transmute.is_empty() {
        build_primitive_result_cast(&f.return_type, f.returns_ref)
    } else {
        String::new()
    };

    let body = build_body(&call, &result_cast, &ret_transmute, has_error, f.is_async);

    // Emit pre-call bindings (if any) before the body expression.
    if !pre_call_bindings.is_empty() {
        for binding in &pre_call_bindings {
            out.push_str(binding);
            out.push('\n');
        }
    }
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
/// For Named types with sanitized fields (e.g. a config DTO with `Option<String>`
/// in the mirror but different-sized types in core),
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
                "{{ let _ = {name}; compile_error!(\"alef cannot bridge Vec<(Vec<u8>, ...)> through Dart FRB; configure dart.exclude_functions for this item\") }}"
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
            if p.is_mut {
                return format!("&mut {name}.inner");
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
            return build_named_in_from(name, type_name, &core_ty, p.is_ref, p.is_mut, p.optional);
        }
        return build_named_in_transmute(name, type_name, &core_ty, p.is_ref, p.is_mut, p.optional);
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
                    "{name}.map(|v| v.into_iter().map(|x| unsafe {{ std::mem::transmute::<{type_name}, {core_ty}>(x) }}).collect::<Vec<_>>())"
                );
            }
            if p.is_ref {
                // &[MirrorT] → &[CoreT] via transmute (same layout, same size).
                // Must produce a &[CoreT] slice, not a raw *const pointer.
                return format!(
                    "unsafe {{ std::slice::from_raw_parts(\
                        std::mem::transmute::<*const {type_name}, *const {core_ty}>({name}.as_ptr()), \
                        {name}.len()) }}"
                );
            }
            if p.is_mut {
                // &mut [MirrorT] → &mut [CoreT] via transmute (same layout, same size).
                // Must produce a &mut [CoreT] slice, not a raw *mut pointer.
                return format!(
                    "unsafe {{ std::slice::from_raw_parts_mut(\
                        std::mem::transmute::<*mut {type_name}, *mut {core_ty}>({name}.as_mut_ptr()), \
                        {name}.len()) }}"
                );
            }
            return format!(
                "{name}.into_iter().map(|x| unsafe {{ std::mem::transmute::<{type_name}, {core_ty}>(x) }}).collect::<Vec<_>>()"
            );
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
fn build_named_in_from(
    name: &str,
    _mirror_name: &str,
    core_ty: &str,
    is_ref: bool,
    is_mut: bool,
    optional: bool,
) -> String {
    if optional {
        return format!("{name}.map({core_ty}::from)");
    }
    if is_mut {
        // Cannot take &mut of a temporary — convert to a local binding first,
        // then borrow mutably. A `let mut` binding is emitted at the call site.
        return format!("&mut {core_ty}::from({name})");
    }
    if is_ref {
        // Cannot take a reference to a temporary value — convert to owned then borrow.
        // The calling convention becomes by-value and is re-borrowed at the call site.
        return format!("&{core_ty}::from({name})");
    }
    format!("{core_ty}::from({name})")
}

/// Emit the transmute call for a single Named parameter going INTO the core fn.
fn build_named_in_transmute(
    name: &str,
    mirror_name: &str,
    core_ty: &str,
    is_ref: bool,
    is_mut: bool,
    optional: bool,
) -> String {
    if optional {
        return format!("{name}.map(|v| unsafe {{ std::mem::transmute::<{mirror_name}, {core_ty}>(v) }})");
    }
    if is_mut {
        // SAFETY: MirrorT and CoreT are guaranteed layout-compatible (same fields, repr(C) or
        // plain struct). Casting &mut MirrorT → &mut CoreT is sound.
        return format!("unsafe {{ std::mem::transmute::<&mut {mirror_name}, &mut {core_ty}>(&mut {name}) }}");
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
    returns_ref: bool,
) -> String {
    match ty {
        TypeRef::Named(mirror_name) => {
            if opaque_type_names.contains(mirror_name.as_str()) {
                // Opaque wrapper: construct the wrapper struct from the core value.
                format!("|inner| {mirror_name} {{ inner }}")
            } else {
                // Use From conversion: core type implements Into<MirrorType> via the
                // generated From impl. Emit the bare path so clippy::redundant_closure
                // is satisfied at the call site (`.map(MirrorName::from)`).
                format!("{mirror_name}::from")
            }
        }
        TypeRef::Vec(inner) => {
            if let TypeRef::Named(mirror_name) = inner.as_ref() {
                if returns_ref {
                    // v is &[T]; iter() yields &T — must clone before converting via From.
                    if opaque_type_names.contains(mirror_name.as_str()) {
                        format!(
                            "|v: &[_]| v.iter().map(|inner| {mirror_name} {{ inner: inner.clone() }}).collect::<Vec<_>>()"
                        )
                    } else {
                        format!("|v: &[_]| v.iter().map(|x| {mirror_name}::from(x.clone())).collect::<Vec<_>>()")
                    }
                } else if opaque_type_names.contains(mirror_name.as_str()) {
                    format!("|v: Vec<_>| v.into_iter().map(|inner| {mirror_name} {{ inner }}).collect::<Vec<_>>()")
                } else {
                    format!("|v: Vec<_>| v.into_iter().map({mirror_name}::from).collect::<Vec<_>>()")
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

/// Build a Rust default-value expression for a type sanitized from a core Named type.
///
/// Mirrors `gen_unimplemented_body` in `alef-codegen` for primitive/string returns: a
/// sanitized return collapses the core Named type to `String`/`Option<String>`/etc., and
/// the dart bridge stubs the body because the core value cannot be expressed in the
/// sanitized type without `serde_json::to_string` (which requires Serialize bounds the
/// extract pass cannot verify here).
fn sanitized_return_default(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Unit => "()".to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path => "String::new()".to_string(),
        TypeRef::Bytes => "Vec::new()".to_string(),
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "false".to_string(),
            crate::core::ir::PrimitiveType::F32 => "0.0f32".to_string(),
            crate::core::ir::PrimitiveType::F64 => "0.0f64".to_string(),
            _ => "0".to_string(),
        },
        TypeRef::Optional(_) => "None".to_string(),
        TypeRef::Vec(_) => "Vec::new()".to_string(),
        TypeRef::Map(_, _) => "Default::default()".to_string(),
        TypeRef::Duration => "0".to_string(),
        TypeRef::Named(_) | TypeRef::Json => "Default::default()".to_string(),
    }
}

/// Build primitive/string result cast (suffix after the raw value).
fn build_primitive_result_cast(ty: &TypeRef, returns_ref: bool) -> String {
    match ty {
        TypeRef::Primitive(_) => {
            let target = frb_rust_type_inner(ty);
            format!(" as {target}")
        }
        TypeRef::Path => ".display().to_string()".to_string(),
        TypeRef::String | TypeRef::Char => ".to_string()".to_string(),
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path | TypeRef::Char) && returns_ref =>
        {
            // Borrowed string-like core return (e.g. `Option<&str>`) must become `Option<String>`
            // for the FRB bridge. Use `to_string()` for the raw value — `format!("{:?}", v)`
            // would emit the Debug repr (quoted) producing `"bash"` instead of `bash` at the
            // dart call site.
            ".map(|v| v.to_string())".to_string()
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
            TypeRef::Path => {
                // PathBuf does not implement Display; use to_string_lossy().into_owned().
                ".into_iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>()".to_string()
            }
            TypeRef::String | TypeRef::Char => ".into_iter().map(|s| s.to_string()).collect::<Vec<_>>()".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_param(name: &str, ty: TypeRef, is_ref: bool, is_mut: bool, optional: bool) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref,
            is_mut,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    #[test]
    fn is_mut_named_opaque_emits_mut_inner() {
        // Regression: opaque handle parameter with is_mut=true must produce &mut name.inner,
        // not &name.inner (which would fail when core fn takes &mut T).
        let p = make_param(
            "result",
            TypeRef::Named("ExtractionResult".to_string()),
            false,
            true,
            false,
        );
        let mut opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        opaque.insert("ExtractionResult".to_string());
        let needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
        let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
        assert_eq!(expr, "&mut result.inner", "is_mut opaque param must use &mut: {expr}");
    }

    #[test]
    fn is_mut_named_from_emits_mut_borrow() {
        // Regression: Named param with types_needing_from_conversion and is_mut=true
        // must produce &mut CoreTy::from(name).
        let p = make_param(
            "cfg",
            TypeRef::Named("TranslationConfig".to_string()),
            false,
            true,
            false,
        );
        let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
        needs_from.insert("TranslationConfig".to_string());
        let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
        assert!(
            expr.contains("&mut"),
            "is_mut From-converted Named param must emit &mut borrow: {expr}"
        );
    }

    #[test]
    fn is_mut_named_transmute_emits_mut_transmute() {
        // Regression: Named param with transmute path and is_mut=true must produce
        // a mutable transmute, not an immutable one.
        let p = make_param("config", TypeRef::Named("MyConfig".to_string()), false, true, false);
        let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
        let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
        assert!(
            expr.contains("&mut"),
            "is_mut transmute Named param must emit &mut transmute: {expr}"
        );
        assert!(
            expr.contains("transmute"),
            "is_mut transmute Named param must emit transmute: {expr}"
        );
    }

    #[test]
    fn vec_named_is_ref_emits_slice_not_raw_pointer() {
        // Regression: Vec<Named> with is_ref=true must produce a &[CoreT] slice via
        // slice::from_raw_parts, not a raw *const CoreT pointer.
        let p = make_param(
            "categories",
            TypeRef::Vec(Box::new(TypeRef::Named("PiiCategory".to_string()))),
            true,
            false,
            false,
        );
        let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let needs_from: std::collections::HashSet<String> = std::collections::HashSet::new();
        let type_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        let expr = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque);
        assert!(
            expr.contains("from_raw_parts"),
            "Vec<Named> is_ref must use slice::from_raw_parts, got: {expr}"
        );
        // The result must be a &[T] slice, not a bare *const pointer.
        // The transmute internally uses *const for type punning, but the outer
        // expression must be a slice via from_raw_parts.
        assert!(
            expr.contains(".len()"),
            "Vec<Named> is_ref must include .len() for slice bounds, got: {expr}"
        );
    }

    #[test]
    fn collect_in_return_transmute_vec_has_type_annotation() {
        // Regression: collect() in Vec<Named> return conversion must use collect::<Vec<_>>()
        // so Rust can infer the target type without E0282.
        let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("QrCode".to_string())));
        let expr = return_transmute_expr(&ty, "mylib", &std::collections::HashMap::new(), &opaque, false);
        assert!(
            expr.contains("collect::<Vec<_>>()"),
            "Vec<Named> collect must have type annotation: {expr}"
        );
    }

    #[test]
    fn path_vec_result_cast_uses_to_string_lossy() {
        // Regression: Vec<Path> result cast must use .to_string_lossy().into_owned()
        // because PathBuf does not implement Display.
        let ty = TypeRef::Vec(Box::new(TypeRef::Path));
        let cast = build_primitive_result_cast(&ty, false);
        assert!(
            cast.contains("to_string_lossy"),
            "Vec<Path> cast must use to_string_lossy: {cast}"
        );
        assert!(
            !cast.contains(".to_string()"),
            "Vec<Path> must NOT use .to_string(): {cast}"
        );
    }

    #[test]
    fn scalar_path_result_cast_uses_display_not_to_string() {
        // Regression: scalar Path return cast must use .display().to_string()
        // not .to_string() because PathBuf does not implement Display.
        let ty = TypeRef::Path;
        let cast = build_primitive_result_cast(&ty, false);
        assert!(cast.contains("display()"), "Path cast must use .display(): {cast}");
        assert!(cast.contains("to_string()"), "Path cast must use to_string(): {cast}");
    }
}
