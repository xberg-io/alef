//! The backend-agnostic half of the opt-in lazy visitor-context accessor (ADR 0019): validate
//! the configured `context type -> method` pairs and hand each backend the scope runtime it
//! compiles into its generated crate.
//!
//! Python was the first implementation; Node, WebAssembly, Ruby, PHP and R each opt in with the
//! same config shape. Keeping the validation here means the five ports cannot disagree about
//! what counts as a valid accessor.

use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, ReceiverKind, TypeRef};
use std::collections::BTreeMap;

/// The shared callback-scoped reader runtime, emitted into every generated crate that opts in.
///
/// Each backend emits it inside its own generated module and keys a `CallbackAccessorScope` by
/// the borrowed core context's address, so nested callbacks keep separate readers and a getter
/// reached outside its callback sees no entry.
pub(crate) const SCOPE_RUNTIME: &str = include_str!("visitor_context_scope.rs");

/// One configured lazy accessor, validated against the IR.
#[derive(Debug, Clone)]
pub(crate) struct VisitorAccessorSpec {
    context: String,
    method: String,
}

impl VisitorAccessorSpec {
    pub(crate) fn resolve(
        api: &ApiSurface,
        bridges: &[TraitBridgeConfig],
        configured: &BTreeMap<String, String>,
    ) -> anyhow::Result<BTreeMap<String, Self>> {
        configured
            .iter()
            .map(|(context, method)| {
                Self::validate(api, bridges, context, method)?;
                Ok((
                    context.clone(),
                    Self {
                        context: context.clone(),
                        method: method.clone(),
                    },
                ))
            })
            .collect()
    }

    fn validate(api: &ApiSurface, bridges: &[TraitBridgeConfig], context: &str, method: &str) -> anyhow::Result<()> {
        let matching = bridges
            .iter()
            .filter(|bridge| bridge.context_type.as_deref() == Some(context))
            .count();
        anyhow::ensure!(
            matching == 1,
            "visitor context `{context}` must belong to exactly one active trait bridge, found {matching}"
        );
        let typ = api
            .types
            .iter()
            .find(|typ| typ.name == context)
            .ok_or_else(|| anyhow::anyhow!("visitor context `{context}` is not in the API"))?;
        let valid = typ.methods.iter().any(|candidate| {
            candidate.name == method
                && !candidate.binding_excluded
                && !candidate.is_async
                && candidate.cfg.is_none()
                && candidate.receiver == Some(ReceiverKind::Ref)
                && candidate.params.is_empty()
                && candidate.error_type.is_none()
                && matches!(
                    &candidate.return_type,
                    TypeRef::Map(key, value)
                        if **key == TypeRef::String && **value == TypeRef::String
                )
        });
        anyhow::ensure!(
            valid,
            "visitor context accessor `{context}.{method}` must be a visible synchronous no-argument \
             string-map getter"
        );
        Ok(())
    }

    pub(crate) fn method(&self) -> &str {
        &self.method
    }

    pub(crate) fn context(&self) -> &str {
        &self.context
    }

    /// A Rust identifier-safe suffix for per-context statics and helpers.
    pub(crate) fn ident(&self) -> String {
        self.context
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect()
    }
}

/// The `#[extendr]` helper that reads a R visitor scope by identity.
pub(crate) fn scope_read_helper(context: &str) -> String {
    let ident: String = context
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("alef_visitor_scope_read_{ident}")
}
