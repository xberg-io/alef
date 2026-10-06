use crate::backends::php::type_map::PhpMapper;
use crate::codegen::builder::ImplBuilder;
use crate::codegen::conversions::{
    VariantDeclaration, enum_conversion_needs_catch_all_for_features, enum_variant_declaration,
};
use crate::codegen::naming::wire_variant_value;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use ahash::AHashSet;
use std::collections::HashSet;

use super::structs::is_php_copy_type;

/// Generate a `#[php_class]` with class constants for unit-variant enums.
///
/// Emits a PHP-visible class that allows consumers to reference enum values as constants.
///
/// `is_host_enum`/`configured_features` are forwarded to [`enum_constant_entries`] so a
/// FOREIGN cfg-gated variant this binding's own configured feature set proves unreachable is
/// never advertised as a constant here -- see that function's doc comment. ~keep
pub(crate) fn gen_enum_constants(
    enum_def: &EnumDef,
    php_namespace: Option<&str>,
    is_host_enum: bool,
    configured_features: Option<&HashSet<&str>>,
) -> String {
    let mut lines = vec![];

    // Emit the #[php_class] decorator with optional namespace.
    // ext-php-rs 0.15+ removed `#[php_class(name = "...")]` / `(namespace = "...")`;
    // namespace must use the two-attribute form `#[php_class]` + `#[php(name = "Ns\\ClassName")]`.
    if let Some(ns) = php_namespace {
        let ns_escaped = ns.replace('\\', "\\\\");
        lines.push("#[php_class]".to_string());
        lines.push(format!("#[php(name = \"{}\\\\{}\")]", ns_escaped, enum_def.name));
    } else {
        lines.push("#[php_class]".to_string());
    }

    lines.push(format!("pub struct {} {{}}", enum_def.name));
    lines.push(String::new());

    // Emit the #[php_impl] block with class constants.
    lines.push("#[php_impl]".to_string());
    lines.push(format!("impl {} {{", enum_def.name));

    for (const_name, wire_value) in enum_constant_entries(enum_def, is_host_enum, configured_features) {
        lines.push(format!("    pub const {const_name}: &str = \"{wire_value}\";"));
    }

    lines.push("}".to_string());

    lines.join("\n")
}

