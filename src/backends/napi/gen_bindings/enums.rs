//! NAPI-RS enum code generation: plain enums and tagged union helpers.

use crate::backends::napi::type_map::NapiMapper;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

pub(super) fn tagged_enum_field_is_tuple(field: &FieldDef) -> bool {
    field
        .name
        .strip_prefix('_')
        .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()))
}

pub(super) fn tagged_enum_field_name(variant: &EnumVariant, field: &FieldDef) -> String {
    if let Some(index) = field
        .name
        .strip_prefix('_')
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
    {
        if variant.fields.len() == 1 {
            let source_name = field
                .serde_rename
                .as_deref()
                .or(variant.serde_rename.as_deref())
                .unwrap_or(&variant.name);
            return crate::codegen::naming::to_python_name(source_name);
        }
        return format!("field_{index}");
    }

    field.name.clone()
}

pub(super) fn tagged_enum_field_js_name(variant: &EnumVariant, field: &FieldDef) -> String {
    if let Some(index) = field
        .name
        .strip_prefix('_')
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
    {
        if variant.fields.len() == 1 {
            return field
                .serde_rename
                .clone()
                .or_else(|| variant.serde_rename.clone())
                .unwrap_or_else(|| crate::codegen::naming::to_node_name(&variant.name));
        }
        return format!("field{index}");
    }

    crate::codegen::naming::to_node_name(&field.name)
}

pub(super) fn tagged_enum_binding_field_name(enum_def: &EnumDef, variant: &EnumVariant, field: &FieldDef) -> String {
    if enum_def.serde_content.is_some() && variant.fields.len() == 1 && tagged_enum_field_is_tuple(field) {
        return crate::codegen::naming::to_python_name(
            enum_def.serde_content.as_deref().expect("adjacent content is present"),
        );
    }
    tagged_enum_field_name(variant, field)
}

pub(crate) fn tagged_enum_binding_field_js_name(enum_def: &EnumDef, variant: &EnumVariant, field: &FieldDef) -> String {
    if enum_def.serde_content.is_some() && variant.fields.len() == 1 && tagged_enum_field_is_tuple(field) {
        return enum_def.serde_content.clone().expect("adjacent content is present");
    }
    tagged_enum_field_js_name(variant, field)
}

pub(crate) fn tagged_enum_discriminant_js_name(enum_def: &EnumDef) -> &str {
    crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def)
}

pub(crate) fn string_enum_variant_js_value(enum_def: &EnumDef, variant_name: &str) -> Option<String> {
    declared_string_enum_variants(enum_def, false, None)?
        .into_iter()
        .find(|(variant, _)| variant.name == variant_name)
        .map(|(_, value)| value)
}

/// Collect synthesized variant-data field names emitted on the binding struct for tagged enums
/// where a variant carries a single-tuple Named field. These are the per-variant optional
/// properties (e.g. `excel: Option<JsExcelMetadata>`) added on top of the discriminator and
/// shared variant fields, enabling direct property access in TypeScript.
pub(super) fn variant_data_field_names(enum_def: &EnumDef) -> Vec<String> {
    let mut names = Vec::new();
    for v in &enum_def.variants {
        if v.fields.len() != 1 {
            continue;
        }
        let field = &v.fields[0];
        if !tagged_enum_field_is_tuple(field) {
            continue;
        }
        if matches!(&field.ty, TypeRef::Named(_)) {
            names.push(tagged_enum_binding_field_name(enum_def, v, field));
        }
    }
    names
}

/// The napi `string_enum` case name for an enum's `#[serde(rename_all)]`, if any.
fn napi_string_enum_case(enum_def: &EnumDef) -> Option<&'static str> {
    enum_def.serde_rename_all.as_deref().and_then(|s| match s {
        "snake_case" => Some("snake_case"),
        "camelCase" => Some("camelCase"),
        "kebab-case" => Some("kebab-case"),
        "SCREAMING_SNAKE_CASE" => Some("UPPER_SNAKE"),
        "lowercase" => Some("lowercase"),
        "UPPERCASE" => Some("UPPERCASE"),
        "PascalCase" => Some("PascalCase"),
        _ => None,
    })
}

