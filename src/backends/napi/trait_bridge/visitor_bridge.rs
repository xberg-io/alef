use crate::codegen::generators::trait_bridge::{bridge_param_type as param_type, to_camel_case, visitor_param_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, ParamDef, TypeDef, TypeRef};
use std::cell::RefCell;
use std::collections::HashMap;

/// One site where [`build_napi_args`] could not marshal a parameter natively.
///
/// Recorded instead of silently falling back to a `{:?}` Debug-string encoding (the historic
/// behavior, and the root of several trait-bridge argument-corruption bugs found in production).
/// A caller that owns a whole bridge/trait generation pass collects these and turns a non-empty
/// list into a hard `anyhow::bail!` naming every site, so an unhandled parameter shape is a
/// generation-time error instead of a runtime protocol corruption.
#[derive(Debug, Clone)]
pub(crate) struct UnsupportedArg {
    pub trait_name: String,
    pub method_name: String,
    pub param_name: String,
    pub type_desc: String,
}

pub(super) fn gen_visitor_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
    api: &ApiSurface,
) -> anyhow::Result<String> {
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Napi,
    )?;
    let mut method_impls = String::with_capacity(4096);
    let unsupported: RefCell<Vec<UnsupportedArg>> = RefCell::new(Vec::new());
    let json_safe = json_safe_named_types(api);
    for method in crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg) {
        gen_visitor_method_napi(
            &mut method_impls,
            method,
            trait_path,
            core_crate,
            bridge_cfg,
            type_paths,
            &result_metadata,
            &trait_type.name,
            &unsupported,
            &json_safe,
        );
    }

    let unsupported = unsupported.into_inner();
    if !unsupported.is_empty() {
        let sites = unsupported
            .iter()
            .map(|u| {
                format!(
                    "  - {}::{}({}: {})",
                    u.trait_name, u.method_name, u.param_name, u.type_desc
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!(
            "napi visitor bridge for '{}' cannot marshal {} parameter(s) to JS natively:\n{}",
            trait_type.name,
            unsupported.len(),
            sites
        );
    }

    Ok(crate::backends::napi::template_env::render(
        "visitor_bridge.jinja",
        crate::alef_context! {
            core_crate => core_crate,
            context_type_path => context_helper.type_path,
            context_field_lines => context_helper.field_lines,
            struct_name => struct_name,
            trait_path => trait_path,
            method_impls => method_impls,
        },
    ))
}

/// Named enum types safe to JSON-encode as a trait-bridge callback argument: real,
/// serde-enabled, non-excluded enums. Anything else reaching a bare `Named` param
/// (an opaque/handle type, or one the extractor couldn't otherwise resolve) has no proven
/// `Serialize` impl, so `build_napi_args` keeps the historic Debug-string representation for it
/// instead of emitting a `serde_json::to_value` call that might not compile against the real
/// core type.
pub(super) fn json_safe_named_types(api: &ApiSurface) -> std::collections::HashSet<String> {
    api.enums
        .iter()
        .filter(|e| e.has_serde && !e.binding_excluded)
        .map(|e| e.name.clone())
        .collect()
}

/// Build the Function args tuple type string for a given number of Unknown args.
pub(super) fn unknown_tuple_type(count: usize) -> String {
    if count == 0 {
        return "()".to_string();
    }
    let parts = vec!["napi::bindgen_prelude::Unknown"; count];
    format!("({}{})", parts.join(", "), if count == 1 { "," } else { "" })
}

/// Generate a single visitor method that checks for a camelCase JS property and calls it.
#[allow(clippy::too_many_arguments)]
fn gen_visitor_method_napi(
    out: &mut String,
    method: &MethodDef,
    _trait_path: &str,
    _core_crate: &str,
    bridge_cfg: &TraitBridgeConfig,
    type_paths: &HashMap<String, String>,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
    trait_name: &str,
    unsupported: &RefCell<Vec<UnsupportedArg>>,
    json_safe_named_types: &std::collections::HashSet<String>,
) {
    let name = &method.name;
    let js_method_name = to_camel_case(name);

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let signature = sig_parts.join(", ");

    let return_type = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    let arg_count = method.params.len();
    let empty_args = arg_count == 0;
    let inner_tuple_ty = unknown_tuple_type(arg_count);
    let args_tuple_ty = if empty_args {
        inner_tuple_ty
    } else {
        format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
    };

    let js_args_exprs = build_napi_args(
        method,
        bridge_cfg,
        &std::collections::HashSet::new(),
        "Js",
        trait_name,
        unsupported,
        json_safe_named_types,
    );
    let arg_exprs: Vec<String> = js_args_exprs
        .iter()
        .map(|expr| expr.replace("self.env()", "__env"))
        .collect();

    let tuple_args = if arg_count == 1 {
        "(arg_0,)".to_string()
    } else if arg_count > 0 {
        let arg_names: Vec<String> = (0..arg_count).map(|i| format!("arg_{i}")).collect();
        format!("({})", arg_names.join(", "))
    } else {
        String::new()
    };

    out.push_str(&crate::backends::napi::template_env::render(
        "visitor_method.jinja",
        crate::alef_context! {
            method_name => name,
            js_method_name => js_method_name,
            signature => signature,
            return_type => return_type,
            default_result_expr => crate::codegen::visitor_result::default_result_expr(&return_type, result_metadata),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &return_type,
                result_metadata,
                "s",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            empty_args => empty_args,
            arg_exprs => arg_exprs,
            tuple_args => tuple_args,
            args_tuple_ty => args_tuple_ty,
        },
    ));
}

/// Returns true if a napi value for `ty` can be produced directly via `ToNapiValue`
/// (Rust -> JS), recursing into `Vec`. Used to decide when a trait-bridge callback
/// argument can be passed as a native JS value instead of a Debug-string encoded one.
pub(super) fn is_napi_encodable(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String => true,
        TypeRef::Primitive(_) => true,
        TypeRef::Vec(inner) => is_napi_encodable(inner),
        _ => false,
    }
}

/// Returns true if a napi value for `ty` can be decoded directly via `FromNapiValue`
/// (JS -> Rust), recursing into `Vec`. Unlike [`is_napi_encodable`], `f32` is excluded:
/// napi-rs implements `ToNapiValue` for `f32` but not `FromNapiValue` (JS numbers decode
/// as `f64`), so a `Vec<f32>` / `Vec<Vec<f32>>` return type cannot be decoded natively.
pub(super) fn is_napi_decodable(ty: &TypeRef) -> bool {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::String => true,
        // `f32` has `ToNapiValue` but no `FromNapiValue` (see the module doc above this
        // function's caller). `u64`/`usize`/`isize` have neither: napi-rs's blanket integer
        // macro (`bindgen_runtime::js_values::number`) covers u8/i8/u16/i16/u32/i32/i64/f64
        // only -- u64/usize/isize instead get a BigInt-based `ToNapiValue` (encodable as an
        // argument) with no `FromNapiValue` counterpart at all (not decodable as a return).
        // Verified against napi 3.12.4's source directly; a mismatch here previously would have
        // produced a `from_napi_value` call on `u64` that fails to compile (E0277).
        TypeRef::Primitive(p) => !matches!(
            p,
            PrimitiveType::F32 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize
        ),
        TypeRef::Vec(inner) => is_napi_decodable(inner),
        _ => false,
    }
}

