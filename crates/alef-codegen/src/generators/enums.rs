use crate::generators::RustBindingConfig;
use alef_core::ir::EnumDef;
use alef_core::keywords::PYTHON_KEYWORDS;
use std::fmt::Write;

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
    let mut out = String::with_capacity(512);

    writeln!(out, "#[derive(Clone)]").ok();
    writeln!(out, "#[pyclass(frozen)]").ok();
    writeln!(out, "pub struct {name} {{").ok();
    writeln!(out, "    pub(crate) inner: {core_path},").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    writeln!(out, "#[pymethods]").ok();
    writeln!(out, "impl {name} {{").ok();
    if has_sanitized {
        // The core type cannot be serde round-tripped from a Python dict (contains
        // non-representable variant fields). Omit the #[new] constructor — the type
        // is still useful as a return value from Rust passed back via From impls.
        write_pyo3_enum_string_methods(&mut out, name, "&self.inner");
        writeln!(out, "}}").ok();
    } else {
        writeln!(out, "    #[new]").ok();
        writeln!(
            out,
            "    fn new(py: Python<'_>, value: &Bound<'_, pyo3::types::PyDict>) -> PyResult<Self> {{"
        )
        .ok();
        writeln!(out, "        let json_mod = py.import(\"json\")?;").ok();
        writeln!(
            out,
            "        let json_str: String = json_mod.call_method1(\"dumps\", (value,))?.extract()?;"
        )
        .ok();
        writeln!(out, "        let inner: {core_path} = serde_json::from_str(&json_str)").ok();
        writeln!(
            out,
            "            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!(\"Invalid {name}: {{e}}\")))?;"
        )
        .ok();
        writeln!(out, "        Ok(Self {{ inner }})").ok();
        writeln!(out, "    }}").ok();
        write_pyo3_enum_string_methods(&mut out, name, "&self.inner");
        writeln!(out, "}}").ok();
    }
    writeln!(out).ok();

    // From binding → core
    writeln!(out, "impl From<{name}> for {core_path} {{").ok();
    writeln!(out, "    fn from(val: {name}) -> Self {{ val.inner }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // From core → binding
    writeln!(out, "impl From<{core_path}> for {name} {{").ok();
    writeln!(out, "    fn from(val: {core_path}) -> Self {{ Self {{ inner: val }} }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Serialize: forward to inner so parent structs that derive serde::Serialize compile.
    // Always generated — the wrapper delegates to the core type which always implements Serialize.
    writeln!(out, "impl serde::Serialize for {name} {{").ok();
    writeln!(
        out,
        "    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {{"
    )
    .ok();
    writeln!(out, "        self.inner.serialize(serializer)").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Default: forward to inner's Default so parent structs that derive Default compile.
    // Always generated — the wrapper delegates to the core type which always implements Default.
    writeln!(out, "impl Default for {name} {{").ok();
    writeln!(
        out,
        "    fn default() -> Self {{ Self {{ inner: Default::default() }} }}"
    )
    .ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Deserialize: forward to inner so parent structs that derive serde::Deserialize compile.
    // Always generated — the wrapper delegates to the core type which always implements Deserialize.
    writeln!(out, "impl<'de> serde::Deserialize<'de> for {name} {{").ok();
    writeln!(
        out,
        "    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {{"
    )
    .ok();
    writeln!(out, "        let inner = {core_path}::deserialize(deserializer)?;").ok();
    writeln!(out, "        Ok(Self {{ inner }})").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

    out
}

/// Generate an enum.
pub fn gen_enum(enum_def: &EnumDef, cfg: &RustBindingConfig) -> String {
    // All enums are generated as unit-variant-only in the binding layer.
    // Data variants are flattened to unit variants; the From/Into conversions
    // handle the lossy mapping (discarding / providing defaults for field data).
    let mut out = String::with_capacity(512);
    let mut derives: Vec<&str> = cfg.enum_derives.to_vec();
    // Binding enums always derive Default, Serialize, and Deserialize.
    // Default: enables using unwrap_or_default() in constructors.
    // Serialize/Deserialize: required for FFI/type conversion across binding boundaries.
    derives.push("Default");
    derives.push("serde::Serialize");
    derives.push("serde::Deserialize");
    if !derives.is_empty() {
        writeln!(out, "#[derive({})]", derives.join(", ")).ok();
    }
    if let Some(rename_all) = &enum_def.serde_rename_all {
        writeln!(out, "#[serde(rename_all = \"{rename_all}\")]").ok();
    }
    for attr in cfg.enum_attrs {
        writeln!(out, "#[{attr}]").ok();
    }
    // Detect PyO3 context so we can rename Python keyword variants via #[pyo3(name = "...")].
    // The Rust identifier stays unchanged; only the Python-exposed attribute name gets the suffix.
    let is_pyo3 = cfg.enum_attrs.iter().any(|a| a.contains("pyclass"));
    writeln!(out, "pub enum {} {{", enum_def.name).ok();
    // Determine which variant carries #[default].
    // Prefer the variant that has is_default=true in the source (mirrors the Rust core's
    // #[default] attribute); fall back to the first variant when none is explicitly marked.
    let default_idx = enum_def.variants.iter().position(|v| v.is_default).unwrap_or(0);
    for (idx, variant) in enum_def.variants.iter().enumerate() {
        if is_pyo3 && PYTHON_KEYWORDS.contains(&variant.name.as_str()) {
            writeln!(out, "    #[pyo3(name = \"{}_\")]", variant.name).ok();
        }
        // Mark the default variant as #[default] so derive(Default) works
        if idx == default_idx {
            writeln!(out, "    #[default]").ok();
        }
        writeln!(out, "    {} = {idx},", variant.name).ok();
    }
    writeln!(out, "}}").ok();
    if is_pyo3 {
        writeln!(out).ok();
        writeln!(out, "#[pymethods]").ok();
        writeln!(out, "impl {} {{", enum_def.name).ok();
        write_pyo3_enum_string_methods(&mut out, &enum_def.name, "self");
        writeln!(out, "}}").ok();
    }

    out
}

fn write_pyo3_enum_string_methods(out: &mut String, name: &str, value_expr: &str) {
    writeln!(out).ok();
    writeln!(out, "    fn __str__(&self) -> PyResult<String> {{").ok();
    writeln!(
        out,
        "        serde_json::to_value({value_expr})\n            .map(|value| match value {{\n                serde_json::Value::String(value) => value,\n                other => other.to_string(),\n            }})\n            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!(\"Failed to serialize {name}: {{e}}\")))"
    )
    .ok();
    writeln!(out, "    }}").ok();
    writeln!(out).ok();
    writeln!(out, "    fn __repr__(&self) -> PyResult<String> {{").ok();
    writeln!(out, "        self.__str__()").ok();
    writeln!(out, "    }}").ok();
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
            serde_rename_all: None,
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
        };
        let generated = gen_enum(&enum_def("StructureKind", vec![variant("Function", Vec::new())]), &cfg);

        assert!(
            generated.contains("fn __str__(&self) -> PyResult<String>"),
            "{generated}"
        );
        assert!(generated.contains("serde_json::to_value(self)"), "{generated}");
    }
}