/// The variants a `#[napi(string_enum)]` wrapper actually declares, paired with the runtime
/// string value each one accepts, in declaration order.
///
/// `None` when [`gen_enum`] does not emit the enum as a string enum — tagged and untagged data
/// enums become objects and value wrappers instead, and have no set of string literals.
///
/// Mirrors [`gen_enum`] on both axes. The value: `#[napi(value = "...")]` from `#[serde(rename)]`
/// wins per variant, otherwise napi applies the enum-wide case to the variant name. The
/// membership: variants are filtered through `codegen::conversions::enum_variant_declaration`,
/// the SAME authority `gen_enum` consults, so a foreign cfg-gated variant this binding's
/// configured feature set proves unreachable — one `gen_enum` omits from the emitted Rust enum
/// body entirely — is never advertised on any TypeScript surface either. Passing
/// `configured_features: None` reproduces the conservative pre-feature-analysis behavior (keep
/// every variant). ~keep alef's public `index.d.ts` overlay and the `ts_type` attribute both
/// declared such a variant after `gen_enum` learned to drop it, so a consumer could write a
/// literal that type-checked cleanly and could never exist at runtime.
pub(super) fn declared_string_enum_variants<'a>(
    enum_def: &'a EnumDef,
    is_host_enum: bool,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> Option<Vec<(&'a EnumVariant, String)>> {
    // Asks the same `is_tagged_data_enum`/`is_untagged_data_enum` authority `gen_enum` routes
    // through, so a string enum is only claimed here when `gen_enum` actually emits one. (~keep)
    if is_tagged_data_enum(enum_def) || is_untagged_data_enum(enum_def) || enum_def.variants.is_empty() {
        return None;
    }
    let case = napi_string_enum_case(enum_def);
    Some(
        enum_def
            .variants
            .iter()
            .filter(|variant| {
                !matches!(
                    crate::codegen::conversions::enum_variant_declaration(variant, is_host_enum, configured_features),
                    crate::codegen::conversions::VariantDeclaration::Drop
                )
            })
            .map(|variant| {
                let value = match variant.serde_rename.as_deref() {
                    Some(rename) => rename.to_string(),
                    None => apply_napi_case(&variant.name, case),
                };
                (variant, value)
            })
            .collect(),
    )
}

/// Runtime string values a `#[napi(string_enum)]` accepts, in declaration order.
///
/// Thin projection of [`declared_string_enum_variants`] for callers that need only the literals.
pub(super) fn string_enum_js_values(
    enum_def: &EnumDef,
    is_host_enum: bool,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> Option<Vec<String>> {
    declared_string_enum_variants(enum_def, is_host_enum, configured_features)
        .map(|declared| declared.into_iter().map(|(_, value)| value).collect())
}

/// Applies the same case transform `napi-derive-backend` applies to a `#[napi(string_enum)]`
/// variant name, so the value alef believes a variant serializes to never drifts from what
/// napi-rs's own macro actually emits at runtime.
///
/// napi-rs computes this with the `convert_case` crate, not `heck`: the two libraries agree on
/// letter-only identifiers but disagree whenever a variant name has a letter-to-digit boundary
/// (`Bm25` -> heck's `snake_case` gives `"bm25"`, convert_case's gives `"bm_25"`). Using `heck`
/// here silently produced a TypeScript `ts_type` literal the Rust side would then reject at
/// runtime. `convert_case::Casing::to_case` is the exact function napi-derive-backend calls
/// (`napi-derive-backend/src/util.rs::to_case`), including its leading-underscore trim.
///
/// ~keep Verified against the shipped sources, not inferred: `napi-derive`'s `string_enum`
/// branch resolves each variant to `to_case(v.ident.to_string(), case)`
/// (`napi-derive-3.6.3/src/parser/mod.rs`), and `convert_case` 0.11 lists `LowerDigit` among
/// `Boundary::defaults()`, which is what splits `Bm` from `25`. A consumer's checked-in
/// `index.d.ts` may still show the old `heck` spelling — that file is a generated artifact and
/// is stale until the binding is rebuilt, so it is not evidence about current runtime behavior.
/// `napi-derive-backend/src/util.rs::to_case` trims *every* leading underscore with
/// `trim_start_matches('_')`, not just the first — mirror that exactly, since a single
/// `strip_prefix('_')` would diverge on a name like `__Private`.
fn apply_napi_case(name: &str, case: Option<&str>) -> String {
    use convert_case::Casing;
    let Some(case) = case.and_then(napi_convert_case) else {
        return name.to_string();
    };
    name.trim_start_matches('_').to_case(case)
}

fn napi_convert_case(case: &str) -> Option<convert_case::Case<'static>> {
    use convert_case::Case;
    match case {
        "snake_case" => Some(Case::Snake),
        "camelCase" => Some(Case::Camel),
        "kebab-case" => Some(Case::Kebab),
        "UPPER_SNAKE" => Some(Case::UpperSnake),
        "lowercase" => Some(Case::Flat),
        "UPPERCASE" => Some(Case::UpperFlat),
        "PascalCase" => Some(Case::Pascal),
        _ => None,
    }
}

