//! Type declarations for values that bypass napi objects and go directly through serde JSON.

use super::errors::dts_type;
use crate::codegen::naming::ts_property_key::ts_property_key;
use crate::codegen::naming::{node_type_name, wire_field_name_camel, wire_variant_value};
use crate::codegen::serde_enum_repr::{SerdeEnumRepr, serde_enum_repr};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};
use std::collections::{BTreeMap, BTreeSet};

/// Which spelling the declared wire members use.
///
/// ~keep `Serde` is the honest default: an untagged passthrough enum's runtime is serde's own
/// output, so declaring anything else is a lie the compiler cannot catch. `Camel` is used ONLY for
/// a fully flattened internally tagged enum, whose runtime this backend re-cases on the way
/// through (`codegen::json_wire_types`). The two must never be mixed up -- that is exactly the
/// mismatch `untagged_enum_wire_cross_backend_tests` exists to catch.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WireCasing {
    Serde,
    Camel,
}

pub(super) struct WireTypes<'a> {
    api: &'a ApiSurface,
    casing: WireCasing,
    names: BTreeMap<String, String>,
    used_names: BTreeSet<String>,
    declarations: Vec<String>,
}

impl<'a> WireTypes<'a> {
    pub(super) fn new(api: &'a ApiSurface) -> Self {
        Self::with_casing(api, WireCasing::Serde)
    }

