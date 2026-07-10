use crate::core::keywords::swift_ident;

/// Emit Rust free-function shims and opaque `StreamHandle` types for streaming
/// adapters that have an `owner_type`.
///
/// For each streaming adapter, emits three free functions + one handle struct:
///
/// - `pub struct {Owner}{Adapter}StreamHandle` — owns a tokio runtime + boxed
///   stream, exposes `next_json(&mut self) -> Result<String, String>` to advance.
/// - `pub fn {owner_snake}_{name}_start(client: &OwnerType, ...params...) -> Result<*mut Handle, String>`
///   — kicks the request (HTTP errors propagate before any chunks arrive).
/// - `pub fn {owner_snake}_{name}_next(handle: &mut Handle) -> Result<String, String>`
///   — blocks on the next chunk; returns the JSON-encoded chunk, or an empty
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
/// Each handle clones a reference to the process-wide `__alef_tokio_runtime()`.
/// This ensures that spawned tasks (registered via `tokio::spawn` in the core API)
/// and `block_on` calls in `next()` operate on the same executor. A shared runtime
/// avoids cross-runtime deadlocks where a receiver on one runtime cannot consume
/// items spawned on a different runtime.
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

        let call_args: Vec<String> = adapter
            .params
            .iter()
            .map(|p| {
                let name = p.name.to_snake_case();
                format!("{name}.0.clone()")
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
            minijinja::context! {
                item_type => &item_type,
                fn_start => &fn_start,
                handle_name => &handle_name,
                core_item => &core_item,
            },
        ));

        out.push_str(&crate::backends::swift::template_env::render(
            "rust_stream_handle_start.rs.jinja",
            minijinja::context! {
                owner_type => owner_type,
                adapter_name => &adapter.name,
                handle_name => &handle_name,
                fn_start => &fn_start,
                start_params => &start_params_str,
                core_call => &core_call,
                core_item => &core_item,
            },
        ));

        // #[allow(clippy::should_implement_trait)] — the method name `next` deliberately
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_stream_handle_next.rs.jinja",
            minijinja::context! {
                handle_name => &handle_name,
            },
        ));
    }

    out
}
