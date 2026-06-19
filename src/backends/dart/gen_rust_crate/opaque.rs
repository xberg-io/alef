use std::collections::HashSet;

use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::{AdapterConfig, Language, ResolvedCrateConfig};
use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};

use super::{bridge_fn, conversions};

pub(super) fn has_duration_or_path_field(ty: &TypeRef) -> bool {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::Duration | TypeRef::Path | TypeRef::Json => true,
        // Non-identity primitive widening: u8/i8/u16/i16/u32/i32/u64/usize/isize/f32 all
        // get widened to i64 (or f64 for floats) in the FRB bridge, causing size mismatches.
        TypeRef::Primitive(p) => !matches!(p, PrimitiveType::I64 | PrimitiveType::F64 | PrimitiveType::Bool),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => has_duration_or_path_field(inner),
        _ => false,
    }
}

/// Returns true if `f` has any param the Dart bridge cannot reconstruct.
///
/// Currently the only such case is `Vec<(Vec<u8>, …)>` — a tuple-of-bytes
/// container whose IR-flattened form (`Vec<String>`) cannot be losslessly
/// converted back into `Vec<(Vec<u8>, T)>`. Skipping the function entirely
/// at the bridge surface is preferred over emitting a panicking shim.
pub(super) fn has_unbridgeable_param(f: &crate::core::ir::FunctionDef) -> bool {
    for p in &f.params {
        let Some(orig) = p.original_type.as_deref() else {
            continue;
        };
        let stripped_orig = orig.replace(' ', "");
        // IR format for a tuple-vec param after sanitization:
        //   Vec(Named("(Vec<u8>, T, …)"))
        // (the `original_type` records the tuple shape; the real `ty` is `Vec<String>`).
        // Round-tripping `Vec<u8>` through a JSON string is lossy, so skip emission
        // entirely rather than emit a panicking shim.
        if stripped_orig.starts_with("Vec(Named(\"(Vec<u8>,") {
            return true;
        }
    }
    false
}

