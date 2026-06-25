use crate::codegen::generators::RustBindingConfig;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, TypeRef};

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
fn enum_has_sanitized_fields(enum_def: &EnumDef) -> bool {
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
pub fn gen_pyo3_data_enum_with_mapper(
    enum_def: &EnumDef,
    core_import: &str,
    mapper: Option<&dyn TypeMapper>,
) -> String {
    let name = &enum_def.name;
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let has_sanitized = enum_has_sanitized_fields(enum_def);
    // A delegating `impl Default` (`Self { inner: Default::default() }`) only compiles when the
    // CORE enum implements `Default`. Two signals indicate this:
    // 1. A variant marked `#[default]` (`is_default = true`) — only emitted with
    //    `#[derive(Default)]`, surfaced in the IR as `EnumVariant::is_default`.
    // 2. `enum_def.has_default = true` — set when the extractor finds a manual
    //    `impl Default for Enum` (no `#[default]` variant). Data enums with
    //    `impl Default { Self::Custom(String::new()) }` fall into this category.
    // When neither signal is present, the core type has no `Default` and emitting the
    // wrapper `Default` would produce `error[E0277]: the trait bound
    // `core::Type: std::default::Default` is not satisfied`.
    let has_default = enum_def.has_default || enum_def.variants.iter().any(|v| v.is_default);
    let string_methods_content = crate::codegen::template_env::render(
        "generators/enums/enum_string_methods.jinja",
        minijinja::context! {
            name => name,
            value_expr => "&self.inner",
        },
    );

    let mut variant_accessors = String::new();
    write_pyo3_variant_accessors(&mut variant_accessors, enum_def, &core_path);

    let mut serde_tag_content = String::new();
    if let Some(tag_field) = &enum_def.serde_tag {
        write_pyo3_serde_tag_getter(&mut serde_tag_content, tag_field);
    }

    // Generate a per-variant constructor for each data-carrying struct variant when a mapper
    // is provided (the mapper maps field types into the binding signature).
    let variant_constructors_content = match mapper {
        Some(m) => gen_pyo3_enum_variant_constructors_content(enum_def, &core_path, m),
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
    /// Rust PascalCase variant name (used in the `<Variant> { .. }` literal).
    pub(crate) variant_name: &'a str,
    /// snake_case constructor name exposed to the host language.
    pub(crate) snake_name: String,
    /// Variant fields modeled as params for the shared signature/conversion machinery.
    pub(crate) params: Vec<crate::core::ir::ParamDef>,
}

/// Collect the data-carrying struct variants of `enum_def` that need a generated constructor.
///
/// Skips unit variants (no fields), tuple variants (`is_tuple`), and `binding_excluded` variants.
/// A variant whose snake_case name matches a hand-written `enum_def.methods` entry is skipped too:
/// the consumer-authored method wins. Backend-agnostic selection shared by the pyo3 and magnus
/// per-variant-constructor emitters.
pub(crate) fn collect_variant_constructors(enum_def: &EnumDef) -> Vec<VariantConstructor<'_>> {
    use crate::codegen::naming::pascal_to_snake;
    use crate::core::ir::ParamDef;

    // Hand-written associated functions suppress the generated constructor of the same name.
    let method_names: ahash::AHashSet<&str> = enum_def
        .methods
        .iter()
        .filter(|m| !m.binding_excluded)
        .map(|m| m.name.as_str())
        .collect();

    enum_def
        .variants
        .iter()
        // Skip variants that cannot be constructed from binding-side values: a `sanitized` field
        // has no resolvable type (e.g. `[(u32, u32); 4]` -> String) and a `binding_excluded` field
        // is hidden from the binding surface entirely, so the core variant cannot be built.
        .filter(|v| {
            !v.fields.is_empty()
                && !v.is_tuple
                && !v.binding_excluded
                && !v.fields.iter().any(|f| f.sanitized || f.binding_excluded)
        })
        .filter_map(|v| {
            let snake_name = pascal_to_snake(&v.name);
            if method_names.contains(snake_name.as_str()) {
                return None;
            }
            let params = v
                .fields
                .iter()
                .map(|f| ParamDef {
                    name: f.name.clone(),
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
            Some(VariantConstructor {
                variant_name: &v.name,
                snake_name,
                params,
            })
        })
        .collect()
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
/// `promoted` is true when the pyo3 signature widened a non-optional core field to `Option<T>`
/// because it follows an optional param. Such a param arrives as `Option<T>` but the core field is
/// `T`, so the value is unwrapped (`unwrap_or_default()`) before any element conversion.
fn variant_field_init(param: &crate::core::ir::ParamDef, promoted: bool) -> String {
    use crate::core::ir::TypeRef;

    let name = &param.name;

    // Genuinely-optional core field (`Option<T>`): convert through the Option, leaving it intact.
    if param.optional {
        let inner = match &param.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        return match inner {
            TypeRef::Named(_) | TypeRef::Path => format!("{name}.map(Into::into)"),
            TypeRef::Json => format!("{name}.as_ref().and_then(|s| serde_json::from_str(s).ok())"),
            TypeRef::Char => format!("{name}.and_then(|s| s.chars().next())"),
            TypeRef::Duration => format!("{name}.map(std::time::Duration::from_millis)"),
            TypeRef::Bytes => format!("{name}.map(|v| v.to_vec().into())"),
            TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Named(_)) => {
                format!("{name}.map(|v| v.into_iter().map(Into::into).collect())")
            }
            TypeRef::Vec(_) => format!("{name}.map(|v| v.into_iter().collect())"),
            // String / primitive / Map: the binding `Option<T>` already matches the core field.
            _ => name.clone(),
        };
    }

    // Core field is `T`. A promoted param arrives as `Option<T>`; unwrap to the field default first,
    // then apply the same per-type conversion to the resulting owned value.
    let base = if promoted {
        format!("{name}.unwrap_or_default()")
    } else {
        name.clone()
    };
    match &param.ty {
        TypeRef::Named(_) | TypeRef::Path => format!("{base}.into()"),
        TypeRef::Json => format!("serde_json::from_str(&{base}).unwrap_or_default()"),
        TypeRef::Char => format!("{base}.chars().next().unwrap_or('*')"),
        TypeRef::Duration => format!("std::time::Duration::from_millis({base})"),
        TypeRef::Bytes => format!("{base}.to_vec().into()"),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("{base}.into_iter().map(Into::into).collect()")
        }
        TypeRef::Vec(_) => format!("{base}.into_iter().collect()"),
        // String / primitive / Map: pass the (possibly unwrapped) value through unchanged.
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
fn gen_pyo3_enum_variant_constructors_content(enum_def: &EnumDef, core_path: &str, mapper: &dyn TypeMapper) -> String {
    use crate::codegen::shared::{function_params, function_sig_defaults, is_promoted_optional};

    let constructors = collect_variant_constructors(enum_def);
    if constructors.is_empty() {
        return String::new();
    }

    let map_fn = |ty: &TypeRef| mapper.map_type(ty);

    let mut out = String::new();
    for ctor in &constructors {
        let params_str = function_params(&ctor.params, &map_fn);

        // Build each `field: <expr>` init inline. `field: field` collapses to the shorthand `field`
        // for an unchanged passthrough.
        let field_inits: Vec<String> = ctor
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let expr = variant_field_init(p, is_promoted_optional(&ctor.params, idx));
                if expr == p.name {
                    p.name.clone()
                } else {
                    format!("{}: {expr}", p.name)
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
                },
            )
            .trim_end()
            .to_string(),
        ];

        // Always collides with the variant accessor of the same name → `_factory_<name>`.
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
                body_lines => body_lines,
            },
        ));
        out.push_str("\n\n");
    }

    out.trim_end().to_string()
}

