//! Regression coverage for the `extern "Rust"` block emitted for streaming adapters.
//!
//! swift-bridge resolves type references module-wide, so when the owner already carries a canonical
//! `type Owner;` declaration elsewhere, the streaming block references it bare. Re-declaring it would
//! suppress the owner's Swift class and `$_free`.

use super::emit_extern_block_for_streaming_adapters;
use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern};
use std::collections::HashSet;

fn streaming_adapter_with_owner(name: &str, owner: &str) -> AdapterConfig {
    AdapterConfig {
        name: name.to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: format!("sample_crate::{name}"),
        params: vec![AdapterParam {
            name: "req".to_string(),
            ty: "sample_crate::StreamRequest".to_string(),
            optional: false,
        }],
        returns: None,
        error_type: Some("String".to_string()),
        owner_type: Some(owner.to_string()),
        item_type: Some("StreamItem".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some("sample_crate::StreamRequest".to_string()),
        skip_languages: vec![],
    }
}

#[test]
fn streaming_extern_block_references_declared_owner_bare() {
    let adapters = vec![
        streaming_adapter_with_owner("crawl_stream", "CrawlEngineHandle"),
        streaming_adapter_with_owner("event_stream", "CrawlEngineHandle"),
    ];
    let declared: HashSet<String> = HashSet::from(["CrawlEngineHandle".to_string()]);
    let block = emit_extern_block_for_streaming_adapters(&adapters, &declared)
        .expect("streaming adapter should produce a block");

    assert!(
        !block.contains("already_declared"),
        "owner declared elsewhere must not be re-declared as already_declared:\n{block}"
    );
    assert!(
        !block.contains("type CrawlEngineHandle;"),
        "owner declared elsewhere must be referenced bare, not re-declared:\n{block}"
    );
    assert!(
        block.contains("client: &CrawlEngineHandle"),
        "streaming `_start` must reference the owner by `&` reference:\n{block}"
    );
    assert!(
        block.contains("type CrawlEngineHandleCrawlStreamStreamHandle;"),
        "streaming block must declare its stream handle type:\n{block}"
    );
}

#[test]
fn streaming_extern_block_declares_owner_when_undeclared_elsewhere() {
    let adapters = vec![
        streaming_adapter_with_owner("crawl_stream", "CrawlEngineHandle"),
        streaming_adapter_with_owner("event_stream", "CrawlEngineHandle"),
    ];
    let declared: HashSet<String> = HashSet::new();
    let block = emit_extern_block_for_streaming_adapters(&adapters, &declared)
        .expect("streaming adapter should produce a block");

    assert!(
        !block.contains("already_declared"),
        "the streaming-block owner declaration must be canonical, not already_declared:\n{block}"
    );
    assert_eq!(
        block.matches("type CrawlEngineHandle;").count(),
        1,
        "undeclared owner must be declared exactly once across adapters:\n{block}"
    );
}
