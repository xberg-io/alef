use super::dto_coercion::{coercible_field_init, coercible_payload};
use crate::codegen::cfg::is_host_owned_rust_path;
use crate::codegen::conversions::{VariantDeclaration, enum_variant_declaration};
use crate::codegen::generators::RustBindingConfig;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

/// Returns true if any variant of the enum has data fields.
/// These enums cannot be represented as flat integer enums in bindings.
pub fn enum_has_data_variants(enum_def: &EnumDef) -> bool {
    enum_def.variants.iter().any(|v| !v.fields.is_empty())
}

/// Returns true if any variant of the enum has a sanitized field.
///
/// A sanitized field means the extractor could not resolve the field's concrete type
/// (e.g. a tuple like `Vec<(String, String)>` that has no direct IR representation).
/// When this is true the `#[new]` constructor that round-trips via serde/JSON cannot
/// be generated, because the Python-dict → JSON → core deserialization path would not
/// produce a valid value for the sanitized field. The forwarding trait impls
/// (`Default`, `Serialize`, `Deserialize`) are still generated unconditionally since
/// the wrapper struct always delegates to the core type.
pub fn enum_has_sanitized_fields(enum_def: &EnumDef) -> bool {
    enum_def.variants.iter().any(|v| v.fields.iter().any(|f| f.sanitized))
}

/// Generate a PyO3 data enum as a `#[pyclass]` struct wrapping the core type.
///
/// Data enums (tagged unions like `AuthConfig`) can't be flat int enums in PyO3.
/// Instead, generate a frozen struct with `inner` that accepts a Python dict,
/// serializes it to JSON, and deserializes into the core Rust type via serde.
///
/// When any variant field is sanitized (its type could not be resolved — e.g. contains
/// `dyn Stream + Send` which is not `Serialize`/`Deserialize`/`Default`), the serde-
/// based `#[new]` constructor is omitted. The type is still useful as a return value
/// from Rust (passed back via From impls). The forwarding impls for Default, Serialize,
/// and Deserialize are always generated regardless of sanitized fields, because the
/// wrapper struct always delegates to the core type which implements those traits.
pub fn gen_pyo3_data_enum(enum_def: &EnumDef, core_import: &str) -> String {
    gen_pyo3_data_enum_with_mapper(enum_def, core_import, None)
}

/// Like `gen_pyo3_data_enum` but with a type mapper for generating per-variant constructors.
///
/// When `mapper` is `Some`, each data-carrying struct variant of the enum gets a
/// `#[staticmethod]` constructor inside the `#[pymethods]` impl block — `Shape.circle(radius=...)`
/// rather than the stringly-typed `Shape(type="circle", ...)` form. The mapper maps each field's
/// type into the binding signature. Without a mapper the constructor section is omitted.
///
/// Delegates to [`gen_pyo3_data_enum_with_coercion`] with no config-DTO coercion (empty set) — the
/// stable entry point for callers that don't classify dataclass-backed types.
pub fn gen_pyo3_data_enum_with_mapper(
    enum_def: &EnumDef,
    core_import: &str,
    mapper: Option<&dyn TypeMapper>,
) -> String {
    gen_pyo3_data_enum_with_coercion(enum_def, core_import, mapper, &AHashSet::new())
}

/// Like [`gen_pyo3_data_enum_with_mapper`] but also threads the set of dataclass-backed config-DTO
/// type names. A variant-constructor payload field whose `Named` type is in `coercible_dto_names`
/// is generated to accept the public wrapper (a `@dataclass`) or a `dict` — coerced into the core
/// type via the [`super::dto_coercion`] runtime helper — instead of demanding the compiled
/// `#[pyclass]` instance. Restores parity with struct-field coercion (`_to_rust_*`) for enum-variant
/// payloads.
pub fn gen_pyo3_data_enum_with_coercion(
    enum_def: &EnumDef,
    core_import: &str,
    mapper: Option<&dyn TypeMapper>,
    coercible_dto_names: &AHashSet<&str>,
) -> String {
    let name = &enum_def.name;
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let is_host_enum = is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let has_sanitized = enum_has_sanitized_fields(enum_def);
    // 1. A variant marked `#[default]` (`is_default = true`) — only emitted with
    //    `#[derive(Default)]`, surfaced in the IR as `EnumVariant::is_default`.
    //    `impl Default for Enum` (no `#[default]` variant). Data enums with
    let has_default = enum_def.has_default || enum_def.variants.iter().any(|v| v.is_default);
    let string_methods_content = crate::codegen::template_env::render(
        "generators/enums/enum_string_methods.jinja",
        minijinja::context! {
            name => name,
            value_expr => "&self.inner",
        },
    );

    let mut variant_accessors = String::new();
    write_pyo3_variant_accessors(&mut variant_accessors, enum_def, &core_path, is_host_enum);

    let mut serde_tag_content = String::new();
    if let Some(tag_field) = &enum_def.serde_tag {
        write_pyo3_serde_tag_getter(&mut serde_tag_content, tag_field);
    }

    let variant_constructors_content = match mapper {
        Some(m) => {
            gen_pyo3_enum_variant_constructors_content(enum_def, &core_path, m, coercible_dto_names, is_host_enum)
        }
        None => String::new(),
    };

    crate::codegen::template_env::render(
        "generators/enums/pyo3_data_enum.jinja",
        minijinja::context! {
            name => name,
            core_path => core_path,
            has_sanitized => has_sanitized,
            has_default => has_default,
            string_methods_content => string_methods_content,
            variant_accessors_content => variant_accessors,
            serde_tag_content => serde_tag_content,
            serde_tag => enum_def.serde_tag,
            variant_constructors_content => variant_constructors_content,
        },
    )
}

