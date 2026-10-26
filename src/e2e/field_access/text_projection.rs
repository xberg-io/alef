use super::FieldResolver;
use super::ir_enum::leaf_owner_type_at_path;

impl FieldResolver {
    pub(crate) fn with_text_projected_enums(mut self, names: &[String]) -> Self {
        self.text_projected_enum_names = names.iter().cloned().collect();
        self
    }

    pub(crate) fn is_text_projected_enum(&self, name: &str) -> bool {
        self.text_projected_enum_names.contains(name)
    }

    pub(crate) fn text_projected_crossing(&self, path: &str) -> Option<String> {
        let (_, owner, _, _) = self.ir_tagged_union_split(self.resolve(path))?;
        self.is_text_projected_enum(&owner).then_some(owner)
    }

    /// Record which `(type, field)` pairs get a lossless `<field>Value` getter in the WASM binding.
    ///
    /// The set comes from `text_value_enum_names` and `text_value_companion`, the functions the WASM
    /// backend uses to decide which getters it emits, so the two cannot disagree. Call this after
    /// `with_text_projected_enums`. `types` are the types the binding emits and `exclude_types`
    /// are the type names the binding leaves out. ~keep
    pub(crate) fn with_wasm_text_value_fields(
        mut self,
        types: &[crate::core::ir::TypeDef],
        enums: &[crate::core::ir::EnumDef],
        exclude_types: &[String],
    ) -> Self {
        use crate::codegen::conversions::helpers::{text_value_companion, text_value_enum_names};
        let text_names: ahash::AHashSet<String> = self.text_projected_enum_names.iter().cloned().collect();
        let names = text_value_enum_names(enums, &text_names, exclude_types);
        self.wasm_text_value_fields = types
            .iter()
            .filter(|definition| !exclude_types.contains(&definition.name))
            .flat_map(|definition| {
                let names = &names;
                definition.fields.iter().filter_map(move |field| {
                    text_value_companion(&definition.fields, field, names)
                        .map(|_| (definition.name.clone(), field.name.clone()))
                })
            })
            .collect();
        self
    }

    /// The WASM expression that reads the whole union behind a text-projected field.
    ///
    /// The ordinary `content` getter of such a field returns the display text, so the parts of a
    /// list are gone from it. The binding backend adds a lossless `<field>Value` getter beside it
    /// for every serializable union (`backends::wasm::gen_bindings::types_accessors`), and this method spells that
    /// getter. `prefix` is the path of the field, `owner` the union type it holds. The method
    /// answers `None` for a field the backend gives no getter: an unserializable or excluded union,
    /// or a cfg-gated or sanitized field.
    /// ~keep
    pub(crate) fn wasm_text_value_accessor(&self, prefix: &str, owner: &str, result: &str) -> Option<String> {
        if !self.is_text_projected_enum(owner) {
            return None;
        }
        let (parent, leaf) = prefix
            .rsplit_once('.')
            .map_or((None, prefix), |(parent, leaf)| (Some(parent), leaf));
        if leaf.is_empty() || leaf.contains('[') {
            return None;
        }
        let declaring_type = leaf_owner_type_at_path(&self.ir_enum_map, prefix)?;
        if !self
            .wasm_text_value_fields
            .contains(&(declaring_type, leaf.to_string()))
        {
            return None;
        }
        let getter = crate::codegen::naming::to_node_name(&format!("{leaf}_value"));
        let base = parent.map_or_else(
            || result.to_string(),
            |parent| self.accessor(parent, "typescript", result),
        );
        let access = if base.contains("?.") { "?." } else { "." };
        Some(format!("{base}{access}{getter}"))
    }

    /// True when a WASM assertion on `path` can read the parts of a text-projected union through
    /// the lossless getter, so the generator must not refuse it.
    pub(crate) fn wasm_text_value_readable(&self, path: &str) -> bool {
        let Some((prefix, owner, variant, _)) = self.ir_tagged_union_split(self.resolve(path)) else {
            return false;
        };
        self.union_variant_payload_is_collection(&owner, &variant)
            && self.wasm_text_value_accessor(&prefix, &owner, "result").is_some()
    }
}
