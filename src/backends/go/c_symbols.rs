//! Go's spelling of the C symbols the FFI backend exports.
//!
//! Go is a *consumer* of the C ABI: every `C.foo_bar_baz(...)` it emits must name a symbol the
//! FFI backend actually exported, or cgo fails to link. The FFI backend spells those symbols
//! through [`crate::codegen::c_consumer`], so this module asks that same helper rather than
//! re-deriving the name from the IR. Two independent derivations of one symbol is the defect
//! shape the `two-generators-disagree` skill describes, and it bites exactly on the names a
//! casing helper disagrees about — an acronym, a digit run, a leading underscore. ~keep

use crate::codegen::c_consumer;
use crate::codegen::naming::pascal_to_snake;

/// Cgo call target for a generated free function: `{prefix}_{function_snake}`.
pub(crate) fn free_function_symbol(ffi_prefix: &str, function_name: &str) -> String {
    c_consumer::free_function_symbol(ffi_prefix, function_name)
}

/// Cgo call target for a method generated on a type: `{prefix}_{type_snake}_{method_name}`.
pub(crate) fn method_symbol(ffi_prefix: &str, type_name: &str, method_name: &str) -> String {
    c_consumer::method_symbol(ffi_prefix, type_name, method_name)
}

/// Cgo call target for one operation (`start`, `next`, `free`) of a streaming adapter.
pub(crate) fn stream_adapter_symbol(ffi_prefix: &str, owner_type: &str, adapter_name: &str, operation: &str) -> String {
    c_consumer::stream_adapter_symbol(ffi_prefix, owner_type, adapter_name, operation)
}

/// Cgo call target for a service constructor: `{prefix_lower}_{service_snake}_new`.
pub(crate) fn service_new_symbol(ffi_prefix: &str, service_name: &str) -> String {
    c_consumer::service_new_symbol(ffi_prefix, service_name)
}

/// Cgo call target for a service destructor: `{prefix_lower}_{service_snake}_free`.
pub(crate) fn service_free_symbol(ffi_prefix: &str, service_name: &str) -> String {
    c_consumer::service_free_symbol(ffi_prefix, service_name)
}

/// Cgo call target for a handler-registration entry point.
pub(crate) fn service_register_symbol(ffi_prefix: &str, service_name: &str, method_name: &str) -> String {
    c_consumer::service_register_symbol(ffi_prefix, service_name, method_name)
}

/// Cgo call target for a registration-variant shortcut or a configurator method.
pub(crate) fn service_method_symbol(ffi_prefix: &str, service_name: &str, method_name: &str) -> String {
    c_consumer::service_method_symbol(ffi_prefix, service_name, method_name)
}

/// Cgo call target for a run/finalize entry point.
pub(crate) fn service_entrypoint_symbol(ffi_prefix: &str, service_name: &str, method_name: &str) -> String {
    c_consumer::service_entrypoint_symbol(ffi_prefix, service_name, method_name)
}

/// Cgo call target for a trait bridge's `register` entry point.
///
/// The FFI backend names this symbol from the bridge's configured `register_fn`, not from the
/// trait name, so Go must pass the configured name through rather than re-derive
/// `register_{trait_snake}` — a bridge whose `register_fn` spells anything else links against
/// nothing. ~keep
pub(crate) fn trait_register_symbol(ffi_prefix: &str, register_fn: &str) -> String {
    c_consumer::trait_register_symbol(ffi_prefix, register_fn)
}

/// Cgo call target for a trait bridge's `unregister` entry point.
pub(crate) fn trait_unregister_symbol(ffi_prefix: &str, trait_name: &str) -> String {
    c_consumer::trait_unregister_symbol(ffi_prefix, trait_name)
}

/// Cgo call target for a trait bridge's `clear` entry point.
pub(crate) fn trait_clear_symbol(ffi_prefix: &str, trait_name: &str) -> String {
    c_consumer::trait_clear_symbol(ffi_prefix, trait_name)
}

/// Cgo call target for the FFI backend's string-deallocation function.
pub(crate) fn free_string_symbol(ffi_prefix: &str) -> String {
    c_consumer::free_string_symbol(ffi_prefix)
}

/// Cgo call target for reading the thread-local last-error code.
pub(crate) fn last_error_code_symbol(ffi_prefix: &str) -> String {
    c_consumer::last_error_code_symbol(ffi_prefix)
}

/// Cgo call target for reading the thread-local last-error context message.
pub(crate) fn last_error_context_symbol(ffi_prefix: &str) -> String {
    c_consumer::last_error_context_symbol(ffi_prefix)
}