/// A data-carrying struct variant prepared for constructor generation.
///
/// Holds exactly the values a backend needs to emit one per-variant constructor. Backend-agnostic:
/// the pyo3 and magnus emitters both consume it. `params` are the variant's named fields turned into
/// `ParamDef`s so the shared param/signature machinery applies unchanged.
pub(crate) struct VariantConstructor<'a> {
    pub(crate) is_tuple: bool,
    /// Rust PascalCase variant name (used in the `<Variant> { .. }` literal).
    pub(crate) variant_name: &'a str,
    /// snake_case constructor name exposed to the host language.
    pub(crate) snake_name: String,
    /// Variant fields modeled as params for the shared signature/conversion machinery.
    pub(crate) params: Vec<crate::core::ir::ParamDef>,
    /// Per-param `is_boxed` flag (parallel to `params`): true when the core variant field is
    /// `Box<T>`/`Option<Box<T>>` for a Named `T`, so the factory boxes the converted value.
    pub(crate) boxed: Vec<bool>,
}

/// Collect the data-carrying struct variants of `enum_def` that need a generated constructor.
///
/// Skips unit variants (no fields), tuple variants (`is_tuple`), and `binding_excluded` variants.
/// Backend-agnostic selection shared by every per-variant-constructor emitter.
///
/// Deliberately does *not* skip a variant whose snake_case name matches an `enum_def.methods`
/// entry: no backend forwards `enum_def.methods` into its generated bindings (that field exists
/// purely for extraction bookkeeping), so suppressing the derived factory on a name collision
/// silently dropped the variant constructor entirely (`ContentPart.text(...)` raised
/// `AttributeError`, and the identical shape hit Elixir's `ContentPart.text/1`). Always emitting
/// the derived factory is a strict improvement: it is reachable, even where its struct-literal
/// signature is less ergonomic than the hand-written inherent method would have been.
pub(crate) fn collect_all_variant_constructors(enum_def: &EnumDef) -> Vec<VariantConstructor<'_>> {
    collect_variant_constructors(enum_def, false)
}

/// Python tuple factories use `from_<variant>` to retain the existing payload getters.
pub(crate) fn collect_pyo3_variant_constructors(enum_def: &EnumDef) -> Vec<VariantConstructor<'_>> {
    collect_variant_constructors(enum_def, true)
}

fn collect_variant_constructors(enum_def: &EnumDef, include_tuples: bool) -> Vec<VariantConstructor<'_>> {
    use crate::codegen::naming::pascal_to_snake;
    use crate::core::ir::ParamDef;

    enum_def
        .variants
        .iter()
        .filter(|v| {
            !v.fields.is_empty()
                && (include_tuples || !v.is_tuple)
                && !v.binding_excluded
                && !v.fields.iter().any(|f| f.sanitized || f.binding_excluded)
        })
        .map(|v| {
            let snake_name = if v.is_tuple {
                format!("from_{}", pascal_to_snake(&v.name))
            } else {
                pascal_to_snake(&v.name)
            };
            let params = v
                .fields
                .iter()
                .enumerate()
                .map(|(index, f)| ParamDef {
                    name: if v.is_tuple {
                        if v.fields.len() == 1 {
                            "value".into()
                        } else {
                            format!("value_{index}")
                        }
                    } else {
                        f.name.clone()
                    },
                    ty: f.ty.clone(),
                    optional: f.optional,
                    default: f.default.clone(),
                    sanitized: f.sanitized,
                    typed_default: f.typed_default.clone(),
                    newtype_wrapper: f.newtype_wrapper.clone(),
                    original_type: f.original_type.clone(),
                    core_wrapper: f.core_wrapper.clone(),
                    ..ParamDef::default()
                })
                .collect();
            let boxed = v.fields.iter().map(|f| f.is_boxed).collect();
            VariantConstructor {
                is_tuple: v.is_tuple,
                variant_name: &v.name,
                snake_name,
                params,
                boxed,
            }
        })
        .collect()
}