/// Convert a Rust PascalCase variant name to `UPPER_SNAKE_CASE` for PyO3 `#[pyo3(name = "...")]`.
///
/// Handles acronym-style names where 2+ leading uppercase characters are followed only by
/// lowercase letters (e.g. `RDFa` → `RDFA` instead of `RD_FA`). For Python-keyword variants
/// whose Rust identifier was appended with `_` (e.g. `None_`), the screaming form preserves
/// the trailing underscore (`NONE_`) so `setattr`-based aliases in `options.py` continue to
/// work correctly.
fn to_pyo3_screaming(name: &str) -> String {
    use heck::ToShoutySnakeCase;
    let chars: Vec<char> = name.chars().collect();
    let upper_prefix_len = chars.iter().take_while(|c| c.is_uppercase()).count();
    // Acronym: 2+ leading uppercase chars with only lowercase (or empty) remainder
    if upper_prefix_len >= 2 && chars[upper_prefix_len..].iter().all(|c| c.is_lowercase() || *c == '_') {
        name.to_ascii_uppercase()
    } else {
        name.to_shouty_snake_case()
    }
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
        // Unknown rule: pass through; this matches serde's tolerant behavior.
        _ => name.to_string(),
    }
}

/// Generate an enum.
pub fn gen_enum(enum_def: &EnumDef, cfg: &RustBindingConfig) -> String {
    // All enums are generated as unit-variant-only in the binding layer.
    // Data variants are flattened to unit variants; the From/Into conversions
    // handle the lossy mapping (discarding / providing defaults for field data).
    let mut derives: Vec<&str> = cfg.enum_derives.to_vec();
    // Binding enums always derive Default, Serialize, and Deserialize.
    // Default: enables using unwrap_or_default() in constructors.
    // Serialize/Deserialize: required for FFI/type conversion across binding boundaries.
    derives.push("Default");
    derives.push("serde::Serialize");
    derives.push("serde::Deserialize");

    // Detect PyO3 context so we can rename all variants via #[pyo3(name = "UPPER_SNAKE_CASE")].
    // PEP 8 mandates UPPER_SNAKE_CASE for enum members; pyclass variants must carry this
    // rename so Python callers see `BatchStatus.VALIDATING` rather than `BatchStatus.Validating`.
    let is_pyo3 = cfg.enum_attrs.iter().any(|a| a.contains("pyclass"));

    // Determine which variant carries #[default].
    // Prefer the variant that has is_default=true in the source (mirrors the Rust core's
    // #[default] attribute); fall back to the first variant when none is explicitly marked.
    let default_idx = enum_def.variants.iter().position(|v| v.is_default).unwrap_or(0);

    let serde_rename_all = enum_def.serde_rename_all.as_deref().unwrap_or("");
    let variants: Vec<_> = enum_def
        .variants
        .iter()
        .enumerate()
        .map(|(idx, v)| {
            // In pyo3 context every variant gets #[pyo3(name = "UPPER_SNAKE_CASE")] so the
            // Python-exposed name is PEP 8-compliant. For Python-keyword variants the
            // Rust identifier is already escaped (None → None_) so we produce "NONE_" as
            // the screaming form of that escaped name — callers use BatchStatus.NONE.
            let pyo3_name = if is_pyo3 {
                to_pyo3_screaming(&v.name)
            } else {
                String::new()
            };
            // Compute the on-the-wire (serde) name: explicit per-variant rename takes
            // precedence; otherwise derive from the enum-level rename_all rule. This is what
            // FromStr-style constructors must accept in addition to the raw variant name.
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

/// Rust keywords that cannot be used as bare identifiers in function names.
const RUST_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate", "do", "dyn", "else",
    "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in", "let", "loop", "macro", "match", "mod",
    "move", "mut", "override", "priv", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
    "true", "try", "type", "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// Generate variant accessor properties for a data enum.
/// For single-tuple variants with a Named inner type, returns the typed binding struct directly.
/// For all other variants, returns the variant data as a Python dict, or None if not active.
pub(crate) fn write_pyo3_variant_accessors(out: &mut String, enum_def: &EnumDef, core_path: &str) {
    use crate::core::ir::TypeRef;

    for variant in &enum_def.variants {
        let variant_name_lower = crate::codegen::naming::pascal_to_snake(&variant.name);
        let fn_name = if RUST_KEYWORDS.contains(&variant_name_lower.as_str()) {
            format!("r#{}", variant_name_lower)
        } else {
            variant_name_lower.clone()
        };

        if variant.fields.len() == 1 {
            let field = &variant.fields[0];
            let is_tuple_field = field
                .name
                .strip_prefix('_')
                .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit()));
            if is_tuple_field {
                if let TypeRef::Named(inner_type_name) = &field.ty {
                    let variant_pascal = &variant.name;
                    let clone_expr = if field.is_boxed {
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
                    out.push_str(&crate::codegen::template_env::render(
                        "generators/enums/match_variant.jinja",
                        minijinja::context! {
                            core_path => &core_path,
                            variant_pascal => variant_pascal,
                            clone_expr => &clone_expr,
                        },
                    ));
                    out.push_str("            _ => None,\n");
                    out.push_str("        }\n");
                    out.push_str("    }\n");
                    continue;
                }
            }
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
        let tag_field = enum_def.serde_tag.as_deref().unwrap_or("tag");
        out.push_str(&crate::codegen::template_env::render(
            "generators/enums/tag_field_check.jinja",
            minijinja::context! {
                tag_field => tag_field,
            },
        ));
        out.push_str("        let tag_value = json.get(tag_field)\n");
        out.push_str("            .and_then(|v| v.as_str())\n");
        out.push_str("            .unwrap_or(\"\");\n");
        out.push_str(&crate::codegen::template_env::render(
            "generators/enums/variant_tag_match.jinja",
            minijinja::context! {
                variant_name_lower => &variant_name_lower,
            },
        ));
        out.push_str("            return Ok(None);\n");
        out.push_str("        }\n");
        out.push_str("        let json_str = json.to_string();\n");
        out.push_str("        let json_mod = py.import(\"json\")?;\n");
        out.push_str("        let py_dict = json_mod.call_method1(\"loads\", (&json_str,))?.cast_into::<pyo3::types::PyDict>()?;\n");
        out.push_str("        Ok(Some(py_dict.unbind()))\n");
        out.push_str("    }\n");
    }
}

pub(crate) fn write_pyo3_serde_tag_getter(out: &mut String, tag_field: &str) {
    let fn_name = if RUST_KEYWORDS.contains(&tag_field) {
        format!("r#{tag_field}")
    } else {
        tag_field.to_string()
    };
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
mod tests {
    use super::*;
    use crate::codegen::generators::AsyncPattern;
    use crate::core::ir::{CoreWrapper, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

    fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_tuple: false,
            originally_had_data_fields: false,
            cfg: None,
            version: Default::default(),
        }
    }

    fn field(name: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }

    fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("crate::{name}"),
            original_rust_path: String::new(),
            variants,
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }
    }

    #[test]
    fn gen_pyo3_data_enum_emits_string_methods() {
        let generated = gen_pyo3_data_enum(
            &enum_def("StructureKind", vec![variant("Other", vec![field("value")])]),
            "core",
        );

        assert!(
            generated.contains("fn __str__(&self) -> PyResult<String>"),
            "{generated}"
        );
        assert!(generated.contains("serde_json::to_value(&self.inner)"), "{generated}");
        assert!(
            generated.contains("fn __repr__(&self) -> PyResult<String>"),
            "{generated}"
        );
    }

    #[test]
    fn gen_pyo3_data_enum_emits_default_when_core_derives_default() {
        // A data enum whose core type derives `Default` is surfaced as a variant marked
        // `#[default]` (`is_default = true`). The wrapper must keep its delegating `Default`.
        let mut default_variant = variant("Pending", vec![]);
        default_variant.is_default = true;
        let generated = gen_pyo3_data_enum(
            &enum_def(
                "EnrichStatus",
                vec![default_variant, variant("Done", vec![field("value")])],
            ),
            "core",
        );

        assert!(
            generated.contains("impl Default for EnrichStatus"),
            "expected delegating Default impl when a variant is #[default]: {generated}"
        );
        assert!(generated.contains("Self { inner: Default::default() }"), "{generated}");
    }

    #[test]
    fn gen_pyo3_data_enum_emits_default_when_core_has_manual_default() {
        let mut enum_def = enum_def(
            "ClassificationMode",
            vec![variant("Known", vec![]), variant("Custom", vec![field("value")])],
        );
        enum_def.has_default = true;

        let generated = gen_pyo3_data_enum(&enum_def, "core");

        assert!(
            generated.contains("impl Default for ClassificationMode"),
            "expected delegating Default impl when the core enum has a manual Default impl: {generated}"
        );
        assert!(generated.contains("Self { inner: Default::default() }"), "{generated}");
    }

    #[test]
    fn gen_pyo3_data_enum_omits_default_when_core_lacks_default() {
        // No variant is marked `#[default]`, so the core enum does NOT implement `Default`.
        // Emitting a delegating `Default` would fail with E0277 on the core type, so the
        // wrapper `Default` impl must be omitted entirely.
        let generated = gen_pyo3_data_enum(
            &enum_def(
                "ChunkingReason",
                vec![variant("TooLong", vec![field("limit")]), variant("Forced", vec![])],
            ),
            "core",
        );

        assert!(
            !generated.contains("impl Default for ChunkingReason"),
            "expected no Default impl when no variant is #[default]: {generated}"
        );
        assert!(
            !generated.contains("inner: Default::default()"),
            "expected no inner: Default::default() when core lacks Default: {generated}"
        );
    }

    #[test]
    fn gen_pyo3_data_enum_wraps_string_for_internally_tagged_enum() {
        // For an internally-tagged enum (`#[serde(tag = "...")]`), serde cannot deserialize a
        // bare JSON string into the enum. The `__new__` string branch must wrap it as
        // `{"<tag>": s}` so serde can resolve the variant.
        let mut def = enum_def(
            "ImageOutputFormat",
            vec![variant("Png", vec![]), variant("Jpeg", vec![field("quality")])],
        );
        def.serde_tag = Some("type".to_string());

        let generated = gen_pyo3_data_enum(&def, "core");

        assert!(
            generated.contains(r#"serde_json::to_string(&serde_json::json!({ "type": s }))"#),
            "expected tagged string wrap for internally-tagged enum: {generated}"
        );
        assert!(
            !generated.contains("serde_json::to_string(&s)"),
            "internally-tagged enum must not emit the bare-string path: {generated}"
        );
    }

    #[test]
    fn gen_pyo3_data_enum_keeps_bare_string_for_externally_tagged_enum() {
        // An externally-tagged enum (no `#[serde(tag)]`) accepts a bare string for unit variants,
        // so the string branch must keep the existing `to_string(&s)` behavior.
        let generated = gen_pyo3_data_enum(
            &enum_def("StructureKind", vec![variant("Other", vec![field("value")])]),
            "core",
        );

        assert!(
            generated.contains("serde_json::to_string(&s)"),
            "expected bare-string path for externally-tagged enum: {generated}"
        );
        assert!(
            !generated.contains("serde_json::json!({"),
            "externally-tagged enum must not wrap the string in a tag object: {generated}"
        );
    }

    fn typed_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef { ty, ..field(name) }
    }

    fn static_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            is_static: true,
            ..Default::default()
        }
    }

    #[test]
    fn variant_constructors_emit_factory_per_struct_variant() {
        use crate::codegen::type_mapper::IdentityMapper;
        // `Shape` with two struct variants → one `#[staticmethod]` constructor each.
        let mut def = enum_def(
            "Shape",
            vec![
                variant(
                    "Circle",
                    vec![typed_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
                ),
                variant(
                    "Rect",
                    vec![
                        typed_field("width", TypeRef::Primitive(PrimitiveType::F64)),
                        typed_field("height", TypeRef::Primitive(PrimitiveType::F64)),
                    ],
                ),
            ],
        );
        def.serde_tag = Some("type".to_string());

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        // Constructors always collide with the variant accessor of the same name, so they use
        // the `_factory_<name>` Rust ident plus `#[pyo3(name = "<name>")]`.
        assert!(generated.contains(r#"#[pyo3(name = "circle")]"#), "{generated}");
        assert!(
            generated.contains("pub fn _factory_circle(radius: f64) -> Self"),
            "{generated}"
        );
        assert!(
            generated.contains("Self { inner: crate::Shape::Circle { radius } }"),
            "{generated}"
        );
        assert!(generated.contains(r#"#[pyo3(name = "rect")]"#), "{generated}");
        assert!(
            generated.contains("pub fn _factory_rect(width: f64, height: f64) -> Self"),
            "{generated}"
        );
        assert!(
            generated.contains("Self { inner: crate::Shape::Rect { width, height } }"),
            "{generated}"
        );
    }

    #[test]
    fn variant_constructors_convert_named_dto_fields() {
        use crate::codegen::type_mapper::IdentityMapper;
        // A field whose type is a binding DTO (Named) converts inline via `.into()` in the struct
        // literal — no typed `let <field>_core: <path>` binding, so the core type path is never
        // named (non-re-exported core types resolve through inference).
        let def = enum_def(
            "Wrapper",
            vec![variant(
                "Llm",
                vec![typed_field("llm", TypeRef::Named("LlmConfig".to_string()))],
            )],
        );

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        assert!(
            generated.contains("pub fn _factory_llm(llm: LlmConfig) -> Self"),
            "{generated}"
        );
        assert!(
            generated.contains("Self { inner: crate::Wrapper::Llm { llm: llm.into() } }"),
            "{generated}"
        );
        // No typed let-binding naming the core path.
        assert!(!generated.contains("llm_core"), "{generated}");
    }

    #[test]
    fn variant_constructors_pair_interleaved_field_exprs_by_position() {
        use crate::codegen::type_mapper::IdentityMapper;
        // Interleave a primitive, a Named-DTO (`.into()`), and a Vec<Named> DTO
        // (`.into_iter().map(Into::into).collect()`) so each field's init lands in its own
        // struct-literal slot, converted inline.
        let def = enum_def(
            "Job",
            vec![variant(
                "Run",
                vec![
                    typed_field("retries", TypeRef::Primitive(PrimitiveType::U32)),
                    typed_field("config", TypeRef::Named("RunConfig".to_string())),
                    typed_field("steps", TypeRef::Vec(Box::new(TypeRef::Named("Step".to_string())))),
                ],
            )],
        );

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        assert!(
            generated.contains(
                "Self { inner: crate::Job::Run { retries, config: config.into(), steps: steps.into_iter().map(Into::into).collect() } }"
            ),
            "{generated}"
        );
    }

    #[test]
    fn variant_constructors_skip_unit_tuple_and_excluded() {
        use crate::codegen::type_mapper::IdentityMapper;
        let mut tuple_variant = variant("Pair", vec![typed_field("_0", TypeRef::String)]);
        tuple_variant.is_tuple = true;
        let mut excluded = variant("Hidden", vec![typed_field("value", TypeRef::String)]);
        excluded.binding_excluded = true;

        let def = enum_def(
            "Mixed",
            vec![
                variant("Empty", vec![]),
                tuple_variant,
                excluded,
                variant("Real", vec![typed_field("value", TypeRef::String)]),
            ],
        );

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        // Unit, tuple, and binding_excluded variants get no constructor.
        assert!(!generated.contains("_factory_empty"), "{generated}");
        assert!(!generated.contains("_factory_pair"), "{generated}");
        assert!(!generated.contains("_factory_hidden"), "{generated}");
        // The struct variant still gets one.
        assert!(
            generated.contains("pub fn _factory_real(value: String) -> Self"),
            "{generated}"
        );
    }

    #[test]
    fn variant_constructors_skip_variant_with_sanitized_field() {
        use crate::codegen::type_mapper::IdentityMapper;
        // A sanitized field (e.g. core `[(u32, u32); 4]` downgraded to String) has no faithful
        // binding value, so the core variant cannot be built — skip the whole variant.
        let mut sanitized = typed_field("points", TypeRef::String);
        sanitized.sanitized = true;
        let def = enum_def(
            "OcrBoundingGeometry",
            vec![
                variant("Quadrilateral", vec![sanitized]),
                variant(
                    "Rectangle",
                    vec![typed_field("left", TypeRef::Primitive(PrimitiveType::U32))],
                ),
            ],
        );

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        assert!(!generated.contains("_factory_quadrilateral"), "{generated}");
        assert!(generated.contains("_factory_rectangle"), "{generated}");
    }

    #[test]
    fn variant_constructors_skip_variant_with_binding_excluded_field() {
        use crate::codegen::type_mapper::IdentityMapper;
        // A `binding_excluded` field (e.g. `#[alef(skip)]`) is hidden from the binding surface, so
        // no value exists to fill it — skip the variant rather than emit a broken literal.
        let mut excluded = typed_field("entries", TypeRef::String);
        excluded.binding_excluded = true;
        let def = enum_def(
            "NodeContent",
            vec![
                variant("MetadataBlock", vec![excluded]),
                variant("Title", vec![typed_field("text", TypeRef::String)]),
            ],
        );

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        assert!(!generated.contains("_factory_metadata_block"), "{generated}");
        assert!(generated.contains("_factory_title"), "{generated}");
    }

    #[test]
    fn variant_constructors_pass_through_genuine_optional_core_field() {
        use crate::codegen::type_mapper::IdentityMapper;
        // A field whose CORE type is `Option<String>` (optional=true) keeps the binding Option
        // unchanged in the struct literal — no unwrap.
        let mut opt = typed_field("value", TypeRef::String);
        opt.optional = true;
        let def = enum_def("AnnotationKind", vec![variant("Custom", vec![opt])]);

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        assert!(
            generated.contains("Self { inner: crate::AnnotationKind::Custom { value } }"),
            "{generated}"
        );
        assert!(!generated.contains("unwrap_or_default"), "{generated}");
    }

    #[test]
    fn variant_constructors_unwrap_promoted_optional_field() {
        use crate::codegen::type_mapper::IdentityMapper;
        // `extra` is a non-optional core field (`Vec<String>`, defaulted) that FOLLOWS an optional
        // field, so pyo3 promotes it to `Option<Vec<String>>` in the signature. The core field is
        // still `Vec<String>`, so the struct literal must `unwrap_or_default()` the promoted param.
        let mut model_file = typed_field("model_file", TypeRef::String);
        model_file.optional = true;
        let extra = typed_field("extra", TypeRef::Vec(Box::new(TypeRef::String)));
        let def = enum_def("RerankerModelType", vec![variant("Custom", vec![model_file, extra])]);

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        assert!(
            generated
                .contains("pub fn _factory_custom(model_file: Option<String>, extra: Option<Vec<String>>) -> Self"),
            "{generated}"
        );
        assert!(
            generated.contains(
                "Self { inner: crate::RerankerModelType::Custom { model_file, extra: extra.unwrap_or_default().into_iter().collect() } }"
            ),
            "{generated}"
        );
    }

    #[test]
    fn variant_constructors_convert_optional_path_field() {
        use crate::codegen::type_mapper::IdentityMapper;
        // A `Path` field maps to `String` in the binding but `PathBuf` in core; an optional one
        // must convert through the Option (`.map(Into::into)`), not pass the `Option<String>`
        // through unchanged (which would mismatch `Option<PathBuf>`).
        let mut cache_dir = typed_field("cache_dir", TypeRef::Path);
        cache_dir.optional = true;
        let def = enum_def(
            "ChunkSizing",
            vec![variant(
                "Tokenizer",
                vec![typed_field("model", TypeRef::String), cache_dir],
            )],
        );

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        assert!(
            generated.contains(
                "Self { inner: crate::ChunkSizing::Tokenizer { model, cache_dir: cache_dir.map(Into::into) } }"
            ),
            "{generated}"
        );
    }

    #[test]
    fn variant_constructors_use_inline_into_for_non_reexported_core_type() {
        use crate::codegen::type_mapper::IdentityMapper;
        // The core type lives at a nested module path (`pkg::enrich::EnrichStatus`), and the field
        // type is also non-re-exported. The inline `.into()` must not name any core type path, so
        // the generated body resolves the target via inference from the variant literal.
        let mut def = enum_def(
            "EnrichStatus",
            vec![variant(
                "Completed",
                vec![typed_field("result", TypeRef::Named("EnrichResult".to_string()))],
            )],
        );
        def.rust_path = "pkg::enrich::EnrichStatus".to_string();

        let generated = gen_pyo3_data_enum_with_mapper(&def, "pkg", Some(&IdentityMapper));

        assert!(
            generated.contains("Self { inner: pkg::enrich::EnrichStatus::Completed { result: result.into() } }"),
            "{generated}"
        );
        // The brittle `let result_core: pkg::EnrichResult` path annotation must not appear.
        assert!(!generated.contains("EnrichResult ="), "{generated}");
        assert!(!generated.contains("result_core"), "{generated}");
    }

    #[test]
    fn variant_constructors_yield_to_hand_written_method() {
        use crate::codegen::type_mapper::IdentityMapper;
        // A hand-written `impl` method named `circle` wins; no generated constructor for Circle.
        let mut def = enum_def(
            "Shape",
            vec![
                variant(
                    "Circle",
                    vec![typed_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
                ),
                variant(
                    "Square",
                    vec![typed_field("side", TypeRef::Primitive(PrimitiveType::F64))],
                ),
            ],
        );
        def.methods = vec![static_method("circle")];

        let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&IdentityMapper));

        // No generated constructor body for Circle (consumer method wins).
        assert!(
            !generated.contains("Self { inner: core::Shape::Circle"),
            "consumer method must win for Circle: {generated}"
        );
        // Square is untouched by the consumer method, so it gets a constructor.
        assert!(
            generated.contains("pub fn _factory_square(side: f64) -> Self"),
            "{generated}"
        );
    }

    #[test]
    fn variant_constructors_absent_without_mapper() {
        // Without a mapper, no variant constructors are generated (back-compat).
        let def = enum_def(
            "Shape",
            vec![variant(
                "Circle",
                vec![typed_field("radius", TypeRef::Primitive(PrimitiveType::F64))],
            )],
        );
        let generated = gen_pyo3_data_enum(&def, "core");
        assert!(!generated.contains("_factory_circle"), "{generated}");
    }

    #[test]
    fn gen_pyo3_unit_enum_emits_string_methods() {
        let cfg = RustBindingConfig {
            struct_attrs: &[],
            field_attrs: &[],
            struct_derives: &[],
            method_block_attr: None,
            constructor_attr: "",
            static_attr: None,
            function_attr: "",
            enum_attrs: &["pyclass(eq, eq_int, from_py_object)"],
            enum_derives: &["Clone", "PartialEq"],
            needs_signature: false,
            signature_prefix: "",
            signature_suffix: "",
            core_import: "core",
            async_pattern: AsyncPattern::None,
            has_serde: true,
            type_name_prefix: "",
            option_duration_on_defaults: false,
            opaque_type_names: &[],
            skip_impl_constructor: false,
            cast_uints_to_i32: false,
            cast_large_ints_to_f64: false,
            named_non_opaque_params_by_ref: false,
            lossy_skip_types: &[],
            serializable_opaque_type_names: &[],
            never_skip_cfg_field_names: &[],
            emit_delegating_default_impl: false,
            skip_methods_when_not_delegatable: false,
        };
        let generated = gen_enum(&enum_def("StructureKind", vec![variant("Function", Vec::new())]), &cfg);

        assert!(
            generated.contains("fn __str__(&self) -> PyResult<String>"),
            "{generated}"
        );
        assert!(generated.contains("serde_json::to_value(self)"), "{generated}");
    }
}
