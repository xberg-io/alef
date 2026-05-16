use crate::generators::RustBindingConfig;
use alef_core::ir::EnumDef;

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
    let name = &enum_def.name;
    let core_path = crate::conversions::core_enum_path(enum_def, core_import);
    let has_sanitized = enum_has_sanitized_fields(enum_def);
    let string_methods_content = crate::template_env::render(
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

    crate::template_env::render(
        "generators/enums/pyo3_data_enum.jinja",
        minijinja::context! {
            name => name,
            core_path => core_path,
            has_sanitized => has_sanitized,
            string_methods_content => string_methods_content,
            variant_accessors_content => variant_accessors,
            serde_tag_content => serde_tag_content,
        },
    )
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
            minijinja::context! {
                name => v.name.clone(),
                idx => idx,
                is_default => idx == default_idx,
                has_pyo3_rename => is_pyo3,
                pyo3_name => pyo3_name,
                serde_rename => v.serde_rename.clone().unwrap_or_default(),
            }
        })
        .collect();

    let string_methods = if is_pyo3 {
        crate::template_env::render(
            "generators/enums/enum_string_methods.jinja",
            minijinja::context! {
                name => enum_def.name,
                value_expr => "self",
            },
        )
    } else {
        String::new()
    };

    crate::template_env::render(
        "generators/enums/enum_definition.jinja",
        minijinja::context! {
            enum_name => enum_def.name,
            derives => derives.join(", "),
            serde_rename_all => enum_def.serde_rename_all.as_deref().unwrap_or(""),
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
    use alef_core::ir::TypeRef;

    for variant in &enum_def.variants {
        let variant_name_lower = crate::naming::pascal_to_snake(&variant.name);
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
                    out.push_str(&crate::template_env::render(
                        "generators/enums/getter_accessor.jinja",
                        minijinja::context! {
                            fn_name => &fn_name,
                            inner_type_name => inner_type_name,
                        },
                    ));
                    out.push_str("        match &self.inner {\n");
                    out.push_str(&crate::template_env::render(
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
        out.push_str(&crate::template_env::render(
            "generators/enums/py_dict_getter.jinja",
            minijinja::context! {
                fn_name => &fn_name,
            },
        ));
        out.push_str("        let json = serde_json::to_value(&self.inner)\n");
        out.push_str("            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n");
        let tag_field = enum_def.serde_tag.as_deref().unwrap_or("tag");
        out.push_str(&crate::template_env::render(
            "generators/enums/tag_field_check.jinja",
            minijinja::context! {
                tag_field => tag_field,
            },
        ));
        out.push_str("        let tag_value = json.get(tag_field)\n");
        out.push_str("            .and_then(|v| v.as_str())\n");
        out.push_str("            .unwrap_or(\"\");\n");
        out.push_str(&crate::template_env::render(
            "generators/enums/variant_tag_match.jinja",
            minijinja::context! {
                variant_name_lower => &variant_name_lower,
            },
        ));
        out.push_str("            return Ok(None);\n");
        out.push_str("        }\n");
        out.push_str("        let json_str = json.to_string();\n");
        out.push_str("        let json_mod = py.import(\"json\")?;\n");
        out.push_str("        let py_dict = json_mod.call_method1(\"loads\", (&json_str,))?.downcast_into::<pyo3::types::PyDict>()?;\n");
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
    out.push_str(&crate::template_env::render(
        "generators/enums/tag_getter_header.jinja",
        minijinja::context! {
            fn_name => &fn_name,
        },
    ));
    out.push_str("        let json = serde_json::to_value(&self.inner)\n");
    out.push_str("            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n");
    out.push_str(&crate::template_env::render(
        "generators/enums/json_get_field.jinja",
        minijinja::context! {
            tag_field => tag_field,
        },
    ));
    out.push_str("            .and_then(|v| v.as_str())\n");
    out.push_str("            .map(String::from)\n");
    out.push_str(&crate::template_env::render(
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
    use crate::generators::AsyncPattern;
    use alef_core::ir::{CoreWrapper, EnumVariant, FieldDef, TypeRef};

    fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            doc: String::new(),
            is_default: false,
            serde_rename: None,
            is_tuple: false,
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
        }
    }

    fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("crate::{name}"),
            original_rust_path: String::new(),
            variants,
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
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
        };
        let generated = gen_enum(&enum_def("StructureKind", vec![variant("Function", Vec::new())]), &cfg);

        assert!(
            generated.contains("fn __str__(&self) -> PyResult<String>"),
            "{generated}"
        );
        assert!(generated.contains("serde_json::to_value(self)"), "{generated}");
    }
}