/// True when a data-carrying struct variant's per-variant factory constructor may safely
/// reference `core_path::<Variant>` (or an equivalent single-source-of-truth variant name, e.g.
/// Magnus's own wrapper `enum` declared by `gen_enum`) unconditionally, with no compiler-deferred
/// fallback the way a `#[cfg(...)]`-gated match arm's `_ => ...` wildcard has.
///
/// Keeps an ungated variant and any host-owned variant unconditionally -- `enum_variant_declaration`
/// never resolves a host-owned gate to `Drop`, so a host-owned variant's own crate always compiles
/// it in. Drops any FOREIGN-owned variant behind a `#[cfg(...)]` this crate cannot declare as its
/// own Cargo feature, regardless of `configured_features`: an "unproven, might still be reachable"
/// foreign gate is exactly what `enum_variant_declaration` keeps (a declaration line is deferred to
/// the compiler, which CAN express "maybe"), but a whole `#[staticmethod] fn` body that names
/// `core_path::Variant` has no such deferral -- if the dependency compiled the variant out, the
/// reference is a hard `E0599`/`E0433`, not a warning.
///
/// This is the same policy `write_pyo3_variant_accessors` / `gen_pyo3_enum_variant_constructors_content`
/// already apply inline for PyO3's own data-enum accessors and factories; extracted here so other
/// backends whose generated code shape has the identical "no fallback" property (extendr's
/// JSON-passthrough factory, PHP's flat-enum factory) apply the identical rule instead of
/// re-deriving it independently. ~keep
pub(crate) fn variant_constructor_is_reachable(variant: &EnumVariant, is_host_enum: bool) -> bool {
    match variant.cfg.as_deref() {
        None => true,
        Some(_) => is_host_enum,
    }
}

/// Build the struct-literal init expression for one variant field.
///
/// Returns the value placed at `<field>: <expr>` in `<core>::<Variant> { .. }`. The conversion is
/// inlined (e.g. `field.into()`) rather than routed through a typed `let <field>_core: <path> = …`
/// binding, so type inference resolves the target from the variant literal and no core type path
/// has to be named — non-re-exported types (`pkg::enrich::EnrichResult`) work unchanged.
///
/// The conversions mirror the binding→core struct-field rules (`field_conversion_to_core`) but on a
/// bare param rather than a `val.<field>` receiver: `Path` (String→PathBuf), `Json`
/// (String→Value), `Duration` (u64→Duration), `Char`, `Bytes`, and Named/Vec/Map element
/// conversions all run inline.
///
/// `promoted` is true when the binding signature widened a non-optional core field to `Option<T>`
/// because it follows an optional param. Such a param arrives as `Option<T>` but the core field is
/// `T`, so the value is unwrapped (`unwrap_or_default()`) before any element conversion.
///
/// `cast_uints_to_i32` / `cast_large_ints_to_f64` mirror a backend's numeric remapping (extendr maps
/// `u8..=u32`→`i32` and `u64`/`usize`/`isize`/`f32`→`f64`); when set, primitive fields whose binding
/// type was remapped are cast back to the core type (`field as u32`). Backends that do not remap
/// primitives (pyo3) pass `false`.
/// Wrap a `TypeRef::Json` variant-constructor argument's JSON parse so a caller-supplied string
/// that fails to parse emits a `tracing::warn!` before falling back to `Default::default()`,
/// instead of silently becoming `serde_json::Value::Null`. Shared by pyo3 (via this module) and
/// extendr (`variant_field_init` is called directly from `backends::extendr`); the expression
/// shape stays a plain value (no signature change), so both callers keep compiling unchanged. ~keep
fn json_field_parse_or_warn(access: &str, field_name: &str) -> String {
    let context = format!("field = \"{field_name}\"");
    crate::codegen::template_env::render(
        "conversions/sanitized_json_parse_or_warn",
        minijinja::context! {
            access => access,
            context => context,
            message => "constructor argument was not valid JSON; substituting default",
        },
    )
    .trim_end()
    .to_string()
}

