use crate::core::ir::TypeDef;
use heck::ToSnakeCase;
use std::collections::HashSet;

/// Emit no-op method shims for deferred-empty-handle types that have no methods.
///
/// These types are declared in the functions extern block because they're returned
/// by value and have empty type blocks. The no-op method in the extern block
/// signals to swift-bridge to generate $_free, and this provides the matching Rust
/// implementation so the declaration compiles.
pub(super) fn emit_shims(deferred_empty_handle_types: &HashSet<String>, visible_types: &[&TypeDef]) -> String {
    let mut out = String::new();
    for ty_name in deferred_empty_handle_types {
        let type_snake = ty_name.to_snake_case();
        let noop_fn_name = format!("{type_snake}_noop");
        if let Some(ty) = visible_types.iter().find(|t| t.name == *ty_name) {
            if let Some(cfg) = ty.cfg.as_deref() {
                out.push_str(&format!("#[cfg({cfg})]\n"));
            }
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "rust_wrapper_free_fn.rs.jinja",
            minijinja::context! {
                fn_name => &noop_fn_name,
                params => format!("client: &{ty_name}"),
                return_clause => "",
                body => "    // No-op method for swift-bridge destructor synthesis",
            },
        ));
        out.push('\n');
    }
    out
}
