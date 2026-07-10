use super::helpers::rust_to_go_type;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToPascalCase;

/// Generate the Go interface method signature for a trait method.
pub(super) fn gen_interface_method(out: &mut String, method: &MethodDef) {
    let mut params = Vec::new();
    for p in &method.params {
        let go_type = rust_to_go_type(&p.ty);
        params.push(format!("{} {}", p.name, go_type));
    }

    let return_type = if method.error_type.is_some() {
        match &method.return_type {
            TypeRef::Unit => "error".to_string(),
            _ => {
                let ret = rust_to_go_type(&method.return_type);
                format!("({}, error)", ret)
            }
        }
    } else {
        rust_to_go_type(&method.return_type)
    };

    let params_str = params.join(", ");
    out.push_str(&crate::backends::go::template_env::render(
        "trait_interface_method.jinja",
        minijinja::context! {
            doc => &method.name,
            method_name => method.name.to_pascal_case(),
            params => params_str,
            return_type => return_type,
        },
    ));
    out.push('\n');
}

/// Generate the Path A Bridge struct and its methods.
///
/// The Bridge struct wraps a user-implemented interface and delegates all trait method calls to it.
/// This pattern allows Go's structural interface satisfaction to work with hand-authored plugin
/// interfaces that may not match the auto-generated trait interface exactly.
pub(super) fn gen_bridge_wrapper(
    out: &mut String,
    trait_def: &TypeDef,
    trait_name: &str,
    ffi_skip_methods: &[String],
    excluded_named_types: &std::collections::HashSet<&str>,
) {
    let trait_pascal = trait_name.to_pascal_case();
    let bridge_name = format!("{}Bridge", trait_pascal);

    out.push_str(&format!(
        "// {}Bridge wraps a {} implementation and exposes it to the C plugin system.\n",
        trait_pascal, trait_name
    ));
    out.push_str(&format!("type {} struct {{\n", bridge_name));
    out.push_str(&format!("\timpl {}\n", trait_name));
    out.push_str("}\n\n");

    out.push_str(&format!(
        "// New{} creates a new Bridge wrapping the given implementation.\n",
        bridge_name
    ));
    out.push_str(&format!(
        "func New{}(impl {}) *{} {{\n",
        bridge_name, trait_name, bridge_name
    ));
    out.push_str(&format!("\treturn &{}{{impl: impl}}\n", bridge_name));
    out.push_str("}\n\n");

    gen_bridge_lifecycle_method(out, &bridge_name, "Name", "string");
    gen_bridge_lifecycle_method(out, &bridge_name, "Version", "string");
    gen_bridge_lifecycle_method(out, &bridge_name, "Initialize", "error");
    gen_bridge_lifecycle_method(out, &bridge_name, "Shutdown", "error");

    for method in &trait_def.methods {
        if ffi_skip_methods.contains(&method.name) {
            continue;
        }
        gen_bridge_trait_method(out, &bridge_name, method, excluded_named_types);
    }
}

/// Generate a delegating bridge method for plugin lifecycle methods.
fn gen_bridge_lifecycle_method(out: &mut String, bridge_name: &str, method_name: &str, return_type: &str) {
    out.push_str(&format!(
        "func (b *{}) {}() {} {{\n",
        bridge_name, method_name, return_type
    ));
    out.push_str(&format!("\treturn b.impl.{}()\n", method_name));
    out.push_str("}\n\n");
}

/// Generate a delegating bridge method for a trait method.
fn gen_bridge_trait_method(
    out: &mut String,
    bridge_name: &str,
    method: &MethodDef,
    excluded_named_types: &std::collections::HashSet<&str>,
) {
    let method_pascal = method.name.to_pascal_case();
    let mut params = Vec::new();
    for p in &method.params {
        let go_type = rust_to_go_type_with_excluded(&p.ty, excluded_named_types);
        params.push(format!("{} {}", p.name, go_type));
    }

    let return_type = if method.error_type.is_some() {
        match &method.return_type {
            TypeRef::Unit => "error".to_string(),
            _ => {
                let ret = rust_to_go_type_with_excluded(&method.return_type, excluded_named_types);
                format!("({}, error)", ret)
            }
        }
    } else {
        rust_to_go_type_with_excluded(&method.return_type, excluded_named_types)
    };

    let params_str = params.join(", ");
    let params_call = method
        .params
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&format!(
        "func (b *{}) {}({}) {} {{\n",
        bridge_name, method_pascal, params_str, return_type
    ));
    out.push_str(&format!("\treturn b.impl.{}({})\n", method_pascal, params_call));
    out.push_str("}\n\n");
}

/// Map a TypeRef to Go type, handling excluded types as json.RawMessage.
fn rust_to_go_type_with_excluded(ty: &TypeRef, excluded_named_types: &std::collections::HashSet<&str>) -> String {
    match ty {
        TypeRef::Named(name) if excluded_named_types.contains(name.as_str()) => "json.RawMessage".to_string(),
        _ => rust_to_go_type(ty),
    }
}