pub(crate) fn variant_field_init(
    param: &crate::core::ir::ParamDef,
    promoted: bool,
    cast_uints_to_i32: bool,
    cast_large_ints_to_f64: bool,
    is_boxed: bool,
) -> String {
    use crate::codegen::conversions::helpers::{core_prim_str, needs_f64_cast, needs_i32_cast};
    use crate::core::ir::TypeRef;

    let name = &param.name;

    if let TypeRef::Primitive(prim) = &param.ty {
        let needs_cast =
            (cast_uints_to_i32 && needs_i32_cast(prim)) || (cast_large_ints_to_f64 && needs_f64_cast(prim));
        if needs_cast {
            let core_ty = core_prim_str(prim);
            return if param.optional {
                format!("{name}.map(|v| v as {core_ty})")
            } else if promoted {
                format!("{name}.unwrap_or_default() as {core_ty}")
            } else {
                format!("{name} as {core_ty}")
            };
        }
    }

    if param.optional {
        let inner = match &param.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        return match inner {
            TypeRef::Named(_) if is_boxed => format!("{name}.map(Into::into).map(Box::new)"),
            TypeRef::Named(_) | TypeRef::Path => format!("{name}.map(Into::into)"),
            TypeRef::Json => format!("{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())"),
            TypeRef::Char => format!("{name}.and_then(|s| s.chars().next())"),
            TypeRef::Duration => format!("{name}.map(std::time::Duration::from_millis)"),
            TypeRef::Bytes => format!("{name}.map(|v| v.to_vec().into())"),
            TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                format!("{name}.map(|v| v.into_iter().map(Into::into).collect())")
            }
            TypeRef::Vec(_) => format!("{name}.map(|v| v.into_iter().collect())"),
            _ => name.clone(),
        };
    }

    let base = if promoted {
        format!("{name}.unwrap_or_default()")
    } else {
        name.clone()
    };
    match &param.ty {
        TypeRef::Named(_) if is_boxed => format!("Box::new({base}.into())"),
        TypeRef::Named(_) | TypeRef::Path => format!("{base}.into()"),
        TypeRef::Json => json_field_parse_or_warn(&base, name),
        TypeRef::Char => format!("{base}.chars().next().unwrap_or('*')"),
        TypeRef::Duration => format!("std::time::Duration::from_millis({base})"),
        TypeRef::Bytes => format!("{base}.to_vec().into()"),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("{base}.into_iter().map(Into::into).collect()")
        }
        TypeRef::Vec(_) => format!("{base}.into_iter().collect()"),
        _ => base,
    }
}

/// Generate a `#[staticmethod]` constructor for each data-carrying struct variant of `enum_def`.
///
/// Reuses the shared param machinery (and the `pyo3_factory_method.jinja` template) but builds the
/// core variant struct literal (`Self { inner: <core_path>::<Variant> { field: <expr>, .. } }`)
/// directly via [`variant_field_init`]. Every generated constructor collides with the variant
/// accessor of the same snake_case name, so they always use the `_factory_<name>` Rust ident plus
/// `#[pyo3(name = "<name>")]`.
fn gen_pyo3_enum_variant_constructors_content(
    enum_def: &EnumDef,
    core_path: &str,
    mapper: &dyn TypeMapper,
    coercible_dto_names: &AHashSet<&str>,
    is_host_enum: bool,
) -> String {
    use crate::codegen::shared::{function_params, function_sig_defaults, is_promoted_optional};

    let constructors = collect_pyo3_variant_constructors(enum_def);
    if constructors.is_empty() {
        return String::new();
    }

    // `#[pyclass]`, so it must be coerced rather than required as the compiled instance.
    let is_coercible = |ty: &TypeRef| coercible_payload(ty, coercible_dto_names).is_some();

    let map_fn = |ty: &TypeRef| {
        if is_coercible(ty) {
            "&Bound<'_, pyo3::types::PyAny>".to_string()
        } else {
            mapper.map_type(ty)
        }
    };

    let mut out = String::new();
    for ctor in &constructors {
        // `VariantConstructor` doesn't carry the source variant's cfg -- it's a backend-agnostic
        // struct shared by pyo3, magnus, rustler, php, and extendr's own emitters (see its doc
        // comment) -- so this looks the original `EnumVariant` back up by name rather than
        // widening that shared struct's surface for one backend's cfg policy. A factory
        // constructor is a whole `#[staticmethod] fn`, not a match arm: a host-owned cfg gates
        // the entire function; a variant merged in from a foreign `[[crates.source_crates]]`
        // crate (whose cfg this pyo3 crate cannot declare as a Cargo feature) drops the
        // constructor entirely instead, mirroring `write_pyo3_variant_accessors` above. ~keep
        let source_cfg = enum_def
            .variants
            .iter()
            .find(|v| v.name == ctor.variant_name)
            .and_then(|v| v.cfg.as_deref());
        let ctor_cfg = match source_cfg {
            None => None,
            Some(_) if is_host_enum => source_cfg,
            Some(cfg) => {
                tracing::debug!(
                    enum_name = %enum_def.name,
                    enum_rust_path = %enum_def.rust_path,
                    variant_name = %ctor.variant_name,
                    cfg = cfg,
                    "dropping pyo3 variant-factory constructor for a foreign-crate variant \
                     behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the \
                     constructor is unreachable"
                );
                continue;
            }
        };
        let needs_coercion = ctor.params.iter().any(|p| is_coercible(&p.ty));
        let params_str = function_params(&ctor.params, &map_fn);
        let params_str = if needs_coercion {
            format!("py: Python<'_>, {params_str}")
        } else {
            params_str
        };

        let field_inits: Vec<String> = ctor
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let promoted = is_promoted_optional(&ctor.params, idx);
                let expr = if let Some((dto, shape)) = coercible_payload(&p.ty, coercible_dto_names) {
                    coercible_field_init(&p.name, dto, shape, p.optional, promoted)
                } else {
                    variant_field_init(p, promoted, false, false, ctor.boxed[idx])
                };
                if ctor.is_tuple {
                    expr
                } else {
                    crate::codegen::field_init::struct_field_init(&p.name, &expr)
                }
            })
            .collect();

        let body_lines = vec![
            crate::codegen::template_env::render(
                "generators/enums/pyo3_variant_constructor_body.jinja",
                minijinja::context! {
                    core_path => core_path,
                    variant_name => ctor.variant_name,
                    field_inits => field_inits,
                    is_tuple => ctor.is_tuple,
                },
            )
            .trim_end()
            .to_string(),
        ];

        let rust_fn_name = format!("_factory_{}", ctor.snake_name);

        let has_optional = ctor.params.iter().any(|p| p.optional);
        let signature_defaults = if has_optional {
            function_sig_defaults(&ctor.params)
        } else {
            String::new()
        };

        out.push_str(&crate::codegen::template_env::render(
            "generators/enums/pyo3_factory_method.jinja",
            minijinja::context! {
                doc_lines => Vec::<String>::new(),
                has_pyo3_name => true,
                pyo3_name => ctor.snake_name,
                has_signature => has_optional,
                signature_defaults => signature_defaults,
                rust_fn_name => rust_fn_name,
                params => params_str,
                returns_result => needs_coercion,
                body_lines => body_lines,
                cfg => ctor_cfg,
            },
        ));
        out.push_str("\n\n");
    }

    out.trim_end().to_string()
}

