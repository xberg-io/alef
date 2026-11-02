use crate::core::ir::{ApiSurface, ParamDef, TypeRef};

fn contains_map(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => contains_map(inner),
        _ => false,
    }
}

fn signature_contains_map(return_type: &TypeRef, params: &[ParamDef]) -> bool {
    contains_map(return_type) || params.iter().any(|parameter| contains_map(&parameter.ty))
}

pub(super) fn needs_map_import(api: &ApiSurface) -> bool {
    api.types.iter().any(|typ| {
        crate::codegen::shared::binding_fields(&typ.fields).any(|field| contains_map(&field.ty))
            || typ
                .methods
                .iter()
                .any(|method| signature_contains_map(&method.return_type, &method.params))
    }) || api
        .functions
        .iter()
        .any(|function| signature_contains_map(&function.return_type, &function.params))
        || api
            .enums
            .iter()
            .filter(|definition| !super::enums::is_json_passthrough_data_enum(definition, &api.types))
            .flat_map(|definition| &definition.variants)
            .any(|variant| crate::codegen::shared::binding_fields(&variant.fields).any(|field| contains_map(&field.ty)))
}

/// Prepend `#[cfg(<pred>)]` to a code item when the source symbol carries a cfg predicate.
pub(super) fn prepend_cfg(cfg: Option<&str>, item: String) -> String {
    match cfg {
        Some(pred) if !pred.is_empty() => format!("#[cfg({pred})]\n{item}"),
        _ => item,
    }
}

pub(super) fn js_bytes_def() -> &'static str {
    r#"
/// Wrapper for byte arrays that implements custom FromNapiValue to accept Buffer.from(...).
///
/// NAPI v3's default FromNapiValue for `Vec<u8>` expects `Array<number>`, not Buffer.
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

/// Per-crate runtime support for trait-bridge threadsafe-function calls.
///
/// Emitted once per crate when `[[crates.trait_bridges]]` is non-empty, alongside
/// [`js_bytes_def`]. `AlefJsReply<T>` is the return type every bridged async method's
/// `ThreadsafeFunction` decodes into: `FromNapiValue::from_napi_value` runs inside the
/// threadsafe-function trampoline, on the JS thread, inside the live `HandleScope` napi-rs
/// opens for that callback -- the only place a `napi_value` can safely become a `Promise<T>`.
/// That is the fix for the root cause of a class of trait-bridge crashes: the bridge previously
/// called `self.env()`/`self.obj(&env)` synchronously from whatever thread happened to run the
/// async method body (a tokio worker), with no `HandleScope` at all.
///
/// `settle`/`settle_sync` are then safe to call from any thread: a `Promise<T>` only polls a
/// `oneshot` channel the JS-thread trampoline already resolved, so awaiting it does no further
/// napi work.
pub(super) fn trait_bridge_runtime_def() -> &'static str {
    r#"
/// A JS-thread-decoded reply from a bridged plugin method call: either the callback already
/// returned a plain value, or it returned a `Promise` the caller resolves later. See the
/// module-level doc on why this decode must happen on the JS thread.
pub enum AlefJsReply<T: 'static + napi::bindgen_prelude::FromNapiValue> {
    Ready(T),
    Pending(napi::bindgen_prelude::Promise<T>),
}

impl<T: 'static + napi::bindgen_prelude::FromNapiValue> napi::bindgen_prelude::FromNapiValue for AlefJsReply<T> {
    unsafe fn from_napi_value(env: napi::sys::napi_env, napi_val: napi::sys::napi_value) -> napi::Result<Self> {
        let mut is_promise = false;
        let status = unsafe { napi::sys::napi_is_promise(env, napi_val, &mut is_promise) };
        if status != napi::sys::Status::napi_ok {
            return Err(napi::Error::new(
                napi::Status::GenericFailure,
                "Failed to check whether the host callback's return value is a Promise".to_string(),
            ));
        }
        if is_promise {
            let promise = unsafe { napi::bindgen_prelude::Promise::<T>::from_napi_value(env, napi_val) }?;
            Ok(AlefJsReply::Pending(promise))
        } else {
            let value = unsafe { T::from_napi_value(env, napi_val) }?;
            Ok(AlefJsReply::Ready(value))
        }
    }
}

impl<T: 'static + napi::bindgen_prelude::FromNapiValue> AlefJsReply<T> {
    /// Resolve to the final value, awaiting the JS Promise off-thread when needed.
    pub async fn settle(self) -> napi::Result<T> {
        match self {
            AlefJsReply::Ready(value) => Ok(value),
            AlefJsReply::Pending(promise) => promise.await,
        }
    }

    /// Resolve a reply for a Rust trait method that is declared synchronous (no executor is
    /// available here to await a Promise). A host implementation that returns a Promise from a
    /// synchronous method is a protocol error, reported with the plugin and method name rather
    /// than silently dropped or blocked on.
    pub fn settle_sync(self, plugin: &str, method: &str) -> napi::Result<T> {
        match self {
            AlefJsReply::Ready(value) => Ok(value),
            AlefJsReply::Pending(_) => Err(napi::Error::new(
                napi::Status::GenericFailure,
                format!(
                    "Plugin '{plugin}' method '{method}' is declared synchronous but returned a \
                     Promise; synchronous plugin methods must return a value directly"
                ),
            )),
        }
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
        assert!(
            content.contains("`Vec<u8>`"),
            "js_bytes_def should contain backtick-wrapped `Vec<u8>` to prevent rustdoc unclosed-tag warnings"
        );
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
