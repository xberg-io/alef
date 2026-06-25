use crate::backends::php::type_map::PhpMapper;
use crate::codegen::builder::ImplBuilder;
use crate::codegen::naming::wire_variant_value;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, TypeRef};
use ahash::AHashSet;

use super::structs::is_php_copy_type;

/// Generate a `#[php_class]` with class constants for unit-variant enums.
///
/// Emits a PHP-visible class that allows consumers to reference enum values as constants.
pub(crate) fn gen_enum_constants(enum_def: &EnumDef, php_namespace: Option<&str>) -> String {
    let mut lines = vec![];

    // Emit the #[php_class] decorator with optional namespace.
    // ext-php-rs 0.15+ removed `#[php_class(name = "...")]` / `(namespace = "...")`;
    // namespace must use the two-attribute form `#[php_class]` + `#[php(name = "Ns\\ClassName")]`.
    // Backslashes in the namespace must be escaped in the generated Rust string literal.
    if let Some(ns) = php_namespace {
        let ns_escaped = ns.replace('\\', "\\\\");
        lines.push("#[php_class]".to_string());
        lines.push(format!("#[php(name = \"{}\\\\{}\")]", ns_escaped, enum_def.name));
    } else {
        lines.push("#[php_class]".to_string());
    }

    // Emit the PHP class struct (with no fields — exists only for constants).
    lines.push(format!("pub struct {} {{}}", enum_def.name));
    lines.push(String::new());

    // Emit the #[php_impl] block with class constants.
    lines.push("#[php_impl]".to_string());
    lines.push(format!("impl {} {{", enum_def.name));

    for variant in &enum_def.variants {
        let const_name = escape_php_reserved_constant(&variant.name.to_uppercase());
        lines.push(format!("    pub const {}: &str = \"{}\";", const_name, variant.name));
    }

    lines.push("}".to_string());

    lines.join("\n")
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

/// Return true if an enum is a "tagged data enum" — has a serde tag AND at least one variant
/// with named fields. These are lowered to flat PHP classes rather than string constants.
pub(crate) fn is_tagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|v| !v.fields.is_empty())
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
fn flat_field_name(variant: &EnumVariant, field_index: usize) -> String {
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

/// Generate a flat `#[php_class]` struct for a tagged data enum.
///
/// The struct unions all variant fields as `Option<T>` plus a string discriminator named
/// after the serde tag (defaulting to `"type"`). This lets `HashMap<String, SecuritySchemeInfo>`
/// stay as `HashMap<String, SecuritySchemeInfo>` (the flat PHP class) with working `From` impls.
pub(crate) fn gen_flat_data_enum(enum_def: &EnumDef, mapper: &PhpMapper, php_namespace: Option<&str>) -> String {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    let php_attrs: String = if let Some(ns) = php_namespace {
        let ns_escaped = ns.replace('\\', "\\\\");
        let php_name_attr = format!("php(name = \"{}\\\\{}\")", ns_escaped, enum_def.name);
        format!("#[php_class]\n#[{php_name_attr}]")
    } else {
        "#[php_class]".to_string()
    };

    let mut out = String::new();
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_struct_start.jinja",
        minijinja::context! {
            php_attrs => &php_attrs,
            enum_name => &enum_def.name,
        },
    ));
    // Discriminator field
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_tag_field.jinja",
        minijinja::context! {
            tag_field => tag_field,
        },
    ));

    // Collect all unique flat fields across variants, all made Optional.
    // For tuple variants each positional field gets a per-variant name so that
    // `System(_0: SystemMessage)` and `User(_0: UserMessage)` produce distinct
    // fields `system: Option<SystemMessage>` and `user: Option<UserMessage>`.
    // For struct variants the original field name is used (shared across variants).
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for (idx, field) in variant.fields.iter().enumerate() {
            let flat_name = flat_field_name(variant, idx);
            if seen.insert(flat_name.clone()) {
                let mapped = mapper.map_type(&field.ty).to_string();
                // All variant fields become Option in the flat struct. If the core
                // field is already optional (field.optional == true), the mapped type
                // is the inner type and we still wrap it in Option.
                let field_ty = format!("Option<{mapped}>");
                out.push_str(&crate::backends::php::template_env::render(
                    "php_flat_enum_option_field.jinja",
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
    out
}

/// Generate `#[php_impl]` accessor methods, a `from_json` constructor, and per-variant constructors
/// for the flat data enum. `opaque_types` / `bridge_type_aliases` / `enum_names` / `core_import` are
/// threaded into the per-variant constructor machinery so it reuses the same param and conversion
/// logic the flat-enum `From` impl and method bodies use.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_flat_data_enum_methods(
    enum_def: &EnumDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    bridge_type_aliases: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    core_import: &str,
) -> String {
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let mut impl_builder = ImplBuilder::new(&enum_def.name);
    impl_builder.add_attr("php_impl");

    // from_json constructor so PHP can construct the value.
    let from_json = "#[php(name = \"from_json\")]\npub fn from_json(json: String) -> PhpResult<Self> {\n    \
        serde_json::from_str(&json)\n        \
        .map_err(|e| PhpException::default(e.to_string()))\n\
        }"
    .to_string();
    impl_builder.add_method(&from_json);

    // Per-variant constructors — `Shape::circle($radius)` rather than a hand-built `from_json` blob.
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

    // Getter for the tag discriminator field
    let tag_getter =
        format!("#[php(getter)]\npub fn get_{tag_field}_tag(&self) -> String {{\n    self.{tag_field}_tag.clone()\n}}");
    impl_builder.add_method(&tag_getter);

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for (idx, field) in variant.fields.iter().enumerate() {
            let flat_name = flat_field_name(variant, idx);
            if seen.insert(flat_name.clone()) {
                let mapped = mapper.map_type(&field.ty).to_string();
                // Getter returns Option<T> for every variant field (all are optional in flat struct).
                let field_ty = format!("Option<{mapped}>");
                // For Copy types (Option<primitive>), omit `.clone()` to avoid clone_on_copy.
                let body_expr = if is_php_copy_type(&field.ty) {
                    format!("self.{flat_name}")
                } else {
                    format!("self.{flat_name}.clone()")
                };
                let getter_body =
                    format!("#[php(getter)]\npub fn get_{flat_name}(&self) -> {field_ty} {{\n    {body_expr}\n}}",);
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
pub(crate) fn gen_flat_data_enum_from_impls(enum_def: &EnumDef, core_import: &str) -> String {
    use crate::core::ir::{PrimitiveType, TypeRef};
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = &enum_def.name;

    // Pre-compute the complete set of flat struct field names (excluding the tag discriminator).
    // This lets us detect when a variant covers ALL fields so we can omit `..Default::default()`.
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

    // --- core → binding ---
    // Converts a core enum value to the flat PHP binding struct.
    // Destructuring patterns for boxed fields use the raw IR name (e.g. `_0`);
    // sanitized fields are excluded from the pattern (bound with `_`-prefixed name),
    // Path fields are converted via `to_string_lossy()`, Usize/U64/Isize are cast to i64.
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_from_start.jinja",
        minijinja::context! {
            core_path => &core_path,
            binding_name => &binding_name,
        },
    ));
    for variant in &enum_def.variants {
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            // No variant fields: only the tag is set; all Option fields default to None.
            // `..Default::default()` is only needed when the struct has other fields.
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_variant_match_empty.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    variant_name => &variant.name,
                    tag_field => tag_field,
                    tag_val => &tag_val,
                    needs_default => !all_flat_fields.is_empty(),
                },
            ));
        } else {
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
            // Build destructuring pattern.
            // - Tuple variants: positional names (IR names like `_0`, `_1`); sanitized fields
            //   use a discard binding with a leading underscore prefix (e.g. `__0`).
            // - Struct variants: `field` or `field: _field` for sanitized fields (the `_` prefix
            //   suppresses the "unused variable" warning while keeping Rust happy that the field
            //   is mentioned in the pattern — `_field` can't be used in the pattern directly for
            //   struct variants).
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
                            // `field_name: _field_name` — Rust ignores the value but accepts the pattern.
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
                // The destructuring variable name:
                // - tuple variants: sanitized fields use `_`-prefixed IR name.
                // - struct variants: sanitized fields use `_`-prefixed field name (from pattern above).
                // - non-sanitized: use the plain field name.
                let bound_var = if f.sanitized {
                    format!("_{}", f.name)
                } else {
                    f.name.clone()
                };
                // f.optional means the core field is Option<T>; binding is always Option<T>.
                let expr = flat_enum_core_to_binding_field_expr(f, &bound_var);
                out.push_str(&crate::backends::php::template_env::render(
                    "php_flat_enum_variant_field.jinja",
                    minijinja::context! {
                        flat_name => &flat_name,
                        expr => &expr,
                    },
                ));
            }
            // Omit `..Default::default()` when this variant's fields cover every flat struct
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
    // match arms don't cover them, making the match non-exhaustive. Emit a wildcard arm so the
    // match is always exhaustive regardless of which feature flags are active.
    if !enum_def.excluded_variants.is_empty() {
        out.push_str("            _ => Default::default(),\n");
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_match_end.jinja",
        minijinja::Value::default(),
    ));

    // --- binding → core: match on the tag field to reconstruct the correct variant ---
    // We use tag-value matching rather than serde round-trip to avoid serde field rename
    // mismatches between the flat struct (uses Rust snake_case names) and the core type
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
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            out.push_str(&crate::backends::php::template_env::render(
                "php_flat_enum_variant_match_into_empty.jinja",
                minijinja::context! {
                    tag_val => &tag_val,
                    core_path => &core_path,
                    variant_name => &variant.name,
                },
            ));
        } else {
            let is_tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
            let pattern_start = if is_tuple {
                format!("            \"{tag_val}\" => {core_path}::{}(", variant.name)
            } else {
                format!("            \"{tag_val}\" => {core_path}::{}{{", variant.name)
            };
            out.push_str(&pattern_start);
            if is_tuple {
                // Tuple variant: positional syntax uses `, ` separators.
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
                // Struct variant: `field_name: <expr>,` for each field.
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
    // The conversion matches on the tag discriminator `&str`, which is never exhaustively
    // coverable — a wildcard arm is ALWAYS required. Its body depends on whether the core
    // enum has a visible `Default` impl (a variant with `#[default]`): delegate to
    // `<CorePath>::default()` when present, otherwise fail loudly. When the core type's
    // `impl Default` is marked `#[cfg_attr(alef, alef(skip))]` it is invisible to Alef's IR,
    // so `core_has_default` is false and we must emit a non-`Default` wildcard arm.
    let core_has_default = enum_def.variants.iter().any(|v| v.is_default);
    if core_has_default {
        out.push_str(&crate::backends::php::template_env::render(
            "php_flat_enum_default_fallback_match_arm.jinja",
            minijinja::context! {
                core_path => &core_path,
            },
        ));
    } else {
        // No visible `Default`: the binding only ever emits known tags, so an unrecognised
        // tag is unreachable by construction. Emit a panicking wildcard so the `&str` match
        // stays exhaustive without calling a non-existent `Default` impl.
        out.push_str(
            "            _ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\"),\n",
        );
    }
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_impl_match_end.jinja",
        minijinja::Value::default(),
    ));

    // Suppress the unused import warning that would appear when TypeRef/PrimitiveType
    // are only referenced inside the helper closures above (Rust may not see the use).
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
        // Sanitized fields have an unknown/complex core type; we can't produce a PHP value.
        return "None".to_string();
    }

    // Helper: produce `Some(<inner>)` from a raw owned expression.
    let wrap_some = |inner: String| -> String { format!("Some({inner})") };

    match &f.ty {
        TypeRef::Path => {
            // PathBuf → String via to_string_lossy
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
            // Boxed Named: unbox then convert.
            if f.optional {
                format!("{bound_var}.map(|v| (*v).into())")
            } else {
                wrap_some(format!("(*{bound_var}).into()"))
            }
        }
        // Primitives that map to the same type in PHP (all except u64/usize/isize which map
        // to i64) and String: binding type == core type, no conversion needed.
        TypeRef::Primitive(_) | TypeRef::String => {
            if f.optional {
                // Core field is Option<T>, binding field is Option<T>: pass through directly.
                bound_var.to_string()
            } else {
                // Core field is T, binding field is Option<T>: wrap in Some.
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
        // Primitives that map to the same type in PHP (all except u64/usize/isize) and
        // String: binding type == core type, no conversion needed.
        TypeRef::Primitive(_) | TypeRef::String => {
            // Binding field is Option<T>; unwrap for non-optional core fields.
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
/// Variant selection (skipping unit/tuple/`binding_excluded` variants and yielding to a hand-written
/// `impl` method of the same name) is shared with pyo3/magnus via `collect_variant_constructors`. The
/// Rust fn is `_factory_<snake>` (exposed to PHP under the camelCase snake name) to mirror the
/// pyo3/magnus disambiguation against the same-named variant accessor.
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
    use crate::codegen::generators::collect_variant_constructors;
    use crate::codegen::naming::to_php_name;

    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let mutex_types: AHashSet<String> = AHashSet::new();

    // `collect_variant_constructors` decides which variants qualify (skip rules + hand-written
    // suppression); look the EnumVariant back up by name to read its concrete fields.
    let qualifying = collect_variant_constructors(enum_def);
    qualifying
        .iter()
        .filter_map(|ctor| {
            let variant = enum_def.variants.iter().find(|v| v.name == ctor.variant_name)?;

            // Constructor params: each variant field as its PHP binding type. Reuse the shared
            // function-param machinery so refs / enum-as-String / Vec handling stay consistent with
            // the rest of the PHP backend.
            let params = gen_php_function_params(&ctor.params, mapper, opaque_types, bridge_type_aliases);

            // Build the CORE variant struct literal, pairing each field with its converted expression
            // (the same per-param machinery method bodies use). Core field names are the original Rust
            // names; the expressions reference the camelCase PHP params / `_core` let-bindings.
            let let_bindings = gen_php_named_let_bindings(&ctor.params, opaque_types, enum_names, core_import);
            let arg_exprs = gen_php_call_args_with_let_bindings_vec(&ctor.params, opaque_types, &mutex_types);
            let field_inits: Vec<String> = ctor
                .params
                .iter()
                .zip(arg_exprs)
                .map(|(p, expr)| format!("{}: {expr}", p.name))
                .collect();
            let core_variant = format!("{core_path}::{} {{ {} }}", variant.name, field_inits.join(", "));

            // A `Vec<NamedStruct>` field decodes element-by-element and can `return Err`, which forces
            // the whole constructor to return `PhpResult<Self>`.
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
mod variant_constructor_tests {
    use super::*;
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};

    fn mapper() -> PhpMapper {
        PhpMapper {
            enum_names: AHashSet::new(),
            data_enum_names: AHashSet::new(),
            untagged_data_enum_names: AHashSet::new(),
            json_string_enum_names: AHashSet::new(),
        }
    }

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..Default::default()
        }
    }

    fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
            ..Default::default()
        }
    }

    fn shape_enum() -> EnumDef {
        EnumDef {
            name: "Shape".to_string(),
            rust_path: "test_lib::Shape".to_string(),
            variants: vec![
                variant("Circle", vec![field("radius", TypeRef::Primitive(PrimitiveType::F64))]),
                variant(
                    "Rect",
                    vec![
                        field("width", TypeRef::Primitive(PrimitiveType::F64)),
                        field("height", TypeRef::Primitive(PrimitiveType::F64)),
                    ],
                ),
            ],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        }
    }

    fn join(parts: Vec<String>) -> String {
        parts.join("\n")
    }

    /// Run the generator with the common (empty opaque/bridge/enum) sets and the `crate` core import.
    fn run(def: &EnumDef, mapper: &PhpMapper) -> String {
        let empty = AHashSet::new();
        join(gen_flat_data_enum_variant_constructors(
            def, mapper, &empty, &empty, &empty, "crate",
        ))
    }

    #[test]
    fn emits_static_constructor_building_core_variant_then_into() {
        let code = run(&shape_enum(), &mapper());

        // Exposed to PHP under the snake name; Rust fn is `_factory_<snake>` to avoid colliding with
        // the `get_circle` accessor. Wrapper-convert: build the CORE variant then `.into()`.
        assert!(code.contains(r#"#[php(name = "circle")]"#), "{code}");
        assert!(code.contains("pub fn _factory_circle(radius: f64) -> Self"), "{code}");
        assert!(
            code.contains("test_lib::Shape::Circle { radius: radius }.into()"),
            "{code}"
        );
        assert!(code.contains(r#"#[php(name = "rect")]"#), "{code}");
        assert!(
            code.contains("pub fn _factory_rect(width: f64, height: f64) -> Self"),
            "{code}"
        );
        assert!(
            code.contains("test_lib::Shape::Rect { width: width, height: height }.into()"),
            "{code}"
        );
    }

    #[test]
    fn converts_named_dto_field_via_shared_let_binding() {
        // A Named-DTO field arrives as `&T`; the SHARED let-binding machinery emits `<field>_core`
        // and the core variant references it — same path method bodies use, no parallel converter.
        let def = EnumDef {
            name: "Wrapper".to_string(),
            rust_path: "test_lib::Wrapper".to_string(),
            variants: vec![variant(
                "Llm",
                vec![field("llm", TypeRef::Named("LlmConfig".to_string()))],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(code.contains("pub fn _factory_llm(llm: &LlmConfig) -> Self"), "{code}");
        assert!(
            code.contains("let llm_core"),
            "must use the shared _core let binding: {code}"
        );
        assert!(
            code.contains("test_lib::Wrapper::Llm { llm: llm_core }.into()"),
            "{code}"
        );
    }

    #[test]
    fn converts_bytes_field() {
        // Bytes params arrive as `PhpBytes`; the shared call-arg machinery unwraps to `.0` (Vec<u8>)
        // for the core field — a branch the old hand-rolled converter dropped silently.
        let def = EnumDef {
            name: "Blob".to_string(),
            rust_path: "test_lib::Blob".to_string(),
            variants: vec![variant("Raw", vec![field("data", TypeRef::Bytes)])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(code.contains("pub fn _factory_raw(data: PhpBytes) -> Self"), "{code}");
        assert!(code.contains("test_lib::Blob::Raw { data: data.0 }.into()"), "{code}");
    }

    #[test]
    fn converts_json_field() {
        // Json params arrive as a JSON `String`; the shared machinery parses via `serde_json::from_str`
        // through the `<name>_json` let binding — another branch the old converter dropped.
        let def = EnumDef {
            name: "Payload".to_string(),
            rust_path: "test_lib::Payload".to_string(),
            variants: vec![variant("Doc", vec![field("body", TypeRef::Json)])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(code.contains("pub fn _factory_doc(body: String) -> Self"), "{code}");
        assert!(
            code.contains("test_lib::Payload::Doc { body: body_json }.into()"),
            "{code}"
        );
    }

    #[test]
    fn converts_vec_named_struct_field_fallibly() {
        // A `Vec<NamedStruct>` field decodes element-by-element and can `return Err`, so the
        // constructor must return `PhpResult<Self>` and wrap the build in `Ok(...)`.
        let def = EnumDef {
            name: "Batch".to_string(),
            rust_path: "test_lib::Batch".to_string(),
            variants: vec![variant(
                "Many",
                vec![field(
                    "items",
                    TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string()))),
                )],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(
            code.contains("pub fn _factory_many(items: &ext_php_rs::types::ZendHashTable) -> PhpResult<Self>"),
            "{code}"
        );
        assert!(
            code.contains("Ok(test_lib::Batch::Many { items: items_core }.into())"),
            "{code}"
        );
    }

    #[test]
    fn converts_enum_as_string_field() {
        // A field whose type is a unit-variant enum maps to `String` in PHP and round-trips via
        // `From<String>` for the core enum — handled by the shared call-arg path.
        let mut m = mapper();
        m.enum_names.insert("Color".to_string());
        let def = EnumDef {
            name: "Painted".to_string(),
            rust_path: "test_lib::Painted".to_string(),
            variants: vec![variant(
                "Fill",
                vec![field("color", TypeRef::Named("Color".to_string()))],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &m);
        // Enum params are owned `String`, not `&T`. The shared let-binding machinery emits a
        // `<field>_core` binding that round-trips the string through `From<String>` for the core enum.
        assert!(code.contains("pub fn _factory_fill(color: String) -> Self"), "{code}");
        assert!(
            code.contains("let color_core"),
            "must use the shared _core let binding: {code}"
        );
        assert!(
            code.contains("test_lib::Painted::Fill { color: color_core }.into()"),
            "{code}"
        );
    }

    #[test]
    fn casts_wide_int_field() {
        // A `u64` field arrives as PHP `i64`; the shared call-arg machinery casts it back to the core
        // type at the call site.
        let def = EnumDef {
            name: "Sized_".to_string(),
            rust_path: "test_lib::Sized_".to_string(),
            variants: vec![variant(
                "Big",
                vec![field("count", TypeRef::Primitive(PrimitiveType::U64))],
            )],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(
            code.contains("test_lib::Sized_::Big { count: count as u64 }.into()"),
            "{code}"
        );
    }

    #[test]
    fn skips_unit_tuple_and_excluded_variants() {
        let mut tuple_variant = variant("Pair", vec![field("_0", TypeRef::String)]);
        tuple_variant.is_tuple = true;
        let mut excluded = variant("Hidden", vec![field("value", TypeRef::String)]);
        excluded.binding_excluded = true;

        let def = EnumDef {
            name: "Mixed".to_string(),
            rust_path: "test_lib::Mixed".to_string(),
            variants: vec![
                variant("Empty", vec![]),
                tuple_variant,
                excluded,
                variant("Real", vec![field("value", TypeRef::String)]),
            ],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let code = run(&def, &mapper());
        assert!(!code.contains("_factory_empty"), "{code}");
        assert!(!code.contains("_factory_pair"), "{code}");
        assert!(!code.contains("_factory_hidden"), "{code}");
        assert!(code.contains("pub fn _factory_real(value: String) -> Self"), "{code}");
        assert!(code.contains("test_lib::Mixed::Real { value: value }.into()"), "{code}");
    }

    #[test]
    fn yields_to_hand_written_method() {
        let def = EnumDef {
            methods: vec![MethodDef {
                name: "circle".to_string(),
                is_static: true,
                ..Default::default()
            }],
            ..shape_enum()
        };
        let code = run(&def, &mapper());
        assert!(
            !code.contains("test_lib::Shape::Circle"),
            "consumer method wins for Circle: {code}"
        );
        assert!(
            code.contains("pub fn _factory_rect(width: f64, height: f64) -> Self"),
            "{code}"
        );
    }

    #[test]
    fn empty_for_unit_only_enum() {
        let def = EnumDef {
            name: "UnitOnly".to_string(),
            rust_path: "test_lib::UnitOnly".to_string(),
            variants: vec![variant("A", vec![]), variant("B", vec![])],
            serde_tag: Some("type".to_string()),
            ..Default::default()
        };
        let empty = AHashSet::new();
        let code = gen_flat_data_enum_variant_constructors(&def, &mapper(), &empty, &empty, &empty, "crate");
        assert!(code.is_empty(), "no constructors for unit-only enum: {code:?}");
    }
}

#[cfg(test)]
mod escape_php_reserved_constant_tests {
    use super::escape_php_reserved_constant;

    #[test]
    fn appends_underscore_to_reserved_words() {
        assert_eq!(escape_php_reserved_constant("CLASS"), "CLASS_");
        assert_eq!(escape_php_reserved_constant("INTERFACE"), "INTERFACE_");
        assert_eq!(escape_php_reserved_constant("ENUM"), "ENUM_");
    }

    #[test]
    fn leaves_normal_identifiers_alone() {
        assert_eq!(escape_php_reserved_constant("VARIABLE"), "VARIABLE");
        assert_eq!(escape_php_reserved_constant("STRUCT"), "STRUCT");
        assert_eq!(escape_php_reserved_constant("OTHER"), "OTHER");
    }
}

#[cfg(test)]
mod flat_data_enum_from_impls_tests {
    use super::gen_flat_data_enum_from_impls;
    use crate::core::ir::{EnumDef, EnumVariant};

    /// Constructs a minimal EnumDef for testing.
    fn make_enum(name: &str, serde_tag: Option<&str>, has_default_variant: bool, has_excluded: bool) -> EnumDef {
        let variants = vec![
            EnumVariant {
                name: "Variant1".to_string(),
                is_default: has_default_variant,
                ..Default::default()
            },
            EnumVariant {
                name: "Variant2".to_string(),
                is_default: false,
                ..Default::default()
            },
        ];

        let excluded_variants = if has_excluded {
            vec![EnumVariant {
                name: "ExcludedVariant".to_string(),
                is_default: false,
                ..Default::default()
            }]
        } else {
            vec![]
        };

        EnumDef {
            name: name.to_string(),
            rust_path: format!("module::{}", name),
            serde_tag: serde_tag.map(|s| s.to_string()),
            variants,
            excluded_variants,
            ..Default::default()
        }
    }

    #[test]
    fn flat_enum_with_default_variant_emits_default_fallback() {
        let enum_def = make_enum("Message", Some("role"), true, false);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate");

        // When the enum has a #[default] variant, should emit `_ => CorePath::default()`
        assert!(
            generated.contains("_ => module::Message::default()"),
            "Should emit default() fallback for enum with #[default] variant; got:\n{generated}"
        );
    }

    #[test]
    fn flat_enum_without_default_and_with_excluded_emits_unreachable() {
        let enum_def = make_enum("Message", Some("role"), false, true);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate");

        // When the enum has NO #[default] variant but HAS excluded variants,
        // should emit a wildcard arm that calls unreachable!() instead of Default::default()
        assert!(
            generated.contains("_ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\")"),
            "Should emit unreachable!() fallback for enum with excluded variants but no default; got:\n{generated}"
        );

        // Must NOT try to call Default::default() since the core type doesn't have one
        assert!(
            !generated.contains("_ => module::Message::default()"),
            "Should NOT emit default() fallback when core type has no visible Default impl; got:\n{generated}"
        );
    }

    #[test]
    fn flat_enum_without_default_and_no_excluded_emits_unreachable() {
        let enum_def = make_enum("SimpleEnum", Some("type"), false, false);
        let generated = gen_flat_data_enum_from_impls(&enum_def, "crate");

        // The binding→core conversion matches on the tag discriminator `&str`, which can never
        // be matched exhaustively — a wildcard arm is ALWAYS required regardless of whether the
        // enum has excluded variants. With no visible `Default`, that wildcard must be the
        // non-`Default` `unreachable!` arm.
        assert!(
            generated.contains("_ => unreachable!(\"unrecognised tag for flat enum, not constructible from PHP\")"),
            "Should emit unreachable!() wildcard for &str match when core has no visible Default; got:\n{generated}"
        );
        // Must NOT call a non-existent Default impl.
        assert!(
            !generated.contains("_ => module::SimpleEnum::default()"),
            "Should NOT emit default() fallback when core type has no visible Default impl; got:\n{generated}"
        );
    }
}
