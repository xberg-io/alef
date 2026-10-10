use crate::core::keywords::swift_ident;

/// Emit Rust free-function shims and opaque `StreamHandle` types for streaming
/// adapters that have an `owner_type`.
///
/// For each streaming adapter, emits three free functions + one handle struct:
///
/// - `pub struct {Owner}{Adapter}StreamHandle` — owns a tokio runtime + boxed
///   stream, exposes `next_json(&mut self) -> Result<String, String>` to advance.
/// - `pub async fn {owner_snake}_{name}_start(client: &OwnerType, ...params...) -> Result<Handle, String>`
///   — kicks the request (HTTP errors propagate before any chunks arrive).
/// - `pub async fn next(&self) -> Result<String, String>` on the handle
///   — awaits the next chunk; returns the JSON-encoded chunk, or an empty
///   string `""` to signal clean end-of-stream. Errors propagate as `Err(String)`.
/// - `pub fn {owner_snake}_{name}_free(handle: *mut Handle)` — drops the handle.
///
/// ### Why JSON-string at the bridge boundary
///
/// swift-bridge 0.1.x's support for `Result<Option<OpaqueRustType>, String>` is
/// not exercised in the upstream codegen tests, and `Option<RustString>` works
/// reliably across versions. We pick the most stable encoding — a JSON string —
/// matching the FFI/Java backends' item-to-JSON protocol and reusing the
/// item type's existing `Serialize` impl (every adapter `item_type` is a
/// serde-bridged DTO in current consumers).
///
/// An empty string `""` is never a valid JSON value, so it is a safe EOF sentinel.
///
/// ### Runtime ownership (SAFETY)
///
/// Each handle clones a reference to the process-wide `__alef_tokio_runtime()`, and the
/// request is opened on it, so tasks the core API registers via `tokio::spawn` live on the
/// same executor for the stream's whole life. `next()` only awaits the stream, so it is
/// polled by swift-bridge's runtime; a channel receiver works across runtimes, and the
/// shared runtime avoids the orphaned-connection-pool problem of a runtime per call.
pub(crate) fn emit_streaming_adapter_shims(
    adapters: &[crate::core::config::AdapterConfig],
    source_crate: &str,
) -> String {
    use crate::core::config::AdapterPattern;
    use heck::{ToPascalCase, ToSnakeCase};

    let mut out = String::new();

    for adapter in adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming))
        .filter(|a| a.owner_type.is_some())
    {
        let owner_type = adapter.owner_type.as_deref().unwrap_or("");
        let item_type = adapter
            .item_type
            .as_deref()
            .expect("streaming adapter must declare item_type for Swift backend");
        let owner_snake = owner_type.to_snake_case();
        let adapter_pascal = adapter.name.to_pascal_case();
        let owner_pascal = owner_type.to_pascal_case();
        let handle_name = format!("{owner_pascal}{adapter_pascal}StreamHandle");
        let fn_start = format!("{owner_snake}_{}_start", adapter.name);

        let core_item = format!("{source_crate}::{item_type}");

        let mut start_params_vec: Vec<String> = vec![format!("client: &{owner_type}")];
        for p in &adapter.params {
            let simple_ty = p.ty.rsplit("::").next().unwrap_or(&p.ty);
            let param_name = swift_ident(&p.name.to_snake_case());
            start_params_vec.push(format!("{param_name}: &{simple_ty}"));
        }
        let start_params_str = start_params_vec.join(", ");

        // Request parameters are cloned before the spawn: the task must be `'static`, and a
        // clone is cheap next to the request it configures. ~keep
        let mut param_bindings = String::new();
        let call_args: Vec<String> = adapter
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                let bound = format!("__alef_{name}");
                param_bindings.push_str(&format!("    let {bound} = {name}.0.clone();\n"));
                bound
            })
            .collect();
        let call_args_str = call_args.join(", ");

        let core_call = if adapter.core_path.contains("::") {
            format!("{}(&client.0, {call_args_str})", adapter.core_path)
        } else {
            format!("client.0.{}({call_args_str})", adapter.core_path)
        };

        out.push_str(&crate::backends::swift::template_env::render(
            "rust_stream_handle_struct.rs.jinja",
            crate::alef_context! {
                item_type => &item_type,
                fn_start => &fn_start,
                handle_name => &handle_name,
                core_item => &core_item,
            },
        ));

        out.push_str(&crate::backends::swift::template_env::render(
            "rust_stream_handle_start.rs.jinja",
            crate::alef_context! {
                owner_type => owner_type,
                adapter_name => &adapter.name,
                handle_name => &handle_name,
                fn_start => &fn_start,
                start_params => &start_params_str,
                core_call => &core_call,
                core_item => &core_item,
                param_bindings => &param_bindings,
            },
        ));

        // #[allow(clippy::should_implement_trait)] — the method name `next` deliberately
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_stream_handle_next.rs.jinja",
            crate::alef_context! {
                handle_name => &handle_name,
            },
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern};

    fn adapter() -> AdapterConfig {
        AdapterConfig {
            name: "chat_stream".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: "chat_stream".to_string(),
            params: vec![AdapterParam {
                name: "req".to_string(),
                ty: "sample_crate::ChatRequest".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: Some("String".to_string()),
            owner_type: Some("Client".to_string()),
            item_type: Some("Chunk".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: Some("sample_crate::ChatRequest".to_string()),
            skip_languages: vec![],
        }
    }

    /// Opening a stream and pulling each chunk are the two waits a Swift caller sits through, so
    /// both must be `async fn`s (matching the `async fn` extern declarations) rather than
    /// `block_on` calls that park a thread for the stream's whole life.
    #[test]
    fn start_and_next_are_async_and_never_block_a_thread() {
        let out = emit_streaming_adapter_shims(&[adapter()], "sample_crate");

        assert!(
            out.contains("pub async fn client_chat_stream_start(client: &Client, req: &ChatRequest)"),
            "start must be an `async fn`, got:\n{out}"
        );
        assert!(
            out.contains("pub async fn next(&self) -> Result<String, String>"),
            "next must be an `async fn` over `&self`, got:\n{out}"
        );
        assert!(
            !out.contains("block_on"),
            "an async streaming shim must never block a thread on the runtime, got:\n{out}"
        );
        assert!(
            out.contains("::futures_util::lock::Mutex<"),
            "a guard held across `.await` needs an async mutex, got:\n{out}"
        );
        assert!(
            !out.contains("std::sync::Mutex"),
            "a std mutex guard must not be held across `.await`, got:\n{out}"
        );
    }

    /// The spawned task must be `'static`, so the borrowed request is cloned before the spawn and
    /// the core call receives the owned clone, not the borrow.
    #[test]
    fn start_clones_request_params_before_spawning() {
        let out = emit_streaming_adapter_shims(&[adapter()], "sample_crate");

        assert!(out.contains("let __alef_req = req.0.clone();"), "got:\n{out}");
        assert!(
            out.contains("client.0.chat_stream(__alef_req)"),
            "the core call must take the pre-spawn clone, got:\n{out}"
        );
        let clone_at = out.find("let __alef_req = req.0.clone();").expect("checked above");
        let spawn_at = out.find(".spawn(async move").expect("start must spawn");
        assert!(clone_at < spawn_at, "the clone must precede the spawn, got:\n{out}");
    }
}
