//! Structural TypeScript type generation for `#[serde(untagged)]` data enums.
//!
//! `enums::is_untagged_data_enum` fields are bridged at runtime as `JsValue` via
//! `serde_wasm_bindgen` (see `enums.rs`'s `~keep` note) — that mechanism is correct and this
//! module does not touch it. What it fixes is the `.d.ts` surface: without a real TypeScript
//! type, wasm-bindgen has nothing but `JsValue` to describe the field, which it renders as
//! `any`. wasm-bindgen's `#[wasm_bindgen(typescript_type = "...")]` extern-type attribute lets a
//! `JsValue`-shaped value carry an arbitrary hand-written TypeScript type instead — confirmed
//! against the pinned wasm-bindgen release by building a throwaway crate and reading the emitted
//! `.d.ts`. The extern type is a zero-cost `JsValue` wrapper (`Into<JsValue>` / `unchecked_into`
//! convert both ways), so it can sit at the getter/setter boundary without changing the
//! underlying field's storage type or the serde bridging that reads/writes it. ~keep
//!
//! The runtime value is a plain JS object/array/primitive produced by `serde_wasm_bindgen`, not
//! a `#[wasm_bindgen]` class instance — so every type this module emits is *structural*
//! (interfaces, type aliases, literal unions), never a reference to a generated wrapper class.

use ahash::{AHashMap, AHashSet};

use crate::codegen::naming::ts_property_key::ts_property_key;
use crate::codegen::naming::{wire_field_name, wire_variant_value};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

use super::enums::{is_untagged_data_enum, is_variant_untagged_string_enum};

/// One named auxiliary TS declaration a variant/field recursively depended on: a struct's
/// interface, a fieldless enum's string-literal union, a nested untagged union's own alias, or
/// (via `build_untagged_enum_ts_plans`) a top-level union's own alias.
enum TsAuxDecl {
    Interface { name: String, fields: Vec<TsField> },
    Alias { name: String, members: Vec<String> },
}

struct TsField {
    name: String,
    ts_type: String,
}

/// The per-enum half of an untagged data enum's TS plan: the Rust-only extern wrapper type a
/// field's getter/setter is typed as, and the small `extern "C" { type ...; }` block that
/// declares it. The TS text itself (the union alias plus every auxiliary interface/alias it
/// depends on) is NOT per-enum — see `AllUntaggedEnumsTsPlan::custom_section`.
pub(super) struct UntaggedEnumTsPlan {
    /// wasm-bindgen resolves this to the exported TS type name (declared in
    /// `AllUntaggedEnumsTsPlan::custom_section`) in the emitted `.d.ts`.
    pub(super) value_type_name: String,
    /// Rendered `extern "C" { type ...; }` block, ready to append to the generated file.
    pub(super) extern_type_declaration: String,
}

/// The complete TS plan for every untagged data enum in one crate's API surface.
pub(super) struct AllUntaggedEnumsTsPlan {
    pub(super) plans: AHashMap<String, UntaggedEnumTsPlan>,
    /// The plain TypeScript text of every declaration, with no Rust wrapper -- what
    /// `ts_custom_section.jinja` embeds verbatim into `custom_section`'s `r#"..."#` string, and
    /// what doc support (`docs_ts_type_for_untagged_enum`) wants directly rather than having to
    /// strip the Rust wrapper back off. Empty if there were no untagged data enums.
    pub(super) ts_body: String,
    /// One shared `typescript_custom_section`, or empty if there were no untagged data enums.
    /// Built once, across every enum, so a struct or fieldless enum reachable from more than one
    /// union (e.g. two different unions both carry a `ContentPart`-shaped variant) is declared
    /// exactly once — TypeScript `interface` declarations merge when duplicated, but `type`
    /// aliases do not (confirmed with `tsc`: redeclaring a `type` twice, even identically, is
    /// `TS2300: Duplicate identifier`), so this cannot be built per-enum and concatenated. ~keep
    pub(super) custom_section: String,
}