/// Returns the "f64 analog" of `ty` if `ty` is a (possibly `Vec`-nested) `f32` — i.e. the
/// only reason [`is_napi_decodable`] rejects it is the `f32` leaf. napi-rs has no
/// `FromNapiValue for f32`, but every JS number already round-trips losslessly through
/// `f64` (which does implement `FromNapiValue`), so such a return type can still decode
/// natively: decode into this f64 analog, then cast element-wise back to `f32` (see
/// [`f32_bridge_cast_expr`]).
///
/// Returns `None` for anything else: already-decodable types (no bridging needed) and
/// genuinely non-native leaves (`Named`/`Bytes`/`Map`/...), which keep the JSON fallback.
pub(super) fn f32_bridge_target(ty: &TypeRef) -> Option<TypeRef> {
    match ty {
        TypeRef::Primitive(crate::core::ir::PrimitiveType::F32) => {
            Some(TypeRef::Primitive(crate::core::ir::PrimitiveType::F64))
        }
        TypeRef::Vec(inner) => f32_bridge_target(inner).map(|t| TypeRef::Vec(Box::new(t))),
        _ => None,
    }
}

/// Build the Rust expression that element-wise casts a decoded f64-analog value bound to
/// `var` back into the original `f32`-leaved shape described by `ty`. Mirrors `ty`'s `Vec`
/// nesting with `.into_iter().map(|v| ...).collect()`, bottoming out in `as f32`.
pub(super) fn f32_bridge_cast_expr(ty: &TypeRef, var: &str) -> String {
    match ty {
        TypeRef::Vec(inner) => {
            let inner_expr = f32_bridge_cast_expr(inner, "v");
            format!("{var}.into_iter().map(|v| {inner_expr}).collect()")
        }
        _ => format!("{var} as f32"),
    }
}

