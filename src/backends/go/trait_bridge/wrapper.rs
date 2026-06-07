use super::helpers::rust_to_go_type;
use crate::core::ir::{MethodDef, TypeRef};
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