/// Apply a serde `rename_all = "..."` rule to a Rust-style variant name. Returns the
/// transformed wire identifier (`ElementBased` + `snake_case` → `element_based`). An empty
/// rule (no enum-level rename_all attribute) returns the input unchanged so callers can
/// uniformly dedup against `variant.name`.
fn apply_rename_all(name: &str, rule: &str) -> String {
    use heck::{ToKebabCase, ToLowerCamelCase, ToShoutyKebabCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
    match rule {
        "" => name.to_string(),
        "lowercase" => name.to_ascii_lowercase(),
        "UPPERCASE" => name.to_ascii_uppercase(),
        "snake_case" => name.to_snake_case(),
        "kebab-case" => name.to_kebab_case(),
        "camelCase" => name.to_lower_camel_case(),
        "PascalCase" => name.to_upper_camel_case(),
        "SCREAMING_SNAKE_CASE" => name.to_shouty_snake_case(),
        "SCREAMING-KEBAB-CASE" => name.to_shouty_kebab_case(),
        _ => name.to_string(),
    }
}

/// Generate an enum.
///
/// `configured_features` is this binding's own configured feature set (see
/// `ConversionConfig::configured_features`'s doc comment): threaded through to
/// [`enum_variant_declaration`](crate::codegen::conversions::enum_variant_declaration), the same
/// authority every conversion arm for this enum already consults, so a FOREIGN cfg-gated variant
/// this binding's own feature set proves unreachable is never declared here either -- matching
/// `backends::rustler::gen_bindings::types::gen_enum`, which resolved the identical blind spot for
/// Rustler's own NIF declaration. Only the Keep/Drop verdict is used, never the `cfg` a `Keep`
/// carries: like Rustler, a variant this call keeps is always declared unconditionally, with no
/// per-variant `#[cfg(...)]` forwarded onto the declaration -- `enum_variant_declaration` never
/// resolves a host-owned gate to `Drop`, so a host-owned variant is always kept regardless. ~keep
pub fn gen_enum(enum_def: &EnumDef, cfg: &RustBindingConfig, configured_features: Option<&[String]>) -> String {
    let mut derives: Vec<&str> = cfg.enum_derives.to_vec();
    derives.push("Default");
    derives.push("serde::Serialize");
    derives.push("serde::Deserialize");

    // Detect PyO3 context so we can rename all variants via #[pyo3(name = "UPPER_SNAKE_CASE")].
    let is_pyo3 = cfg.enum_attrs.iter().any(|a| a.contains("pyclass"));

    let is_host_enum = is_host_owned_rust_path(cfg.core_import, &enum_def.rust_path);
    let configured_features_set: Option<HashSet<&str>> =
        configured_features.map(|features| features.iter().map(String::as_str).collect());
    let declared: Vec<(usize, &EnumVariant)> = enum_def
        .variants
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            !matches!(
                enum_variant_declaration(v, is_host_enum, configured_features_set.as_ref()),
                VariantDeclaration::Drop
            )
        })
        .collect();

    // Determine which variant carries #[default], among the ones actually declared -- falling
    // back to the first declared variant when none is explicitly marked, or when the marked one
    // was itself dropped as a provably-unreachable foreign variant.
    let default_idx = declared
        .iter()
        .find(|(_, v)| v.is_default)
        .or_else(|| declared.first())
        .map(|(idx, _)| *idx)
        .unwrap_or(0);

    let serde_rename_all = enum_def.serde_rename_all.as_deref().unwrap_or("");
    let variants: Vec<_> = declared
        .iter()
        .map(|(idx, v)| {
            let idx = *idx;
            // In pyo3 context every variant gets #[pyo3(name = "UPPER_SNAKE_CASE")] so the
            let pyo3_name = if is_pyo3 {
                crate::codegen::naming::pascal_to_screaming_snake(&v.name)
            } else {
                String::new()
            };
            let wire_name = v
                .serde_rename
                .clone()
                .unwrap_or_else(|| apply_rename_all(&v.name, serde_rename_all));
            minijinja::context! {
                name => v.name.clone(),
                idx => idx,
                is_default => idx == default_idx,
                has_pyo3_rename => is_pyo3,
                pyo3_name => pyo3_name,
                serde_rename => v.serde_rename.clone().unwrap_or_default(),
                wire_name => wire_name,
            }
        })
        .collect();

    let string_methods = if is_pyo3 {
        crate::codegen::template_env::render(
            "generators/enums/enum_string_methods.jinja",
            minijinja::context! {
                name => enum_def.name,
                value_expr => "self",
            },
        )
    } else {
        String::new()
    };

    crate::codegen::template_env::render(
        "generators/enums/enum_definition.jinja",
        minijinja::context! {
            enum_name => enum_def.name,
            derives => derives.join(", "),
            serde_rename_all => serde_rename_all,
            enum_attrs => cfg.enum_attrs.to_vec(),
            variants => variants,
            is_pyo3 => is_pyo3,
            string_methods => string_methods,
        },
    )
}

