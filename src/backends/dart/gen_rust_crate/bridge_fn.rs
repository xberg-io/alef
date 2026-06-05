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
    let ret_transform = return_transform(
        &f.return_type,
        source_crate_name,
        type_paths,
        opaque_type_names,
        f.returns_ref,
    );

    // Build suffix cast for primitives / Strings.
    let result_cast = if matches!(ret_transform, RetTransform::None) {
        build_primitive_result_cast(&f.return_type, f.returns_ref)
    } else {
        String::new()
    };

    let body = build_body(&call, &result_cast, &ret_transform, has_error, f.is_async);

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

/// Describes how to transform the raw core-fn return value into the bridge return value.
///
/// Variants are chosen to keep generated Rust clean under clippy 1.95:
/// no `(|v| ...)(expr)` shapes (`clippy::redundant_closure_call`) and no
/// `.into_iter()` on `&[T]` (`clippy::into_iter_on_ref`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RetTransform {
    /// No mirror transform needed; the raw call value is used as-is (with optional
    /// `result_cast` suffix appended for primitive/string casts).
    None,
    /// Apply `.map(callable)` to the raw call value. Used for scalar `Named` returns
    /// and for `Result<Named, _>` (the `Ok` arm is mapped). The callable is a bare
    /// path like `MirrorName::from` so clippy::redundant_closure is also satisfied.
    Map(String),
    /// Append a suffix expression directly to the raw call value (e.g.
    /// `.into_iter().map(X::from).collect::<Vec<_>>()`). Used for `Vec<Named>` and
    /// `Option<Named>` returns so the emitted Rust never wraps a closure literal
    /// around the call site.
    Suffix(String),
}