/// `mod.rs` entry point: filters `api.enums` down to the non-excluded untagged data enums and
/// builds their combined plan. Kept separate from `build_untagged_enum_ts_plans` so the filtering
/// (which `mod.rs` would otherwise have to inline) lives next to the code it's built for.
pub(super) fn build_untagged_enum_ts_plan_for_api(
    api: &ApiSurface,
    exclude_types: &[String],
    opaque_type_names: &AHashSet<String>,
    text_field_enum_names: &AHashSet<String>,
    prefix: &str,
) -> AllUntaggedEnumsTsPlan {
    let exclude_types_set: AHashSet<String> = exclude_types.iter().cloned().collect();
    let untagged_enum_defs: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| gets_a_ts_union(e, &exclude_types_set, text_field_enum_names))
        .collect();
    build_untagged_enum_ts_plans(&untagged_enum_defs, api, &exclude_types_set, opaque_type_names, prefix)
}

/// Whether this enum gets a structural TypeScript union rather than staying `any`/`String`.
///
/// The binding and the docs page must answer this identically -- the docs promising a type the
/// binding does not emit is the exact defect `f1fa69fd0` fixed, and two copies of the predicate
/// is how it comes back.
///
/// `untagged_union_text_types` pins its members to `String` on the field, the getter and the
/// setter alike. Handing one an extern wrapper type here would retype only the accessors and
/// reintroduce the E0308 that `3678c3e8a` fixed, so the text opt-in wins first. ~keep
fn gets_a_ts_union(
    enum_def: &EnumDef,
    exclude_types: &AHashSet<String>,
    text_field_enum_names: &AHashSet<String>,
) -> bool {
    !exclude_types.contains(&enum_def.name)
        && !text_field_enum_names.contains(&enum_def.name)
        && is_untagged_data_enum(enum_def)
}

/// `mod.rs` entry point for [`is_variant_untagged_string_enum`] enums: filters `api.enums` down
/// to the non-excluded ones and builds their combined plan. Reuses [`AllUntaggedEnumsTsPlan`]'s
/// shape (and the same `ts_extern_value_type`/`ts_custom_section` templates
/// [`build_untagged_enum_ts_plans`] renders) so `mod.rs` merges the two plans the same way it
/// already merges everything else about a "no nominal `Wasm{Enum}` type" enum -- but the
/// declared TypeScript differs, so this stays a separate builder rather than a branch inside
/// `build_untagged_enum_ts_plans`/`TsMapContext::map_variant` (see
/// [`is_variant_untagged_string_enum`]'s doc comment for why a unit variant must NOT render as
/// `null` here the way it does there). ~keep
pub(super) fn build_variant_untagged_string_enum_ts_plan_for_api(
    api: &ApiSurface,
    exclude_types: &[String],
    opaque_type_names: &AHashSet<String>,
    text_field_enum_names: &AHashSet<String>,
    prefix: &str,
) -> AllUntaggedEnumsTsPlan {
    let exclude_types_set: AHashSet<String> = exclude_types.iter().cloned().collect();
    let enum_defs: Vec<&EnumDef> = api
        .enums
        .iter()
        .filter(|e| gets_a_variant_untagged_string_enum_ts_union(e, &exclude_types_set, text_field_enum_names))
        .collect();
    build_variant_untagged_string_enum_ts_plans(&enum_defs, api, &exclude_types_set, opaque_type_names, prefix)
}

/// Whether this enum gets the flat literal-union-plus-widening-tail TypeScript type rather than
/// staying `any`/`String`. Same opt-out precedence as [`gets_a_ts_union`]: an explicit
/// `untagged_union_text_types` entry pins the field type to plain `String` and must win over
/// declaring a structural union here. ~keep
fn gets_a_variant_untagged_string_enum_ts_union(
    enum_def: &EnumDef,
    exclude_types: &AHashSet<String>,
    text_field_enum_names: &AHashSet<String>,
) -> bool {
    !exclude_types.contains(&enum_def.name)
        && !text_field_enum_names.contains(&enum_def.name)
        && is_variant_untagged_string_enum(enum_def)
}