/// Compute each variant's `(PHP constant name, wire value)` pair for a unit-variant enum's
/// constants-only class.
///
/// The VALUE is the serde wire name, not the Rust variant ident, because both directions of the
/// extension's own conversion speak wire names: binding->core matches on `wire_variant_value`
/// (`helpers::enum_defaults::gen_string_to_enum_expr`) and core->binding produces
/// `serde_json::to_value(..)`. A constant carrying the Rust ident therefore matches nothing the
/// extension accepts and equals nothing it returns — `BatchStatus::INPROGRESS` ("InProgress") falls
/// through to the match's fallback arm and silently becomes the default variant, and
/// `$obj->status === BatchStatus::INPROGRESS` is never true.
///
/// Shared by [`gen_enum_constants`] (the runtime `#[php_impl]` block) and the PHPStan stub's
/// `gen_enum_stub` (`type_stubs.rs`), so the two surfaces cannot independently drift on member
/// names or values — the stub declares exactly what this function tells the runtime to register,
/// not a native PHP `enum` the extension never registers. ~keep
///
/// A variant this binding's own configured feature set proves unreachable
/// ([`enum_variant_declaration`] returns [`VariantDeclaration::Drop`]) is omitted entirely: unlike
/// a Rust enum declaration, `pub const NAME: &str = "value"` compiles regardless of whether the
/// real Rust variant exists, so nothing here would otherwise catch a constant that advertises a
/// value the generated binding->core conversion can never actually produce (that conversion
/// already drops the match arm for any foreign cfg-gated variant unconditionally). Dropping the
/// entry here closes that gap instead of leaving it to a runtime PHP caller. A host-owned
/// cfg-gated variant is never dropped -- `enum_variant_declaration` always resolves it to `Keep`,
/// matching every other backend's declaration surface. ~keep
pub(crate) fn enum_constant_entries(
    enum_def: &EnumDef,
    is_host_enum: bool,
    configured_features: Option<&HashSet<&str>>,
) -> Vec<(String, String)> {
    enum_def
        .variants
        .iter()
        .filter(|variant| {
            !matches!(
                enum_variant_declaration(variant, is_host_enum, configured_features),
                VariantDeclaration::Drop
            )
        })
        .map(|variant| {
            let const_name = escape_php_reserved_constant(&variant.name.to_uppercase());
            let wire_value = wire_variant_value(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            (const_name, wire_value)
        })
        .collect()
}

/// PHP class constant names are case-insensitively reserved against PHP keywords.
/// `pub const CLASS: ...` fails to load with "A class constant must not be called 'class';
/// it is reserved for class name fetching". Append `_` to keep the literal variant
/// name distinguishable while sidestepping the reserved set.
fn escape_php_reserved_constant(name: &str) -> String {
    const RESERVED: &[&str] = &[
        "CLASS",
        "INTERFACE",
        "TRAIT",
        "ENUM",
        "FUNCTION",
        "NAMESPACE",
        "CONST",
        "STATIC",
        "ABSTRACT",
        "FINAL",
        "PRIVATE",
        "PROTECTED",
        "PUBLIC",
        "CASE",
        "DEFAULT",
        "EXTENDS",
        "IMPLEMENTS",
        "NEW",
        "USE",
        "RETURN",
        "IF",
        "ELSE",
        "ELSEIF",
        "ENDIF",
        "WHILE",
        "FOR",
        "FOREACH",
        "AS",
        "DO",
        "SWITCH",
        "BREAK",
        "CONTINUE",
        "AND",
        "OR",
        "XOR",
        "TRUE",
        "FALSE",
        "NULL",
        "ECHO",
        "PRINT",
        "ISSET",
        "UNSET",
        "EMPTY",
        "EXIT",
        "DIE",
        "GLOBAL",
        "GOTO",
        "TRY",
        "CATCH",
        "FINALLY",
        "THROW",
        "INSTANCEOF",
        "MATCH",
        "FN",
        "YIELD",
        "READONLY",
    ];
    if RESERVED.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

/// Return true if an enum is a "tagged data enum" — either has a serde tag AND at least one
/// struct-field variant, OR is a [`is_labeled_string_enum`] (an externally-tagged enum whose only
/// data is a caller-supplied `String` label). Both shapes are lowered to flat PHP classes rather
/// than string constants.
///
/// Folding `is_labeled_string_enum` into this predicate is deliberate, not incidental: every call
/// site below (struct-field type mapping, the PHPStan stub, the constants-vs-flat-class dispatch
/// in `rust_bindings.rs`/`public_api.rs`/`type_stubs.rs`) already branches on this single function,
/// so routing a labeled string enum through it is the only way to move a type from "string
/// constants that silently discard a payload" to "flat class that keeps it" without re-deriving the
/// same dispatch decision at every call site. ~keep
pub(crate) fn is_tagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.variants.iter().any(|v| !v.fields.is_empty())
        && (enum_def.serde_tag.is_some() || is_labeled_string_enum(enum_def))
}

/// Return true if `enum_def` is externally tagged (no `#[serde(tag = "...")]`, not
/// `#[serde(untagged)]`) and every data-carrying variant is a single-field tuple variant whose
/// field is a bare, non-boxed `String` — the caller-supplied-label shape (`Custom(String)`) used
/// commonly used for category-style enums in a consumer crate.
///
/// This predicate is deliberately narrower than "any externally-tagged enum with a data variant".
/// `gen_flat_data_enum_from_impls`'s per-variant match arms pattern-match the real core variant
/// directly (`core_path::Custom(_0) => ..`), so tag style genuinely does not matter to correctness
/// there — but `alef.toml`'s `[crates.php] exclude_types` records that this SAME flat-class
/// machinery does not compile cleanly (`E0716`/`E0596`/`E0507`) for enums whose data variants carry
/// richer payloads (`Vec`, boxed fields, multiple fields per variant: `NodeContent`,
/// `OcrBoundingGeometry`, `ChunkSizing`, `EmbeddingModelType`). Restricting this predicate to the
/// simplest possible payload — one field, plain `String`, never boxed — keeps this fix inside the
/// shape that is known to work rather than gambling on the shape that is known not to. ~keep
pub(crate) fn is_labeled_string_enum(enum_def: &EnumDef) -> bool {
    if enum_def.serde_tag.is_some() || enum_def.serde_untagged {
        return false;
    }
    let mut has_label_variant = false;
    for variant in &enum_def.variants {
        if variant.fields.is_empty() {
            continue;
        }
        let is_single_string_tuple = variant.fields.len() == 1
            && crate::codegen::conversions::is_tuple_variant(&variant.fields)
            && matches!(variant.fields[0].ty, TypeRef::String)
            && !variant.fields[0].is_boxed
            && !variant.fields[0].optional;
        if !is_single_string_tuple {
            return false;
        }
        has_label_variant = true;
    }
    has_label_variant
}

/// Return true if an enum is an "untagged data enum" — has `#[serde(untagged)]` AND at
/// least one variant carrying data (e.g. `Single(String) | Multiple(Vec<String>)`).
/// These cannot be lowered to a single `String` in the PHP binding because the wire
/// JSON shape varies per variant; they are mapped to `serde_json::Value` and converted
/// to the typed core enum via `serde_json::from_value` in the binding→core `From` impl.
pub(crate) fn is_untagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.serde_untagged && enum_def.variants.iter().any(|v| !v.fields.is_empty())
}

/// Returns true if `ty` references (directly or via Optional/Vec wrap) a Named type whose
/// name is in `untagged_data_enum_names`.  Used to choose the correct getter / From-impl
/// branch in the PHP binding code generator.
pub(crate) fn ty_references_untagged_data_enum(ty: &TypeRef, untagged_data_enum_names: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Named(n) => untagged_data_enum_names.contains(n.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            ty_references_untagged_data_enum(inner, untagged_data_enum_names)
        }
        _ => false,
    }
}

/// Compute the flat struct field name for a single field of a variant.
///
/// For tuple variants (fields named `_0`, `_1`, …), the flat field name is derived from
/// the variant name to avoid collisions when multiple variants each have a positional `_0`:
/// - Single-field tuple variant `Foo(_0)` → `foo`
/// - Multi-field tuple variant `Foo(_0, _1)` → `foo_0`, `foo_1`
///
/// For struct variants (named fields), the field's own name is used unchanged.
///
/// `pub(crate)` because the PHPStan stub must name the flat enum's `#[php(getter)]`-backed
/// properties identically to the runtime (`get_<flat_name>` registers the property `<flat_name>`);
/// re-deriving the tuple-variant collision rule in `type_stubs.rs` is exactly how the two paths
/// drift apart. ~keep
pub(crate) fn flat_field_name(variant: &EnumVariant, field_index: usize) -> String {
    if crate::codegen::conversions::is_tuple_variant(&variant.fields) {
        let base = crate::codegen::naming::pascal_to_snake(&variant.name);
        if variant.fields.len() == 1 {
            base
        } else {
            format!("{base}_{field_index}")
        }
    } else {
        variant.fields[field_index].name.clone()
    }
}

