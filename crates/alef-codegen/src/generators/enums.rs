use crate::generators::RustBindingConfig;
use alef_core::ir::EnumDef;
use std::fmt::Write;

/// Returns true if any variant of the enum has data fields.
/// These enums cannot be represented as flat integer enums in bindings.
pub fn enum_has_data_variants(enum_def: &EnumDef) -> bool {
    enum_def.variants.iter().any(|v| !v.fields.is_empty())
}

/// Generate a PyO3 data enum as a `#[pyclass]` struct wrapping the core type.
///
/// Data enums (tagged unions like `AuthConfig`) can't be flat int enums in PyO3.
/// Instead, generate a frozen struct with `inner` that accepts a Python dict,
/// serializes it to JSON, and deserializes into the core Rust type via serde.
pub fn gen_pyo3_data_enum(enum_def: &EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = crate::conversions::core_enum_path(enum_def, core_import);
    let mut out = String::with_capacity(512);

    writeln!(out, "#[derive(Clone)]").ok();
    writeln!(out, "#[pyclass(frozen)]").ok();
    writeln!(out, "pub struct {name} {{").ok();
    writeln!(out, "    pub(crate) inner: {core_path},").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    writeln!(out, "#[pymethods]").ok();
    writeln!(out, "impl {name} {{").ok();
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
    writeln!(out, "}}").ok();
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
    writeln!(out, "impl Default for {name} {{").ok();
    writeln!(
        out,
        "    fn default() -> Self {{ Self {{ inner: Default::default() }} }}"
    )
    .ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Deserialize: forward to inner so parent structs that derive serde::Deserialize compile.
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

/// Python keywords and builtins that cannot be used as variant identifiers in PyO3 enums.
/// When a variant name matches one of these, a `#[pyo3(name = "...")]` rename attribute
/// is emitted so the Rust identifier remains unchanged while Python sees a safe name.
const PYTHON_KEYWORDS: &[&str] = &[
    "None", "True", "False", "from", "import", "class", "def", "return", "yield", "pass", "break", "continue", "and",
    "or", "not", "is", "in", "if", "else", "elif", "for", "while", "with", "as", "try", "except", "finally", "raise",
    "del", "global", "nonlocal", "lambda", "assert", "type",
];

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
    for attr in cfg.enum_attrs {
        writeln!(out, "#[{attr}]").ok();
    }
    // Detect PyO3 context so we can rename Python keyword variants via #[pyo3(name = "...")].
    // The Rust identifier stays unchanged; only the Python-exposed attribute name gets the suffix.
    let is_pyo3 = cfg.enum_attrs.iter().any(|a| a.contains("pyclass"));
    writeln!(out, "pub enum {} {{", enum_def.name).ok();
    for (idx, variant) in enum_def.variants.iter().enumerate() {
        if is_pyo3 && PYTHON_KEYWORDS.contains(&variant.name.as_str()) {
            writeln!(out, "    #[pyo3(name = \"{}_\")]", variant.name).ok();
        }
        writeln!(out, "    {} = {idx},", variant.name).ok();
    }
    writeln!(out, "}}").ok();

    out
}