/// Emit an `impl TypeName { }` block for an opaque type that exposes methods.
///
/// FRB v2 generates Dart-side instance methods on opaque handles only when the
/// bridge crate contains `impl TypeName { #[frb] pub fn method(...) }` blocks.
/// Without these blocks FRB emits an empty abstract class with no methods.
///
/// Each method body delegates to `self.inner.method_name(...)` after converting
/// mirror-type parameters to the core type via `unsafe { transmute }` (for types
/// whose mirror layout is identical to core) or `From` conversion (for sanitized
/// types). Async methods use `.await` and return `Result<MirrorType, String>`.
///
/// Methods listed in `stub_methods` are omitted from the FRB surface; unsupported
/// methods should be hidden with explicit backend config instead of generated as
/// callable runtime fallbacks.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_opaque_impl_block(
    out: &mut String,
    ty: &TypeDef,
    source_crate_name: &str,
    stub_methods: &[String],
    types_needing_from_conversion: &HashSet<String>,
    opaque_type_names: &HashSet<String>,
    streaming_adapters: &std::collections::HashMap<String, &AdapterConfig>,
    config: &ResolvedCrateConfig,
    type_paths: &std::collections::HashMap<String, String>,
    mirror_type_names: &HashSet<String>,
) {
    let type_name = &ty.name;

    // Collect unique paths that need to be brought into scope:
    //   1. Trait sources, so trait-provided methods on `self.inner` resolve.
    //   2. Named types referenced in method param/return signatures — FRB strips full
    //      module paths from generated code, so excluded types and
    //      `SyncExtractor` referenced via fully-qualified paths in the IR appear bare
    //      in the emitted bridge and need a `use` to resolve.
    let mut trait_uses: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    fn collect_named(ty: &crate::core::ir::TypeRef, out: &mut std::collections::BTreeSet<String>) {
        use crate::core::ir::TypeRef;
        match ty {
            TypeRef::Named(n) => {
                out.insert(n.clone());
            }
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named(inner, out),
            TypeRef::Map(k, v) => {
                collect_named(k, out);
                collect_named(v, out);
            }
            _ => {}
        }
    }
    let mut named_refs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for method in &ty.methods {
        if stub_methods.contains(&method.name) {
            continue;
        }
        if method.sanitized {
            let adapter_key = format!("{type_name}.{}", method.name);
            if !streaming_adapters.contains_key(&adapter_key) {
                continue;
            }
        }
        if let Some(path) = method.trait_source.as_deref() {
            trait_uses.insert(path.to_string());
        }
        for p in &method.params {
            collect_named(&p.ty, &mut named_refs);
        }
        collect_named(&method.return_type, &mut named_refs);
    }
    // Map each Named name → its qualified Rust path, and skip names that:
    //   - resolve to the current type (already in scope), OR
    //   - already have a `#[frb(mirror(TypeName))]` struct in scope (would cause E0255
    //     if we also emit a `use source_crate::TypeName;` for the same short name), OR
    //   - lack a path entry (primitives, sanitized types, etc.).
    for name in &named_refs {
        if name == type_name {
            continue;
        }
        // Skip types that already have a local struct in scope — the frb(mirror) macro
        // (for mirror types) and the opaque wrapper struct (for opaque types) both
        // bring the type's short name into scope, so a separate `use` would conflict
        // (E0255: "the name X is defined multiple times").
        if mirror_type_names.contains(name) || opaque_type_names.contains(name) {
            continue;
        }
        if let Some(path) = type_paths.get(name)
            && path.contains("::")
        {
            trait_uses.insert(path.clone());
        }
    }
    for path in &trait_uses {
        out.push_str(&crate::backends::dart::template_env::render(
            "rust_use.rs.jinja",
            minijinja::context! {
                path => path.as_str(),
            },
        ));
    }

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_impl_open.rs.jinja",
        minijinja::context! {
            type_name => type_name,
        },
    ));

    for method in &ty.methods {
        let method_name = &method.name;
        if stub_methods.contains(method_name) {
            continue;
        }

        // Sanitized methods: check for a streaming adapter.
        // If one exists, emit a StreamSink<T> variant; otherwise skip entirely.
        if method.sanitized {
            let adapter_key = format!("{type_name}.{method_name}");
            if let Some(adapter) = streaming_adapters.get(&adapter_key) {
                emit_streaming_sink_method(out, method_name, adapter, types_needing_from_conversion, config);
            } else {
                out.push_str(&crate::backends::dart::template_env::render(
                    "rust_skipped_sanitized_method_comment.rs.jinja",
                    minijinja::context! {
                        method_name => method_name,
                    },
                ));
            }
            continue;
        }

        // Static methods: emit as standalone functions within the impl block (FRB recognizes this).
        if method.is_static {
            emit_static_opaque_method(
                out,
                ty,
                method,
                source_crate_name,
                stub_methods,
                types_needing_from_conversion,
                opaque_type_names,
            );
            continue;
        }

        emit_opaque_method(
            out,
            ty,
            method,
            source_crate_name,
            stub_methods,
            types_needing_from_conversion,
            opaque_type_names,
        );
    }

    out.push_str("}\n");
}

/// Emit a `pub fn method_name(&self, params..., sink: StreamSink<ItemType>)` method
/// for a streaming adapter. FRB v2 recognises the `StreamSink<T>` parameter and generates
/// a `Stream<T>` accessor on the Dart side.
fn emit_streaming_sink_method(
    out: &mut String,
    method_name: &str,
    adapter: &AdapterConfig,
    _types_needing_from_conversion: &HashSet<String>,
    config: &ResolvedCrateConfig,
) {
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    // Build the Rust parameter list (excluding self and sink).
    let params: Vec<String> = adapter
        .params
        .iter()
        .map(|p| {
            let ty = if p.optional {
                format!("Option<{}>", p.ty)
            } else {
                p.ty.clone()
            };
            format!("{}: {ty}", p.name)
        })
        .collect();
    let params_str = if params.is_empty() {
        String::new()
    } else {
        format!(", {}", params.join(", "))
    };

    // Delegate to the streaming body generator for the Dart language.
    let (body, _struct_def) =
        crate::adapters::streaming::generate_body(adapter, crate::core::config::Language::Dart, config)
            .unwrap_or_else(|_| {
                (
                    String::from(
                        "compile_error!(\"alef cannot generate this Dart streaming adapter; configure a supported adapter body or exclude the method\")",
                    ),
                    None,
                )
            });

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_streaming_sink_method.rs.jinja",
        minijinja::context! {
            method_name => method_name,
            params_str => params_str.as_str(),
            item_type => item_type,
            body => body.as_str(),
        },
    ));
}