/// Generate `#[php]` static factory methods for an [`is_labeled_string_enum`] flat data enum: a
/// zero-arg factory per unit variant (`EntityCategory::person()`) and a one-arg factory per
/// single-field `String` tuple variant (`EntityCategory::custom($label)`).
///
/// Deliberately bypasses the shared `collect_all_variant_constructors`/`variant_field_init`
/// machinery `gen_flat_data_enum_variant_constructors` uses for struct-variant enums: that
/// machinery is built to convert an arbitrary field shape (opaque types, `Vec<NamedStruct>`,
/// boxed fields) through the generic param/let-binding pipeline, and unconditionally filters out
/// exactly the two variant shapes this predicate exists for (unit, tuple). Every param here is a
/// plain `String`, so the trivial per-shape templates below are both correct and the whole reason
/// this narrower shape avoids the `E0716`/`E0596`/`E0507` risk `is_labeled_string_enum`'s doc
/// comment describes for the general machinery. ~keep
fn gen_labeled_string_enum_variant_constructors(enum_def: &EnumDef, core_import: &str) -> Vec<String> {
    use crate::codegen::generators::variant_constructor_is_reachable;
    use crate::codegen::naming::{pascal_to_snake, to_php_name};

    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);

    enum_def
        .variants
        .iter()
        .filter(|variant| !variant.binding_excluded && variant_constructor_is_reachable(variant, is_host_enum))
        .filter_map(|variant| {
            let snake_name = pascal_to_snake(&variant.name);
            let php_name = to_php_name(&snake_name);
            let rust_fn_name = format!("_factory_{snake_name}");
            if variant.fields.is_empty() {
                Some(crate::backends::php::template_env::render(
                    "php_flat_enum_unit_variant_constructor.jinja",
                    minijinja::context! {
                        php_name => php_name,
                        rust_fn_name => rust_fn_name,
                        core_path => &core_path,
                        variant_name => &variant.name,
                    },
                ))
            } else if variant.fields.len() == 1 && crate::codegen::conversions::is_tuple_variant(&variant.fields) {
                let param_name = to_php_name(&flat_field_name(variant, 0));
                Some(crate::backends::php::template_env::render(
                    "php_flat_enum_label_variant_constructor.jinja",
                    minijinja::context! {
                        php_name => php_name,
                        rust_fn_name => rust_fn_name,
                        core_path => &core_path,
                        variant_name => &variant.name,
                        param_name => &param_name,
                    },
                ))
            } else {
                None
            }
        })
        .collect()
}

/// Generate a flat `#[php_class]` struct for a tagged data enum.
///
/// The struct unions all variant fields as `Option<T>` plus a string discriminator named
/// after the serde tag (defaulting to `"type"`). This lets `HashMap<String, SecuritySchemeInfo>`
/// stay as `HashMap<String, SecuritySchemeInfo>` (the flat PHP class) with working `From` impls.
pub(crate) fn gen_flat_data_enum(enum_def: &EnumDef, mapper: &PhpMapper, php_namespace: Option<&str>) -> String {
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);

    let php_attrs: String = if let Some(ns) = php_namespace {
        let ns_escaped = ns.replace('\\', "\\\\");
        let php_name_attr = format!("php(name = \"{}\\\\{}\")", ns_escaped, enum_def.name);
        format!("#[php_class]\n#[{php_name_attr}]")
    } else {
        "#[php_class]".to_string()
    };

    // An externally tagged enum's class cannot carry `#[derive(serde::Serialize, Deserialize)]`:
    // the derived impls read and write this class's own flat `{"type": ..}` object, while serde
    // writes a bare string for a unit variant and a single-keyed object for a data one. It gets
    // hand-written impls from `gen_external_enum_serde_impls` instead, which also means the
    // field-level `#[serde(..)]` attributes must go -- they do not compile without a derive. ~keep
    let external = is_labeled_string_enum(enum_def);
    let (start_template, tag_template, field_template) = if external {
        (
            "php_external_enum_struct_start.jinja",
            "php_external_enum_tag_field.jinja",
            "php_external_enum_option_field.jinja",
        )
    } else {
        (
            "php_flat_enum_struct_start.jinja",
            "php_flat_enum_tag_field.jinja",
            "php_flat_enum_option_field.jinja",
        )
    };

    let mut out = String::new();
    out.push_str(&crate::backends::php::template_env::render(
        start_template,
        minijinja::context! {
            php_attrs => &php_attrs,
            enum_name => &enum_def.name,
        },
    ));
    out.push_str(&crate::backends::php::template_env::render(
        tag_template,
        minijinja::context! {
            tag_field => tag_field,
        },
    ));

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for (idx, field) in variant.fields.iter().enumerate() {
            let flat_name = flat_field_name(variant, idx);
            if seen.insert(flat_name.clone()) {
                let mapped = mapper.map_type(&field.ty).to_string();
                let field_ty = format!("Option<{mapped}>");
                out.push_str(&crate::backends::php::template_env::render(
                    field_template,
                    minijinja::context! {
                        flat_name => &flat_name,
                        field_ty => &field_ty,
                    },
                ));
            }
        }
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_struct_end.jinja",
        minijinja::Value::default(),
    ));
    if external {
        out.push_str(&gen_external_enum_serde_impls(enum_def));
    }
    out
}

