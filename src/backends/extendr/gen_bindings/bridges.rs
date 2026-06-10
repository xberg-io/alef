use crate::backends::extendr::template_env;
use crate::codegen::generators::RustBindingConfig;
use crate::codegen::generators::trait_bridge::BridgeFieldMatch;
use crate::codegen::naming::wire_variant_value;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, TypeRef};
use ahash::AHashSet;

/// Recursively collect all Named type names from a TypeRef into a set.
pub(super) fn collect_named_types_into(ty: &TypeRef, out: &mut AHashSet<String>) {
    match ty {
        TypeRef::Named(name) => {
            out.insert(name.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types_into(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types_into(k, out);
            collect_named_types_into(v, out);
        }
        _ => {}
    }
}

pub(super) fn is_flat_data_enum(e: &EnumDef) -> bool {
    let has_data = e.variants.iter().any(|v| !v.fields.is_empty());
    has_data
        && e.variants
            .iter()
            .filter(|v| !v.fields.is_empty())
            .all(|v| v.fields.len() == 1)
}

/// Returns true if a flat data enum can safely generate a binding→core From impl.
/// Only enums whose tuple variant data is String or Option<String> are safe — complex
/// output-only struct types (DocxMetadata, PdfMetadata, etc.) have no reverse conversion.
pub(super) fn can_flat_data_enum_round_trip(e: &EnumDef) -> bool {
    e.variants.iter().all(|v| {
        if v.fields.is_empty() {
            return true;
        }
        if v.is_tuple && v.fields.len() == 1 {
            let ty = &v.fields[0].ty;
            matches!(ty, TypeRef::String)
                || matches!(ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String))
        } else {
            false
        }
    })
}

/// Returns true if `e` is a tagged data enum (i.e. has `serde_tag`) that cannot be
/// represented as a flat struct, but can be safely round-tripped through serde JSON
/// — at least one variant has data and `is_flat_data_enum` returns false. These
/// enums get a JSON-passthrough binding (newtype around the core type's serde
/// JSON encoding) so the variant payload survives the FFI boundary.
///
/// The core type must implement `Serialize`/`Deserialize` consistently with the wire
/// format. Tagged enums in supported source crates derive both unconditionally, so this is safe.
pub(super) fn is_json_passthrough_data_enum(e: &EnumDef) -> bool {
    if is_flat_data_enum(e) {
        return false;
    }
    if e.serde_tag.is_none() {
        return false;
    }
    e.variants.iter().any(|v| !v.fields.is_empty())
}

/// Generate a JSON-passthrough wrapper struct for a tagged data enum.
///
/// The wrapper carries the serde-JSON encoding of the core enum value in a private
/// `__inner` field. `#[serde(from, into)]` plugs the wrapper into serde so nested
/// deserialization through parent binding structs (e.g. `EmbeddingConfig::from_json`)
/// preserves the inner variant data transparently — the parent's serde derives drive
/// the bridge with no extra glue.
///
/// The struct exposes `from_json(json: String)` (for direct construction from R) and
/// `default()`. From/Into impls bridge to the core type via serde round-trip.
pub(super) fn gen_extendr_json_passthrough_enum_struct(enum_def: &EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    format!(
        r#"#[extendr]
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "{core_path}", into = "{core_path}")]
pub struct {name} {{
    /// Serde-JSON encoding of the underlying core enum value. Preserves the
    /// tagged-variant payload across the FFI boundary so round trips don't drop
    /// inner field data. The field is private-by-convention (double-underscore
    /// prefix) and not surfaced in R; construction goes through `from_json`.
    #[serde(skip)]
    pub __inner: String,
}}

impl From<{core_path}> for {name} {{
    fn from(value: {core_path}) -> Self {{
        Self {{
            __inner: serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string()),
        }}
    }}
}}

impl From<{name}> for {core_path} {{
    fn from(value: {name}) -> Self {{
        if value.__inner.is_empty() {{
            return <{core_path}>::default();
        }}
        serde_json::from_str(&value.__inner).unwrap_or_default()
    }}
}}

#[extendr]
impl {name} {{
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> {name} {{
        <{core_path}>::default().into()
    }}
    pub fn from_json(json: String) -> extendr_api::Result<{name}> {{
        let core: {core_path} =
            serde_json::from_str(&json).map_err(|e| extendr_api::Error::Other(e.to_string()))?;
        Ok(core.into())
    }}
}}
"#
    )
}