/// Build the full TS plan for every [`is_variant_untagged_string_enum`] enum: a flat
/// string-literal union over the unit variants (their serde WIRE value, per `wire_variant_value`
/// -- NOT any napi-style runtime case table, since this value round-trips through the CORE
/// type's own serde impl via `serde_wasm_bindgen`, not through a napi-derive macro with its own
/// casing algorithm), plus one widening tail member per untagged data variant. A single-field
/// tuple variant widens via `(T & {})` when `T` is exactly `string` -- the standard TypeScript
/// "loosen a literal union" idiom, which only helps when the wider type and the literals share a
/// primitive -- and otherwise (including multi-field tuple and struct-shaped variants) falls
/// back to [`TsMapContext::map_variant`]'s ordinary structural rendering unwrapped, reusing the
/// same recursive type mapping [`build_untagged_enum_ts_plans`] uses so a struct-payload variant
/// still gets its fields expanded into a nested interface declaration. ~keep
fn build_variant_untagged_string_enum_ts_plans(
    enum_defs: &[&EnumDef],
    api: &ApiSurface,
    exclude_types: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    prefix: &str,
) -> AllUntaggedEnumsTsPlan {
    let mut ctx = TsMapContext {
        api,
        exclude_types,
        opaque_type_names,
        prefix,
        in_progress: AHashMap::default(),
        resolved_names: AHashMap::default(),
        decls: Vec::new(),
    };
    let mut plans = AHashMap::default();

    for enum_def in enum_defs {
        let ts_type_name = format!("{prefix}{}", enum_def.name);
        let mut members: Vec<String> = enum_def
            .variants
            .iter()
            .filter(|v| v.fields.is_empty())
            .map(|v| {
                format!(
                    "\"{}\"",
                    wire_variant_value(&v.name, v.serde_rename.as_deref(), enum_def.serde_rename_all.as_deref())
                )
            })
            .collect();
        let rename_all_fields = enum_def.rename_all_fields.as_deref();
        for variant in enum_def.variants.iter().filter(|v| !v.fields.is_empty()) {
            if variant.is_tuple && variant.fields.len() == 1 {
                let ty = ctx.map_type(&variant.fields[0].ty);
                members.push(if ty == "string" {
                    "(string & {})".to_string()
                } else {
                    ty
                });
            } else {
                members.push(ctx.map_variant(variant, rename_all_fields));
            }
        }
        ctx.decls.push(TsAuxDecl::Alias {
            name: ts_type_name.clone(),
            members,
        });

        let value_type_name = format!("{ts_type_name}Value");
        let extern_type_declaration = crate::backends::wasm::template_env::render(
            "ts_extern_value_type",
            minijinja::context! {
                ts_type_name => ts_type_name,
                value_type_name => value_type_name.clone(),
            },
        );
        plans.insert(
            enum_def.name.clone(),
            UntaggedEnumTsPlan {
                value_type_name,
                extern_type_declaration,
            },
        );
    }

    let ts_body = if ctx.decls.is_empty() {
        String::new()
    } else {
        ctx.decls.iter().map(render_aux_decl).collect::<Vec<_>>().join("\n\n")
    };
    let custom_section = if ts_body.is_empty() {
        String::new()
    } else {
        crate::backends::wasm::template_env::render(
            "ts_custom_section",
            minijinja::context! {
                const_name => "ALEF_VARIANT_UNTAGGED_STRING_ENUMS_TS",
                ts_body => ts_body.clone(),
            },
        )
    };

    AllUntaggedEnumsTsPlan {
        plans,
        ts_body,
        custom_section,
    }
}

/// The per-enum extern wrapper type name, by Rust enum name — what `gen_struct_methods` needs to
/// type an untagged-enum-typed field's getter/setter.
pub(super) fn value_type_names(plan: &AllUntaggedEnumsTsPlan) -> AHashMap<String, String> {
    plan.plans
        .iter()
        .map(|(name, enum_plan)| (name.clone(), enum_plan.value_type_name.clone()))
        .collect()
}