    pub(super) fn with_casing(api: &'a ApiSurface, casing: WireCasing) -> Self {
        Self {
            api,
            casing,
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

    fn field_wire_name(&self, field: &FieldDef, rename_all: Option<&str>) -> String {
        match self.casing {
            WireCasing::Camel => wire_field_name_camel(&field.name, field.serde_rename.as_deref()),
            WireCasing::Serde => {
                crate::codegen::naming::wire_field_name(&field.name, field.serde_rename.as_deref(), rename_all)
            }
        }
    }

    fn tag_wire_name(&self, tag: &str) -> String {
        match self.casing {
            WireCasing::Camel => wire_field_name_camel(tag, None),
            WireCasing::Serde => tag.to_string(),
        }
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

    /// Resolve a `TypeRef::Named` reached from a JSON-passthrough payload to the TypeScript
    /// name that describes it.
    ///
    /// A binding presents exactly ONE public shape per Rust type -- never a second, JSON-shaped
    /// duplicate that only this passthrough path was ever meant to see. `errors::gen_dts`
    /// already declares every non-opaque struct as `export interface {Name}` (`Decl::Interface`,
    /// with real `to_node_name` field casing and `?`-optionality) and every enum, of ANY
    /// representation -- plain string enum, tagged-object, untagged union, or this very
    /// fully-flattened-internal shape -- as its own top-level `export type`/`export declare enum
    /// {Name}` (`Decl::Enum`). Referencing that declared name here is correct for the identical
    /// reason `errors::dts_type`'s `TypeRef::Named` arm has always trusted it for every OTHER
    /// field in the file: the declaration exists, under this exact name, unconditionally.
    ///
    /// Three shapes have no such declaration, or an INCOMPATIBLE one, and still need the
    /// synthesized `__AlefWire*` alias below:
    ///
    /// - An OPAQUE struct (`Decl::Class`, a handle type with no plain-object shape at all, so
    ///   referencing its class name here would describe a JS class instance where the runtime
    ///   hands back plain JSON).
    /// - A `#[serde(transparent)]` single-field newtype (serde's wire form for it is the bare
    ///   inner value with no wrapper object -- the idiomatic interface, a real one-field napi
    ///   struct, does not express that collapse).
    /// - A struct whose declared interface CANNOT be trusted to name the same keys serde
    ///   actually puts on the wire here: `Decl::Interface`'s `js_name` comes from `to_node_name`
    ///   applied to the bare Rust field identifier ONLY (by design -- see the `~keep` note on
    ///   its call site in `types.rs`, which exists precisely to keep a napi struct's ABI-level
    ///   `js_name` independent of an unrelated wire rename). That independence is exactly backwards
    ///   for THIS path: the runtime value here is not a napi struct populated through `js_name`
    ///   marshalling, it is raw JSON serde produced through `wire_field_name`/`rename_all`. A
    ///   snake_case field, container-level `rename_all` or explicit `#[serde(rename = "...")]` makes
    ///   the two surfaces name a key differently, so the idiomatic interface would advertise a
    ///   property the actual JSON does not carry under that name. Falling back to a dedicated
    ///   wire alias for exactly this struct sidesteps the disagreement rather than mis-declaring
    ///   it. ~keep
    fn named(&mut self, name: &str) -> String {
        if let Some(alias) = self.names.get(name) {
            return alias.clone();
        }
        let definition = self.api.types.iter().find(|definition| definition.name == name);
        let enumeration = self.api.enums.iter().find(|definition| definition.name == name);
        if self.casing == WireCasing::Camel
            && let Some(struct_def) = definition
            && !struct_def.is_opaque
            && !(struct_def.serde_container_conversion.transparent && struct_def.fields.len() == 1)
            && struct_def.serde_rename_all.is_none()
            && struct_def.fields.iter().all(|field| {
                field.serde_rename.is_none()
                    && (self.casing == WireCasing::Camel
                        || self.field_wire_name(field, None) == crate::codegen::naming::to_node_name(&field.name))
            })
        {
            // Camel mode only: the fully-flattened runtime re-cases payload keys to host
            // camelCase, matching the declared interface exactly. In Serde mode the runtime
            // value is raw serde JSON whose keys (snake_case Rust names unless explicitly
            // renamed) never match the interface's camelCase `js_name` fields, so the
            // interface can never be trusted there -- always synthesize the wire alias.
            return node_type_name(name).to_string();
        }
        if definition.is_none() && enumeration.is_none() {
            return node_type_name(name).to_string();
        }
        // ~keep Register before descending: recursive payloads refer to an alias, never expand recursively.
        let mut alias = format!("__AlefWire{}", node_type_name(name));
        while !self.used_names.insert(alias.clone()) {
            alias.push('_');
        }
        self.names.insert(name.into(), alias.clone());
        // ~keep An ENUM member of a wire union inlines serde's own literal values, it does not
        // reference the declared TypeScript enum. The declared `enum Mode { Required = "required" }`
        // is a HOST type: TypeScript does not accept a bare `"required"` where `Mode` is expected,
        // so naming it here would declare a union that rejects the very wire value serde produces.
        // Short-circuiting enums to their type name here regressed exactly that.
        let body = if let Some(definition) = definition {
            if definition.serde_container_conversion.transparent && definition.fields.len() == 1 {
                self.ty(&definition.fields[0].ty)
            } else {
                self.fields(
                    &definition.fields,
                    definition.serde_container_default,
                    definition.serde_rename_all.as_deref(),
                )
            }
        } else {
            self.enum_members(enumeration.expect("known enum or struct"))
                .join(" | ")
        };
        self.declarations.push(format!("export type {alias} = {body};"));
        alias
    }

    /// `rename_all` is load-bearing for `WireCasing::Serde` (an untagged passthrough enum's
    /// declared members ARE serde's own output, container rename_all included) and irrelevant for
    /// `WireCasing::Camel`, where only an explicit `#[serde(rename = "...")]` overrides the
    /// mechanical camelCase. Threaded either way so the Serde path cannot silently lose it. ~keep
    fn fields(&mut self, fields: &[FieldDef], defaulted: bool, rename_all: Option<&str>) -> String {
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
            let name = ts_property_key(&self.field_wire_name(field, rename_all));
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
        self.fields(&variant.fields, false, definition.rename_all_fields.as_deref())
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
                // The discriminant KEY is a field name on this boundary's JSON just like any
                // payload field, so it gets the same camelCase treatment (`format_type` ->
                // `formatType`) -- leaving it snake_case while every payload field around it is
                // camelCase produced exactly the mixed-casing object the "one surface" decision
                // exists to prevent. The discriminant VALUE (`wire`/`literal`, e.g. `"excel"`) is
                // data, never an identifier, and stays untouched. ~keep
                let tag = format!("{{ {}: {literal} }}", ts_property_key(&self.tag_wire_name(&tag)));
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
