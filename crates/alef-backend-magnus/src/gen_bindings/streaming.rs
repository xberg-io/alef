//! Streaming adapter code generation for Magnus (Ruby) backend.
//!
//! Emits a custom iterator wrapper struct + an instance method on the owning
//! opaque type that drives the Rust core's async stream natively. The Ruby
//! surface supports both block-yielding (`client.chat_stream(req) { |c| ... }`)
//! and Enumerator-style consumption via the returned iterator's `each`/`next`.
//!
//! This bypasses the default `gen_opaque_async_instance_method` path because the
//! IR represents `BoxStream` returns as `String` (sanitized), which would emit a
//! `chat_stream_async` stub raising `NotImplementedError`.

use alef_core::config::{AdapterConfig, AdapterPattern};

/// Adapter info needed to generate one streaming iterator + method pair.
pub(super) struct StreamingAdapter<'a> {
    pub name: &'a str,
    pub owner_type: &'a str,
    pub item_type: &'a str,
    pub error_type: &'a str,
    pub request_binding_type: &'a str,
    pub request_core_path: &'a str,
    pub core_path: &'a str,
    pub iterator_struct_name: String,
    pub class_path: String,
}

impl<'a> StreamingAdapter<'a> {
    pub(super) fn from_config(adapter: &'a AdapterConfig, module_name: &str) -> Option<Self> {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            return None;
        }
        let owner = adapter.owner_type.as_deref()?;
        let item = adapter.item_type.as_deref()?;
        let error = adapter.error_type.as_deref()?;
        let request_full = adapter.request_type.as_deref()?;
        let req_binding = request_full.rsplit("::").next().unwrap_or(request_full);
        // PascalCase the adapter name for the iterator class name (e.g. chat_stream → ChatStream)
        let pascal = pascal_case(&adapter.name);
        let iterator_struct_name = format!("{}Iterator", pascal);
        let class_path = format!("{}::{}", module_name, iterator_struct_name);
        Some(Self {
            name: &adapter.name,
            owner_type: owner,
            item_type: item,
            error_type: error,
            request_binding_type: req_binding,
            request_core_path: request_full,
            core_path: &adapter.core_path,
            iterator_struct_name,
            class_path,
        })
    }
}

fn pascal_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = true;
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Generate the iterator opaque struct, its `IntoValueFromNative`/`TryConvert`
/// glue, and the inherent `next_chunk`/`each` methods.
pub(super) fn gen_iterator_struct(adapter: &StreamingAdapter<'_>) -> String {
    let iter_name = &adapter.iterator_struct_name;
    let class_path = &adapter.class_path;
    let item_binding = adapter.item_type;
    let item_core = format!("liter_llm::{}", adapter.item_type); // assumes item lives in core crate root
    let error_core = format!("liter_llm::{}", adapter.error_type);

    format!(
        r#"
#[derive(Clone)]
#[magnus::wrap(class = "{class_path}")]
pub struct {iter_name} {{
    inner: Arc<tokio::sync::Mutex<Option<futures::stream::BoxStream<'static, std::result::Result<{item_core}, {error_core}>>>>>,
    runtime: Arc<tokio::runtime::Runtime>,
}}

unsafe impl IntoValueFromNative for {iter_name} {{}}

impl magnus::TryConvert for {iter_name} {{
    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{
        let r: &{iter_name} = magnus::TryConvert::try_convert(val)?;
        Ok(r.clone())
    }}
}}

unsafe impl TryConvertOwned for {iter_name} {{}}

impl {iter_name} {{
    /// Pop the next chunk from the underlying stream synchronously.
    /// Returns `nil` once the stream is exhausted.
    fn next_chunk(&self) -> Result<magnus::Value, Error> {{
        use magnus::IntoValue;
        use magnus::value::ReprValue;
        let inner = self.inner.clone();
        let runtime = self.runtime.clone();
        let chunk_opt = runtime.block_on(async move {{
            let mut guard = inner.lock().await;
            match guard.as_mut() {{
                Some(stream) => futures::StreamExt::next(stream).await,
                None => None,
            }}
        }});
        let ruby = unsafe {{ Ruby::get_unchecked() }};
        match chunk_opt {{
            Some(Ok(chunk)) => {{
                let binding: {item_binding} = chunk.into();
                Ok(binding.into_value_with(&ruby))
            }}
            Some(Err(e)) => Err(Error::new(ruby.exception_runtime_error(), e.to_string())),
            None => {{
                // Drop the stream to release any resources.
                let inner = self.inner.clone();
                let runtime = self.runtime.clone();
                runtime.block_on(async move {{
                    let mut guard = inner.lock().await;
                    *guard = None;
                }});
                Ok(ruby.qnil().as_value())
            }}
        }}
    }}

    /// Yield each chunk to the given block (or build an Enumerator if no block was given).
    fn each(&self) -> Result<magnus::Value, Error> {{
        use magnus::IntoValue;
        use magnus::value::ReprValue;
        let ruby = unsafe {{ Ruby::get_unchecked() }};
        if !ruby.block_given() {{
            // Without a block, return an Enumerator over `each` so the caller can
            // call `.to_a`, `.lazy`, etc.
            let self_val: magnus::Value = self.clone().into_value_with(&ruby);
            let enumerator = self_val.enumeratorize(ruby.to_symbol("each"), ());
            return Ok(enumerator.as_value());
        }}
        loop {{
            let val = self.next_chunk()?;
            if val.is_nil() {{
                break;
            }}
            let _: magnus::Value = ruby.yield_value(val)?;
        }}
        Ok(self.clone().into_value_with(&ruby))
    }}
}}
"#
    )
}