/// Build the full TS plan for every untagged data enum, recursively expanding every variant's
/// payload type.
///
/// `exclude_types` / `opaque_type_names` mark types this backend cannot describe structurally
/// (consumer-excluded, or an opaque handle type with no public fields) — any variant reaching
/// one of those falls back to `any` for that variant's slot only, never for the whole union.
pub(super) fn build_untagged_enum_ts_plans(
    untagged_enums: &[&EnumDef],
    api: &ApiSurface,
    exclude_types: &AHashSet<String>,
    opaque_type_names: &AHashSet<String>,
    prefix: &str,
) -> AllUntaggedEnumsTsPlan {
    let mut ctx = TsMapContext {
        api,
        exclude_types,
        opaque_type_names,
        prefix,
        in_progress: AHashMap::default(),
        resolved_names: AHashMap::default(),
        decls: Vec::new(),
    };
    let mut plans = AHashMap::default();

    for enum_def in untagged_enums {
        // A sibling union processed earlier may already have reached this enum as a nested
        // variant payload (see `map_named_enum`'s untagged-enum branch) and queued its alias —
        // in that case its declaration is already pending in `ctx.decls`; only expand it here
        // if that has not happened yet. ~keep
        if !ctx.resolved_names.contains_key(&enum_def.name) {
            // An untagged data enum keeps the bare `{prefix}{name}`: unlike a struct it gets no
            // `#[wasm_bindgen]` class of its own (it is bridged as `JsValue`), so there is no
            // class declaration for its alias to merge with — and the name is what
            // `typescript_type = "..."` points the getter at. ~keep
            let ts_type_name = format!("{prefix}{}", enum_def.name);
            ctx.in_progress.insert(enum_def.name.clone(), ts_type_name.clone());
            let rename_all_fields = enum_def.rename_all_fields.as_deref();
            let members: Vec<String> = enum_def
                .variants
                .iter()
                .map(|v| ctx.map_variant(v, rename_all_fields))
                .collect();
            ctx.in_progress.remove(&enum_def.name);
            ctx.resolved_names.insert(enum_def.name.clone(), ts_type_name.clone());
            ctx.decls.push(TsAuxDecl::Alias {
                name: ts_type_name,
                members,
            });
        }

        let ts_type_name = format!("{prefix}{}", enum_def.name);
        let value_type_name = format!("{ts_type_name}Value");
        let extern_type_declaration = crate::backends::wasm::template_env::render(
            "ts_extern_value_type",
            minijinja::context! {
                ts_type_name => ts_type_name,
                value_type_name => value_type_name.clone(),
            },
        );
        plans.insert(
            enum_def.name.clone(),
            UntaggedEnumTsPlan {
                value_type_name,
                extern_type_declaration,
            },
        );
    }

    let ts_body = if ctx.decls.is_empty() {
        String::new()
    } else {
        ctx.decls.iter().map(render_aux_decl).collect::<Vec<_>>().join("\n\n")
    };
    let custom_section = if ts_body.is_empty() {
        String::new()
    } else {
        crate::backends::wasm::template_env::render(
            "ts_custom_section",
            minijinja::context! {
                const_name => "ALEF_UNTAGGED_UNIONS_TS",
                ts_body => ts_body.clone(),
            },
        )
    };

    AllUntaggedEnumsTsPlan {
        plans,
        ts_body,
        custom_section,
    }
}

fn render_alias(name: &str, members: &[String]) -> String {
    crate::backends::wasm::template_env::render(
        "ts_type_alias",
        minijinja::context! { name => name, members => members },
    )
    .trim_end()
    .to_string()
}

fn render_aux_decl(decl: &TsAuxDecl) -> String {
    match decl {
        TsAuxDecl::Interface { name, fields } => render_interface(name, fields),
        TsAuxDecl::Alias { name, members } => render_alias(name, members),
    }
}

fn render_interface(name: &str, fields: &[TsField]) -> String {
    crate::backends::wasm::template_env::render(
        "ts_interface",
        minijinja::context! {
            name => name,
            fields => fields.iter().map(|f| minijinja::context! {
                name => f.name,
                ts_type => f.ts_type,
            }).collect::<Vec<_>>(),
        },
    )
    .trim_end()
    .to_string()
}