/// Hand-written `Serialize`/`Deserialize` for an [`is_labeled_string_enum`] flat class, matching
/// the wire serde produces for an EXTERNALLY tagged enum.
///
/// serde writes a unit variant as a bare string (`"markdown"`) and a data variant as a
/// single-keyed object whose value IS the payload (`{"custom":"latex"}`) -- and a variant carrying
/// `#[serde(untagged)]` as its bare payload with no wrapper at all. The derived impl this replaces
/// read and wrote the class's own flat `{"type":"markdown"}` object instead, so no real wire value
/// ever deserialized: a downstream PHP binding raised
/// `invalid type: string "markdown", expected struct OutputFormat` on every `outputFormat` call.
///
/// `#[serde(untagged)]` is honoured per VARIANT, not just per enum (`EnumVariant::serde_untagged`);
/// `serde_enum_repr` models only the enum-level flag, so an otherwise externally tagged enum can
/// still carry one variant written bare. Emitting the keyed form for such a variant would produce
/// impls that cannot read their own wire.
///
/// `visit_map` still accepts the flat object form so JSON this class previously emitted keeps
/// deserializing; the tag key disambiguates it from serde's single-keyed form. ~keep
pub(crate) fn gen_external_enum_serde_impls(enum_def: &EnumDef) -> String {
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let enum_name = &enum_def.name;

    let mut serialize_arms = String::new();
    let mut visit_str_arms = String::new();
    let mut visit_map_arms = String::new();
    let mut flat_field_arms = String::new();
    let mut untagged_fallback: Option<String> = None;
    let mut seen_flat_fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut variant_list: Vec<String> = Vec::new();

    // Every flat field the struct declares, in declaration order, so each constructed value can
    // list all of them. `..Default::default()` would be shorter but trips `clippy::needless_update`
    // on a two-field enum where the arm already sets both -- and the generated crate is compiled
    // under `-D warnings`. ~keep
    let flat_fields: Vec<String> = {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        enum_def
            .variants
            .iter()
            .flat_map(|variant| (0..variant.fields.len()).map(move |index| flat_field_name(variant, index)))
            .filter(|flat_name| seen.insert(flat_name.clone()))
            .collect()
    };
    // `{tag_field}_tag: <tag>` plus every flat field, the named one carrying `value` and the rest
    // `None`.
    let literal = |tag: &str, set_field: Option<&str>, value_expr: &str| {
        let mut parts = vec![format!("{tag_field}_tag: {tag:?}.to_string()")];
        for flat in &flat_fields {
            if Some(flat.as_str()) == set_field {
                parts.push(format!("{flat}: {value_expr}"));
            } else {
                parts.push(format!("{flat}: None"));
            }
        }
        format!("{enum_name} {{ {} }}", parts.join(", "))
    };

    for variant in &enum_def.variants {
        let tag = variant_tag_value(variant, enum_def);
        variant_list.push(format!("{tag:?}"));

        if variant.fields.is_empty() {
            serialize_arms.push_str(&format!("            {tag:?} => serializer.serialize_str({tag:?}),\n"));
            visit_str_arms.push_str(&format!(
                "                    {tag:?} => Ok({}),\n",
                literal(&tag, None, "")
            ));
            continue;
        }

        let label = flat_field_name(variant, 0);
        if seen_flat_fields.insert(label.clone()) {
            flat_field_arms.push_str(&format!(
                "                            {label:?} => value.{label} = entry.as_str().map(str::to_string),\n"
            ));
        }

        if variant.serde_untagged {
            serialize_arms.push_str(&format!(
                "            {tag:?} => serializer.serialize_str(self.{label}.as_deref().unwrap_or_default()),\n"
            ));
            untagged_fallback = Some(format!(
                "                    value => Ok({}),\n",
                literal(&tag, Some(&label), "Some(value.to_string())")
            ));
            continue;
        }

        serialize_arms.push_str(&format!(
            "            {tag:?} => {{\n                use serde::ser::SerializeMap;\n                let mut map = serializer.serialize_map(Some(1))?;\n                map.serialize_entry({tag:?}, self.{label}.as_deref().unwrap_or_default())?;\n                map.end()\n            }}\n"
        ));
        visit_map_arms.push_str(&format!(
            "                    Some((key, entry)) if key == {tag:?} => Ok({}),\n",
            literal(&tag, Some(&label), "entry.as_str().map(str::to_string)")
        ));
    }

    // `Default::default()` leaves the tag empty -- the state a container with `#[serde(default)]`
    // lands in when the field is absent -- and `Serialize` then writes a bare `""`. Without this
    // arm an untagged fallback would claim it and turn the default back into `Custom("")` on the
    // next parse, so a construct-default / serialize / parse round trip would not be stable. ~keep
    visit_str_arms.push_str(&format!("                    \"\" => Ok({enum_name}::default()),\n"));

    let variant_list = variant_list.join(", ");
    let visit_str_fallback = untagged_fallback.unwrap_or_else(|| {
        format!("                    value => Err(serde::de::Error::unknown_variant(value, &[{variant_list}])),\n")
    });

    crate::backends::php::template_env::render(
        "php_external_enum_serde.jinja",
        minijinja::context! {
            enum_name => enum_name,
            tag_field => tag_field,
            serialize_arms => &serialize_arms,
            visit_str_arms => &visit_str_arms,
            visit_str_fallback => &visit_str_fallback,
            visit_map_arms => &visit_map_arms,
            flat_field_arms => &flat_field_arms,
            variant_list => &variant_list,
        },
    )
}