/// Generate the streaming method on the owning opaque type. Returns the method
/// fragment to be appended inside the type's `impl` block.
pub(super) fn gen_streaming_method_body(adapter: &StreamingAdapter<'_>) -> String {
    let method_name = adapter.name;
    let core_method = adapter.core_path;
    let iter_name = &adapter.iterator_struct_name;
    let request_binding = adapter.request_binding_type;
    let request_core = adapter.request_core_path;

    format!(
        r#"    /// Streaming variant of `{method_name}`. Drives the Rust core stream
    /// natively, yielding each chunk to the caller's block. When called
    /// without a block, returns a `{iter_name}` (Enumerable via its `each`).
    fn {method_name}(&self, req: {request_binding}) -> Result<magnus::Value, Error> {{
        use magnus::IntoValue;
        use magnus::value::ReprValue;
        let inner = self.inner.clone();
        let core_req: {request_core} = req.into();
        let runtime = std::sync::Arc::new(tokio::runtime::Runtime::new().map_err(|e| {{
            magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string())
        }})?);
        let stream = runtime.block_on(async {{ inner.{core_method}(core_req).await }})
            .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;
        let iterator = {iter_name} {{
            inner: Arc::new(tokio::sync::Mutex::new(Some(stream))),
            runtime,
        }};
        let ruby = unsafe {{ Ruby::get_unchecked() }};
        if ruby.block_given() {{
            // Drive the stream synchronously, yielding each chunk to the block.
            iterator.each()?;
            Ok(ruby.qnil().as_value())
        }} else {{
            Ok(iterator.into_value_with(&ruby))
        }}
    }}
"#
    )
}

/// Generate the Ruby `define_class` + `define_method` registration lines for the
/// iterator class. Appended into the `ruby_init` body after the regular module setup.
pub(super) fn gen_iterator_registration(adapter: &StreamingAdapter<'_>) -> Vec<String> {
    let iter_name = &adapter.iterator_struct_name;
    vec![
        format!(r#"    let class = module.define_class("{iter_name}", ruby.class_object())?;"#),
        format!(r#"    class.define_method("next_chunk", method!({iter_name}::next_chunk, 0))?;"#),
        format!(r#"    class.define_method("each", method!({iter_name}::each, 0))?;"#),
        // Mix in Enumerable so .to_a, .map, etc. work.
        format!(r#"    class.include_module(ruby.module_enumerable())?;"#),
    ]
}

/// Generate the `define_method` call to register the streaming method on the
/// owner class. The owner class binding is named `class` in `gen_module_init`.
pub(super) fn gen_streaming_method_registration(adapter: &StreamingAdapter<'_>) -> String {
    let name = adapter.name;
    let owner = adapter.owner_type;
    format!(r#"    class.define_method("{name}", method!({owner}::{name}, 1))?;"#)
}