fn render_inline_object(fields: &[TsField]) -> String {
    crate::backends::wasm::template_env::render(
        "ts_inline_object",
        minijinja::context! {
            fields => fields.iter().map(|f| minijinja::context! {
                name => f.name,
                ts_type => f.ts_type,
            }).collect::<Vec<_>>(),
        },
    )
    .trim_end()
    .to_string()
}

struct TsMapContext<'a> {
    api: &'a ApiSurface,
    exclude_types: &'a AHashSet<String>,
    opaque_type_names: &'a AHashSet<String>,
    prefix: &'a str,
    /// Rust source name -> the TS name it is *being* declared under, for names whose expansion is
    /// still on the stack. Breaks cycles (self- or mutually-recursive types) by resolving a
    /// re-entrant reference to that name instead of recursing again.
    ///
    /// It carries the declared name rather than being a bare set because a struct's declared name
    /// is not derivable from its Rust name alone — `map_named_struct` suffixes it — and a
    /// self-recursive struct hits this branch, not `resolved_names`. Recomputing `{prefix}{name}`
    /// here instead would point the recursive field at the wasm-bindgen *class*, which is the
    /// exact merge this module's suffixing exists to prevent. ~keep
    in_progress: AHashMap<String, String>,
    /// Rust source name -> the TS name it was actually declared under, once expansion finishes.
    /// A nested untagged union keeps the bare `{prefix}{name}`; a struct and a fieldless enum
    /// each get a `Wire`-suffixed name instead (see `map_named_struct` / `map_named_enum`) — so a
    /// *second* reference to an already-resolved name must look up the name actually used here
    /// rather than recomputing `{prefix}{name}` blind, or it would point at the wrong
    /// (unsuffixed, colliding) name. ~keep
    resolved_names: AHashMap<String, String>,
    decls: Vec<TsAuxDecl>,
}

