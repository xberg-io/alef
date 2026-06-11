pub(super) fn js_bytes_def() -> &'static str {
    r#"
/// Wrapper for byte arrays that implements custom FromNapiValue to accept Buffer.from(...).
///
/// NAPI v3's default FromNapiValue for `Vec<u8>` expects Array[number], not Buffer.
/// This wrapper provides custom deserialization that accepts Buffer, Uint8Array, or Array,
/// converting them to `Vec<u8>`. Implements Clone and serde traits for use in struct fields.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct JsBytes(pub Vec<u8>);

impl From<Vec<u8>> for JsBytes {
    fn from(v: Vec<u8>) -> Self {
        JsBytes(v)
    }
}

impl From<JsBytes> for Vec<u8> {
    fn from(js_bytes: JsBytes) -> Self {
        js_bytes.0
    }
}

impl AsRef<[u8]> for JsBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::ops::Deref for JsBytes {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for JsBytes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl napi::bindgen_prelude::FromNapiValue for JsBytes {
    unsafe fn from_napi_value(env: napi::sys::napi_env, napi_val: napi::sys::napi_value) -> napi::Result<Self> {
        use napi::bindgen_prelude::FromNapiValue;

        // Try Buffer first (most common for binary data in JS)
        if let Ok(buffer) = unsafe { napi::bindgen_prelude::Buffer::from_napi_value(env, napi_val) } {
            return Ok(JsBytes(buffer.as_ref().to_vec()));
        }

        // Try Uint8Array
        if let Ok(ua) = unsafe { napi::bindgen_prelude::Uint8Array::from_napi_value(env, napi_val) } {
            return Ok(JsBytes(ua.to_vec()));
        }

        // Fall back to Array[number]
        if let Ok(vec) = unsafe { Vec::<u8>::from_napi_value(env, napi_val) } {
            return Ok(JsBytes(vec));
        }

        Err(napi::Error::new(
            napi::Status::InvalidArg,
            "Expected Buffer, Uint8Array, or Array<number> for bytes field",
        ))
    }
}

impl napi::bindgen_prelude::ToNapiValue for JsBytes {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> napi::Result<napi::sys::napi_value> {
        // Delegate to Vec<u8>'s implementation (which returns an Uint8Array/Buffer).
        unsafe { <Vec<u8> as napi::bindgen_prelude::ToNapiValue>::to_napi_value(env, val.0) }
    }
}
"#
}

pub(super) fn js_visitor_ref_def() -> &'static str {
    r#"
/// Wrapper for trait visitor types (napi::Object<'static>) that implements Clone.
///
/// Object is not Clone. This wrapper uses Arc<Object<'static>> internally for cheap cloning.
/// The .inner field is public for compatibility with generated code that needs to access
/// the underlying Object for trait dispatch.
pub struct JsVisitorRef {
    pub inner: std::sync::Arc<napi::bindgen_prelude::Object<'static>>,
}

impl Clone for JsVisitorRef {
    fn clone(&self) -> Self {
        JsVisitorRef {
            inner: std::sync::Arc::clone(&self.inner),
        }
    }
}

#[allow(clippy::arc_with_non_send_sync)]
impl From<napi::bindgen_prelude::Object<'static>> for JsVisitorRef {
    fn from(visitor: napi::bindgen_prelude::Object<'static>) -> Self {
        JsVisitorRef {
            inner: std::sync::Arc::new(visitor),
        }
    }
}

impl From<JsVisitorRef> for napi::bindgen_prelude::Object<'static> {
    fn from(visitor_ref: JsVisitorRef) -> Self {
        // Object<'static> is Copy (it just holds an env+handle pair), so deref directly.
        *visitor_ref.inner
    }
}
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_js_bytes_def_has_backtick_wrapped_vec_types() {
        let content = js_bytes_def();
        // Verify that `Vec<u8>` is backtick-wrapped to avoid rustdoc HTML tag warnings
        assert!(
            content.contains("`Vec<u8>`"),
            "js_bytes_def should contain backtick-wrapped `Vec<u8>` to prevent rustdoc unclosed-tag warnings"
        );
        // Verify no unwrapped Vec<u8> exists in doc comments
        let lines: Vec<&str> = content.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("///") && !line.contains("`Vec<u8>`") {
                assert!(
                    !line.contains("Vec<u8>"),
                    "Line {} should not contain unwrapped 'Vec<u8>' in doc comments: {}",
                    idx + 1,
                    line
                );
            }
        }
    }
}