/// Whether this enum's napi/wire shape is an internally-tagged object (`{ type: "...", ... }`)
/// rather than a plain `#[napi(string_enum)]`: either it is explicitly `#[serde(tag = "...")]`,
/// or a variant carries data and the enum is not `#[serde(untagged)]`. Internal tagging always
/// produces an object on the wire (`{"kind":"A"}` for a unit variant), so it must route to the
/// object emitter even when no variant carries fields. A default (externally tagged, no
/// `#[serde(tag/content/untagged)]`) enum that *does* carry a payload variant (e.g.
/// `Custom(String)`) has no `serde_tag` either, but a `#[napi(string_enum)]` can only hold unit
/// variants -- routing it there would silently drop the payload. Route any data-carrying enum
/// through the same tagged-object emitter, defaulting the discriminant field to "type" like the
/// explicitly tagged case already does.
///
/// This is the single authority for that verdict: [`gen_enum`] (the compiled `#[napi]` struct
/// that actually executes at runtime), the binding<->core conversion emitters in `mod.rs`, and
/// `errors::gen_dts` (the declared `.d.ts` shape) all call this instead of re-deriving the
/// condition, so the runtime struct and the TypeScript declaration for the same enum can never
/// disagree about which shape it takes. ~keep
pub(crate) fn is_tagged_data_enum(enum_def: &EnumDef) -> bool {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());
    enum_def.serde_tag.is_some() || (has_data_variants && !enum_def.serde_untagged)
}

/// Whether this enum's wire shape is `#[serde(untagged)]` with at least one data-carrying
/// variant -- routed through `gen_untagged_data_enum_as_value_wrapper` instead of a plain string
/// enum or the tagged-object emitter. Same single-authority relationship as
/// [`is_tagged_data_enum`]. ~keep
pub(crate) fn is_untagged_data_enum(enum_def: &EnumDef) -> bool {
    let has_data_variants = enum_def.variants.iter().any(|v| !v.fields.is_empty());
    enum_def.serde_untagged && has_data_variants
}

