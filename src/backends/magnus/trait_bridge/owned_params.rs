use crate::backends::magnus::template_env;
use crate::core::ir::{MethodDef, TypeRef};

pub(super) fn owned_param_bindings(method: &MethodDef, suffix: &str) -> String {
    method
        .params
        .iter()
        .filter(|param| param.is_ref || !suffix.is_empty())
        .map(|param| {
            let conversion = if !param.is_ref {
                param.name.clone()
            } else {
                match &param.ty {
                    TypeRef::String => format!("{}.to_string()", param.name),
                    TypeRef::Bytes => format!("{}.to_vec()", param.name),
                    TypeRef::Path => format!("{}.to_path_buf()", param.name),
                    _ => format!("{}.clone()", param.name),
                }
            };

            template_env::render(
                "trait_bridge_owned_binding.rs.jinja",
                minijinja::context! {
                    name => param.name.as_str(),
                    suffix => suffix,
                    conversion => conversion,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::owned_param_bindings;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};

    fn method(params: Vec<ParamDef>) -> MethodDef {
        MethodDef {
            params,
            ..Default::default()
        }
    }

    fn param(name: &str, ty: TypeRef, is_ref: bool) -> ParamDef {
        ParamDef {
            name: name.into(),
            ty,
            is_ref,
            ..Default::default()
        }
    }

    #[test]
    fn sync_bindings_only_clone_borrowed_parameters() {
        let output = owned_param_bindings(
            &method(vec![
                param("owned", TypeRef::String, false),
                param("text", TypeRef::String, true),
                param("bytes", TypeRef::Bytes, true),
                param("path", TypeRef::Path, true),
                param("named", TypeRef::Named("Request".into()), true),
            ]),
            "",
        );

        assert!(!output.contains("let owned ="));
        assert!(output.contains("let text = text.to_string();"));
        assert!(output.contains("let bytes = bytes.to_vec();"));
        assert!(output.contains("let path = path.to_path_buf();"));
        assert!(output.contains("let named = named.clone();"));
    }

    #[test]
    fn async_bindings_own_both_borrowed_and_owned_parameters() {
        let output = owned_param_bindings(
            &method(vec![
                param("owned", TypeRef::String, false),
                param("text", TypeRef::String, true),
            ]),
            "_owned",
        );

        assert!(output.contains("let owned_owned = owned;"));
        assert!(output.contains("let text_owned = text.to_string();"));
    }
}