impl TsMapContext<'_> {
    fn map_variant(&mut self, variant: &EnumVariant, rename_all_fields: Option<&str>) -> String {
        if variant.fields.is_empty() {
            // A fieldless variant of an otherwise data-carrying untagged enum serializes as
            // serde's unit representation: JSON `null`.
            return "null".to_string();
        }
        if variant.is_tuple {
            if variant.fields.len() == 1 {
                return self.map_type(&variant.fields[0].ty);
            }
            let members: Vec<String> = variant.fields.iter().map(|f| self.map_type(&f.ty)).collect();
            return format!("[{}]", members.join(", "));
        }
        let fields = self.map_fields(&variant.fields, rename_all_fields);
        render_inline_object(&fields)
    }

    /// Every member this module declares is STRUCTURAL: the runtime value is a plain JS object
    /// `serde_wasm_bindgen` produced from, or will deserialize into, the CORE Rust type (the
    /// field's `JsValue` goes straight through `serde_wasm_bindgen::from_value` into
    /// `core::{Type}` -- see `codegen::conversions::binding_to_core::fields`). Its keys are
    /// therefore serde WIRE names. A `#[wasm_bindgen]` getter's `to_node_name` host name never
    /// appears on this path, because no wrapper class sits on it.
    ///
    /// Declaring the host name here disagreed with the very deserializer the module doc says it
    /// does not touch: a core field `max_chars` was declared `maxChars`, and
    /// `serde_wasm_bindgen::from_value` on an object written against that declaration falls
    /// through to `unwrap_or_default()`. `backends::napi::gen_bindings::errors`'s
    /// `untagged_variant_dts_type` is the same declaration for the same runtime mechanism and
    /// resolves the key the same way; `backends::go`'s `go_data_enum_variant_field` is the
    /// sibling that has always kept the host name and the wire key apart. ~keep
    ///
    /// A wire name, unlike a host identifier, is not guaranteed to be spellable bare —
    /// `#[serde(rename = "content-type")]` emitted raw is a `.d.ts` syntax error that takes the
    /// whole `typescript_custom_section` down with it — so every key goes through
    /// `naming::ts_property_key`, shared with the napi emitter that declares the same shape. ~keep
    fn map_fields(&mut self, fields: &[FieldDef], rename_all: Option<&str>) -> Vec<TsField> {
        fields
            .iter()
            .map(|f| TsField {
                name: ts_property_key(&wire_field_name(&f.name, f.serde_rename.as_deref(), rename_all)),
                ts_type: self.map_field_type(f),
            })
            .collect()
    }

    fn map_field_type(&mut self, field: &FieldDef) -> String {
        let base = self.map_type(&field.ty);
        if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
            format!("{base} | undefined")
        } else {
            base
        }
    }

    fn map_type(&mut self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => primitive_ts_type(p).to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => "string".to_string(),
            TypeRef::Bytes => "Uint8Array".to_string(),
            TypeRef::Unit => "null".to_string(),
            // An arbitrary `serde_json::Value` genuinely has no narrower structural type. ~keep
            TypeRef::Json => "any".to_string(),
            TypeRef::Duration => "number".to_string(),
            TypeRef::Optional(inner) => format!("{} | undefined", self.map_type(inner)),
            // `[]` binds tighter than `|` in TypeScript, so appending it to an unparenthesized
            // union type only applies to the union's last operand: `T | undefined[]` parses as
            // `T | (undefined[])`, not `(T | undefined)[]`. `TypeRef::Optional` is the only arm
            // above that renders a top-level union, so it is the only element type that needs
            // the parens; every other arm (`Named`, `Vec`, `Map`, primitives) already renders a
            // single type reference `[]` can suffix unambiguously. ~keep
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Optional(_) => format!("({})[]", self.map_type(inner)),
                _ => format!("{}[]", self.map_type(inner)),
            },
            TypeRef::Map(key, value) => {
                let key_ts = self.map_type(key);
                if key_ts == "string" {
                    format!("Record<string, {}>", self.map_type(value))
                } else {
                    // A non-string-keyed map doesn't serialize to a plain JSON object, so it
                    // can't be described as a `Record`. ~keep
                    "any".to_string()
                }
            }
            TypeRef::Named(name) => self.map_named(name),
        }
    }

    fn map_named(&mut self, name: &str) -> String {
        if self.exclude_types.contains(name) || self.opaque_type_names.contains(name) {
            return "any".to_string();
        }
        if let Some(resolved) = self.resolved_names.get(name) {
            return resolved.clone();
        }
        let ts_name = format!("{}{name}", self.prefix);
        if let Some(being_declared) = self.in_progress.get(name) {
            return being_declared.clone();
        }
        if let Some(enum_def) = self.api.enums.iter().find(|e| e.name == name) {
            return self.map_named_enum(enum_def, ts_name);
        }
        if let Some(type_def) = self.api.types.iter().find(|t| t.name == name) {
            return self.map_named_struct(type_def, ts_name);
        }
        // Unresolvable: a generic parameter or a type outside this crate's API surface.
        "any".to_string()
    }

    /// Declare a struct payload's shape as an interface — under a `Wire`-suffixed name, never the
    /// bare `{prefix}{Name}`.
    ///
    /// The bare name is already taken: `gen_struct` emits a `#[wasm_bindgen] pub struct
    /// {prefix}{Name}`, which wasm-bindgen renders into the SAME `.d.ts` as `export class
    /// {prefix}{Name}`. TypeScript merges an `interface` into a `class` of the same name silently
    /// — it is a legal declaration merge, not an error — so the bare name would not collide
    /// loudly, it would graft this interface's members onto the class type. The class's real
    /// members are `to_node_name` host accessors (`maxChars`); this interface's are serde wire
    /// keys (`max_chars`). Merging publishes the union of both on every class instance, so `tsc`
    /// accepts `instance.max_chars` — a property that is `undefined` at runtime, on the exact
    /// host/wire boundary this module's field-naming fix exists to keep straight.
    ///
    /// `map_named_enum` already suffixes a fieldless enum's alias for the same reason (its bare
    /// name is claimed by a real wasm-bindgen `enum`); this is that precedent applied to the
    /// struct case, which has the worse failure mode because `interface`/`class` merge instead of
    /// erroring. ~keep
    fn map_named_struct(&mut self, type_def: &TypeDef, ts_name: String) -> String {
        if type_def.is_opaque {
            return "any".to_string();
        }
        let wire_name = format!("{ts_name}Wire");
        self.in_progress.insert(type_def.name.clone(), wire_name.clone());
        let fields = self.map_fields(&type_def.fields, type_def.serde_rename_all.as_deref());
        self.in_progress.remove(&type_def.name);
        self.resolved_names.insert(type_def.name.clone(), wire_name.clone());
        self.decls.push(TsAuxDecl::Interface {
            name: wire_name.clone(),
            fields,
        });
        wire_name
    }

    fn map_named_enum(&mut self, enum_def: &EnumDef, ts_name: String) -> String {
        if enum_def.variants.iter().all(|v| v.fields.is_empty()) {
            // A fieldless enum always gets its own real `Wasm{Enum}` wasm-bindgen TS `enum`
            // elsewhere in the file (see `gen_enum`), unconditionally — regardless of whether
            // any struct field references it by that name. That native enum's runtime value is
            // the numeric ABI discriminant, NOT the serde wire string this union member actually
            // carries, so the two are incompatible representations that happen to share a Rust
            // name — reusing the bare name would either collide (a `type` alias cannot merge
            // with an `enum`, confirmed with `tsc`) or, if it merged, describe the wrong runtime
            // shape. A distinct suffix sidesteps both. ~keep
            let literal_name = format!("{ts_name}Wire");
            let values: Vec<String> = enum_def
                .variants
                .iter()
                .map(|v| {
                    let wire =
                        wire_variant_value(&v.name, v.serde_rename.as_deref(), enum_def.serde_rename_all.as_deref());
                    format!("\"{wire}\"")
                })
                .collect();
            self.resolved_names.insert(enum_def.name.clone(), literal_name.clone());
            self.decls.push(TsAuxDecl::Alias {
                name: literal_name.clone(),
                members: values,
            });
            return literal_name;
        }
        if is_untagged_data_enum(enum_def) {
            // This enum gets its own top-level entry in `untagged_enums`, processed by the same
            // driving loop that reached `self` (before or after — order doesn't matter to TS).
            // That loop's own guard (`!resolved_names.contains_key(name)`) decides whether it
            // still needs expanding, so `resolved_names` must NOT be written here: registering it
            // now — before that loop reaches it — would make the loop skip it, and since a bare
            // reference (unlike the struct/fieldless-enum branches above) pushes no decl of its
            // own, the enum's `type` alias would then never be emitted at all, leaving
            // `typescript_type = "{ts_name}"` dangling. Just return the reference. ~keep
            return ts_name;
        }
        // An internally-tagged data enum (`#[serde(tag = "...")]`) or another shape this module
        // doesn't structurally model — its own struct-with-discriminator wrapper is a different
        // representation than the plain-JSON shape this module describes. ~keep
        "any".to_string()
    }
}