/// Generate `#[php_impl]` accessor methods, a `from_json` constructor, and per-variant constructors
/// for the flat data enum. `opaque_types` / `bridge_type_aliases` / `enum_names` / `core_import` are
/// threaded into the per-variant constructor machinery so it reuses the same param and conversion
/// logic the flat-enum `From` impl and method bodies use.
///
/// `configured_features` narrows `allowed_tags` -- the `from_json` validation list -- to variants
/// [`enum_variant_declaration`] does not resolve to [`VariantDeclaration::Drop`]. Without this, a
/// FOREIGN cfg-gated variant proven unreachable stayed in `allowed_tags` even though
/// [`gen_flat_data_enum_from_impls`]'s binding->core match already drops its arm unconditionally:
/// `from_json` would accept the tag, then the eventual `.into()` conversion falls through to the
/// catch-all, silently producing a different variant (or `unreachable!()`) instead of ever
/// constructing the one the tag named. Rejecting the tag up front at `from_json` closes that gap
/// instead of deferring to a worse failure downstream. ~keep
#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_flat_data_enum_methods(
    enum_def: &EnumDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    bridge_type_aliases: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    core_import: &str,
    configured_features: Option<&HashSet<&str>>,
) -> String {
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let mut impl_builder = ImplBuilder::new(&enum_def.name);
    impl_builder.add_attr("php_impl");

    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);
    let allowed_tags = enum_def
        .variants
        .iter()
        .filter(|variant| {
            !matches!(
                enum_variant_declaration(variant, is_host_enum, configured_features),
                VariantDeclaration::Drop
            )
        })
        .map(|variant| format!("{:?}", variant_tag_value(variant, enum_def)))
        .collect::<Vec<_>>()
        .join(" | ");
    let from_json = crate::backends::php::template_env::render(
        "php_flat_enum_from_json.jinja",
        minijinja::context! {
            enum_name => &enum_def.name,
            tag_field => tag_field,
            allowed_tags => allowed_tags,
        },
    );
    impl_builder.add_method(&from_json);

    for ctor in gen_flat_data_enum_variant_constructors(
        enum_def,
        mapper,
        opaque_types,
        bridge_type_aliases,
        enum_names,
        core_import,
    ) {
        impl_builder.add_method(&ctor);
    }

    // `collect_all_variant_constructors` (used above) skips BOTH unit and tuple variants -- it
    // exists for internally-tagged struct-variant enums, where a bare unit value isn't a
    // meaningful "constructor" the way a tagged data enum's named fields are. A labeled string
    // enum needs the opposite: its unit variants are exactly the values PHP callers previously
    // reached via the class constants `is_tagged_data_enum` now routes away from
    // (`EntityCategory::PERSON` -> `EntityCategory::person()`), and its label variant is the
    // whole point of this shape, so a dedicated generator handles both here instead. ~keep
    if is_labeled_string_enum(enum_def) {
        for ctor in gen_labeled_string_enum_variant_constructors(enum_def, core_import) {
            impl_builder.add_method(&ctor);
        }
    }

    let tag_getter = format!(
        "#[php(getter, change_case = \"snake_case\")]\npub fn get_{tag_field}_tag(&self) -> String {{\n    self.{tag_field}_tag.clone()\n}}"
    );
    impl_builder.add_method(&tag_getter);

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for (idx, field) in variant.fields.iter().enumerate() {
            let flat_name = flat_field_name(variant, idx);
            if seen.insert(flat_name.clone()) {
                let mapped = mapper.map_type(&field.ty).to_string();
                let field_ty = format!("Option<{mapped}>");
                let body_expr = if is_php_copy_type(&field.ty) {
                    format!("self.{flat_name}")
                } else {
                    format!("self.{flat_name}.clone()")
                };
                let getter_body = format!(
                    "#[php(getter, change_case = \"snake_case\")]\npub fn get_{flat_name}(&self) -> {field_ty} {{\n    {body_expr}\n}}",
                );
                impl_builder.add_method(&getter_body);
            }
        }
    }

    let mut out = String::new();
    let impl_code = impl_builder.build();
    out.push_str(&impl_code);
    out.push('\n');
    out
}

/// Returns the serde-renamed tag string for a variant.
fn variant_tag_value(variant: &EnumVariant, enum_def: &EnumDef) -> String {
    wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        enum_def.serde_rename_all.as_deref(),
    )
}