pub(super) fn gen_enum(
    enum_def: &EnumDef,
    prefix: &str,
    has_serde: bool,
    core_import: &str,
    configured_features: Option<&std::collections::HashSet<&str>>,
) -> String {
    if is_tagged_data_enum(enum_def) {
        return gen_tagged_enum_as_object(enum_def, prefix, has_serde);
    }

    if is_untagged_data_enum(enum_def) {
        return gen_untagged_data_enum_as_value_wrapper(enum_def, prefix);
    }

    let napi_case = napi_string_enum_case(enum_def);

    let js_name = &enum_def.name;
    let string_enum_attr = match napi_case {
        Some(case) => format!("#[napi(string_enum = \"{case}\", js_name = \"{js_name}\")]"),
        None => format!("#[napi(string_enum, js_name = \"{js_name}\")]"),
    };

    let derives = if has_serde {
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]".to_string()
    } else {
        "#[derive(Clone)]".to_string()
    };
    let mut enum_doc = String::new();
    let sanitized_enum_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
        &enum_def.doc,
        crate::codegen::doc_emission::DocTarget::TsDoc,
    );
    crate::codegen::doc_emission::emit_rustdoc(&mut enum_doc, &sanitized_enum_doc, "");
    let mut lines: Vec<String> = Vec::new();
    if !enum_doc.is_empty() {
        lines.push(enum_doc.trim_end_matches('\n').to_string());
    }
    lines.push(string_enum_attr);
    lines.push(derives);
    lines.push(format!("pub enum {prefix}{} {{", enum_def.name));

    // The SAME authority the conversion arms consult (`codegen::conversions::enum_variant_declaration`)
    // decides which variants this wrapper declares and under what `#[cfg(...)]` guard -- keeping
    // this declaration and the `From` impls' match arms from ever disagreeing about which variants
    // exist. See that function's doc comment for the two alef defects that disagreement caused. ~keep
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let declared_variants: Vec<(&EnumVariant, Option<String>)> = enum_def
        .variants
        .iter()
        .filter_map(|variant| {
            match crate::codegen::conversions::enum_variant_declaration(variant, is_host_enum, configured_features) {
                crate::codegen::conversions::VariantDeclaration::Keep { cfg } => Some((variant, cfg)),
                crate::codegen::conversions::VariantDeclaration::Drop => None,
            }
        })
        .collect();

    for (variant, cfg) in &declared_variants {
        let mut variant_doc = String::new();
        let sanitized_variant_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
            &variant.doc,
            crate::codegen::doc_emission::DocTarget::TsDoc,
        );
        let escaped_variant_doc = sanitized_variant_doc.replace("*/", "* /");
        crate::codegen::doc_emission::emit_rustdoc(&mut variant_doc, &escaped_variant_doc, "    ");
        if !variant_doc.is_empty() {
            lines.push(variant_doc.trim_end_matches('\n').to_string());
        }
        if let Some(cfg) = cfg {
            lines.push(format!("    #[cfg({cfg})]"));
        }
        if let Some(rename) = variant.serde_rename.as_deref() {
            lines.push(format!("    #[napi(value = \"{rename}\")]"));
        }
        lines.push(format!("    {},", variant.name));
    }

    lines.push("}".to_string());

    if !declared_variants.is_empty() {
        let candidates = default_impl_cfg_cascade(&declared_variants);
        lines.push(String::new());
        lines.push(
            crate::backends::napi::template_env::render(
                "enum_default_impl_cascade.jinja",
                minijinja::context! {
                    binding_name => format!("{prefix}{}", enum_def.name),
                    candidates,
                },
            )
            .trim_end()
            .to_string(),
        );
    }

    lines.join("\n")
}

/// Build the mutually-exclusive `#[cfg(...)]` guards for a cascade of single-variant `Default`
/// impls, one candidate per declared variant in declaration order, so that exactly one impl
/// compiles under any feature combination that leaves at least one declared variant enabled.
///
/// Emitting the whole `impl Default` block under only the FIRST declared variant's own `cfg` (the
/// previous approach) is wrong the moment that variant's feature is off but a LATER variant's
/// feature is on: the enum type itself carries no `cfg` of its own (only individual variants do),
/// so it still exists and still needs a `Default` impl, but the impl vanished along with the first
/// variant. Reported against a real consumer: an enum with per-variant feature-gated variants fed
/// a struct's `#[derive(Default)]`, and building with only a later variant's feature enabled left
/// that struct with no working `Default` bound. ~keep
///
/// Each subsequent candidate's guard is `all(<this variant's cfg>, not(any(<all prior
/// variants' cfgs>)))`, so it only "wins" when every earlier-declared alternative is unavailable --
/// deterministically preferring the first-declared variant when more than one is enabled, exactly
/// like the previous single-impl behavior did in the common case where the first variant's feature
/// is on. A declared variant with no `cfg` at all (unconditionally present) always satisfies its
/// own guard once reached, so it terminates the cascade: nothing declared after it can ever be
/// needed as a fallback. ~keep
fn default_impl_cfg_cascade(declared_variants: &[(&EnumVariant, Option<String>)]) -> Vec<minijinja::Value> {
    let mut candidates = Vec::new();
    let mut prior_cfgs: Vec<String> = Vec::new();
    for (variant, cfg) in declared_variants {
        let candidate_cfg = match cfg {
            None if prior_cfgs.is_empty() => None,
            None => Some(format!("not(any({}))", prior_cfgs.join(", "))),
            Some(c) if prior_cfgs.is_empty() => Some(c.clone()),
            Some(c) => Some(format!("all({c}, not(any({})))", prior_cfgs.join(", "))),
        };
        candidates.push(minijinja::context! {
            variant_name => variant.name.clone(),
            cfg => candidate_cfg,
        });
        let is_unconditional = cfg.is_none();
        if let Some(c) = cfg {
            prior_cfgs.push(c.clone());
        }
        if is_unconditional {
            break;
        }
    }
    candidates
}

