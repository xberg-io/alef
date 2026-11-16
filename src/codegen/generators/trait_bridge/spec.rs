use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

use super::bridge_wrapper_name;

/// Snake-case form of a trait name, as every generated symbol that embeds the trait name spells
/// it.
///
/// Exposed as a free function so a caller that has only the trait name — a backend building its
/// `Backend::trait_bridge_registration_surface` — derives the same spelling as
/// [`TraitBridgeSpec::trait_snake`], which is what the emitters use. ~keep
pub fn trait_snake_of(trait_name: &str) -> String {
    trait_name.to_snake_case()
}

pub struct TraitBridgeSpec<'a> {
    /// The trait definition from the IR.
    pub trait_def: &'a TypeDef,
    /// Bridge configuration from `alef.toml`.
    pub bridge_config: &'a TraitBridgeConfig,
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: &'a str,
    /// Language-specific prefix for the wrapper type (e.g., `"Python"`, `"Js"`, `"Wasm"`).
    pub wrapper_prefix: &'a str,
    /// Map of type name → fully-qualified Rust path for qualifying `Named` types.
    pub type_paths: HashMap<String, String>,
    /// Set of core type names that carry a lifetime parameter (e.g. `NodeContext<'a>`).
    /// When non-empty, method signatures emit `TypeName<'_>` for these types so the
    /// generated `impl Trait for Wrapper` matches the trait definition exactly.
    pub lifetime_type_names: HashSet<String>,
    /// The crate's error type name (e.g., `"SampleCrateError"`). Defaults to `"Error"`.
    pub error_type: String,
    /// Error constructor pattern with `{msg}` and an optional `{source}`.
    pub error_constructor: String,
}

impl<'a> TraitBridgeSpec<'a> {
    /// Fully qualified error type path (e.g., `"sample_core::SampleCrateError"`).
    ///
    /// If `error_type` already looks fully-qualified (contains `::`) or is a generic
    /// type expression (contains `<`), it is returned as-is without prefixing
    /// `core_import`. This lets backends specify rich error types like
    /// `"Box<dyn std::error::Error + Send + Sync>"` directly.
    pub fn error_path(&self) -> String {
        if self.error_type.contains("::") || self.error_type.contains('<') {
            self.error_type.clone()
        } else {
            format!("{}::{}", self.core_import, self.error_type)
        }
    }

    /// Generate an error construction expression from a message expression.
    pub fn make_error(&self, msg_expr: &str) -> String {
        self.error_constructor
            .replace("{source}", "None")
            .replace("{msg}", msg_expr)
    }

    /// Pass the host error into a source-aware constructor when configured.
    pub(crate) fn make_error_with_source(&self, msg_expr: &str, source_expr: &str) -> String {
        if !self.error_constructor.contains("{source}") {
            return self.make_error(msg_expr);
        }
        let constructor = self
            .error_constructor
            .replace("{source}", "__alef_error_source")
            .replace("{msg}", "__alef_error_message");
        format!("{{ let __alef_error_message = {msg_expr}; let __alef_error_source = {source_expr}; {constructor} }}")
    }

    /// Wrapper struct name: `{prefix}{TraitName}Bridge` (e.g., `PythonOcrBackendBridge`).
    pub fn wrapper_name(&self) -> String {
        bridge_wrapper_name(self.wrapper_prefix, self.bridge_config)
    }

    /// Snake-case version of the trait name (e.g., `"ocr_backend"`).
    pub fn trait_snake(&self) -> String {
        trait_snake_of(&self.trait_def.name)
    }

    /// Full Rust path to the trait (e.g., `sample_core::OcrBackend`).
    pub fn trait_path(&self) -> String {
        self.trait_def.rust_path.replace('-', "_")
    }

    /// Methods that are required (no default impl) — must be provided by the foreign object.
    pub fn required_methods(&self) -> Vec<&'a MethodDef> {
        self.trait_def.methods.iter().filter(|m| !m.has_default_impl).collect()
    }

    /// Methods that have a default impl — optional on the foreign object.
    pub fn optional_methods(&self) -> Vec<&'a MethodDef> {
        self.trait_def.methods.iter().filter(|m| m.has_default_impl).collect()
    }
}

/// The trait's own methods that each occupy one C vtable slot, in declaration order.
///
/// Single source for the predicate every vtable emitter must agree on:
/// super-trait methods (`trait_source` set) are served by the four fixed leading
/// plugin slots rather than a per-method slot, and `ffi_skip_methods` are absent
/// from the C ABI entirely.
pub fn own_vtable_methods<'a>(trait_def: &'a TypeDef, ffi_skip_methods: &[String]) -> Vec<&'a MethodDef> {
    trait_def
        .methods
        .iter()
        .filter(|method| method.trait_source.is_none() && !ffi_skip_methods.iter().any(|skip| skip == &method.name))
        .collect()
}

/// Field names of the generated Rust vtable struct, in ABI slot order.
///
/// Mirrors what `backends::ffi::trait_bridge::vtable::gen_vtable_struct` emits:
/// the four `Plugin` slots when a super trait is configured, then one slot per
/// own method, then the two destructors.
///
/// Managed-side emitters (Java Panama, C# interop) write their upcall stubs at
/// `index * pointer_size` into a flat block, so a managed vtable that enumerates a
/// different method set than this — even one that merely reorders it — silently
/// dispatches through the wrong slot and reads past the allocation on the last
/// field. Comparing against this list is the only check that catches an omitted
/// slot; a null-pointer check on the Rust side cannot, because an omission shifts
/// a valid neighbouring pointer into the missing field rather than nulling it.
pub fn vtable_slot_names(trait_def: &TypeDef, has_super_trait: bool, ffi_skip_methods: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if has_super_trait {
        names.extend(["name_fn", "version_fn", "initialize_fn", "shutdown_fn"].map(String::from));
    }
    names.extend(
        own_vtable_methods(trait_def, ffi_skip_methods)
            .into_iter()
            .map(|method| method.name.clone()),
    );
    names.push("free_string".to_string());
    names.push("free_user_data".to_string());
    names
}

/// Return visitor callback methods configured for visitor-style bridges.
///
/// A visitor callback is an own trait method whose return type is the configured
/// visitor result type and whose parameters include the configured context type.
pub fn visitor_callback_methods<'a>(trait_def: &'a TypeDef, bridge_config: &TraitBridgeConfig) -> Vec<&'a MethodDef> {
    trait_def
        .methods
        .iter()
        .filter(|method| is_visitor_callback_method(method, bridge_config))
        .collect()
}

fn is_visitor_callback_method(method: &MethodDef, bridge_config: &TraitBridgeConfig) -> bool {
    if method.trait_source.is_some() {
        return false;
    }

    let Some(result_type) = bridge_config.result_type.as_deref() else {
        return false;
    };
    let Some(context_type) = bridge_config.context_type.as_deref() else {
        return false;
    };

    type_ref_matches_name(&method.return_type, result_type)
        && method
            .params
            .iter()
            .any(|param| type_ref_matches_name(&param.ty, context_type))
}

fn type_ref_matches_name(ty: &TypeRef, name: &str) -> bool {
    match ty {
        TypeRef::Named(type_name) => type_name == name,
        TypeRef::Optional(inner) => type_ref_matches_name(inner, name),
        _ => false,
    }
}