/// Generate `From<core::DataEnum> for PhpDataEnum` and `From<PhpDataEnum> for core::DataEnum`
/// for a tagged data enum lowered to a flat PHP class.
pub(crate) fn gen_flat_data_enum_from_impls(
    enum_def: &EnumDef,
    core_import: &str,
    configured_features: Option<&[String]>,
) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    let tag_field = crate::codegen::serde_enum_repr::tagged_object_tag_key(enum_def);
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = &enum_def.name;
    // A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's
    // own cfg gate; this PHP crate never declares a Cargo feature for it (see
    // `codegen::cfg::collect_cfg_gates`), so forwarding it verbatim as `#[cfg(...)]` produces an
    // `unexpected cfg condition value` error. Such a variant is dropped from both match arms
    // below instead -- named via `tracing::debug!`, with `codegen::foreign_cfg_variants` raising
    // the same fact to WARN once for the whole run -- mirroring
    // `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper` and
    // `codegen::conversions::enums::gen_enum_from_core_to_binding_cfg`. A host-owned cfg keeps
    // its arm and its `#[cfg(...)]`. ~keep
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let all_flat_fields: std::collections::BTreeSet<String> = {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for variant in &enum_def.variants {
            for (idx, _) in variant.fields.iter().enumerate() {
                seen.insert(flat_field_name(variant, idx));
            }
        }
        seen
    };

    let mut out = String::new();

    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_from_start.jinja",
        minijinja::context! {
            core_path => &core_path,
            binding_name => &binding_name,
        },
    ));
    for variant in &enum_def.variants {
        if variant.cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %enum_def.name,
                enum_rust_path = %enum_def.rust_path,
                variant_name = %variant.name,
                cfg = variant.cfg.as_deref().unwrap_or_default(),
                "dropping PHP flat-enum From<CoreType> arm for a foreign-crate enum variant \
                 behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the variant \
                 is unreachable from this conversion"
            );
            continue;
        }
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_variant_match_empty.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    variant_name => &variant.name,
                    tag_field => tag_field,
                    tag_val => &tag_val,
                    needs_default => !all_flat_fields.is_empty(),
                    cfg => variant.cfg.as_deref(),
                },
            ));
        } else {
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
            let pattern = if is_tuple {
                let names: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        if f.sanitized {
                            format!("_{}", f.name)
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect();
                names.join(", ")
            } else {
                let bindings: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        if f.sanitized {
                            format!("{}: _{}", f.name, f.name)
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect();
                bindings.join(", ")
            };
            let pattern_start = if is_tuple {
                format!("            {core_path}::{}({pattern}) => Self {{", variant.name)
            } else {
                format!("            {core_path}::{}{{ {pattern} }} => Self {{", variant.name)
            };
            // `pattern_start` is a pre-existing raw `format!` site (not this fix's doing --
            // see the `jinja-templates` rule note this leaves for a future pass); the cfg
            // guard itself is a plain attribute line, matching how every other backend
            // (dart, swift, ffi) emits a per-arm `#[cfg(...)]` without a dedicated template.
            if let Some(cfg) = variant.cfg.as_deref() {
                out.push_str("            #[cfg(");
                out.push_str(cfg);
                out.push_str(")]\n");
            }
            out.push_str(&pattern_start);
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_tag_assignment.jinja",
                minijinja::context! {
                    tag_field => tag_field,
                    tag_val => &tag_val,
                },
            ));
            for (idx, f) in variant.fields.iter().enumerate() {
                let flat_name = flat_field_name(variant, idx);
                let bound_var = if f.sanitized {
                    format!("_{}", f.name)
                } else {
                    f.name.clone()
                };
                let expr = flat_enum_core_to_binding_field_expr(f, &bound_var);
                out.push_str(&crate::backends::php::template_env::render(
                    "php_flat_enum_variant_field.jinja",
                    minijinja::context! {
                        flat_name => &flat_name,
                        expr => &expr,
                    },
                ));
            }
            // field — the struct update would have no effect and triggers `clippy::needless_update`.
            let variant_flat_names: std::collections::BTreeSet<String> =
                (0..variant.fields.len()).map(|i| flat_field_name(variant, i)).collect();
            if variant_flat_names == all_flat_fields {
                out.push_str(" },\n");
            } else {
                out.push_str(" ..Default::default() },\n");
            }
        }
    }
    // When the IR has excluded variants (e.g. cfg-gated variants with #[alef(skip)] or
    // #[doc(hidden)]), the Rust compiler sees those variants at compile time but the generated
    // match has no arm for them, so a catch-all is required. A foreign-crate cfg-gated variant
    // (dropped above, unconditionally) leaves the same kind of gap UNLESS this binding's own
    // configured feature set proves the variant unreachable, in which case the gap closes and no
    // catch-all is needed. A host-owned cfg-gated variant never needs one: its arm carries the
    // identical `#[cfg(...)]` as the variant itself, so they compile in or out together and the
    // match stays exhaustive either way -- see
    // `codegen::conversions::enum_conversion_needs_catch_all_for_features`, the same
    // configured-feature-aware resolver every `ConversionConfig`-driven enum conversion already
    // uses, which this defers to so the reimplementations here and in NAPI's
    // `gen_tagged_enum_core_to_binding` can't drift back out of sync with each other or with the
    // shared `gen_enum_from_core_to_binding_cfg` (alef #544). This match is over the real CORE
    // type (`core_path` above), a shape this crate does not declare and cannot influence, so
    // `configured_features`' proof about the dependency is already the complete answer -- `true`
    // here. (The sibling `From<PhpDataEnum> for core::DataEnum` impl below matches a runtime
    // string tag, never a real enum, so `unreachable_patterns` can never fire there and it needs
    // no call into this resolver at all.) See
    // `ConversionConfig::declaration_drops_unreachable_foreign_variants`'s doc comment. ~keep
    if enum_conversion_needs_catch_all_for_features(
        enum_def,
        is_host_enum,
        !enum_def.excluded_variants.is_empty(),
        configured_features,
        true,
    ) {
        out.push_str("            _ => Default::default(),\n");
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_match_end.jinja",
        minijinja::Value::default(),
    ));

    // (may have #[serde(rename = "camelCase")] on individual variant fields).
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_into_start.jinja",
        minijinja::context! {
            binding_name => &binding_name,
            core_path => &core_path,
            tag_field => tag_field,
        },
    ));
    for variant in &enum_def.variants {
        if variant.cfg.is_some() && !is_host_enum {
            tracing::debug!(
                enum_name = %enum_def.name,
                enum_rust_path = %enum_def.rust_path,
                variant_name = %variant.name,
                cfg = variant.cfg.as_deref().unwrap_or_default(),
                "dropping PHP flat-enum From<Binding> arm for a foreign-crate enum variant \
                 behind a #[cfg(...)] this crate cannot declare as a Cargo feature; the tag \
                 falls through to the existing fallback arm instead"
            );
            continue;
        }
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_variant_match_into_empty.jinja",
                minijinja::context! {
                    tag_val => &tag_val,
                    core_path => &core_path,
                    variant_name => &variant.name,
                    cfg => variant.cfg.as_deref(),
                },
            ));
        } else {
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
            let pattern_start = if is_tuple {
                format!("            \"{tag_val}\" => {core_path}::{}(", variant.name)
            } else {
                format!("            \"{tag_val}\" => {core_path}::{}{{", variant.name)
            };
            // See the matching note in the core→binding loop above: `pattern_start` is a
            // pre-existing raw `format!` site; the cfg guard is a plain attribute line.
            if let Some(cfg) = variant.cfg.as_deref() {
                out.push_str("            #[cfg(");
                out.push_str(cfg);
                out.push_str(")]\n");
            }
            out.push_str(&pattern_start);
            if is_tuple {
                let exprs: Vec<String> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(idx, f)| flat_enum_binding_to_core_field_expr(f, &flat_field_name(variant, idx)))
                    .collect();
                out.push_str(&crate::backends::php::template_env::render(
                    "php_flat_enum_tuple_exprs.jinja",
                    minijinja::context! {
                        exprs_joined => exprs.join(", "),
                    },
                ));
                out.push_str(" ),\n");
            } else {
                for (idx, f) in variant.fields.iter().enumerate() {
                    let flat_name = flat_field_name(variant, idx);
                    let expr = flat_enum_binding_to_core_field_expr(f, &flat_name);
                    out.push_str(&crate::backends::php::template_env::render(
                        "php_flat_enum_variant_field.jinja",
                        minijinja::context! {
                            flat_name => &flat_name,
                            expr => &expr,
                        },
                    ));
                }
                out.push_str(" },\n");
            }
        }
    }
    // enum has a visible `Default` impl (a variant with `#[default]`): delegate to
    // `impl Default` is marked `#[cfg_attr(alef, alef(skip))]` it is invisible to Alef's IR,
    let core_has_default = enum_def.variants.iter().any(|v| v.is_default);
    if core_has_default {
        out.push_str(&crate::backends::php::template_env::render(
            "php_flat_enum_default_fallback_match_arm.jinja",
            minijinja::context! {
                core_path => &core_path,
            },
        ));
    } else {
        out.push_str(
            "            _ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\"),\n",
        );
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_match_end.jinja",
        minijinja::Value::default(),
    ));

    let _ = TypeRef::Unit;
    let _ = PrimitiveType::Bool;

    out
}