/// Resolve a PyO3 getter's Rust method name without emitting raw identifiers Rust rejects.
///
/// `r#self`, `r#Self`, `r#crate`, and `r#super` are rejected by rustc even though those words
/// appear in the general keyword table. A trailing underscore is the only legal Rust spelling
/// for these getter methods; PyO3 then exposes that spelling to Python. All other Rust keywords
/// use the ordinary raw-identifier form (`r#type`, for example). ~keep
fn pyo3_getter_fn_name(name: &str) -> String {
    match name {
        "crate" | "self" | "Self" | "super" => format!("{name}_"),
        _ => crate::core::keywords::rust_raw_ident(name),
    }
}

/// A variant eligible for a typed PyO3 `#[getter]` accessor: a single-field tuple variant
/// wrapping a `TypeRef::Named` type. Shared between the runtime `#[getter]` emitter
/// ([`write_pyo3_variant_accessors`]) and the `.pyi` stub emitter so both derive the accessor
/// set from the identical rule -- a stub declaring a property the pyclass doesn't have, or
/// omitting one it does, is exactly the "stub contradicts runtime" defect this exists to avoid.
///
/// `is_boxed` mirrors the field's `Box<T>`/`Option<Box<T>>` wrapping and only matters to the
/// runtime clone expression; the stub type annotation (`Inner | None`) is identical either way.
pub(crate) struct VariantAccessor<'a> {
    /// snake_case Python-visible getter name (before any `r#` raw-identifier escaping needed to
    /// embed it as a Rust fn name).
    pub(crate) py_name: String,
    pub(crate) variant_pascal: &'a str,
    pub(crate) inner_type_name: &'a str,
    pub(crate) is_boxed: bool,
}

/// Returns the accessor description for `variant` when it qualifies for a typed `#[getter]`,
/// or `None` when it must fall back to the dict-shaped getter (unit variant, multi-field tuple
/// variant, struct variant, or a payload type alef could not resolve to a bare `TypeRef::Named`).
pub(crate) fn variant_accessor(variant: &EnumVariant) -> Option<VariantAccessor<'_>> {
    if variant.fields.len() != 1 {
        return None;
    }
    let field = &variant.fields[0];
    let is_tuple_field = field
        .name
        .strip_prefix('_')
        .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()));
    if !is_tuple_field {
        return None;
    }
    let TypeRef::Named(inner_type_name) = &field.ty else {
        return None;
    };
    Some(VariantAccessor {
        py_name: crate::codegen::naming::pascal_to_snake(&variant.name),
        variant_pascal: &variant.name,
        inner_type_name,
        is_boxed: field.is_boxed,
    })
}