/// Build NAPI argument expressions for a visitor method.
///
/// Returns one expression per parameter, each producing a `napi::bindgen_prelude::Unknown`.
/// `unsafe { ToNapiValue::to_napi_value(env, null()) }`, the shared "encoding failed / nothing
/// to encode" fallback used throughout the arms below.
fn napi_null_expr() -> String {
    "unsafe { \
     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }"
        .to_string()
}

/// `unsafe { ToNapiValue::to_napi_value(env, <value_expr>) }`, falling back to
/// [`napi_null_expr`] on conversion failure.
fn to_napi_value_expr(value_expr: &str) -> String {
    format!(
        "unsafe {{ \
         let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), {value_expr}).unwrap_or(std::ptr::null_mut()); \
         napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }}"
    )
}

/// Encode `value_expr` (any `Serialize` value) as its `serde_json::Value` JSON representation
/// and hand that to `ToNapiValue` (napi-rs's `serde-json` feature implements it for
/// `serde_json::Value`, always enabled when a crate has trait bridges — see
/// `scaffold::languages::node`). Used for shapes with no cheaper native `ToNapiValue`
/// encoding: enums, maps, `Duration`, already-JSON values, and `Optional<T>` for a non-`String`
/// `T`. Round-trips faithfully (unlike the historic `{:?}` Debug-string encoding it replaces),
/// at the cost of a JSON allocation per call.
fn json_encode_expr(value_expr: &str) -> String {
    // `serde_json::to_value<T: Serialize>(value: T)` takes its argument BY VALUE (not `&T`);
    // every caller already passes an owned or already-reference expression as appropriate
    // (`clippy::needless_borrows_for_generic_args` flags a redundant `&` here otherwise).
    to_napi_value_expr(&format!(
        "serde_json::to_value({value_expr}).unwrap_or(serde_json::Value::Null)"
    ))
}

/// Build the NAPI argument expressions for one trait-bridge method's parameters.
///
/// Every returned expression evaluates to a `napi::bindgen_prelude::Unknown` and textually
/// contains the literal substring `self.env()` wherever it needs the bridge's live `Env` —
/// callers requiring a different env expression (the visitor path's local `__env`, or an
/// async bridge's `ctx.env` inside a threadsafe-function callback) do a literal
/// `.replace("self.env()", ...)` on the result, so every arm MUST spell it exactly that way.
///
/// A parameter shape with no arm below is not silently encoded (the historic `{:?}`
/// Debug-string fallback, root cause of a class of production argument-corruption bugs): it is
/// recorded into `unsupported` instead, and the caller that owns the whole generation pass
/// turns a non-empty `unsupported` into a hard `anyhow::bail!` naming every site.
pub(super) fn build_napi_args(
    method: &MethodDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_param_types: &std::collections::HashSet<String>,
    type_prefix: &str,
    trait_name: &str,
    unsupported: &RefCell<Vec<UnsupportedArg>>,
    json_safe_named_types: &std::collections::HashSet<String>,
) -> Vec<String> {
    let ctx = NapiArgCtx {
        method,
        bridge_cfg,
        struct_param_types,
        type_prefix,
        trait_name,
        unsupported,
        json_safe_named_types,
    };
    method.params.iter().map(|p| build_one_napi_arg(p, &ctx)).collect()
}