/// Emit one method inside an `impl TypeName { }` block for an FRB opaque type.
fn emit_opaque_method(
    out: &mut String,
    _ty: &TypeDef,
    method: &MethodDef,
    source_crate_name: &str,
    _stub_methods: &[String],
    types_needing_from_conversion: &HashSet<String>,
    opaque_type_names: &HashSet<String>,
) {
    use bridge_fn::frb_rust_type_mirror;

    let method_name = &method.name;

    // Receiver: FRB opaque types support `&self`, `&mut self`, and owned `self`.
    // D7: when the inner method takes owned `self` (e.g. builder `build(self)`),
    // emit owned `self` here so Rust can move out of `self.inner` rather than trying
    // to move out of a shared reference (`error[E0507]`).
    let self_param = match &method.receiver {
        Some(ReceiverKind::RefMut) => "&mut self",
        Some(ReceiverKind::Owned) => "self",
        _ => "&self",
    };

    // Build parameter list (excluding the receiver).
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let rust_ty = frb_rust_type_mirror(&p.ty, p.optional);
            format!("{}: {rust_ty}", p.name)
        })
        .collect();

    let async_kw = if method.is_async { "async " } else { "" };

    // Return type: always `Result<MirrorType, String>` when an error type is present.
    let has_error = method.error_type.is_some();
    let ret_ty = if has_error {
        let ok_ty = frb_rust_type_mirror(&method.return_type, false);
        format!("Result<{ok_ty}, String>")
    } else {
        frb_rust_type_mirror(&method.return_type, false)
    };

    let params_str = params.join(", ");
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_opaque_method_open.rs.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_name => method_name,
            self_param => self_param,
            params_str => params_str.as_str(),
            ret_ty => ret_ty.as_str(),
        },
    ));

    emit_opaque_method_body(
        out,
        method,
        source_crate_name,
        types_needing_from_conversion,
        opaque_type_names,
    );

    out.push_str("    }\n");
}

/// Emit a static method inside an `impl TypeName { }` block for an FRB opaque type.
///
/// Static methods are bridged as `pub fn method_name(params...) -> Result<ReturnType, String>`
/// without a receiver. The body calls `TypeName::method_name(...)` on the core type directly.
fn emit_static_opaque_method(
    out: &mut String,
    ty: &TypeDef,
    method: &MethodDef,
    source_crate_name: &str,
    _stub_methods: &[String],
    types_needing_from_conversion: &HashSet<String>,
    opaque_type_names: &HashSet<String>,
) {
    use bridge_fn::frb_rust_type_mirror;

    let method_name = &method.name;

    // Build parameter list (excluding the receiver since this is a static method).
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let rust_ty = frb_rust_type_mirror(&p.ty, p.optional);
            format!("{}: {rust_ty}", p.name)
        })
        .collect();

    let async_kw = if method.is_async { "async " } else { "" };

    // Return type: always `Result<MirrorType, String>` when an error type is present.
    let has_error = method.error_type.is_some();
    let ret_ty = if has_error {
        let ok_ty = frb_rust_type_mirror(&method.return_type, false);
        format!("Result<{ok_ty}, String>")
    } else {
        frb_rust_type_mirror(&method.return_type, false)
    };

    let params_str = params.join(", ");
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_static_opaque_method_open.rs.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_name => method_name,
            params_str => params_str.as_str(),
            ret_ty => ret_ty.as_str(),
        },
    ));

    emit_static_opaque_method_body(
        out,
        ty,
        method,
        source_crate_name,
        types_needing_from_conversion,
        opaque_type_names,
    );

    out.push_str("    }\n");
}