/// Decide how to convert the core-fn return into the bridge return.
///
/// Mirror types may differ in layout from core types (e.g. `Cow<'static, str>` in
/// core vs `String` in mirror), so we rely on the generated `From<CoreT> for MirrorT`
/// impl rather than `mem::transmute`.
fn return_transform(
    ty: &TypeRef,
    _source_crate_name: &str,
    _type_paths: &std::collections::HashMap<String, String>,
    opaque_type_names: &std::collections::HashSet<String>,
    returns_ref: bool,
) -> RetTransform {
    match ty {
        TypeRef::Named(mirror_name) => {
            if opaque_type_names.contains(mirror_name.as_str()) {
                // Opaque wrapper: construct the wrapper struct from the core value.
                // This is a closure literal, but it is only used inside `.map(...)`
                // (never `(closure)(expr)`), so clippy::redundant_closure_call is
                // not triggered.
                RetTransform::Map(format!("|inner| {mirror_name} {{ inner }}"))
            } else {
                // Use From conversion: core type implements Into<MirrorType> via the
                // generated From impl. Emit the bare path so clippy::redundant_closure
                // is satisfied at the call site (`.map(MirrorName::from)`).
                RetTransform::Map(format!("{mirror_name}::from"))
            }
        }
        TypeRef::Vec(inner) => {
            if let TypeRef::Named(mirror_name) = inner.as_ref() {
                if returns_ref {
                    // v is &[T]; must use .iter() (not .into_iter() — clippy 1.95's
                    // `into_iter_on_ref` rejects that) and clone before converting via From.
                    if opaque_type_names.contains(mirror_name.as_str()) {
                        RetTransform::Suffix(
                            ".iter().map(|inner| ".to_string()
                                + mirror_name
                                + " { inner: inner.clone() }).collect::<Vec<_>>()",
                        )
                    } else {
                        RetTransform::Suffix(format!(
                            ".iter().map(|x| {mirror_name}::from(x.clone())).collect::<Vec<_>>()"
                        ))
                    }
                } else if opaque_type_names.contains(mirror_name.as_str()) {
                    RetTransform::Suffix(format!(
                        ".into_iter().map(|inner| {mirror_name} {{ inner }}).collect::<Vec<_>>()"
                    ))
                } else {
                    RetTransform::Suffix(format!(".into_iter().map({mirror_name}::from).collect::<Vec<_>>()"))
                }
            } else {
                RetTransform::None
            }
        }
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(mirror_name) = inner.as_ref() {
                if opaque_type_names.contains(mirror_name.as_str()) {
                    RetTransform::Suffix(format!(".map(|inner| {mirror_name} {{ inner }})"))
                } else {
                    RetTransform::Suffix(format!(".map({mirror_name}::from)"))
                }
            } else {
                RetTransform::None
            }
        }
        _ => RetTransform::None,
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
        TypeRef::Vec(inner) => {
            // When `returns_ref` is true the core fn returns a slice reference
            // (e.g. `&'static [&'static str]`). On a slice reference, `.into_iter()`
            // is equivalent to `.iter()` but clippy 1.95's `into_iter_on_ref` lint
            // rejects the former. Pick the iterator method accordingly.
            let iter_method = if returns_ref { "iter" } else { "into_iter" };
            match inner.as_ref() {
                TypeRef::Primitive(prim) => {
                    let target = primitive_name(prim);
                    if target == "f64" || target == "i64" || target == "bool" {
                        String::new()
                    } else {
                        format!(
                            ".{iter_method}().map(|x| x as {}).collect::<Vec<_>>()",
                            frb_rust_type_inner(inner)
                        )
                    }
                }
                TypeRef::Path => {
                    // PathBuf does not implement Display; use to_string_lossy().into_owned().
                    format!(".{iter_method}().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>()")
                }
                TypeRef::String | TypeRef::Char => {
                    format!(".{iter_method}().map(|s| s.to_string()).collect::<Vec<_>>()")
                }
                TypeRef::Vec(inner2) => {
                    if let TypeRef::Primitive(prim) = inner2.as_ref() {
                        let target = primitive_name(prim);
                        let frb_target = frb_rust_type_inner(inner2);
                        if target != frb_target.as_str() {
                            format!(
                                ".{iter_method}().map(|row| row.into_iter().map(|x| x as {frb_target}).collect::<Vec<_>>()).collect::<Vec<_>>()"
                            )
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Path | TypeRef::Char => ".map(|s| s.to_string())".to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// Build the function body string.
///
/// `ret_transform` describes how to convert the raw call value into the bridge
/// return value. When `ret_transform == RetTransform::None`, `result_cast` is used
/// as a plain suffix (for primitives / String).
///
/// The `RetTransform::Suffix(_)` and `RetTransform::Map(_)` branches both produce
/// expressions of the form `<call><suffix>` or `<call>.map(<callable>)` — never
/// `(<closure>)(<call>)` — so the emitted Rust stays clean under clippy 1.95's
/// `redundant_closure_call` lint.
fn build_body(call: &str, result_cast: &str, ret_transform: &RetTransform, has_error: bool, is_async: bool) -> String {
    match ret_transform {
        RetTransform::Map(callable) => {
            let map_fn = callable.as_str();
            if has_error {
                if is_async {
                    return format!("    {call}.await.map({map_fn}).map_err(|e| e.to_string())\n");
                }
                return format!("    {call}.map({map_fn}).map_err(|e| e.to_string())\n");
            }
            // No error: apply the callable directly to the raw value (or awaited value).
            // Both the bare-path shape (`MirrorName::from(call)`) and the inline opaque-wrap
            // shape (`MirrorName { inner: call }`) avoid clippy::redundant_closure_call.
            let raw = if is_async {
                format!("{call}.await")
            } else {
                call.to_string()
            };
            format!("    {expr}\n", expr = call_callable(map_fn, &raw))
        }
        RetTransform::Suffix(suffix) => {
            let s = suffix.as_str();
            if has_error {
                if is_async {
                    return format!("    {call}.await.map(|v| v{s}).map_err(|e| e.to_string())\n");
                }
                return format!("    {call}.map(|v| v{s}).map_err(|e| e.to_string())\n");
            }
            if is_async {
                return format!("    {call}.await{s}\n");
            }
            format!("    {call}{s}\n")
        }
        RetTransform::None => {
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
    }
}

/// Apply a callable string to a bound identifier without producing a
/// `(closure)(ident)` shape that would trigger `clippy::redundant_closure_call`.
///
/// Recognizes the two shapes `return_transform` emits for `RetTransform::Map`:
/// a bare path (`MirrorName::from`) → `MirrorName::from(v)`, and an opaque-wrap
/// closure (`|inner| MirrorName { inner }`) → `MirrorName { inner: v }`.
fn call_callable(callable: &str, ident: &str) -> String {
    if let Some(rest) = callable.strip_prefix("|inner| ") {
        // Opaque-wrapper closure `|inner| MirrorName { inner }` — rewrite as a
        // direct struct literal so clippy::redundant_closure_call does not fire.
        // `rest` is `MirrorName { inner }`.
        if let Some(name) = rest.strip_suffix(" { inner }") {
            return format!("{name} {{ inner: {ident} }}");
        }
    }
    // Bare path (e.g. `MirrorName::from`): emit as a normal function call.
    format!("{callable}({ident})")
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
        let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, false);
        let suffix = match &transform {
            RetTransform::Suffix(s) => s.clone(),
            other => panic!("expected Suffix, got {other:?}"),
        };
        assert!(
            suffix.contains("collect::<Vec<_>>()"),
            "Vec<Named> collect must have type annotation: {suffix}"
        );
    }

    #[test]
    fn vec_named_return_transform_is_suffix_not_closure_call() {
        // Regression (clippy 1.95 redundant_closure_call): Vec<Named> return MUST be
        // emitted as a suffix expression (`.into_iter().map(X::from).collect::<Vec<_>>()`)
        // applied directly to the call, NOT a closure literal wrapping the call site
        // (e.g. `(|v: Vec<_>| v.into_iter().map(X::from).collect::<Vec<_>>())(call)`).
        let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("QrCode".to_string())));
        let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, false);
        match &transform {
            RetTransform::Suffix(s) => {
                assert!(
                    s.starts_with(".into_iter()"),
                    "expected suffix starting with .into_iter(), got {s}"
                );
                assert!(s.contains("QrCode::from"), "expected QrCode::from in suffix, got {s}");
                assert!(!s.contains("|v"), "suffix must not contain a closure literal, got {s}");
            }
            other => panic!("expected Suffix, got {other:?}"),
        }
        let body = build_body("kreuzberg::detect(&x)", "", &transform, false, false);
        assert!(
            !body.contains("|v: Vec<_>|"),
            "body must not emit closure-literal wrap: {body}"
        );
        assert!(
            body.contains("kreuzberg::detect(&x).into_iter().map(QrCode::from).collect::<Vec<_>>()"),
            "body must apply suffix directly to call: {body}"
        );
    }

    #[test]
    fn vec_named_returns_ref_emits_iter_not_into_iter() {
        // Regression (clippy 1.95 into_iter_on_ref): when the core fn returns &[T]
        // (e.g. `&'static [&'static str]`), the bridge must use `.iter()`, NOT
        // `.into_iter()` (which on &[T] is the same but triggers the lint).
        let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Foo".to_string())));
        let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, true);
        match transform {
            RetTransform::Suffix(s) => {
                assert!(
                    s.starts_with(".iter()"),
                    "ref-return Vec<Named> must start with .iter(): {s}"
                );
                assert!(!s.contains(".into_iter()"), "ref-return must not use .into_iter(): {s}");
            }
            other => panic!("expected Suffix, got {other:?}"),
        }
    }

    #[test]
    fn option_named_return_transform_is_suffix_not_closure_call() {
        // Regression: Option<Named> return must be emitted as `.map(X::from)` suffix,
        // not `(|v: Option<_>| v.map(X::from))(call)`.
        let opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("EmbeddingPreset".to_string())));
        let transform = return_transform(&ty, "mylib", &std::collections::HashMap::new(), &opaque, false);
        match &transform {
            RetTransform::Suffix(s) => {
                assert_eq!(s, ".map(EmbeddingPreset::from)");
            }
            other => panic!("expected Suffix, got {other:?}"),
        }
        let body = build_body("kreuzberg::get(&n)", "", &transform, false, false);
        assert!(
            !body.contains("|v: Option<_>|"),
            "body must not emit closure-literal wrap: {body}"
        );
        assert!(
            body.contains("kreuzberg::get(&n).map(EmbeddingPreset::from)"),
            "body must apply suffix directly to call: {body}"
        );
    }

    #[test]
    fn scalar_named_return_transform_does_not_emit_closure_call() {
        // Sync, no-error, scalar Named return: must NOT emit `(closure)(call)`.
        // Bare-path form: `MirrorName::from(call)`. Opaque form: `MirrorName { inner: call }`.
        let mut opaque: std::collections::HashSet<String> = std::collections::HashSet::new();
        let ty_named = TypeRef::Named("Foo".to_string());

        // From-conversion path.
        let t = return_transform(&ty_named, "mylib", &std::collections::HashMap::new(), &opaque, false);
        let body = build_body("kreuzberg::foo()", "", &t, false, false);
        assert!(
            body.contains("Foo::from(kreuzberg::foo())"),
            "sync scalar Named must emit direct call, got: {body}"
        );
        assert!(!body.contains("(Foo::from)("), "must not use (path)(expr) wrap: {body}");

        // Opaque-wrap path.
        opaque.insert("Foo".to_string());
        let t = return_transform(&ty_named, "mylib", &std::collections::HashMap::new(), &opaque, false);
        let body = build_body("kreuzberg::foo()", "", &t, false, false);
        assert!(
            body.contains("Foo { inner: kreuzberg::foo() }"),
            "sync scalar opaque Named must emit struct literal, got: {body}"
        );
        assert!(!body.contains("|inner|"), "must not emit closure: {body}");
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
    fn vec_string_returns_ref_result_cast_uses_iter_not_into_iter() {
        // Regression (clippy 1.95 into_iter_on_ref): when the core fn returns
        // `&'static [&'static str]` (e.g. `kreuzberg::text::ner::known_models`),
        // the result cast must use `.iter()` not `.into_iter()`. This is the
        // suffix path (no mirror transform), so `build_primitive_result_cast`
        // is responsible.
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        let cast_owned = build_primitive_result_cast(&ty, false);
        let cast_ref = build_primitive_result_cast(&ty, true);
        assert!(
            cast_owned.starts_with(".into_iter()"),
            "owned must use .into_iter(): {cast_owned}"
        );
        assert!(cast_ref.starts_with(".iter()"), "ref must use .iter(): {cast_ref}");
        assert!(
            !cast_ref.contains(".into_iter()"),
            "ref must not use .into_iter(): {cast_ref}"
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