/// Generate an extendr function with bridge field binding support.
///
/// For R, the function accepts the options as an Robj (R list), extracts the bridge field
/// from it, creates the bridge, injects it into the decoded options struct, and calls the
/// core function. This is similar to PyO3's gen_bridge_field_function but tailored to R.
pub(super) fn gen_extendr_bridge_field_function(
    api: &ApiSurface,
    func: &FunctionDef,
    bridge_match: &BridgeFieldMatch<'_>,
    core_import: &str,
) -> String {
    let func_name = &func.name;
    let options_param = &bridge_match.param_name;
    let field_name = &bridge_match.field_name;
    let handle_path =
        crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_match.bridge, core_import);
    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("R", bridge_match.bridge);

    let mut param_parts = Vec::new();
    for param in &func.params {
        if param.name == *options_param {
            param_parts.push(format!("{}: Robj", param.name));
        } else {
            match &param.ty {
                TypeRef::String => param_parts.push(format!("{}: String", param.name)),
                _ => param_parts.push(format!("{}: Robj", param.name)),
            }
        }
    }
    let params_str = param_parts.join(", ");

    let return_type = "Result<Robj>";

    let mut call_args = Vec::new();
    for param in &func.params {
        if param.name == *options_param {
            call_args.push("Some(opts)".to_string());
        } else {
            call_args.push(format!("&{}", param.name));
        }
    }

    template_env::render(
        "bridge_field_function.jinja",
        minijinja::context! {
            func_name => func_name,
            params_str => params_str,
            return_type => return_type,
            field_name => field_name,
            options_param => options_param,
            handle_path => handle_path,
            struct_name => struct_name,
            core_import => core_import,
            options_type => &bridge_match.options_type,
            call_args_str => call_args.join(", "),
        },
    )
}

/// Generate a flat Rust struct for a data enum with all-tuple variants.
///
/// The struct has a discriminator field (from `serde_tag`, defaulting to `"format_type"`)
/// plus one `Option<T>` field per data-carrying variant. The variant field name is the
/// snake_case form of the variant name (e.g. `Excel` → `excel`).
///
/// `#[derive(Default)]` is required so `From` impls can use `..Default::default()`.
/// `serde::Serialize`/`Deserialize` are required so the JSON bridge produces and consumes
/// the nested representation.
pub(super) fn gen_extendr_flat_data_enum_struct(
    enum_def: &EnumDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
) -> String {
    let name = &enum_def.name;
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(1024);

    let mut derives: Vec<&str> = cfg.struct_derives.to_vec();
    derives.push("Default");
    derives.push("serde::Serialize");
    derives.push("serde::Deserialize");
    out.push_str(&template_env::render(
        "flat_enum_derive.jinja",
        minijinja::context! {
            derives => derives.join(", "),
        },
    ));

    out.push_str(&template_env::render(
        "flat_enum_struct_header.jinja",
        minijinja::context! {
            name => name,
        },
    ));
    out.push_str(&template_env::render(
        "flat_enum_discriminator_field.jinja",
        minijinja::context! {
            discriminator => discriminator,
        },
    ));

    for variant in &enum_def.variants {
        if !variant.fields.is_empty() && variant.is_tuple {
            if let Some(first_field) = variant.fields.first() {
                let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
                let inner_ty = mapper.map_type(&first_field.ty);
                out.push_str(&template_env::render(
                    "flat_enum_variant_field.jinja",
                    minijinja::context! {
                        field_name => &field_name,
                        inner_ty => &inner_ty,
                    },
                ));
            }
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_struct_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Generate a `From<core::EnumName> for FlatStruct` impl for flat data enums.
///
/// The generic `gen_enum_from_core_to_binding` generates enum→enum arm matching which does
/// not apply to flat structs. This function generates the correct struct-init form.
pub(super) fn gen_extendr_flat_data_enum_from_core(enum_def: &EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "flat_enum_from_core_impl.jinja",
        minijinja::context! {
            core_path => &core_path,
            name => name,
        },
    ));

    for variant in &enum_def.variants {
        let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
        let wire_name = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if variant.fields.is_empty() {
            out.push_str(&template_env::render(
                "flat_enum_from_core_variant_unit.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                },
            ));
        } else if variant.is_tuple {
            let first_field = variant.fields.first().unwrap();
            let is_boxed = first_field.is_boxed;
            let is_sanitized_to_string = first_field.sanitized && matches!(first_field.ty, TypeRef::String);
            let data_expr: String = if is_sanitized_to_string {
                if is_boxed {
                    "format!(\"{:?}\", *_0)".to_string()
                } else {
                    "format!(\"{:?}\", _0)".to_string()
                }
            } else if is_boxed {
                "(*_0).into()".to_string()
            } else {
                "_0.into()".to_string()
            };
            out.push_str(&template_env::render(
                "flat_enum_from_core_variant_tuple.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                    fname => &field_name,
                    expr => &data_expr,
                },
            ));
        } else {
            out.push_str(&template_env::render(
                "flat_enum_from_core_variant_struct.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_from_core_impl_catch_all.jinja",
        minijinja::context! {},
    ));

    out.push_str(&template_env::render(
        "flat_enum_from_core_impl_footer.jinja",
        minijinja::context! {},
    ));
    out
}

pub(super) fn gen_extendr_flat_data_enum_to_core(enum_def: &EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "flat_enum_from_binding_impl.jinja",
        minijinja::context! {
            name => name,
            core_path => &core_path,
            discriminator => discriminator,
        },
    ));

    for variant in &enum_def.variants {
        let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
        let wire_name = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if variant.fields.is_empty() {
            out.push_str(&template_env::render(
                "flat_enum_from_binding_variant_unit.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    vname => &variant.name,
                },
            ));
        } else if variant.is_tuple {
            out.push_str(&template_env::render(
                "flat_enum_from_binding_variant_tuple.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    vname => &variant.name,
                    fname => &field_name,
                },
            ));
        }
    }

    out.push_str(&template_env::render(
        "flat_enum_from_binding_impl_footer.jinja",
        minijinja::context! {},
    ));
    out
}