/// Doc support: the exact `.d.ts` union declaration text WASM emits for one untagged data
/// enum's own type (plus every auxiliary interface/alias it recursively depends on), so
/// `docs::language_pages::enum_render` can embed alef's OWN lowering decision for this enum
/// instead of reading the IR a second, independently-drifting way -- the same shared-function
/// pattern that closed the doc/binding disagreements fixed by 8d199c0bf and 64aa80692.
///
/// Returns `None` when `enum_def` does not become a JsValue/TS-union field at all: excluded via
/// `[crates.wasm].exclude_types`, a fieldless enum, an internally-tagged data enum
/// (`serde_tag`), or a `untagged_union_text_types` opt-in that pins the field to a plain
/// `string` instead (see `build_untagged_enum_ts_plan_for_api`'s own filter, which this
/// mirrors).
///
/// Uses only the config-declared exclusion/opaque sets (`wasm_exclude_types` /
/// `wasm_opaque_type_names`), not the dynamic additions `generate_bindings` layers on top
/// (cfg-gated features, dropped external crates, unknown-type omissions) -- the caller already
/// hands this function a per-language cfg-filtered `ApiSurface` (see
/// `docs::language_pages::generate_lang_doc`), which covers the common case. A variant payload
/// that transitively reaches a type this binding drops for one of those dynamic reasons is a
/// narrow, rare shape (an externally-defined or unrepresentable type inside a
/// `#[serde(untagged)]` payload) that this function may render as a full interface where the
/// real binding falls back to `any`; that gap is judged acceptable for a documentation aid. ~keep
pub(crate) fn docs_ts_type_for_untagged_enum(
    enum_def: &EnumDef,
    api: &ApiSurface,
    config: &crate::core::config::ResolvedCrateConfig,
) -> Option<String> {
    let exclude_types_vec = wasm_exclude_types(config);
    let text_field_enum_names: AHashSet<String> = config.untagged_union_text_types.iter().cloned().collect();
    let exclude_types: AHashSet<String> = exclude_types_vec.iter().cloned().collect();
    if !gets_a_ts_union(enum_def, &exclude_types, &text_field_enum_names) {
        return None;
    }
    let opaque_type_names = wasm_opaque_type_names(api, &exclude_types_vec);
    let prefix = config.wasm_type_prefix();
    let plan = build_untagged_enum_ts_plans(&[enum_def], api, &exclude_types, &opaque_type_names, &prefix);
    Some(plan.ts_body)
}

