use crate::codegen::type_mapper::TypeMapper;
use std::borrow::Cow;
use std::collections::HashMap;

/// TypeMapper for WASM bindings with configurable type overrides.
pub struct WasmMapper {
    pub overrides: HashMap<String, String>,
    pub prefix: String,
}

impl WasmMapper {
    pub fn new(overrides: HashMap<String, String>, prefix: String) -> Self {
        Self { overrides, prefix }
    }
}

impl TypeMapper for WasmMapper {
    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        match self.overrides.get(name) {
            Some(override_ty) => Cow::Owned(override_ty.clone()),
            None => Cow::Owned(format!("{}{name}", self.prefix)),
        }
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("JsValue")
    }

    /// WASM can't handle HashMap across the boundary — use JsValue instead.
    fn map(&self, _key: &str, _value: &str) -> String {
        "JsValue".to_string()
    }

    /// WASM can't handle nested Vec (Vec<Vec<T>>) — use JsValue instead.
    fn vec(&self, inner: &str) -> String {
        if inner.starts_with("Vec<") || inner == "JsValue" {
            "JsValue".to_string()
        } else {
            format!("Vec<{inner}>")
        }
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }

    /// WASM wraps errors as `Result<T, JsValue>`.
    fn wrap_return(&self, base: &str, has_error: bool) -> String {
        if has_error {
            format!("Result<{base}, JsValue>")
        } else {
            base.to_string()
        }
    }
}