pub(super) fn return_type_needs_json(
    ret: &TypeRef,
    extendr_incompatible_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> bool {
    match ret {
        TypeRef::Named(n) => {
            if enum_names.contains(n.as_str()) {
                return true;
            }
            extendr_incompatible_types.contains(n.as_str())
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => {
                if enum_names.contains(n.as_str()) {
                    return true;
                }
                if opaque_types.contains(n.as_str()) {
                    return true;
                }
                extendr_incompatible_types.contains(n.as_str())
            }
            TypeRef::Vec(_) => true,
            _ => false,
        },
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) if enum_names.contains(n.as_str()) => true,
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) && !enum_names.contains(n.as_str()) => true,
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) => {
                    enum_names.contains(n.as_str())
                        || opaque_types.contains(n.as_str())
                        || extendr_incompatible_types.contains(n.as_str())
                }
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// Generate a JSON-bridged `#[extendr]` free function.
///
/// When a function's return type or parameter types cannot be handled by extendr's automatic
/// Robj conversions, this generates a wrapper that:
///   - For incompatible return types (ExtractionResult, Vec<ExtractionResult>, Vec<Vec<f32/f64>>,
///     Option<Enum>): serializes the Rust result to a JSON string via serde_json.
///   - For incompatible parameter types (Vec<Struct>): takes a JSON `String` and deserializes it.
///   - Async functions use the TokioBlockOn pattern (no `async fn`).
pub(super) fn gen_extendr_json_bridged_function(
    func: &FunctionDef,
    mapper: &dyn TypeMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    cfg: &RustBindingConfig,
    extendr_incompatible_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
) -> String {
    use crate::codegen::generators::binding_helpers::gen_call_args_cfg;

    let err_map = ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))";
    let rt_new = format!("tokio::runtime::Runtime::new(){err_map}?");

    let mut sig_params: Vec<String> = Vec::new();
    let mut body_preamble = String::new();

    for param in &func.params {
        let needs_json_vec = match &param.ty {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) => {
                    enum_names.contains(n.as_str())
                        || opaque_types.contains(n.as_str())
                        || extendr_incompatible_types.contains(n.as_str())
                }
                _ => false,
            },
            TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                    TypeRef::Named(n) => {
                        enum_names.contains(n.as_str())
                            || opaque_types.contains(n.as_str())
                            || extendr_incompatible_types.contains(n.as_str())
                    }
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        };
        let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
            if enum_names.contains(n.as_str()))
            || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
        // Non-optional Named struct when cfg.named_non_opaque_params_by_ref=true: use by-ref instead of JSON
        let needs_by_ref_struct = cfg.named_non_opaque_params_by_ref
            && !param.optional
            && matches!(&param.ty, TypeRef::Named(n)
                if !opaque_types.contains(n.as_str())
                    && !enum_names.contains(n.as_str())
                    && !extendr_incompatible_types.contains(n.as_str()));
        // Force JSON-bridge when the type is extendr_incompatible regardless of optional /
        // by-ref config: these types have no `#[extendr] impl` block emitted, so neither
        // `T` nor `&T` has `TryFrom<&Robj>` and the by-ref path produces E0277.
        let is_named_incompatible = matches!(&param.ty, TypeRef::Named(n)
            if extendr_incompatible_types.contains(n.as_str()));
        let needs_json_struct = !needs_json_enum
            && !needs_by_ref_struct
            && (is_named_incompatible
                || (matches!(&param.ty, TypeRef::Named(n)
                    if !opaque_types.contains(n.as_str())
                        && !enum_names.contains(n.as_str())
                        && !extendr_incompatible_types.contains(n.as_str()))
                    || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n)
                        if !opaque_types.contains(n.as_str())
                            && !enum_names.contains(n.as_str())
                            && !extendr_incompatible_types.contains(n.as_str()))))
                    && (param.optional || !cfg.named_non_opaque_params_by_ref));
        if needs_json_vec {
            let (core_ty_path, is_optional) = match &param.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => (format!("{core_import}::{n}"), false),
                    _ => unreachable!(),
                },
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                        TypeRef::Named(n) => (format!("{core_import}::{n}"), true),
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let mut_kw = if param.is_mut { "mut " } else { "" };
            let param_is_optional = param.optional || is_optional;
            if param_is_optional {
                sig_params.push(format!("{}: Option<String>", param.name));
                body_preamble.push_str(&template_env::render(
                    "json_vec_optional_preamble.jinja",
                    minijinja::context! {
                        name => &param.name,
                        ty => &core_ty_path,
                        err_map => &err_map,
                        mut_kw => &mut_kw,
                    },
                ));
                body_preamble.push_str("    ");
            } else {
                sig_params.push(format!("{}: String", param.name));
                body_preamble.push_str(&template_env::render(
                    "json_vec_required_preamble.jinja",
                    minijinja::context! {
                        name => &param.name,
                        ty => &core_ty_path,
                        err_map => &err_map,
                        mut_kw => &mut_kw,
                    },
                ));
                body_preamble.push_str("    ");
            }
        } else if needs_by_ref_struct {
            // Non-optional Named struct with cfg.named_non_opaque_params_by_ref=true.
            // The signature must take `&LocalBinding` (the R wrapper struct) because
            // extendr derives `TryFrom<&Robj>` only for the local wrapper, not for the
            // upstream core type. The downstream `named_let_bindings` loop in this same
            // function emits `let {name}_core: core::T = {name}.clone().into();` so the
            // call site already receives the core ref via `&{name}_core` — no preamble
            // needed here.
            let local_name = match &param.ty {
                TypeRef::Named(n) => n.clone(),
                _ => unreachable!(),
            };
            sig_params.push(format!("{}: &{local_name}", param.name));
        } else if needs_json_struct || needs_json_enum {
            let (core_ty_path, is_optional) = match &param.ty {
                TypeRef::Named(n) => (format!("{core_import}::{n}"), false),
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Named(n) => (format!("{core_import}::{n}"), true),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let mut_kw = if param.is_mut { "mut " } else { "" };
            let param_is_optional = param.optional || is_optional;
            if param_is_optional {
                sig_params.push(format!("{}: Option<String>", param.name));
                body_preamble.push_str(&template_env::render(
                    "json_struct_optional_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            } else {
                sig_params.push(format!("{}: String", param.name));
                body_preamble.push_str(&template_env::render(
                    "json_struct_required_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            }
        } else {
            let ty_str = mapper.map_type(&param.ty);
            let sig_ty = if matches!(&param.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())) {
                if param.optional {
                    format!("extendr_api::Nullable<&{ty_str}>")
                } else {
                    format!("&{ty_str}")
                }
            } else if param.optional {
                format!("Option<{ty_str}>")
            } else {
                ty_str
            };
            sig_params.push(format!("{}: {sig_ty}", param.name));
        }
    }

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    let mut named_let_bindings = String::new();
    for param in &func.params {
        let needs_json = matches!(&param.ty, TypeRef::Vec(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())));
        let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
            if enum_names.contains(n.as_str()))
            || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
        // Mirror the upstream needs_json_struct calculation: extendr_incompatible Named
        // types unconditionally route through JSON because they lack #[extendr] impl blocks
        // and therefore lack the `&T: TryFrom<&Robj>` / `T: TryFrom<Robj>` impls extendr
        // needs. When the JSON branch owns the preamble, we must skip the named_let
        // binding here — otherwise we emit a duplicate (and now-wrong-typed) let.
        let is_named_incompatible = matches!(&param.ty, TypeRef::Named(n)
            if extendr_incompatible_types.contains(n.as_str()));
        let needs_json_struct = !needs_json_enum
            && (is_named_incompatible
                || (matches!(&param.ty, TypeRef::Named(n)
                    if !opaque_types.contains(n.as_str())
                        && !enum_names.contains(n.as_str())
                        && !extendr_incompatible_types.contains(n.as_str()))
                    || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n)
                        if !opaque_types.contains(n.as_str())
                            && !enum_names.contains(n.as_str()))))
                    && (param.optional || !cfg.named_non_opaque_params_by_ref));
        if !needs_json && !needs_json_struct && !needs_json_enum {
            if let TypeRef::Named(n) = &param.ty {
                if !opaque_types.contains(n.as_str()) {
                    if param.optional {
                        named_let_bindings.push_str(&template_env::render(
                            "named_let_optional_binding.jinja",
                            minijinja::context! {
                                name => &param.name,
                                ci => core_import,
                                n => n,
                            },
                        ));
                        named_let_bindings.push_str("    ");
                    } else {
                        named_let_bindings.push_str(&template_env::render(
                            "named_let_required_binding.jinja",
                            minijinja::context! {
                                name => &param.name,
                                ci => core_import,
                                n => n,
                            },
                        ));
                        named_let_bindings.push_str("    ");
                    }
                }
            }
        }
    }

    let final_call_args: Vec<String> = func
        .params
        .iter()
        .map(|param| {
            let needs_json = match &param.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => {
                        enum_names.contains(n.as_str())
                            || opaque_types.contains(n.as_str())
                            || extendr_incompatible_types.contains(n.as_str())
                    }
                    _ => false,
                },
                _ => false,
            };
            let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
                if enum_names.contains(n.as_str()))
                || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
            // Same is_named_incompatible shortcut as the upstream needs_json_struct calc.
            let is_named_incompatible = matches!(&param.ty, TypeRef::Named(n)
                if extendr_incompatible_types.contains(n.as_str()));
            let needs_json_struct = !needs_json_enum
                && (is_named_incompatible
                    || (matches!(&param.ty, TypeRef::Named(n)
                        if !opaque_types.contains(n.as_str())
                            && !enum_names.contains(n.as_str())
                            && !extendr_incompatible_types.contains(n.as_str()))
                        || matches!(&param.ty, TypeRef::Optional(inner)
                        if matches!(inner.as_ref(), TypeRef::Named(n)
                            if !opaque_types.contains(n.as_str())
                                && !enum_names.contains(n.as_str())
                                && !extendr_incompatible_types.contains(n.as_str()))))
                        && (param.optional || !cfg.named_non_opaque_params_by_ref));
            if needs_json {
                if param.optional {
                    format!("{}_core.as_deref().unwrap_or_default()", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else {
                    format!("{}_core.as_slice()", param.name)
                }
            } else if needs_json_struct || needs_json_enum {
                if param.optional && param.is_ref {
                    format!("{}_core.as_ref()", param.name)
                } else if param.optional {
                    format!("{}_core", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else if param.is_ref {
                    format!("&{}_core", param.name)
                } else {
                    format!("{}_core", param.name)
                }
            } else if matches!(&param.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())) {
                // By-ref Named struct (when cfg.named_non_opaque_params_by_ref=true and !param.optional)
                if cfg.named_non_opaque_params_by_ref && !param.optional {
                    // Signature is `&LocalBinding`; named_let_bindings emits
                    // `let {name}_core: core::T = {name}.clone().into();` — pass
                    // `&{name}_core` to the core fn which expects `&core::T`.
                    format!("&{}_core", param.name)
                } else if param.optional {
                    format!("{}_core.as_ref()", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else {
                    format!("{}_core", param.name)
                }
            } else {
                gen_call_args_cfg(
                    std::slice::from_ref(param),
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                )
            }
        })
        .collect();
    let final_call_args_str = final_call_args.join(", ");

    let params_need_json_deserialize = func.params.iter().any(|p| match &p.ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => {
                enum_names.contains(n.as_str())
                    || opaque_types.contains(n.as_str())
                    || extendr_incompatible_types.contains(n.as_str())
            }
            _ => false,
        },
        TypeRef::Named(n) => {
            (enum_names.contains(n.as_str())
                || extendr_incompatible_types.contains(n.as_str())
                || !opaque_types.contains(n.as_str()))
                && (p.optional
                    || !cfg.named_non_opaque_params_by_ref
                    || enum_names.contains(n.as_str())
                    || extendr_incompatible_types.contains(n.as_str()))
        }
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n)
            if (enum_names.contains(n.as_str())
                || extendr_incompatible_types.contains(n.as_str())
                || !opaque_types.contains(n.as_str()))),
        _ => false,
    });
    let effectively_fallible = func.error_type.is_some() || params_need_json_deserialize;

    let (ret_type, result_convert) = match &func.return_type {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if effectively_fallible {
                let ser = format!(
                    "result.map(|v| serde_json::to_string(&v){err_map}).transpose()",
                    err_map = err_map
                );
                ("Result<Option<String>>".to_string(), ser)
            } else {
                let ser = "result.map(|v| serde_json::to_string(&v).expect(\"serialization failed\"))".to_string();
                ("Option<String>".to_string(), ser)
            }
        }
        _ => {
            if effectively_fallible {
                let ser = format!("serde_json::to_string(&result){err_map}");
                ("Result<String>".to_string(), ser)
            } else {
                (
                    "String".to_string(),
                    "serde_json::to_string(&result).expect(\"serialization failed\")".to_string(),
                )
            }
        }
    };

    let binding_conversion: Option<String> = match &func.return_type {
        TypeRef::Named(n) if extendr_incompatible_types.contains(n.as_str()) => {
            Some(format!("let result: {n} = result.into();"))
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if extendr_incompatible_types.contains(n.as_str()) => Some(format!(
                "let result: Vec<{n}> = result.into_iter().map(Into::into).collect();"
            )),
            _ => None,
        },
        _ => None,
    };
    let convert = binding_conversion.as_deref().unwrap_or("");

    let core_call = format!("{core_fn_path}({final_call_args_str})");

    let core_call_with_err = if func.error_type.is_some() {
        format!("{core_call}{err_map}?")
    } else {
        core_call.clone()
    };

    let body = if func.is_async {
        if func.error_type.is_some() {
            format!(
                "{body_preamble}{named_let_bindings}\
                 let rt = {rt_new};\n    \
                 let result = rt.block_on(async {{ {core_call}.await{err_map} }})?;\n    \
                 {convert}\n    \
                 {result_convert}",
                body_preamble = body_preamble,
                named_let_bindings = named_let_bindings,
                rt_new = rt_new,
                core_call = core_call,
                err_map = err_map,
                convert = convert,
                result_convert = result_convert,
            )
        } else {
            format!(
                "{body_preamble}{named_let_bindings}\
                 let rt = {rt_new};\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 {convert}\n    \
                 {result_convert}",
                body_preamble = body_preamble,
                named_let_bindings = named_let_bindings,
                rt_new = rt_new,
                core_call = core_call,
                convert = convert,
                result_convert = result_convert,
            )
        }
    } else {
        format!(
            "{body_preamble}{named_let_bindings}\
             let result = {core_call_with_err};\n    \
             {convert}\n    \
             {result_convert}",
            body_preamble = body_preamble,
            named_let_bindings = named_let_bindings,
            core_call_with_err = core_call_with_err,
            convert = convert,
            result_convert = result_convert,
        )
    };

    let params_str = sig_params.join(", ");
    let allow = if effectively_fallible {
        "#[allow(clippy::missing_errors_doc)]\n"
    } else {
        ""
    };
    format!(
        "{allow}#[extendr]\npub fn {}({params_str}) -> {ret_type} {{\n    {body}\n}}",
        func.name
    )
}