/// Generate an untagged data enum as a thin wrapper around `serde_json::Value`.
///
/// `#[serde(untagged)]` enums (e.g. `enum Input { Single(String), Multiple(Vec<String>) }`)
/// can't be expressed as a `#[napi(string_enum)]` because that loses the inner data.
/// JS users want to pass either shape directly (`"hi"` or `["a", "b"]`), so we wrap the
/// value through `serde_json::Value` (napi-rs's `serde-json` feature provides FromNapiValue/
/// ToNapiValue for it) and bridge to/from the core enum via serde.
pub(super) fn gen_untagged_data_enum_as_value_wrapper(enum_def: &EnumDef, prefix: &str) -> String {
    let name = format!("{prefix}{}", enum_def.name);
    format!(
        "#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]\n\
         #[serde(transparent)]\n\
         pub struct {name}(pub serde_json::Value);\n\
         \n\
         impl napi::bindgen_prelude::TypeName for {name} {{\n    \
             fn type_name() -> &'static str {{ \"{name}\" }}\n    \
             fn value_type() -> napi::ValueType {{ napi::ValueType::Unknown }}\n\
         }}\n\
         \n\
         impl napi::bindgen_prelude::FromNapiValue for {name} {{\n    \
             unsafe fn from_napi_value(env: napi::sys::napi_env, val: napi::sys::napi_value) -> napi::Result<Self> {{\n        \
                 let v: serde_json::Value = unsafe {{ napi::bindgen_prelude::FromNapiValue::from_napi_value(env, val)? }};\n        \
                 Ok(Self(v))\n    \
             }}\n\
         }}\n\
         \n\
         impl napi::bindgen_prelude::ToNapiValue for {name} {{\n    \
             unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> napi::Result<napi::sys::napi_value> {{\n        \
                 unsafe {{ napi::bindgen_prelude::ToNapiValue::to_napi_value(env, val.0) }}\n    \
             }}\n\
         }}\n\
         \n\
         impl napi::bindgen_prelude::ValidateNapiValue for {name} {{}}\n"
    )
}