/// Cgo call target for reading the variant name of the thread-local last error.
pub(crate) fn last_error_variant_symbol(ffi_prefix: &str) -> String {
    c_consumer::last_error_variant_symbol(ffi_prefix)
}

/// Cgo call target for reading one captured error-method value of the last error.
pub(crate) fn last_error_field_symbol(ffi_prefix: &str, method_name: &str) -> String {
    c_consumer::last_error_field_symbol(ffi_prefix, method_name)
}

/// The name of the `static inline` cgo helper Go declares in its own preamble to heap-allocate a
/// vtable struct: `{prefix}_{trait_snake}_vtable_new`.
///
/// Unlike everything else in this module this is **not** an FFI-exported symbol — nothing in
/// `backends::ffi` emits it. It is private to the generated Go file, declared in
/// `vtable_constructor_helper.jinja` and called from `vtable_allocation_via_c_helper.jinja`, and
/// lives here so those two sites cannot spell it differently. ~keep
pub(crate) fn go_vtable_constructor_symbol(ffi_prefix: &str, trait_name: &str) -> String {
    format!("{ffi_prefix}_{}_vtable_new", heck::AsSnakeCase(trait_name))
}

/// The type component the FFI backend folds into a C symbol.
///
/// Templates that compose `C.{{ ffi_prefix }}_{{ type_snake }}_...` themselves need the
/// component rather than a whole symbol. `type_component_matches_the_symbol_helper` below pins
/// it against [`method_symbol`], so it cannot drift from the spelling the FFI backend exports. ~keep
pub(crate) fn type_component(type_name: &str) -> String {
    pascal_to_snake(type_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #524 (drift risk, not a live defect): `free_string_symbol`/`last_error_code_symbol`/
    /// `last_error_context_symbol` must delegate to `c_consumer`, not reimplement the formula —
    /// otherwise the ~10 templates and Rust call sites that ask this module for the symbol name
    /// can silently diverge from the FFI backend again if `c_consumer`'s spelling ever changes.
    /// The literals below document today's byte-identical output; they are not proof of a bug,
    /// since `c_consumer`'s formula was already bare `format!("{prefix}_...")` before this
    /// refactor. ~keep
    #[test]
    fn free_string_and_last_error_symbols_delegate_to_c_consumer() {
        assert_eq!(free_string_symbol("sample_core"), "sample_core_free_string");
        assert_eq!(last_error_code_symbol("sample_core"), "sample_core_last_error_code");
        assert_eq!(
            last_error_context_symbol("sample_core"),
            "sample_core_last_error_context"
        );
    }

    /// Names chosen to discriminate between `heck::ToSnakeCase` (what Go used to apply) and the
    /// ABI helpers: consecutive capitals, an embedded acronym, digit boundaries, a leading
    /// underscore, and a single-letter segment. A table of ordinary `snake_case` names would
    /// pass under either derivation and prove nothing. ~keep
    const ADVERSARIAL_TYPE_NAMES: &[&str] = &[
        "HTTPServer",
        "URLPath",
        "UTF8Length",
        "Base64Encode",
        "_Internal",
        "AClient",
        "JSONLD",
        "already_snake",
    ];

    const ADVERSARIAL_METHOD_NAMES: &[&str] = &[
        "parseURLPath",
        "utf8Length",
        "Base64Encode",
        "_hidden",
        "a",
        "to_json",
        "parse__inner",
    ];

    #[test]
    fn type_component_matches_the_symbol_helper() {
        for type_name in ADVERSARIAL_TYPE_NAMES {
            assert_eq!(
                method_symbol("p", type_name, "m"),
                format!("p_{}_m", type_component(type_name)),
                "`{type_name}` component drifted from the symbol helper"
            );
        }
    }

    /// The rows above must actually discriminate. If `heck` agreed with the ABI helpers on
    /// every one of them, the parity tests would pass no matter which derivation Go used —
    /// a green result proving nothing. ~keep
    #[test]
    fn adversarial_rows_discriminate_against_the_heck_derivation() {
        use heck::ToSnakeCase;

        let divergent_methods: Vec<&str> = ADVERSARIAL_METHOD_NAMES
            .iter()
            .copied()
            .filter(|name| *name != name.to_snake_case())
            .collect();
        assert_eq!(
            divergent_methods,
            vec!["parseURLPath", "utf8Length", "Base64Encode", "_hidden", "parse__inner"],
            "the method rows that discriminate against heck changed"
        );

        let divergent_types: Vec<&str> = ADVERSARIAL_TYPE_NAMES
            .iter()
            .copied()
            .filter(|name| type_component(name) != name.to_snake_case())
            .collect();
        assert_eq!(
            divergent_types,
            vec!["_Internal"],
            "the type rows that discriminate against heck changed"
        );
    }
}