/// Collect the [`variant_accessor`]-eligible variants of a data enum, in declaration order.
pub(crate) fn collect_variant_accessors(enum_def: &EnumDef) -> Vec<VariantAccessor<'_>> {
    enum_def.variants.iter().filter_map(variant_accessor).collect()
}

/// Generate variant accessor properties for a data enum.
/// For single-tuple variants with a Named inner type, returns the typed binding struct directly.
/// For all other variants, returns the variant data as a Python dict, or None if not active.
pub(crate) fn write_pyo3_variant_accessors(out: &mut String, enum_def: &EnumDef, core_path: &str, is_host_enum: bool) {
    for variant in &enum_def.variants {
        let variant_name_lower = crate::codegen::naming::pascal_to_snake(&variant.name);
        let fn_name = pyo3_getter_fn_name(&variant_name_lower);

        if let Some(accessor) = variant_accessor(variant) {
            let inner_type_name = accessor.inner_type_name;
            let variant_pascal = accessor.variant_pascal;
            let clone_expr = if accessor.is_boxed {
                "(**data).clone().into()".to_string()
            } else {
                "data.clone().into()".to_string()
            };
            out.push('\n');
            out.push_str("    #[getter]\n");
            out.push_str(&crate::codegen::template_env::render(
                "generators/enums/getter_accessor.jinja",
                minijinja::context! {
                    fn_name => &fn_name,
                    inner_type_name => inner_type_name,
                },
            ));
            out.push_str("        match &self.inner {\n");
            // A cfg-gated variant's match arm must not reference `core_path::Variant`
            // unconditionally: a host-owned cfg is safe to re-emit verbatim (the crate's own
            // `[features]` table forwards it), but a variant merged in from a foreign
            // `[[crates.source_crates]]` crate carries a cfg this crate never declares as a
            // Cargo feature, so the arm is dropped entirely instead -- mirroring
            // `codegen::conversions::enums::emit_cfg_gated_arm`. The `_ => None` fallback
            // already covers the dropped case. ~keep
            let keep_arm = match variant.cfg.as_deref() {
                None => true,
                Some(_) if is_host_enum => true,
                Some(cfg) => {
                    tracing::debug!(
                        enum_name = %enum_def.name,
                        enum_rust_path = %enum_def.rust_path,
                        variant_name = %variant.name,
                        cfg = cfg,
                        "dropping pyo3 variant-accessor match arm for a foreign-crate variant \
                         behind a #[cfg(...)] this crate cannot declare as a Cargo feature; \
                         the variant is unreachable from this accessor"
                    );
                    false
                }
            };
            if keep_arm {
                out.push_str(&crate::codegen::template_env::render(
                    "generators/enums/match_variant.jinja",
                    minijinja::context! {
                        core_path => &core_path,
                        variant_pascal => variant_pascal,
                        clone_expr => &clone_expr,
                        cfg => variant.cfg.as_deref(),
                    },
                ));
            }
            out.push_str("            _ => None,\n");
            out.push_str("        }\n");
            out.push_str("    }\n");
            continue;
        }

        // A single-field tuple variant marked `#[serde(untagged)]` on itself (not the whole
        // enum) serializes as its bare field value with no discriminant anywhere -- not the
        // `{"<tag>": payload}` / `{"<tag>": <variant>, ...}` shape the JSON-tag-match getter
        // below assumes for every variant of the enum. That mismatch is why
        // `OutputFormat::Custom(String)` always returned `None`: the getter looked for a
        // `"custom"` key that serde never writes. Matching directly on `&self.inner`, like the
        // typed `variant_accessor` path above, needs no tag at all and works for any field type,
        // not only a `TypeRef::Named` payload. ~keep
        if variant.serde_untagged && variant.fields.len() == 1 {
            write_pyo3_untagged_variant_accessor(out, enum_def, variant, core_path, is_host_enum, &fn_name);
            continue;
        }

        out.push('\n');
        out.push_str("    #[getter]\n");
        out.push_str(&crate::codegen::template_env::render(
            "generators/enums/py_dict_getter.jinja",
            minijinja::context! {
                fn_name => &fn_name,
            },
        ));
        out.push_str("        let json = serde_json::to_value(&self.inner)\n");
        out.push_str("            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n");
        let wire_variant = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        let repr = crate::codegen::serde_enum_repr::serde_enum_repr(enum_def);
        if let Some(tag_field) = repr.tag() {
            out.push_str(&crate::codegen::template_env::render(
                "generators/enums/tag_field_check.jinja",
                minijinja::context! { tag_field => tag_field },
            ));
            out.push_str("        let tag_value = json.get(tag_field)\n");
            out.push_str("            .and_then(|v| v.as_str())\n");
            out.push_str("            .unwrap_or(\"\");\n");
            out.push_str(&crate::codegen::template_env::render(
                "generators/enums/variant_tag_match.jinja",
                minijinja::context! { variant_name_lower => &wire_variant },
            ));
            out.push_str("            return Ok(None);\n");
            out.push_str("        }\n");
            match repr.content() {
                // serde's adjacent form keeps the payload under its own key; handing back the
                // whole document would return the tag envelope instead of the payload. ~keep
                Some(content_field) => out.push_str(&crate::codegen::template_env::render(
                    "generators/enums/adjacent_variant_payload.jinja",
                    minijinja::context! { content_field => content_field },
                )),
                // serde's internal form has the payload's fields flat beside the tag, so the
                // document *is* the payload.
                None => out.push_str("        let payload = json;\n"),
            }
        } else {
            out.push_str(&crate::codegen::template_env::render(
                "generators/enums/external_variant_payload.jinja",
                minijinja::context! { wire_variant => &wire_variant },
            ));
        }
        out.push_str("        let json_str = payload.to_string();\n");
        out.push_str("        let json_mod = py.import(\"json\")?;\n");
        out.push_str("        let value = json_mod.call_method1(\"loads\", (&json_str,))?;\n");
        out.push_str("        Ok(Some(value.unbind()))\n");
        out.push_str("    }\n");
    }
}