/// Emit the body of a static method inside an opaque-type `impl` block.
///
/// Converts each parameter from the local mirror type to the core type, calls
/// `CoreTypeName::method_name(...)` statically, and wraps the return value in the mirror type.
fn emit_static_opaque_method_body(
    out: &mut String,
    ty: &TypeDef,
    method: &MethodDef,
    source_crate_name: &str,
    types_needing_from_conversion: &HashSet<String>,
    opaque_type_names: &HashSet<String>,
) {
    use conversions::frb_rust_type_inner;

    let method_name = &method.name;
    let type_name = &ty.name;
    let core_type_path = format!("{source_crate_name}::{type_name}");

    // Build per-argument conversion: mirror type → core type (same as instance methods).
    let call_args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_name = &p.name;
            match &p.ty {
                TypeRef::Named(mirror_name) => {
                    if opaque_type_names.contains(mirror_name.as_str()) {
                        if p.optional {
                            return format!("{param_name}.map(|h| h.inner)");
                        }
                        if p.is_ref {
                            return format!("&{param_name}.inner");
                        }
                        return format!("{param_name}.inner");
                    }
                    let core_ty = format!("{source_crate_name}::{mirror_name}");
                    if types_needing_from_conversion.contains(mirror_name.as_str()) {
                        if p.optional {
                            format!("{param_name}.map({core_ty}::from)")
                        } else if p.is_ref {
                            format!("&{core_ty}::from({param_name})")
                        } else {
                            format!("{core_ty}::from({param_name})")
                        }
                    } else {
                        if p.optional {
                            format!("{param_name}.map(|v| unsafe {{ ::std::mem::transmute::<{mirror_name}, {core_ty}>(v) }})")
                        } else if p.is_ref {
                            format!("unsafe {{ ::std::mem::transmute::<&{mirror_name}, &{core_ty}>(&{param_name}) }}")
                        } else {
                            format!("unsafe {{ ::std::mem::transmute::<{mirror_name}, {core_ty}>({param_name}) }}")
                        }
                    }
                }
                TypeRef::Vec(inner) => {
                    if let TypeRef::Named(mirror_name) = inner.as_ref() {
                        let core_ty = format!("{source_crate_name}::{mirror_name}");
                        if types_needing_from_conversion.contains(mirror_name.as_str()) {
                            if p.optional {
                                format!("{param_name}.map(|v| v.into_iter().map({core_ty}::from).collect())")
                            } else {
                                format!("{param_name}.into_iter().map({core_ty}::from).collect()")
                            }
                        } else {
                            if p.optional {
                                format!("{param_name}.map(|v| unsafe {{ ::std::mem::transmute::<Vec<{mirror_name}>, Vec<{core_ty}>>(v) }})")
                            } else {
                                format!("unsafe {{ ::std::mem::transmute::<Vec<{mirror_name}>, Vec<{core_ty}>>({param_name}) }}")
                            }
                        }
                    } else if matches!(inner.as_ref(), TypeRef::String) && p.is_ref && p.vec_inner_is_ref {
                        // Core takes `&[&str]`; FRB delivers `Vec<String>`.
                        format!("&{param_name}.iter().map(|s| s.as_str()).collect::<Vec<_>>()")
                    } else if p.is_ref {
                        format!("&{param_name}")
                    } else {
                        param_name.clone()
                    }
                }
                TypeRef::Bytes => {
                    if p.is_ref {
                        format!("&{param_name}")
                    } else {
                        format!("::bytes::Bytes::from({param_name})")
                    }
                }
                TypeRef::Primitive(prim) => {
                    let native = conversions::primitive_name(prim);
                    let frb_ty = frb_rust_type_inner(&TypeRef::Primitive(prim.clone()));
                    if native == frb_ty {
                        param_name.clone()
                    } else if p.optional {
                        format!("{param_name}.map(|v| v as {native})")
                    } else {
                        format!("{param_name} as {native}")
                    }
                }
                TypeRef::String => {
                    if p.is_ref && p.optional {
                        format!("{param_name}.as_deref()")
                    } else if p.is_ref {
                        format!("&{param_name}")
                    } else {
                        param_name.clone()
                    }
                }
                _ => param_name.clone(),
            }
        })
        .collect();

    let call_args_str = call_args.join(", ");

    let call = format!("{core_type_path}::{method_name}({call_args_str})");

    // Wrap the return value using the same logic as instance methods.
    let wrap_return = build_opaque_return_wrap(&method.return_type, method.returns_ref);
    let has_error = method.error_type.is_some();

    emit_opaque_call_return(out, &call, &wrap_return, method.is_async, has_error);
}

