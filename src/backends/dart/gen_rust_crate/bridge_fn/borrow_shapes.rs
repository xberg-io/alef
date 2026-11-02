use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use syn::visit::Visit;

#[derive(Clone, Copy)]
pub(super) enum Shape {
    Owned,
    Reference,
    OptionalReference,
}

pub(super) fn classify(param: &ParamDef) -> anyhow::Result<Shape> {
    if let Some(source) = param
        .original_type
        .as_deref()
        .and_then(|s| syn::parse_str::<syn::Type>(s).ok())
    {
        let (shape, inner) = match &source {
            syn::Type::Reference(reference) => (Shape::Reference, reference.elem.as_ref()),
            syn::Type::Path(path) => {
                let optional_reference =
                    path.path
                        .segments
                        .last()
                        .filter(|s| s.ident == "Option")
                        .and_then(|segment| {
                            let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
                                return None;
                            };
                            args.args.iter().find_map(|a| match a {
                                syn::GenericArgument::Type(syn::Type::Reference(r)) => Some(r),
                                _ => None,
                            })
                        });
                if let Some(reference) = optional_reference {
                    (Shape::OptionalReference, reference.elem.as_ref())
                } else {
                    (Shape::Owned, &source)
                }
            }
            _ => (Shape::Owned, &source),
        };
        let mut visitor = References(false);
        visitor.visit_type(inner);
        if visitor.0 {
            anyhow::bail!("unsupported interior reference in Dart parameter `{}`", param.name);
        }
        return Ok(shape);
    }
    if param.vec_inner_is_ref {
        anyhow::bail!("unsupported interior reference in Dart parameter `{}`", param.name);
    }
    if param.is_ref && param.optional {
        anyhow::bail!(
            "Dart parameter `{}` requires original source borrow placement",
            param.name
        );
    }
    Ok(if param.is_ref { Shape::Reference } else { Shape::Owned })
}

struct References(bool);
impl<'ast> Visit<'ast> for References {
    fn visit_type_reference(&mut self, _: &'ast syn::TypeReference) {
        self.0 = true;
    }
}

pub(super) fn validate_opaque(
    function: &FunctionDef,
    opaque: &std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    for parameter in &function.params {
        if contains_opaque(&parameter.ty, opaque) {
            let shape = classify(parameter)?;
            if !matches!(shape, Shape::Owned) && (parameter.optional || !matches!(parameter.ty, TypeRef::Named(_))) {
                anyhow::bail!(
                    "Dart function `{}` has unsupported container/optional opaque borrow `{}`",
                    function.name,
                    parameter.name
                );
            }
        }
    }
    Ok(())
}

fn contains_opaque(ty: &TypeRef, opaque: &std::collections::HashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => opaque.contains(name),
        TypeRef::Vec(inner) | TypeRef::Optional(inner) => contains_opaque(inner, opaque),
        TypeRef::Map(key, value) => contains_opaque(key, opaque) || contains_opaque(value, opaque),
        _ => false,
    }
}
