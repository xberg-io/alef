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
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_tag_field.jinja",
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

    let from_json = "#[php(name = \"from_json\")]\npub fn from_json(json: String) -> PhpResult<Self> {\n    \
        serde_json::from_str(&json)\n        \
        .map_err(|e| PhpException::default(e.to_string()))\n\
        }"
    .to_string();
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

    let tag_getter =
        format!("#[php(getter)]\npub fn get_{tag_field}_tag(&self) -> String {{\n    self.{tag_field}_tag.clone()\n}}");
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
    if !enum_def.excluded_variants.is_empty() {
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

    let qualifying = collect_variant_constructors(enum_def);
    qualifying
        .iter()
        .filter_map(|ctor| {
            let variant = enum_def.variants.iter().find(|v| v.name == ctor.variant_name)?;

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
                    format!("{}: {value}", p.name)
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
mod tests;