/// Types this WASM binding excludes from generation entirely: `[crates.wasm].exclude_types`
/// plus opaque newtypes whose wrapped path carries a generic parameter (a `Vec<T>`-shaped
/// opaque newtype never becomes a `#[wasm_bindgen]` class). `pub(super)` so `mod.rs`'s
/// `generate_bindings` and this module's own `docs_ts_type_for_untagged_enum` compute the same
/// exclusion set instead of two independently-drifting readings of `config`. ~keep
pub(super) fn wasm_exclude_types(config: &crate::core::config::ResolvedCrateConfig) -> Vec<String> {
    let mut exclude_types = config
        .wasm
        .as_ref()
        .map(|c| c.exclude_types.clone())
        .unwrap_or_default();
    exclude_types.extend(
        config
            .opaque_types
            .iter()
            .filter(|(_, path)| path.contains('<'))
            .map(|(name, _)| name.clone()),
    );
    exclude_types
}

/// The opaque type names this WASM binding wraps as `Arc`-backed handle structs, given the set
/// already excluded from generation. Shared between `mod.rs` and `docs_ts_type_for_untagged_enum`
/// for the same reason as `wasm_exclude_types`. ~keep
pub(super) fn wasm_opaque_type_names(api: &ApiSurface, exclude_types: &[String]) -> AHashSet<String> {
    api.types
        .iter()
        .filter(|t| t.is_opaque && !exclude_types.contains(&t.name))
        .map(|t| t.name.clone())
        .collect()
}

/// Whether wasm-bindgen lowers `prim` to a JavaScript `bigint` rather than a `number`.
///
/// ~keep The single predicate behind both halves of the contract: the TypeScript type this
/// backend *declares* for the primitive ([`primitive_ts_type`]) and the literal alef's e2e/doc
/// generator *emits* for a value of it (`e2e::codegen::typescript::test_file::builders`). Those
/// were decided independently — the declared type came from here, the literal came from a
/// hand-maintained `bigint_fields` list in `alef.toml` — so any `u64`/`i64` field a consumer had
/// not remembered to list got a plain `42` assigned to a `bigint` setter. Keeping one function
/// is what stops the emitted type and the emitted value from disagreeing again.
pub(crate) fn is_bigint_primitive(prim: &crate::core::ir::PrimitiveType) -> bool {
    primitive_ts_type(prim) == "bigint"
}

fn primitive_ts_type(prim: &crate::core::ir::PrimitiveType) -> &'static str {
    use crate::core::ir::PrimitiveType;
    match prim {
        PrimitiveType::Bool => "boolean",
        PrimitiveType::U64 | PrimitiveType::I64 => "bigint",
        PrimitiveType::U8
        | PrimitiveType::U16
        | PrimitiveType::U32
        | PrimitiveType::I8
        | PrimitiveType::I16
        | PrimitiveType::I32
        | PrimitiveType::F32
        | PrimitiveType::F64
        | PrimitiveType::Usize
        | PrimitiveType::Isize => "number",
    }
}

#[cfg(test)]
mod tests;