/// Generate a tagged enum as a flattened `#[napi(object)]` struct.
/// E.g. `AuthConfig { Basic { username, password }, Bearer { token } }` becomes:
/// ```rust,ignore
/// #[napi(object)]
/// struct JsAuthConfig {
///     #[napi(js_name = "type")]
///     pub type_tag: String,
///     pub username: Option<String>,
///     pub password: Option<String>,
///     pub token: Option<String>,
/// }
/// ```
///
/// The discriminant field's TypeScript name (`js_name`) is the Rust `#[serde(tag = "...")]`
/// value verbatim, or `"type"` when the enum has no explicit tag -- see `tag_field` below. The
/// Rust field is always named `{tag_field}_tag` (`type_tag` in the default case above, `role_tag`
/// for `#[serde(tag = "role")]`) to avoid colliding with a same-named data field.
///
/// For tagged enums where every non-empty variant is a single-tuple field with a Named type
/// (e.g. `FormatMetadata`), a `#[napi]` impl block is additionally emitted with per-variant
/// getter methods, enabling `result.metadata.format.excel.sheetCount`-style access.
pub(super) fn gen_tagged_enum_as_object(enum_def: &EnumDef, prefix: &str, has_serde: bool) -> String {
    use crate::codegen::type_mapper::TypeMapper;
    let mapper = NapiMapper::new(prefix.to_string());

    let tag_field = tagged_enum_discriminant_js_name(enum_def);
    let ts_discriminant = tag_field;

    let derive = if has_serde {
        "#[derive(Clone, serde::Serialize, serde::Deserialize)]"
    } else {
        "#[derive(Clone)]"
    };
    let js_name = &enum_def.name;
    let mut lines: Vec<String> = Vec::new();
    let mut enum_doc = String::new();
    let sanitized_enum_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
        &enum_def.doc,
        crate::codegen::doc_emission::DocTarget::TsDoc,
    );
    crate::codegen::doc_emission::emit_rustdoc(&mut enum_doc, &sanitized_enum_doc, "");
    if !enum_doc.is_empty() {
        lines.push(enum_doc.trim_end_matches('\n').to_string());
    }
    lines.push(derive.to_string());
    lines.push(format!("#[napi(object, js_name = \"{js_name}\")]"));
    lines.push(format!("pub struct {prefix}{} {{", enum_def.name));
    lines.push(format!("    #[napi(js_name = \"{ts_discriminant}\")]"));
    // serde will serialize using the Rust field name unless #[serde(rename)] is set.
    if has_serde {
        lines.push(format!("    #[serde(rename = \"{ts_discriminant}\")]"));
    }
    lines.push(format!("    pub {tag_field}_tag: String,"));

    let mixed_named_fields = tagged_enum_mixed_named_fields(enum_def);

    let mut seen_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if tagged_enum_field_is_tuple(field) && matches!(&field.ty, TypeRef::Named(_)) {
                continue;
            }
            let field_name = tagged_enum_binding_field_name(enum_def, variant, field);
            if seen_fields.insert(field_name.clone()) {
                let field_type = if (field.sanitized || mixed_named_fields.contains(&field_name))
                    && matches!(&field.ty, TypeRef::Named(_))
                {
                    "String".to_string()
                } else {
                    mapper.map_type(&field.ty).to_string()
                };
                let js_name = tagged_enum_binding_field_js_name(enum_def, variant, field);
                if js_name != field_name {
                    lines.push(format!("    #[napi(js_name = \"{js_name}\")]"));
                    // When js_name differs from field_name, add #[serde(rename)] for serialization
                    if has_serde {
                        lines.push(format!("    #[serde(rename = \"{js_name}\")]"));
                    }
                }
                lines.push(format!("    pub {field_name}: Option<{field_type}>,"));
            }
        }
    }

    enum_def.variants.iter().for_each(|v| {
        if v.fields.len() != 1 {
            return;
        }
        let field = &v.fields[0];
        if !tagged_enum_field_is_tuple(field) {
            return;
        }
        if let TypeRef::Named(inner_type_name) = &field.ty {
            let field_name = tagged_enum_binding_field_name(enum_def, v, field);
            let binding_type = format!("{prefix}{inner_type_name}");
            let js_name = tagged_enum_binding_field_js_name(enum_def, v, field);
            if js_name != field_name {
                lines.push(format!("    #[napi(js_name = \"{js_name}\")]"));
                // When js_name differs from field_name, add #[serde(rename)] for serialization
                if has_serde {
                    lines.push(format!("    #[serde(rename = \"{js_name}\")]"));
                }
            }
            lines.push(format!("    pub {field_name}: Option<{binding_type}>,"));
        }
    });

    lines.push("}".to_string());

    let synth_fields = variant_data_field_names(enum_def);
    let default_inits: Vec<String> = seen_fields
        .iter()
        .cloned()
        .chain(synth_fields.iter().cloned())
        .map(|f| format!("{f}: None"))
        .collect();
    // The tag field must default to a REAL variant's wire value -- an empty string is not a
    // valid discriminant for any variant, so `Default::default()` on this type would produce a
    // value nothing can deserialize. Prefer the `#[default]`-marked variant, falling back to the
    // first declared variant, exactly like the flat string-enum cascade
    // (`default_impl_cfg_cascade`) and the wasm backend's tagged/plain enum `Default` impl do. ~keep
    let default_variant = enum_def
        .variants
        .iter()
        .find(|variant| variant.is_default)
        .or_else(|| enum_def.variants.first());
    let default_tag_wire_value = default_variant
        .map(|variant| {
            crate::codegen::naming::wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            )
        })
        .unwrap_or_default();
    lines.push(String::new());
    lines.push(format!("impl Default for {prefix}{} {{", enum_def.name));
    lines.push(format!(
        "    fn default() -> Self {{ Self {{ {tag_field}_tag: \"{default_tag_wire_value}\".to_string(), {} }} }}",
        default_inits.join(", ")
    ));
    lines.push("}".to_string());

    // #[napi] impl block with per-variant getters so callers can do `.excel.sheetCount` etc.
    let _tuple_named_variants: Vec<(&crate::core::ir::EnumVariant, &str)> = enum_def
        .variants
        .iter()
        .filter_map(|v| {
            if v.fields.len() != 1 {
                return None;
            }
            let field = &v.fields[0];
            let is_tuple = field
                .name
                .strip_prefix('_')
                .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()));
            if !is_tuple {
                return None;
            }
            if let TypeRef::Named(inner_type_name) = &field.ty {
                Some((v, inner_type_name.as_str()))
            } else {
                None
            }
        })
        .collect();

    if enum_def.serde_content.is_some() {
        // Total distinct fields on the binding struct: the tag plus every shared/synthesized
        // data field. A variant constructor only needs `..Default::default()` when it leaves at
        // least one of those fields unspecified — otherwise clippy::needless_update fires because
        // every field was already given a value.
        let total_field_count = 1 + seen_fields.len() + synth_fields.iter().collect::<ahash::AHashSet<_>>().len();
        let variants: Vec<minijinja::Value> = enum_def
            .variants
            .iter()
            .map(|variant| {
                let wire_value = crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    enum_def.serde_rename_all.as_deref(),
                );
                let payload_type = variant
                    .fields
                    .first()
                    .map(|field| mapper.map_type(&field.ty).to_string());
                let has_payload = payload_type.is_some();
                let rust_name = crate::codegen::naming::internal_rust_identifier(&format!(
                    "{}_{}",
                    crate::codegen::naming::pascal_to_snake(&enum_def.name),
                    crate::codegen::naming::to_python_name(&wire_value),
                ));
                let fields_set = if has_payload { 2 } else { 1 };
                minijinja::context! {
                    variant_name => variant.name.clone(),
                    rust_name,
                    wire_value,
                    payload_type,
                    has_payload,
                    needs_default_spread => fields_set < total_field_count,
                }
            })
            .collect();
        lines.push(String::new());
        lines.push(
            crate::backends::napi::template_env::render(
                "adjacent_enum_namespace.rs.jinja",
                minijinja::context! {
                    enum_name => enum_def.name.clone(),
                    binding_name => format!("{prefix}{}", enum_def.name),
                    tag_field => format!("{tag_field}_tag"),
                    content_field => crate::codegen::naming::to_python_name(
                        enum_def.serde_content.as_deref().expect("adjacent content is present"),
                    ),
                    variants,
                },
            )
            .trim_end()
            .to_string(),
        );
    }

    lines.join("\n")
}