/// Build the expression for a single flat-enum variant field when converting core → binding.
/// The binding struct field is always `Option<MappedType>` for flat data enums.
///
/// - Sanitized fields cannot be converted (the core type is unknown/complex); emit `None`.
/// - `is_boxed` fields: unbox with `*` before converting.
/// - `TypeRef::Path`: convert via `to_string_lossy().into_owned()`.
/// - `TypeRef::Primitive(Usize | U64 | Isize)`: cast to `i64` (PHP's integer representation).
/// - Everything else: use `.into()` / `.map(Into::into)`.
fn flat_enum_core_to_binding_field_expr(f: &crate::core::ir::FieldDef, bound_var: &str) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};

    if f.sanitized {
        return "None".to_string();
    }

    let wrap_some = |inner: String| -> String { format!("Some({inner})") };

    match &f.ty {
        TypeRef::Path => {
            if f.optional {
                format!("{bound_var}.map(|p| p.to_string_lossy().into_owned())")
            } else {
                wrap_some(format!("{bound_var}.to_string_lossy().into_owned()"))
            }
        }
        TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::Isize) => {
            if f.optional {
                format!("{bound_var}.map(|v| v as i64)")
            } else {
                wrap_some(format!("{bound_var} as i64"))
            }
        }
        TypeRef::Named(_) if f.is_boxed => {
            if f.optional {
                format!("{bound_var}.map(|v| (*v).into())")
            } else {
                wrap_some(format!("(*{bound_var}).into()"))
            }
        }
        TypeRef::Primitive(_) | TypeRef::String => {
            if f.optional {
                bound_var.to_string()
            } else {
                wrap_some(bound_var.to_string())
            }
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if f.optional {
                format!("{bound_var}.map(|v| v.into_iter().map(Into::into).collect())")
            } else {
                wrap_some(format!("{bound_var}.into_iter().map(Into::into).collect()"))
            }
        }
        _ => {
            if f.optional {
                format!("{bound_var}.map(Into::into)")
            } else {
                wrap_some(format!("{bound_var}.into()"))
            }
        }
    }
}

/// Build the expression for a single flat-enum variant field when converting binding → core.
/// The binding struct field is always `Option<MappedType>`; the core field may be non-optional.
///
/// - Sanitized fields: emit `Default::default()` (cannot round-trip through PHP).
/// - `is_boxed` fields: wrap the result in `Box::new(...)`.
/// - `TypeRef::Path`: convert `String → PathBuf` via `PathBuf::from`.
/// - `TypeRef::Primitive(Usize | U64 | Isize)`: cast `i64 → usize/u64/isize`.
/// - Everything else: `.into()` / `.map(Into::into)`.
fn flat_enum_binding_to_core_field_expr(f: &crate::core::ir::FieldDef, flat_name: &str) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};

    if f.sanitized {
        return if f.is_boxed {
            "Box::default()".to_string()
        } else {
            "Default::default()".to_string()
        };
    }

    let expr = match &f.ty {
        TypeRef::Path => {
            if f.optional {
                format!("val.{flat_name}.map(std::path::PathBuf::from)")
            } else {
                format!("val.{flat_name}.map(std::path::PathBuf::from).unwrap_or_default()")
            }
        }
        TypeRef::Primitive(p @ (PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::Isize)) => {
            let core_ty = match p {
                PrimitiveType::Usize => "usize",
                PrimitiveType::U64 => "u64",
                PrimitiveType::Isize => "isize",
                _ => unreachable!(),
            };
            if f.optional {
                format!("val.{flat_name}.map(|v| v as {core_ty})")
            } else {
                format!("val.{flat_name}.map(|v| v as {core_ty}).unwrap_or_default()")
            }
        }
        TypeRef::Primitive(_) | TypeRef::String => {
            if f.optional {
                format!("val.{flat_name}")
            } else {
                format!("val.{flat_name}.unwrap_or_default()")
            }
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            if f.optional {
                format!("val.{flat_name}.map(|v| v.into_iter().map(Into::into).collect())")
            } else {
                format!("val.{flat_name}.map(|v| v.into_iter().map(Into::into).collect()).unwrap_or_default()")
            }
        }
        _ => {
            if f.optional {
                format!("val.{flat_name}.map(Into::into)")
            } else {
                format!("val.{flat_name}.map(Into::into).unwrap_or_default()")
            }
        }
    };

    if f.is_boxed { format!("Box::new({expr})") } else { expr }
}

