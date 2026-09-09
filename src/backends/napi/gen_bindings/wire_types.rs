//! Type declarations for values that bypass napi objects and go directly through serde JSON.

use super::errors::dts_type;
use crate::codegen::naming::ts_property_key::ts_property_key;
use crate::codegen::naming::{node_type_name, wire_field_name, wire_variant_value};
use crate::codegen::serde_enum_repr::{SerdeEnumRepr, serde_enum_repr};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct WireTypes<'a> {
    api: &'a ApiSurface,
    names: BTreeMap<String, String>,
    used_names: BTreeSet<String>,
    declarations: Vec<String>,
}

impl<'a> WireTypes<'a> {
    pub(super) fn new(api: &'a ApiSurface) -> Self {
        Self {
            api,
            names: BTreeMap::new(),
            used_names: api
                .types
                .iter()
                .map(|t| node_type_name(&t.name).to_string())
                .chain(api.enums.iter().map(|e| node_type_name(&e.name).to_string()))
                .collect(),
            declarations: Vec::new(),
        }
    }

    pub(super) fn enum_members(&mut self, definition: &EnumDef) -> Vec<String> {
        definition
            .variants
            .iter()
            .filter(|variant| !variant.binding_excluded)
            .map(|variant| self.variant(definition, variant))
            .collect()
    }

    fn enum_body(&mut self, definition: &EnumDef) -> String {
        self.enum_members(definition).join(" | ")
    }

    pub(super) fn declarations(self) -> Vec<String> {
        self.declarations
    }

    fn ty(&mut self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Named(name) => self.named(name),
            TypeRef::Optional(inner) => format!("({} | null)", self.ty(inner)),
            TypeRef::Vec(inner) => format!("Array<{}>", self.ty(inner)),
            TypeRef::Map(key, value) => format!("Record<{}, {}>", self.ty(key), self.ty(value)),
            TypeRef::Bytes => "Array<number>".into(),
            TypeRef::Unit => "null".into(),
            TypeRef::Duration => "{ secs: number; nanos: number }".into(),
            _ => dts_type(ty),
        }
    }

    fn named(&mut self, name: &str) -> String {
        if let Some(alias) = self.names.get(name) {
            return alias.clone();
        }
        let definition = self.api.types.iter().find(|definition| definition.name == name);
        let enumeration = self.api.enums.iter().find(|definition| definition.name == name);
        if definition.is_none() && enumeration.is_none() {
            return node_type_name(name).to_string();
        }
        // ~keep Register before descending: recursive payloads refer to an alias, never expand recursively.
        let mut alias = format!("__AlefWire{}", node_type_name(name));
        while !self.used_names.insert(alias.clone()) {
            alias.push('_');
        }
        self.names.insert(name.into(), alias.clone());
        let body = if let Some(definition) = definition {
            if definition.serde_container_conversion.transparent && definition.fields.len() == 1 {
                self.ty(&definition.fields[0].ty)
            } else {
                self.fields(
                    &definition.fields,
                    definition.serde_rename_all.as_deref(),
                    definition.serde_container_default,
                )
            }
        } else {
            self.enum_body(enumeration.expect("known enum or struct"))
        };
        self.declarations.push(format!("export type {alias} = {body};"));
        alias
    }

    fn fields(&mut self, fields: &[FieldDef], rename_all: Option<&str>, defaulted: bool) -> String {
        let mut members = Vec::new();
        let mut flattened = Vec::new();
        for field in fields
            .iter()
            .filter(|field| !field.serde_skip && !field.binding_excluded)
        {
            let ty = if field.ty == TypeRef::Duration && !crate::codegen::naming::field_uses_duration_map_wire(field) {
                dts_type(&field.ty)
            } else {
                self.ty(&field.ty)
            };
            let ty = if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
                format!("({ty} | null)")
            } else {
                ty
            };
            if field.serde_flatten {
                flattened.push(if field.ty == TypeRef::Json {
                    "Record<string, JsonValue>".into()
                } else {
                    ty
                });
                continue;
            }
            let name = ts_property_key(&wire_field_name(&field.name, field.serde_rename.as_deref(), rename_all));
            let optional =
                if defaulted || field.optional || field.default.is_some() || matches!(field.ty, TypeRef::Optional(_)) {
                    "?"
                } else {
                    ""
                };
            members.push(format!("{name}{optional}: {ty}"));
        }
        let object = format!("{{ {} }}", members.join("; "));
        if flattened.is_empty() {
            object
        } else {
            format!("({object} & {})", flattened.join(" & "))
        }
    }

    fn payload(&mut self, definition: &EnumDef, variant: &EnumVariant) -> String {
        if variant.fields.is_empty() {
            return "null".into();
        }
        if variant.is_tuple {
            if variant.fields.len() == 1 {
                return self.ty(&variant.fields[0].ty);
            }
            let fields = variant
                .fields
                .iter()
                .map(|field| self.ty(&field.ty))
                .collect::<Vec<_>>();
            return format!("[{}]", fields.join(", "));
        }
        self.fields(&variant.fields, definition.rename_all_fields.as_deref(), false)
    }

    fn variant(&mut self, definition: &EnumDef, variant: &EnumVariant) -> String {
        let payload = self.payload(definition, variant);
        if variant.serde_untagged {
            return payload;
        }
        let wire = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            definition.serde_rename_all.as_deref(),
        );
        let literal = serde_json::to_string(&wire).expect("wire enum string");
        match serde_enum_repr(definition) {
            SerdeEnumRepr::Untagged => payload,
            SerdeEnumRepr::External if variant.fields.is_empty() => literal,
            SerdeEnumRepr::External => format!("{{ {}: {payload} }}", ts_property_key(&wire)),
            SerdeEnumRepr::Internal { tag } => {
                let tag = format!("{{ {}: {literal} }}", ts_property_key(&tag));
                if variant.fields.is_empty() {
                    tag
                } else {
                    format!("({tag} & {payload})")
                }
            }
            SerdeEnumRepr::Adjacent { tag, content } => {
                let tag = format!("{}: {literal}", ts_property_key(&tag));
                if variant.fields.is_empty() {
                    format!("{{ {tag} }}")
                } else {
                    format!("{{ {tag}; {}: {payload} }}", ts_property_key(&content))
                }
            }
        }
    }
}