/// The per-bridge context `build_one_napi_arg` consults when a parameter needs a real marshalling
/// decision rather than a type-only lookup.
struct NapiArgCtx<'a> {
    method: &'a MethodDef,
    bridge_cfg: &'a TraitBridgeConfig,
    struct_param_types: &'a std::collections::HashSet<String>,
    type_prefix: &'a str,
    trait_name: &'a str,
    unsupported: &'a RefCell<Vec<UnsupportedArg>>,
    json_safe_named_types: &'a std::collections::HashSet<String>,
}

impl NapiArgCtx<'_> {
    fn record_unsupported(&self, p: &ParamDef, type_desc: String) -> String {
        self.unsupported.borrow_mut().push(UnsupportedArg {
            trait_name: self.trait_name.to_string(),
            method_name: self.method.name.clone(),
            param_name: p.name.clone(),
            type_desc,
        });
        napi_null_expr()
    }
}

fn named_napi_arg(p: &ParamDef, n: &str, ctx: &NapiArgCtx<'_>) -> String {
    if Some(n) == ctx.bridge_cfg.context_type.as_deref() {
        return crate::backends::napi::template_env::render(
            "visitor_context_arg_expr.jinja",
            crate::alef_context! { ref_prefix => if p.is_ref { "" } else { "&" }, name => p.name.as_str() },
        )
        .trim_end()
        .to_string();
    }
    if ctx.struct_param_types.contains(n) {
        let owned = if p.is_ref {
            format!("(*{}).clone()", p.name)
        } else {
            p.name.clone()
        };
        return format!(
            "unsafe {{ \
             let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), {prefix}{ty}::from({owned})).unwrap_or(std::ptr::null_mut()); \
             napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }}",
            prefix = ctx.type_prefix,
            ty = n,
        );
    }
    if ctx.json_safe_named_types.contains(n) {
        // A real, serde-enabled enum: JSON-encode it faithfully instead of
        // Debug-stringifying (`serde_json::to_value` -- the enum genuinely implements
        // `Serialize`, unlike an opaque/unknown `Named` param, which may not).
        return json_encode_expr(&p.name);
    }
    // Opaque/handle or otherwise-unknown `Named` param: no cheap native `ToNapiValue` and no
    // proven `Serialize` impl either, so this keeps the prior Debug-string representation
    // rather than risk a JSON encode that doesn't compile against the real core type.
    to_napi_value_expr(&format!("format!(\"{{:?}}\", {})", p.name))
}

/// The two `Optional` shapes with a dedicated arm; `None` for a parameter that is not optional
/// in a way those arms recognise.
fn optional_napi_arg(p: &ParamDef, ctx: &NapiArgCtx<'_>) -> Option<String> {
    if !p.optional {
        return None;
    }
    if matches!(&p.ty, TypeRef::String) {
        return p.is_ref.then(|| {
            format!(
                "match {name} {{ Some(s) => {some}, None => {null} }}",
                name = p.name,
                some = to_napi_value_expr("s"),
                null = napi_null_expr()
            )
        });
    }
    // `Optional<non-String>` — no per-primitive native arm. `p.ty` here is the IR's INNER
    // type (the `Optional` wrapper is carried on `p.optional`, not as a nested
    // `TypeRef::Optional`, for a top-level parameter), so `Some`/`None` on the owned Rust
    // value round-trips through `Option<T>: Serialize` -- but only when `T` provably
    // implements it (a bare `Named` inner must be a serde-enabled enum, same rule as the
    // bare-`Named` arm above; anything else -- primitives, Bytes, Path, Vec, Map, Json,
    // Duration -- is already known-`Serialize`).
    if let TypeRef::Named(n) = &p.ty
        && !ctx.json_safe_named_types.contains(n.as_str())
    {
        return Some(ctx.record_unsupported(p, format!("Optional({:?})", p.ty)));
    }
    Some(json_encode_expr(&p.name))
}

