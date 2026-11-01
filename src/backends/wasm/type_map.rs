use crate::codegen::type_mapper::TypeMapper;
use ahash::AHashSet;
use std::borrow::Cow;
use std::collections::HashMap;

/// TypeMapper for WASM bindings with configurable type overrides.
pub struct WasmMapper {
    pub overrides: HashMap<String, String>,
    pub prefix: String,
    /// Display-text unions that also keep a lossless `JsValue` companion on every field that
    /// holds one. See `crate::codegen::conversions::helpers::text_value_companion`.
    pub text_value_enums: AHashSet<String>,
}

impl WasmMapper {
    pub fn new(overrides: HashMap<String, String>, prefix: String) -> Self {
        Self {
            overrides,
            prefix,
            text_value_enums: AHashSet::new(),
        }
    }

    pub fn with_text_value_enums(mut self, names: AHashSet<String>) -> Self {
        self.text_value_enums = names;
        self
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

    /// WASM can't handle nested Vec, optional elements or `Vec<bool>` (wasm-bindgen has no vector
    /// ABI for `bool`) — use JsValue instead.
    fn vec(&self, inner: &str) -> String {
        if inner.starts_with("Vec<") || inner.starts_with("Option<") || inner == "JsValue" || inner == "bool" {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_of_bool_uses_jsvalue() {
        let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
        assert_eq!(mapper.vec("bool"), "JsValue");
        assert_eq!(mapper.vec("u32"), "Vec<u32>");
    }

    #[test]
    fn vec_of_optional_values_uses_jsvalue() {
        let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());

        assert_eq!(
            mapper.map_type(&crate::core::ir::TypeRef::Vec(Box::new(
                crate::core::ir::TypeRef::Optional(Box::new(crate::core::ir::TypeRef::String)),
            ))),
            "JsValue"
        );
    }
}