fn emit_opaque_call_return(out: &mut String, call: &str, wrap_return: &str, is_async: bool, has_error: bool) {
    let template = match (is_async, has_error, wrap_return.is_empty()) {
        (true, true, true) => "rust_opaque_call_await_error.rs.jinja",
        (true, true, false) => "rust_opaque_call_await_map_error.rs.jinja",
        (true, false, true) => "rust_opaque_call_await.rs.jinja",
        (true, false, false) => "rust_opaque_call_wrap_await.rs.jinja",
        (false, true, true) => "rust_opaque_call_error.rs.jinja",
        (false, true, false) => "rust_opaque_call_map_error.rs.jinja",
        (false, false, true) => "rust_opaque_call.rs.jinja",
        (false, false, false) => "rust_opaque_call_wrap.rs.jinja",
    };

    // Convert closure `|v| body` to the implementation body `body`.
    // The templates will now wrap this in a `let v = call; body` block.
    let wrap_return_impl = if wrap_return.starts_with("|") {
        // Strip the leading closure syntax and extract the body.
        // Pattern: |v| ... or |v: Type| ...
        if let Some(body_start) = wrap_return.find("|") {
            if let Some(body_end) = wrap_return[body_start + 1..].find("|") {
                let body = &wrap_return[body_start + body_end + 2..].trim();
                body.to_string()
            } else {
                wrap_return.to_string()
            }
        } else {
            wrap_return.to_string()
        }
    } else {
        wrap_return.to_string()
    };

    out.push_str(&crate::backends::dart::template_env::render(
        template,
        minijinja::context! {
            call => call,
            wrap_return => wrap_return,
            wrap_return_impl => wrap_return_impl.as_str(),
        },
    ));
}

