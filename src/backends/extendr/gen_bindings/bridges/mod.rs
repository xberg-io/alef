use crate::backends::extendr::template_env;
use crate::codegen::generators::RustBindingConfig;
use crate::codegen::generators::trait_bridge::BridgeFieldMatch;
use crate::codegen::naming::wire_variant_value;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, TypeRef};
use ahash::AHashSet;

/// Resolve the fully-qualified core path for an enum.
///
/// Prefer the enum's `rust_path` (e.g. `sample_lib::core::config::ImageOutputFormat`),
/// which is required for types that are NOT re-exported at the crate root — a naive
/// `{core_import}::{name}` only resolves for crate-root re-exports and otherwise produces
/// E0433 "cannot find ImageOutputFormat in the crate root". Falls back to `{core_import}::{name}`
/// when `rust_path` is empty.
fn enum_core_path(enum_def: &EnumDef, core_import: &str) -> String {
    if enum_def.rust_path.is_empty() {
        format!("{core_import}::{}", enum_def.name)
    } else {
        enum_def.rust_path.replace('-', "_")
    }
}

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
/// `default()`, plus a per-variant constructor for each data-carrying struct variant
/// (`EmbeddingModelType$preset(name)`). From/Into impls bridge to the core type via serde round-trip.
pub(super) fn gen_extendr_json_passthrough_enum_struct(
    enum_def: &EnumDef,
    mapper: &dyn TypeMapper,
    core_import: &str,
) -> String {
    let name = &enum_def.name;
    let core_path = enum_core_path(enum_def, core_import);
    let variant_constructors = gen_extendr_enum_variant_constructors(enum_def, mapper, &core_path);
    let variant_constructors_block = if variant_constructors.is_empty() {
        String::new()
    } else {
        format!("\n{}", variant_constructors.join("\n"))
    };
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
    }}{variant_constructors_block}
}}
"#
    )
}

/// True when a per-variant factory parameter of type `ty` can be accepted as a `#[extendr]`
/// function argument. extendr derives `TryFrom<&Robj>` only for `&T` of #[extendr] types (never
/// owned `T`), and has no R-object conversion for `Vec<NamedStruct>`, nested vectors, or maps. A
/// factory that takes such a field *by value* fails to compile under the `#[extendr]` proc-macro
/// with `error[E0277]: ... TryFrom<&Robj> not satisfied`. Variants carrying these fields are skipped
/// from the typed factories; callers still construct them through the enum's `from_json` factory.
fn extendr_factory_param_is_constructible(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(_) | TypeRef::Map(_, _) => false,
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char),
        TypeRef::Optional(inner) => extendr_factory_param_is_constructible(inner),
        _ => true,
    }
}

/// Generate per-variant constructor methods for a JSON-passthrough data enum.
///
/// Each data-carrying struct variant gets a `pub fn _factory_<snake>(<params>) -> <Name>` method
/// that builds the CORE variant directly (`<core_path>::<Variant> { field: <expr> }`) and `.into()`s
/// it into the JSON-passthrough wrapper — the wrapper-convert model, since the binding stores the
/// core value as serde JSON, not the binding-shaped fields. Reuses the shared param / let-binding /
/// call-arg machinery (with the extendr numeric remapping cast) so DTO fields convert via
/// `<field>_core` let bindings and primitives are cast back to the core type.
///
/// Variant selection (skipping unit/tuple/`binding_excluded` variants and yielding to a hand-written
/// `impl` method of the same name) is shared with pyo3/magnus via `collect_variant_constructors`. The
/// Rust fn is `_factory_<snake>`; the R wrapper (emitted in `r_wrappers`) exposes it under the bare
/// snake name so callers write `<Name>$<snake>(...)`. Returns one rendered method per qualifying
/// variant (empty when none qualifies). Variants whose fields cannot cross the extendr input
/// boundary (see `extendr_factory_param_is_constructible`) are skipped.
pub(super) fn gen_extendr_enum_variant_constructors(
    enum_def: &EnumDef,
    mapper: &dyn TypeMapper,
    core_path: &str,
) -> Vec<String> {
    use crate::codegen::generators::{collect_variant_constructors, variant_field_init};
    use crate::codegen::shared::{function_params, is_promoted_optional};

    let name = &enum_def.name;
    let map_fn = |ty: &TypeRef| mapper.map_type(ty);

    collect_variant_constructors(enum_def)
        .iter()
        .filter(|ctor| {
            ctor.params
                .iter()
                .all(|p| extendr_factory_param_is_constructible(&p.ty))
        })
        .map(|ctor| {
            let params_str = function_params(&ctor.params, &map_fn);

            let field_inits: Vec<String> = ctor
                .params
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let expr =
                        variant_field_init(p, is_promoted_optional(&ctor.params, idx), true, true, ctor.boxed[idx]);
                    if expr == p.name {
                        p.name.clone()
                    } else {
                        format!("{}: {expr}", p.name)
                    }
                })
                .collect();

            template_env::render(
                "enum_variant_constructor.rs.jinja",
                minijinja::context! {
                    snake => &ctor.snake_name,
                    params_str => &params_str,
                    name => name,
                    let_lines => Vec::<String>::new(),
                    core_path => core_path,
                    variant => &ctor.variant_name,
                    field_inits => field_inits.join(", "),
                },
            )
            .trim_end()
            .to_string()
        })
        .collect()
}

/// R-facing registrations for the per-variant constructors of a JSON-passthrough data enum:
/// `(r_name, rust_fn_name, param_names)`. Used by `r_wrappers` to bind
/// `<Name>$<snake> <- function(<params>) .Call("wrap__<Name>___factory_<snake>", <params>, ...)`.
pub(super) fn extendr_enum_variant_constructor_registrations(enum_def: &EnumDef) -> Vec<(String, String, Vec<String>)> {
    crate::codegen::generators::collect_variant_constructors(enum_def)
        .into_iter()
        .filter(|ctor| {
            ctor.params
                .iter()
                .all(|p| extendr_factory_param_is_constructible(&p.ty))
        })
        .map(|ctor| {
            let param_names: Vec<String> = ctor.params.iter().map(|p| p.name.clone()).collect();
            (
                ctor.snake_name.clone(),
                format!("_factory_{}", ctor.snake_name),
                param_names,
            )
        })
        .collect()
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
    let disc_ident = crate::core::keywords::rust_raw_ident(discriminator);
    let serde_rename_disc: Option<&str> = if disc_ident != discriminator {
        Some(discriminator)
    } else {
        None
    };
    out.push_str(&template_env::render(
        "flat_enum_discriminator_field.jinja",
        minijinja::context! {
            disc_ident => &disc_ident,
            serde_rename => serde_rename_disc,
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
    let core_path = enum_core_path(enum_def, core_import);
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let disc_ident = crate::core::keywords::rust_raw_ident(discriminator);
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
                    disc_ident => &disc_ident,
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
                    disc_ident => &disc_ident,
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
                    disc_ident => &disc_ident,
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
    let core_path = enum_core_path(enum_def, core_import);
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let disc_ident = crate::core::keywords::rust_raw_ident(discriminator);
    let mut out = String::with_capacity(512);

    out.push_str(&template_env::render(
        "flat_enum_from_binding_impl.jinja",
        minijinja::context! {
            name => name,
            core_path => &core_path,
            disc_ident => &disc_ident,
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

mod json_functions;

pub use json_functions::{gen_extendr_json_bridged_function, return_type_needs_json};
