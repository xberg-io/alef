use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::backends::swift::type_map::SwiftMapper;
use crate::core::ir::{FieldDef, TypeDef};

pub(super) struct AliasDecoding {
    pub(super) preamble: String,
    pub(super) keys: HashMap<String, String>,
}

fn literal(value: &str) -> String {
    format!("\"{}\"", crate::e2e::escape::escape_swift(value))
}

fn wire_names(ty: &TypeDef, field: &FieldDef) -> Vec<String> {
    let canonical = crate::codegen::naming::wire_field_name(
        &field.name,
        field.serde_rename.as_deref(),
        ty.serde_rename_all.as_deref(),
    );
    let mut seen = BTreeSet::from([canonical.clone()]);
    let mut names = vec![canonical];
    for alias in ty
        .serde_deserialization
        .field_aliases
        .get(&field.name)
        .into_iter()
        .flatten()
    {
        if seen.insert(alias.clone()) {
            names.push(alias.clone());
        }
    }
    names
}

impl AliasDecoding {
    fn new(ty: &TypeDef, fields: &[&FieldDef]) -> anyhow::Result<Option<Self>> {
        if ty.serde_deserialization.field_aliases.is_empty() {
            return Ok(None);
        }
        for name in ty.serde_deserialization.field_aliases.keys() {
            if !fields.iter().any(|field| &field.name == name) {
                anyhow::bail!("Swift DTO '{}' has an unexposed alias target '{name}'", ty.name);
            }
        }
        let mut owners = BTreeMap::new();
        let mut known = BTreeSet::new();
        let mut keys = HashMap::new();
        let mut selections = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            let names = wire_names(ty, field);
            for name in &names {
                if let Some(owner) = owners.insert(name.clone(), field.name.clone())
                    && owner != field.name
                {
                    anyhow::bail!("Swift DTO '{}' has ambiguous input key '{name}'", ty.name);
                }
                known.insert(literal(name));
            }
            let key = if names.len() > 1 {
                format!("alefKey{index}")
            } else {
                format!("AlefWireKey(stringValue: {})!", literal(&names[0]))
            };
            keys.insert(field.name.clone(), key);
            selections.push(serde_json::json!({
                "index": index,
                "names": names.iter().map(|name| literal(name)).collect::<Vec<_>>(),
                "has_aliases": names.len() > 1,
            }));
        }
        let preamble = crate::backends::swift::template_env::render(
            "swift_alias_decoding.swift.jinja",
            crate::alef_context! {
                strict => ty.serde_deserialization.deny_unknown_fields,
                known_keys => known,
                fields => selections,
            },
        );
        Ok(Some(Self { preamble, keys }))
    }
}

pub(super) fn emit_decoder_init(
    ty: &TypeDef,
    mapper: &SwiftMapper,
    fields: &[&FieldDef],
    zero_arg_names: &HashSet<String>,
    out: &mut String,
) -> anyhow::Result<()> {
    let aliases = AliasDecoding::new(ty, fields)?;
    if let Some(aliases) = aliases {
        super::decoder::emit_decoder_init_with_aliases(
            mapper,
            fields,
            zero_arg_names,
            ty.has_default,
            out,
            super::decoder::DecoderKeys {
                strict: ty.serde_deserialization.deny_unknown_fields,
                aliases: Some(&aliases),
            },
        );
    } else {
        super::emit_decoder_init(
            mapper,
            fields,
            zero_arg_names,
            ty.has_default,
            ty.serde_deserialization.deny_unknown_fields,
            out,
        );
    }
    Ok(())
}