/// Emit the body of a method inside an opaque-type `impl` block.
///
/// Converts each parameter from the local mirror type to the core type, calls
/// `self.inner.method_name(...)`, and wraps the return value in the mirror type.
fn emit_opaque_method_body(
    out: &mut String,
    method: &MethodDef,
    source_crate_name: &str,
    types_needing_from_conversion: &HashSet<String>,
    opaque_type_names: &HashSet<String>,
) {
    use conversions::frb_rust_type_inner;

    let method_name = &method.name;
    let has_error = method.error_type.is_some();

    // Build per-argument conversion: mirror type → core type.
    // For Named mirror types (i.e. FRB mirror structs), transmute is sound ONLY when
    // the mirror layout is identical to the core layout. Types in
    // `types_needing_from_conversion` have sanitized fields (e.g. Option<String>
    // substituted for Option<CancellationToken>) causing size mismatches — use From
    // conversion for those. Transmute is zero-cost unlike From, so we prefer it for
    // types with identical layouts.
    // D3: opaque wrapper types use `.inner` access — transmute across crate boundaries
    // for opaque types is unsound (the inner type may not be pub in the source crate).
    let call_args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_name = &p.name;
            match &p.ty {
                TypeRef::Named(mirror_name) => {
                    // D3: opaque wrapper types (e.g. VisitorHandle) expose .inner directly.
                    if opaque_type_names.contains(mirror_name.as_str()) {
                        if p.optional {
                            return format!("{param_name}.map(|h| h.inner)");
                        }
                        if p.is_mut {
                            return format!("&mut {param_name}.inner");
                        }
                        if p.is_ref {
                            return format!("&{param_name}.inner");
                        }
                        return format!("{param_name}.inner");
                    }
                    let core_ty = format!("{source_crate_name}::{mirror_name}");
                    if types_needing_from_conversion.contains(mirror_name.as_str()) {
                        // Layout differs — use the generated From<MirrorT> for CoreT impl.
                        if p.optional {
                            format!("{param_name}.map({core_ty}::from)")
                        } else if p.is_mut {
                            // Cannot take &mut of a temporary — convert to owned then borrow mutably.
                            format!("&mut {core_ty}::from({param_name})")
                        } else if p.is_ref {
                            // Cannot take a reference to a temporary — convert to owned then borrow.
                            format!("&{core_ty}::from({param_name})")
                        } else {
                            format!("{core_ty}::from({param_name})")
                        }
                    } else {
                        // Named mirror type with identical layout: transmute to the source-crate type.
                        if p.optional {
                            format!("{param_name}.map(|v| unsafe {{ ::std::mem::transmute::<{mirror_name}, {core_ty}>(v) }})")
                        } else if p.is_mut {
                            format!("unsafe {{ ::std::mem::transmute::<&mut {mirror_name}, &mut {core_ty}>(&mut {param_name}) }}")
                        } else if p.is_ref {
                            format!("unsafe {{ ::std::mem::transmute::<&{mirror_name}, &{core_ty}>(&{param_name}) }}")
                        } else {
                            format!("unsafe {{ ::std::mem::transmute::<{mirror_name}, {core_ty}>({param_name}) }}")
                        }
                    }
                }
                TypeRef::Vec(inner) => {
                    if let TypeRef::Named(mirror_name) = inner.as_ref() {
                        let core_ty = format!("{source_crate_name}::{mirror_name}");
                        if types_needing_from_conversion.contains(mirror_name.as_str()) {
                            // Elements have differing layouts — convert each via From.
                            if p.optional {
                                format!("{param_name}.map(|v| v.into_iter().map({core_ty}::from).collect::<Vec<_>>())")
                            } else if p.is_ref {
                                format!("&{param_name}.iter().map(|x| {core_ty}::from(x.clone())).collect::<Vec<_>>()")
                            } else {
                                format!("{param_name}.into_iter().map({core_ty}::from).collect::<Vec<_>>()")
                            }
                        } else {
                            if p.optional {
                                format!("{param_name}.map(|v| unsafe {{ ::std::mem::transmute::<Vec<{mirror_name}>, Vec<{core_ty}>>(v) }})")
                            } else if p.is_mut {
                                // &mut [MirrorT] → &mut [CoreT] via transmute (same layout, same size).
                                // Must produce a &mut [CoreT] slice, not a raw *mut pointer.
                                format!(
                                    "unsafe {{ ::std::slice::from_raw_parts_mut(\
                                        ::std::mem::transmute::<*mut {mirror_name}, *mut {core_ty}>({param_name}.as_mut_ptr()), \
                                        {param_name}.len()) }}"
                                )
                            } else if p.is_ref {
                                // &[MirrorT] → &[CoreT] via transmute (same layout, same size).
                                // Must produce a &[CoreT] slice, not a raw *const pointer.
                                format!(
                                    "unsafe {{ ::std::slice::from_raw_parts(\
                                        ::std::mem::transmute::<*const {mirror_name}, *const {core_ty}>({param_name}.as_ptr()), \
                                        {param_name}.len()) }}"
                                )
                            } else {
                                format!("unsafe {{ ::std::mem::transmute::<Vec<{mirror_name}>, Vec<{core_ty}>>({param_name}) }}")
                            }
                        }
                    } else if matches!(inner.as_ref(), TypeRef::String) && p.is_ref && p.vec_inner_is_ref {
                        // Core takes `&[&str]`; FRB delivers `Vec<String>`.
                        // Borrow the temporary Vec<&str> into &[&str] — the temporary lives
                        // long enough for the enclosing statement.
                        format!("&{param_name}.iter().map(|s| s.as_str()).collect::<Vec<_>>()")
                    } else if p.is_ref {
                        // Core takes a slice reference (e.g. `&[u8]`, `&[u32]`, `&[String]`).
                        // Borrowing Vec<T> produces &Vec<T> which coerces to &[T].
                        format!("&{param_name}")
                    } else {
                        param_name.clone()
                    }
                }
                TypeRef::Bytes => {
                    // FRB bridges `bytes::Bytes` as `Vec<u8>`. When the core takes `&[u8]`
                    // (is_ref=true), borrow the Vec so Rust coerces Vec<u8> → &[u8].
                    if p.is_ref {
                        format!("&{param_name}")
                    } else {
                        // Owned case: core takes `Bytes` — convert via From.
                        format!("::bytes::Bytes::from({param_name})")
                    }
                }
                TypeRef::Primitive(prim) => {
                    // FRB widens all integers to i64 and floats to f64. Use the actual
                    // core primitive name to decide whether a narrowing cast is needed.
                    let native = conversions::primitive_name(prim);
                    let frb_ty = frb_rust_type_inner(&TypeRef::Primitive(prim.clone()));
                    if native == frb_ty {
                        // Types match (e.g. both i64, f64, bool) — pass through unchanged.
                        param_name.clone()
                    } else if p.optional {
                        format!("{param_name}.map(|v| v as {native})")
                    } else {
                        format!("{param_name} as {native}")
                    }
                }
                TypeRef::String => {
                    // Core may take `&str` — pass by reference when is_ref is set.
                    if p.is_ref && p.optional {
                        format!("{param_name}.as_deref()")
                    } else if p.is_ref {
                        format!("&{param_name}")
                    } else {
                        param_name.clone()
                    }
                }
                TypeRef::Json => {
                    if p.optional {
                        format!("{param_name}.as_deref().and_then(|s| serde_json::from_str(s).ok())")
                    } else {
                        format!("serde_json::from_str(&{param_name}).unwrap_or(serde_json::Value::Null)")
                    }
                }
                TypeRef::Path => {
                    // FRB bridges PathBuf as String. Convert to PathBuf for the core call.
                    // If core expects &Path, borrow the constructed PathBuf.
                    if p.optional {
                        if p.is_ref {
                            format!("{param_name}.as_ref().map(|s| std::path::Path::new(s))")
                        } else {
                            format!("{param_name}.map(::std::path::PathBuf::from)")
                        }
                    } else if p.is_ref {
                        format!("std::path::Path::new(&{param_name})")
                    } else {
                        format!("::std::path::PathBuf::from({param_name})")
                    }
                }
                _ => param_name.clone(),
            }
        })
        .collect();

    let call = format!("self.inner.{method_name}({})", call_args.join(", "));

    // Wrap the return value: Named return types need to be converted FROM the core
    // type into the local mirror type using the generated `From<source::T> for T` impl.
    // `TypeRef::Bytes` returns `bytes::Bytes` from core but the bridge declares `Vec<u8>` —
    // convert via `.to_vec()`. Primitive widening (i32→i64, usize→i64, f32→f64) and
    // by-ref returns (`&str`, `&Path`, `&[&str]`) need explicit conversion too.
    let wrap_return = build_opaque_return_wrap(&method.return_type, method.returns_ref);

    emit_opaque_call_return(out, &call, &wrap_return, method.is_async, has_error);
}

