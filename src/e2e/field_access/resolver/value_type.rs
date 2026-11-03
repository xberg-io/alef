use crate::core::ir::TypeRef;

use super::super::types::FieldResolver;

impl FieldResolver {
    /// The declared IR type of the value `field` reads from the anchored result type, when the IR
    /// resolves the path. An empty path names the whole call result. Backends map the declared
    /// type through their own binding type decision.
    pub fn target_value_type(&self, field: &str) -> Option<&TypeRef> {
        if field.is_empty() {
            return self.declared_result_value_type.as_ref();
        }
        let path = self.result_relative_path(field);
        super::super::ir_result_fields::value_type_at_path(&self.ir_result_field_map, &path).or_else(|| {
            let root = self.ir_result_field_map.root_type.as_deref()?;
            self.target_value_type_from(root, &path)
        })
    }

    fn target_value_type_from(&self, root: &str, path: &str) -> Option<&TypeRef> {
        if let Some((_, union, variant, suffix)) = self.ir_tagged_union_split_from(root, path) {
            let definition = self.enum_definition(&union)?;
            let variant = definition.variants.iter().find(|candidate| candidate.name == variant)?;
            let [payload] = variant.fields.as_slice() else {
                return None;
            };
            return self.target_value_type_through(&payload.ty, &suffix);
        }
        if let Some(definition) = self.enum_definition(root) {
            let (head, tail) = path.split_once('.').unwrap_or((path, ""));
            let metadata = self.enum_field_metadata(root, head)?;
            let variant = definition
                .variants
                .iter()
                .find(|variant| variant.name == metadata.variant)?;
            let field = variant.fields.iter().find(|field| field.name == metadata.field)?;
            return self.target_value_type_through(&field.ty, tail);
        }
        super::super::ir_result_fields::value_type_at_path_from(&self.ir_result_field_map, root, path)
    }

    fn target_value_type_through<'a>(&'a self, declared: &'a TypeRef, path: &str) -> Option<&'a TypeRef> {
        if path.is_empty() {
            return Some(declared);
        }
        match declared {
            TypeRef::Optional(inner) => self.target_value_type_through(inner, path),
            TypeRef::Named(owner) => self.target_value_type_from(owner, path),
            TypeRef::Vec(_) => {
                let (indices, tail) = path.split_once('.').unwrap_or((path, ""));
                let mut element = declared;
                for _ in super::super::enum_payload_access::numeric_indices(indices)? {
                    element = super::super::ir_result_fields::vec_element_type(element)?;
                }
                self.target_value_type_through(element, tail)
            }
            _ => None,
        }
    }
}