/// Getter body for a single-field tuple variant carrying its own `#[serde(untagged)]`: match the
/// Rust variant directly (mirroring the typed `variant_accessor` arm) and serialize only that
/// field, instead of parsing a tag out of the whole enum's JSON the way every other variant's
/// getter does -- an untagged variant has no tag to find.
fn write_pyo3_untagged_variant_accessor(
    out: &mut String,
    enum_def: &EnumDef,
    variant: &EnumVariant,
    core_path: &str,
    is_host_enum: bool,
    fn_name: &str,
) {
    out.push('\n');
    out.push_str("    #[getter]\n");
    out.push_str(&crate::codegen::template_env::render(
        "generators/enums/py_dict_getter.jinja",
        minijinja::context! { fn_name => fn_name },
    ));
    out.push_str("        match &self.inner {\n");
    // Same foreign-cfg drop policy as the typed accessor arm above: a host-owned cfg re-emits
    // verbatim, a foreign one this crate cannot declare as a Cargo feature drops the arm and
    // falls through to the `_ => Ok(None)` catch-all. ~keep
    let keep_arm = match variant.cfg.as_deref() {
        None => true,
        Some(_) if is_host_enum => true,
        Some(cfg) => {
            tracing::debug!(
                enum_name = %enum_def.name,
                enum_rust_path = %enum_def.rust_path,
                variant_name = %variant.name,
                cfg = cfg,
                "dropping pyo3 untagged variant-accessor match arm for a foreign-crate variant \
                 behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the variant \
                 is unreachable from this accessor"
            );
            false
        }
    };
    if keep_arm {
        out.push_str(&crate::codegen::template_env::render(
            "generators/enums/untagged_variant_match.jinja",
            minijinja::context! {
                core_path => core_path,
                variant_pascal => &variant.name,
                cfg => variant.cfg.as_deref(),
            },
        ));
    }
    out.push_str("            _ => Ok(None),\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
}

pub(crate) fn write_pyo3_serde_tag_getter(out: &mut String, tag_field: &str) {
    let fn_name = pyo3_getter_fn_name(tag_field);
    out.push('\n');
    out.push_str("    #[getter]\n");
    out.push_str(&crate::codegen::template_env::render(
        "generators/enums/tag_getter_header.jinja",
        minijinja::context! {
            fn_name => &fn_name,
        },
    ));
    out.push_str("        let json = serde_json::to_value(&self.inner)\n");
    out.push_str("            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n");
    out.push_str(&crate::codegen::template_env::render(
        "generators/enums/json_get_field.jinja",
        minijinja::context! {
            tag_field => tag_field,
        },
    ));
    out.push_str("            .and_then(|v| v.as_str())\n");
    out.push_str("            .map(String::from)\n");
    out.push_str(&crate::codegen::template_env::render(
        "generators/enums/json_get_error.jinja",
        minijinja::context! {
            tag_field => tag_field,
        },
    ));
    out.push_str("    }\n");
}

#[cfg(test)]
mod cfg_gate_tests;
#[cfg(test)]
mod declaration_cfg_tests;
#[cfg(test)]
mod tests;