/// Build the return-value wrapping closure for an opaque method return type.
///
/// Returns an empty string when no wrapping is needed (primitive, String, etc.).
/// Returns a closure expression like `|v| ReturnType::from(v)` for Named types.
/// Returns `|v| v.to_vec()` for `TypeRef::Bytes` (core returns `bytes::Bytes`,
/// mirror declares `Vec<u8>`).
///
/// `returns_ref` indicates the core method returns by reference — `&str`, `&Path`,
/// or `&[&str]` need conversion to the owned mirror types (`String`, `Vec<String>`).
fn build_opaque_return_wrap(ty: &TypeRef, returns_ref: bool) -> String {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::Named(mirror_name) => {
            if returns_ref {
                // Core returns `&T` (e.g. a `global()` accessor). The generated
                // `From<core::T> for T` impl takes an owned value, so clone the
                // borrow before converting. Emit a direct call with turbofish or
                // a suffix that works with method chains.
                format!("|v| {mirror_name}::from(v.clone())")
            } else {
                format!("|v| {mirror_name}::from(v)")
            }
        }
        TypeRef::Bytes => {
            // Core returns `bytes::Bytes`, bridge declares `Vec<u8>`.
            "|v| v.to_vec()".to_string()
        }
        TypeRef::String if returns_ref => {
            // Core returns `&str`, mirror declares `String`.
            "|v: &str| v.to_string()".to_string()
        }
        TypeRef::Path => {
            // Core returns `&Path` or `PathBuf`, mirror declares `String`.
            if returns_ref {
                "|v: &::std::path::Path| v.to_string_lossy().to_string()".to_string()
            } else {
                "|v: ::std::path::PathBuf| v.to_string_lossy().to_string()".to_string()
            }
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(mirror_name) if returns_ref => {
                // Core returns `&[T]` / `Vec<&T>`; iterating borrows each element,
                // so clone before the owned `From` conversion.
                format!("|v| v.iter().map(|e| {mirror_name}::from(e.clone())).collect::<Vec<_>>()")
            }
            TypeRef::Named(mirror_name) => {
                format!("|v| v.into_iter().map({mirror_name}::from).collect::<Vec<_>>()")
            }
            TypeRef::String if returns_ref => {
                // Core returns `&[&str]`, mirror declares `Vec<String>`.
                "|v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>()".to_string()
            }
            _ => String::new(),
        },
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(mirror_name) if returns_ref => {
                // Core returns `Option<&T>` (e.g. a `get()` lookup); clone the
                // borrowed inner before the owned `From` conversion. Annotate
                // the closure param to help type inference.
                format!("|v: Option<_>| v.map(|e| {mirror_name}::from(e.clone()))")
            }
            TypeRef::Named(mirror_name) => {
                // Annotate to disambiguate when From impls are overloaded.
                format!("|v: Option<_>| v.map(|e| {mirror_name}::from(e))")
            }
            TypeRef::String if returns_ref => "|v: Option<&str>| v.map(|s| s.to_string())".to_string(),
            TypeRef::Bytes => {
                // Core returns `Option<&[u8]>`, mirror declares `Option<Vec<u8>>`.
                "|v: Option<_>| v.map(|b| b.to_vec())".to_string()
            }
            _ => String::new(),
        },
        TypeRef::Primitive(prim) => {
            // FRB widens integers to i64 and floats to f64. Cast the core value.
            match prim {
                PrimitiveType::I64 | PrimitiveType::F64 | PrimitiveType::Bool => String::new(),
                PrimitiveType::F32 => "|v| v as f64".to_string(),
                _ => "|v| v as i64".to_string(),
            }
        }
        _ => String::new(),
    }
}

