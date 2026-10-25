use super::FieldResolver;

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

    /// The WASM expression that reads the whole union behind a text-projected field.
    ///
    /// The ordinary `content` getter of such a field returns the display text, so the parts of a
    /// list are gone from it. The binding backend adds a lossless `<field>Value` getter beside it
    /// for every serializable union (`backends::wasm::gen_bindings::types_accessors`), and this method spells that
    /// getter. `prefix` is the path of the field, `owner` the union type it holds. The method
    /// answers `None` for a union without serde support, because the backend emits no getter then.
    /// ~keep
    pub(crate) fn wasm_text_value_accessor(&self, prefix: &str, owner: &str, result: &str) -> Option<String> {
        if !self.is_text_projected_enum(owner) || !self.enum_definition(owner)?.has_serde {
            return None;
        }
        let (parent, leaf) = prefix
            .rsplit_once('.')
            .map_or((None, prefix), |(parent, leaf)| (Some(parent), leaf));
        if leaf.is_empty() || leaf.contains('[') {
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