/// Generate a free function binding.
pub(super) fn tagged_enum_mixed_named_fields(enum_def: &EnumDef) -> ahash::AHashSet<String> {
    use crate::core::ir::TypeRef;
    let mut field_types: std::collections::HashMap<&str, ahash::AHashSet<&str>> = std::collections::HashMap::new();

    for variant in &enum_def.variants {
        for field in &variant.fields {
            if field.sanitized {
                continue;
            }
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(&field.name).or_default().insert(n.as_str());
            }
        }
    }

    field_types
        .into_iter()
        .filter(|(_, types)| types.len() > 1)
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Determine which Named fields in a tagged enum use binding structs (Into conversion)
/// vs serde JSON String flattening. A field uses a binding struct only if:
/// 1. The field name maps to a single Named type across all variants
/// 2. That Named type has a binding struct (in struct_names)
/// 3. The field is not sanitized
pub(super) fn tagged_enum_binding_struct_fields<'a>(
    enum_def: &'a EnumDef,
    struct_names: &ahash::AHashSet<String>,
) -> ahash::AHashSet<&'a str> {
    use crate::core::ir::TypeRef;
    let mut field_types: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    let mut sanitized_fields: ahash::AHashSet<&str> = ahash::AHashSet::new();

    for variant in &enum_def.variants {
        for field in &variant.fields {
            if field.sanitized {
                sanitized_fields.insert(&field.name);
            }
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(&field.name).or_default().push(n);
            }
        }
    }

    let mut result = ahash::AHashSet::new();
    for (field_name, types) in &field_types {
        if sanitized_fields.contains(field_name) {
            continue;
        }
        if types.iter().all(|t| *t == types[0]) && struct_names.contains(types[0]) {
            result.insert(*field_name);
        }
    }
    result
}

#[cfg(test)]
#[allow(clippy::print_stderr)] // test-only debug output ~keep
mod tests;

#[cfg(test)]
mod default_impl_cfg_tests;

#[cfg(test)]
mod dts_shape_parity_tests;