/// Parameter shapes whose marshalling depends on the parameter alone (no bridge context).
fn scalar_napi_arg(p: &ParamDef) -> Option<String> {
    use crate::core::ir::PrimitiveType;

    let owned_or_cloned = |ref_suffix: &str| {
        if p.is_ref {
            format!("{}{ref_suffix}", p.name)
        } else {
            format!("{}.clone()", p.name)
        }
    };
    Some(match &p.ty {
        // `&str: ToNapiValue`. Routed through the same raw-pointer `ToNapiValue`/
        // `Unknown::from_raw_unchecked` pattern as every other arm below, rather than
        // `Env::create_string(..).to_unknown()`: the latter returns a `JsString<'_>` borrowed
        // from the `Env` it was created on, which does not compile inside a threadsafe-function
        // `build_callback` closure -- there, `ctx.env: Env` is owned by the closure's own stack
        // frame, so a value borrowed from it cannot be returned as part of the closure's own
        // `Ok((..))` result (E0515). The raw-pointer form only ever moves a `napi_value`
        // (a plain pointer), never a borrow, so it is valid in every context this function's
        // output is spliced into (an inline method-body expression, or a closure return).
        TypeRef::String if p.is_ref => to_napi_value_expr(&p.name),
        TypeRef::String => to_napi_value_expr(&format!("{}.clone()", p.name)),
        TypeRef::Char => to_napi_value_expr(&format!("{}.to_string()", p.name)),
        // `&Path` and `PathBuf` both expose `.to_string_lossy()`; ownership doesn't change the
        // expression, only whether `{name}` is a reference or a value.
        TypeRef::Path => to_napi_value_expr(&format!("{}.to_string_lossy().into_owned()", p.name)),
        TypeRef::Bytes => to_napi_value_expr(&format!(
            "napi::bindgen_prelude::Buffer::from({})",
            owned_or_cloned(".to_vec()")
        )),
        TypeRef::Duration => to_napi_value_expr(&format!("{}.as_secs_f64()", p.name)),
        TypeRef::Json => to_napi_value_expr(&if p.is_ref {
            format!("{}.clone()", p.name)
        } else {
            p.name.clone()
        }),
        TypeRef::Map(_, _) => json_encode_expr(&p.name),
        TypeRef::Primitive(PrimitiveType::Usize) => to_napi_value_expr(&format!("{} as u32", p.name)),
        TypeRef::Primitive(_) => to_napi_value_expr(&p.name),
        _ => return None,
    })
}

fn vec_napi_arg(p: &ParamDef, inner: &TypeRef, ctx: &NapiArgCtx<'_>) -> String {
    if is_napi_encodable(inner) {
        return to_napi_value_expr(&if p.is_ref {
            format!("{}.to_vec()", p.name)
        } else {
            format!("{}.clone()", p.name)
        });
    }
    // Vec of a non-natively-encodable element (Bytes, Path, Map, Json, Duration, a
    // serde-enabled enum, ...): JSON-encode the whole vector. A `Vec<Named>` element must be
    // a proven-`Serialize` enum, same rule as the bare-`Named` arm above -- an opaque/unknown
    // element type is a generation-time error rather than a guessed encoding.
    if let TypeRef::Named(n) = inner
        && !ctx.json_safe_named_types.contains(n.as_str())
    {
        return ctx.record_unsupported(p, format!("{:?}", p.ty));
    }
    json_encode_expr(&p.name)
}

fn build_one_napi_arg(p: &ParamDef, ctx: &NapiArgCtx<'_>) -> String {
    if let TypeRef::Named(n) = &p.ty {
        return named_napi_arg(p, n, ctx);
    }
    if let Some(expr) = optional_napi_arg(p, ctx) {
        return expr;
    }
    if let Some(expr) = scalar_napi_arg(p) {
        return expr;
    }
    if let TypeRef::Vec(inner) = &p.ty {
        return vec_napi_arg(p, inner, ctx);
    }
    ctx.record_unsupported(p, format!("{:?}", p.ty))
}
