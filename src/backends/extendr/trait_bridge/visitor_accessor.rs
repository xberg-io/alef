//! The opt-in lazy visitor-context accessor for the R backend (ADR 0019).
//!
//! Split out of `trait_bridge.rs` to keep that file under the size cap.

/// The context stays a named list. The opted-in element is a function the host calls as
/// `ctx$attributes()`: it captures the borrowed context's address, calls the registered
/// `#[extendr]` reader with it, and that reader only builds the named list when called.
pub(super) fn accessor_support(
    spec: &crate::codegen::visitor_accessor::VisitorAccessorSpec,
    core_type: &str,
) -> (String, String) {
    let ident = spec.ident();
    let method = spec.method();
    let scope = format!("ALEF_VISITOR_SCOPE_{}", ident.to_ascii_uppercase());
    let helper = crate::codegen::visitor_accessor::scope_read_helper(spec.context());
    let runtime = crate::codegen::visitor_accessor::SCOPE_RUNTIME;
    let accessor_lines = format!(
        r#"        ("{method}", {{
            let __alef_accessor_id = ctx as *const {core_type} as usize;
            extendr_api::functions::eval_string_with_params(
                "local({{ id <- param.0; function() .Call(\"wrap__{helper}\", id) }})",
                &[&extendr_api::Robj::from(__alef_accessor_id as f64)],
            ).unwrap_or_else(|_| extendr_api::Robj::from(extendr_api::NULL))
        }}),"#
    );
    let scope_lines = format!(
        r#"
mod alef_visitor_scope_{ident} {{
{runtime}
}}

std::thread_local! {{
    static {scope}: alef_visitor_scope_{ident}::CallbackAccessorScope<std::collections::HashMap<String, String>> =
        const {{ alef_visitor_scope_{ident}::CallbackAccessorScope::new() }};
}}

#[extendr]
fn {helper}(id: f64) -> extendr_api::prelude::List {{
    let map = {scope}.with(|scope| scope.read(id as usize)).unwrap_or_default();
    extendr_api::prelude::List::from_pairs(
        map.iter()
            .map(|(key, value)| (key.as_str(), extendr_api::Robj::from(value.as_str())))
            .collect::<Vec<(&str, extendr_api::Robj)>>(),
    )
}}

fn alef_call_with_scope_{ident}<R>(ctx: &{core_type}, call: impl FnOnce() -> R) -> R {{
    alef_visitor_scope_{ident}::CallbackAccessorScope::with(
        &{scope},
        ctx as *const {core_type} as usize,
        || ctx.{method}().iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
        call,
    )
}}"#
    );
    (accessor_lines, scope_lines)
}