/// Generate `#[php]` per-variant constructors for a flat data enum.
///
/// For a tagged data enum `Shape { Circle { radius }, Rect { width, height } }` lowered to the flat
/// PHP class `Shape { type_tag, circle: Option<..>, rect: Option<..> }`, emits one static method per
/// data-carrying struct variant so PHP callers write `Shape::circle($radius)` /
/// `Shape::rect($width, $height)` instead of hand-rolling a JSON blob for `from_json`.
///
/// Construction is wrapper-convert (like extendr/pyo3): the method builds the CORE variant
/// (`<core_path>::<Variant> { field: <core_expr> }`) and converts to the flat PHP struct via the
/// generated `From<core::Enum> for PhpEnum` impl with `.into()`. This means there is exactly ONE
/// param→value converter — the shared `gen_php_named_let_bindings` / `gen_php_call_args_with_let_bindings_vec`
/// machinery that every PHP method body already uses — so every field shape (`Bytes`, `Json`,
/// `Vec<NamedStruct>`, opaque, enum-as-String, …) is handled identically to a method call, not by a
/// parallel hand-rolled converter. When any param conversion is fallible (a `Vec<NamedStruct>` field
/// decodes element-by-element and can `return Err`), the method returns `PhpResult<Self>`.
///
/// Variant selection (skipping unit/tuple/`binding_excluded` variants; *not* skipping a name
/// collision with a hand-written `impl` method) is shared with pyo3/magnus/rustler via
/// `collect_all_variant_constructors` — no backend forwards `enum_def.methods` into its generated
/// output, so a hand-written static method never actually replaces the derived factory; suppressing
/// it on a name collision dropped the variant constructor with nothing to replace it
/// (`ContentPart.text(...)`/`.image_url(...)` were unreachable). The Rust fn is `_factory_<snake>`
/// (exposed to PHP under the camelCase snake name) to mirror the pyo3/magnus disambiguation against
/// the same-named variant accessor.
///
/// Returns the method bodies to splice into the flat enum's `#[php_impl]` block (empty when no
/// variant qualifies).
#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_flat_data_enum_variant_constructors(
    enum_def: &EnumDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    bridge_type_aliases: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    core_import: &str,
) -> Vec<String> {
    use super::super::helpers::{
        gen_php_call_args_with_let_bindings_vec, gen_php_function_params, gen_php_named_let_bindings,
        param_conversion_is_fallible,
    };
    use crate::codegen::generators::{collect_all_variant_constructors, variant_constructor_is_reachable};
    use crate::codegen::naming::to_php_name;

    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let mutex_types: AHashSet<String> = AHashSet::new();
    let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);

    let qualifying = collect_all_variant_constructors(enum_def);
    qualifying
        .iter()
        .filter_map(|ctor| {
            let variant = enum_def.variants.iter().find(|v| v.name == ctor.variant_name)?;
            // The factory body below builds `<core_path>::<Variant> { .. }` directly (the
            // wrapper-convert model, like extendr/pyo3): a FOREIGN variant behind a `#[cfg(...)]`
            // this crate cannot declare as its own Cargo feature has no compile-safe fallback the
            // way a match arm's `_ => ..` wildcard has, so it must be dropped unconditionally
            // rather than kept on the "maybe reachable" assumption `enum_variant_declaration`
            // affords a mere declaration line. See `variant_constructor_is_reachable`. ~keep
            if !variant_constructor_is_reachable(variant, is_host_enum) {
                return None;
            }

            let params = gen_php_function_params(&ctor.params, mapper, opaque_types, bridge_type_aliases);

            let inline = |p: &crate::core::ir::ParamDef| -> Option<String> {
                match &p.ty {
                    TypeRef::Named(name)
                        if !opaque_types.contains(name.as_str()) && !enum_names.contains(name.as_str()) =>
                    {
                        let php_name = to_php_name(&p.name);
                        Some(if p.optional {
                            format!("{php_name}.map(|v| v.clone().into())")
                        } else {
                            format!("{php_name}.clone().into()")
                        })
                    }
                    _ => None,
                }
            };
            let let_binding_params: Vec<crate::core::ir::ParamDef> =
                ctor.params.iter().filter(|p| inline(p).is_none()).cloned().collect();
            let let_bindings = gen_php_named_let_bindings(&let_binding_params, opaque_types, enum_names, core_import);
            let arg_exprs = gen_php_call_args_with_let_bindings_vec(&ctor.params, opaque_types, &mutex_types);
            let field_inits: Vec<String> = ctor
                .params
                .iter()
                .zip(arg_exprs)
                .enumerate()
                .map(|(idx, (p, expr))| {
                    let value = inline(p).unwrap_or(expr);
                    let value = if ctor.boxed[idx] {
                        if p.optional {
                            format!("{value}.map(Box::new)")
                        } else {
                            format!("Box::new({value})")
                        }
                    } else {
                        value
                    };
                    crate::codegen::field_init::struct_field_init(&p.name, &value)
                })
                .collect();
            let core_variant = format!("{core_path}::{} {{ {} }}", variant.name, field_inits.join(", "));

            let fallible = ctor
                .params
                .iter()
                .any(|p| param_conversion_is_fallible(p, opaque_types, enum_names));

            Some(crate::backends::php::template_env::render(
                "php_flat_enum_variant_constructor.jinja",
                minijinja::context! {
                    php_name => to_php_name(&ctor.snake_name),
                    rust_fn_name => format!("_factory_{}", ctor.snake_name),
                    params => params,
                    let_bindings => let_bindings,
                    core_variant => core_variant,
                    fallible => fallible,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod declaration_tests;
#[cfg(test)]
mod tests;
