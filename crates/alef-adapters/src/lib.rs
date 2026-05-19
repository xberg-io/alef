//! Adapter layer for connecting language-specific patterns to alef's backend trait.
//! Handles callback bridges and custom registrations.

pub mod async_method;
pub mod callback_bridge;
pub mod streaming;
pub mod sync_function;

use ahash::AHashMap;
use alef_core::config::{AdapterConfig, AdapterPattern, Language, ResolvedCrateConfig};

/// Key: "TypeName.method_name" for methods, "function_name" for free functions.
/// For streaming adapters, an additional entry "Owner.adapter_name.__stream_struct__"
/// (or "adapter_name.__stream_struct__" when no owner is set) holds the iterator
/// struct definition. The adapter-name qualifier is required so that multiple
/// streaming adapters declared on the same owner with the same item_type each get
/// their own iterator/handle struct instead of colliding under a shared item_type key.
pub type AdapterBodies = AHashMap<String, String>;

/// Build the streaming-struct lookup key for an adapter. Mirrors the writer in
/// [`build_adapter_bodies`] so per-backend `gen_bindings/mod.rs` files can look up
/// the body via the same canonical scheme.
pub fn stream_struct_key(adapter: &AdapterConfig) -> String {
    match adapter.owner_type.as_deref() {
        Some(owner) => format!("{}.{}.__stream_struct__", owner, adapter.name),
        None => format!("{}.__stream_struct__", adapter.name),
    }
}

/// Build a map of adapter-generated method/function bodies for a language.
pub fn build_adapter_bodies(config: &ResolvedCrateConfig, language: Language) -> anyhow::Result<AdapterBodies> {
    let mut bodies = AHashMap::new();

    for adapter in &config.adapters {
        let key = if let Some(owner) = &adapter.owner_type {
            format!("{}.{}", owner, adapter.name)
        } else {
            adapter.name.clone()
        };

        match adapter.pattern {
            AdapterPattern::SyncFunction => {
                let body = sync_function::generate_body(adapter, language, config)?;
                bodies.insert(key, body);
            }
            AdapterPattern::AsyncMethod => {
                let body = async_method::generate_body(adapter, language, config)?;
                bodies.insert(key, body);
            }
            AdapterPattern::Streaming => {
                let (method_body, struct_def) = streaming::generate_body(adapter, language, config)?;
                bodies.insert(key, method_body);
                if let Some(struct_code) = struct_def {
                    bodies.insert(stream_struct_key(adapter), struct_code);
                }
            }
            AdapterPattern::CallbackBridge => {
                let (struct_code, impl_code) = callback_bridge::generate(adapter, language, config)?;
                let struct_key = format!("{}.__bridge_struct__", adapter.name);
                bodies.insert(struct_key, struct_code);
                let impl_key = format!("{}.__bridge_impl__", adapter.name);
                bodies.insert(impl_key, impl_code);
                continue; // Don't insert into the normal body map
            }
            AdapterPattern::ServerLifecycle => {
                let body = format!(
                    "compile_error!(\"adapter pattern not yet implemented: {}\")",
                    adapter.name
                );
                bodies.insert(key, body);
            }
        }
    }

    Ok(bodies)
}