/// Emit a `#[frb] pub fn create_<snake_name>_from_json(json: String) -> Result<TypeName, String>`
/// free function for a non-opaque mirror struct type.
///
/// FRB generates `static Future<TypeName> createTypeNameFromJson(String json)` on the Dart
/// bridge class from this function. Dart e2e tests call this helper to construct typed
/// request objects from the raw JSON fixtures without manually filling every field — this
/// is the `options_via = "from_json"` path for the Dart e2e codegen.
///
/// The body deserializes via `serde_json::from_str` into the core type and converts to the
/// local mirror type using the `From<source_crate::TypeName> for TypeName` impl that is
/// already emitted by `emit_from_impl_for_struct`.
pub(super) fn emit_from_json_fn(out: &mut String, ty: &TypeDef, source_crate_name: &str) {
    let type_name = &ty.name;
    // snake_case function name: e.g. ChatCompletionRequest → create_chat_completion_request_from_json
    let snake = dart_rust_function_component(type_name);
    let fn_name = format!("create_{snake}_from_json");
    let core_ty_base = if ty.rust_path.is_empty() {
        format!("{source_crate_name}::{type_name}")
    } else {
        ty.rust_path.replace('-', "_")
    };
    // Types with lifetime params need <'static> so serde can deserialize into an owned value.
    let core_ty = if ty.has_lifetime_params {
        format!("{core_ty_base}<'static>")
    } else {
        core_ty_base
    };

    out.push_str(&crate::backends::dart::template_env::render(
        "rust_from_json_bridge_fn.rs.jinja",
        minijinja::context! {
            fn_name => fn_name.as_str(),
            type_name => type_name,
            core_ty => core_ty.as_str(),
            source_cfg => ty.cfg.as_deref().unwrap_or(""),
        },
    ));
}

/// Convert a PascalCase type name to snake_case for use in function names.
/// E.g. `ChatCompletionRequest` → `chat_completion_request`.
fn dart_rust_function_component(s: &str) -> String {
    public_host_identifier(Language::Rust, PublicIdentifierKind::Function, s)
}
