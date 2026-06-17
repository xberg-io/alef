use serde::{Deserialize, Serialize};

use super::items::{EnumDef, ErrorDef, FunctionDef, TypeDef};
use super::service::{HandlerContractDef, ServiceDef};

/// Complete API surface extracted from a Rust crate's public interface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSurface {
    pub crate_name: String,
    pub version: String,
    pub types: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
    pub enums: Vec<EnumDef>,
    pub errors: Vec<ErrorDef>,
    #[serde(default)]
    pub excluded_type_paths: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub excluded_trait_names: std::collections::HashSet<String>,
    #[serde(default)]
    pub services: Vec<ServiceDef>,
    #[serde(default)]
    pub handler_contracts: Vec<HandlerContractDef>,
    #[serde(default)]
    pub unsupported_public_items: Vec<UnsupportedPublicItem>,
}

impl ApiSurface {
    /// Returns a clone of this surface with same-named cfg-variant functions collapsed to one.
    ///
    /// Single-surface backends (Java, C#, Go, Kotlin, Swift, Dart, PHP, Ruby, Elixir) emit one
    /// non-cfg-gated host method per function. When the extractor preserves a real impl and a
    /// stub fallback under disjoint `cfg` gates, those two entries would otherwise become two
    /// host methods with identical signatures — a duplicate-method compile error. This collapses
    /// each such group into a single entry whose `cfg` is the OR of all members'. Rust-cfg-gated
    /// backends (FFI, napi, pyo3, wasm) must NOT call this: they emit both `#[cfg]`-guarded items
    /// and rely on `rustc` selecting one per feature set.
    #[must_use]
    pub fn with_deduped_functions(&self) -> Self {
        let mut deduped = self.clone();
        deduped.functions = crate::codegen::fn_dedup::dedup_same_name_functions(&self.functions);
        deduped
    }
}

/// A public item that was discovered but not extracted into binding IR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnsupportedPublicItem {
    pub item_kind: String,
    pub item_path: String,
    pub reason: String,
    pub suggested_fix: String,
}
