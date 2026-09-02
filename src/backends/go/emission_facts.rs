use std::collections::HashSet;

use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, TypeDef};

pub(crate) struct GoEmissionFacts<'a> {
    pub(crate) structs: HashSet<&'a str>,
    pub(crate) opaque: HashSet<&'a str>,
    pub(crate) enums: HashSet<&'a str>,
    pub(crate) unit_enums: HashSet<&'a str>,
    pub(crate) passthrough_enums: HashSet<&'a str>,
    pub(crate) data_enums: HashSet<&'a str>,
    /// Enums [`super::go_enum_representation`] classifies as
    /// [`super::GoEnumRepresentation::ExternallyTaggedStruct`] -- `gen_externally_tagged_union_type`
    /// emits one struct field per variant, unconditionally typed `*<payload struct>` (every
    /// variant but the active one is absent, so every field must tolerate nil). Recorded here so
    /// `e2e::field_access::ir_result_fields` can extend its owner/field walk across a tagged-union
    /// projection like `format.excel` with the SAME authority a literal struct field gets, instead
    /// of guessing: the pointer-ness is a fact this backend's own generator template always
    /// produces, not an inference over incomplete information. ~keep
    pub(crate) externally_tagged_struct_enums: HashSet<&'a str>,
}

impl<'a> GoEmissionFacts<'a> {
    pub(crate) fn from_config(types: &'a [TypeDef], enums: &'a [EnumDef], config: &ResolvedCrateConfig) -> Self {
        let excluded = excluded_type_names(config, types);
        let visitor_owned = visitor_owned_type_names(config);
        Self::new(types, enums, excluded, visitor_owned)
    }

    pub(crate) fn new(
        types: &'a [TypeDef],
        enums: &'a [EnumDef],
        excluded: HashSet<String>,
        visitor_owned: HashSet<String>,
    ) -> Self {
        let type_is_emitted = |definition: &&TypeDef| {
            !definition.is_trait && !excluded.contains(&definition.name) && !visitor_owned.contains(&definition.name)
        };
        let enum_is_emitted =
            |definition: &&EnumDef| !excluded.contains(&definition.name) && !visitor_owned.contains(&definition.name);
        let emitted_types: Vec<&TypeDef> = types.iter().filter(type_is_emitted).collect();
        let emitted_enums: Vec<&EnumDef> = enums.iter().filter(enum_is_emitted).collect();
        Self {
            structs: emitted_types
                .iter()
                .filter(|definition| !definition.is_opaque)
                .map(|definition| definition.name.as_str())
                .collect(),
            opaque: emitted_types
                .iter()
                .filter(|definition| definition.is_opaque)
                .map(|definition| definition.name.as_str())
                .collect(),
            enums: emitted_enums
                .iter()
                .map(|definition| definition.name.as_str())
                .collect(),
            unit_enums: emitted_enums
                .iter()
                .filter(|definition| super::is_unit_struct_field_enum(definition))
                .map(|definition| definition.name.as_str())
                .collect(),
            passthrough_enums: emitted_enums
                .iter()
                .filter(|definition| super::is_passthrough_raw_message_enum(definition))
                .map(|definition| definition.name.as_str())
                .collect(),
            data_enums: emitted_enums
                .iter()
                .filter(|definition| super::is_data_interface_struct_field_enum(definition))
                .map(|definition| definition.name.as_str())
                .collect(),
            externally_tagged_struct_enums: emitted_enums
                .iter()
                .filter(|definition| {
                    super::go_enum_representation(definition) == super::GoEnumRepresentation::ExternallyTaggedStruct
                })
                .map(|definition| definition.name.as_str())
                .collect(),
        }
    }

    pub(crate) fn emits_type(&self, name: &str) -> bool {
        self.structs.contains(name) || self.opaque.contains(name)
    }
}

pub(crate) fn excluded_type_names(config: &ResolvedCrateConfig, types: &[TypeDef]) -> HashSet<String> {
    let mut names: HashSet<String> = config
        .ffi
        .as_ref()
        .map(|ffi| ffi.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(go) = &config.go {
        names.extend(go.exclude_types.iter().cloned());
    }
    names.extend(
        types
            .iter()
            .filter(|definition| definition.binding_excluded)
            .map(|definition| definition.name.clone()),
    );
    names.extend(
        config
            .opaque_types
            .iter()
            .filter(|(_, path)| path.contains('<'))
            .map(|(name, _)| name.clone()),
    );
    names
}

pub(crate) fn visitor_owned_type_names(config: &ResolvedCrateConfig) -> HashSet<String> {
    let has_bridge_parameter = config.trait_bridges.iter().any(|bridge| bridge.param_name.is_some());
    let has_options_bridge = config
        .trait_bridges
        .iter()
        .any(|bridge| bridge.bind_via == BridgeBinding::OptionsField && bridge.is_active_for("go"));
    if has_bridge_parameter || has_options_bridge {
        config.bridge_associated_types()
    } else {
        HashSet::new()
    }
}
