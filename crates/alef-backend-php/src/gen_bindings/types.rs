use crate::type_map::PhpMapper;
use ahash::AHashSet;
use alef_adapters::AdapterBodies;
use alef_codegen::builder::ImplBuilder;
use alef_codegen::generators::{self, RustBindingConfig};
use alef_codegen::shared::{constructor_parts, partition_methods};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

use super::functions::{
    gen_async_instance_method, gen_async_static_method, gen_instance_method, gen_instance_method_non_opaque,
    gen_static_method,
};

/// Returns true if the type is "scalar-compatible" — i.e. ext-php-rs can handle it as a
/// Check if a type is scalar-compatible for PHP properties, considering enum names.
/// `#[php(prop)]` without needing a manual getter.  Scalar-compatible means the mapped Rust
/// type implements `IntoZval` + `FromZval` automatically:
///   primitives, String, bool, Duration (→ u64), Path (→ String), Option<scalar>,
///   Vec<primitive> (the `Vec<T: IntoZval>` blanket impl).
/// Anything containing a Named struct, Map, nested Vec, Json, or Bytes requires a getter.
/// Enums are mapped as String in the PHP binding, so they count as scalar.
fn is_php_prop_scalar_with_enums(ty: &TypeRef, enum_names: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char | TypeRef::Duration | TypeRef::Path => true,
        TypeRef::Optional(inner) => is_php_prop_scalar_with_enums(inner, enum_names),
        TypeRef::Vec(inner) => {
            matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::String | TypeRef::Char)
                || matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n))
        }
        TypeRef::Named(n) if enum_names.contains(n) => true,
        TypeRef::Named(_) | TypeRef::Map(_, _) | TypeRef::Json | TypeRef::Bytes | TypeRef::Unit => false,
    }
}

/// Returns `true` if the PHP-mapped type is `Copy`, meaning `.clone()` can be omitted.
/// Primitives (bool, integers, floats) are Copy.  Option<Primitive> is also Copy.
/// String, Named structs, Vec, Map are NOT Copy.
fn is_php_copy_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_) => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Primitive(_)),
        _ => false,
    }
}

/// Generate ext-php-rs methods for an opaque struct (delegates to self.inner).
pub(crate) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    adapter_bodies: &AdapterBodies,
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);
    impl_builder.add_attr("php_impl");

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        if method.is_async {
            impl_builder.add_method(&gen_async_instance_method(
                method,
                mapper,
                true,
                &typ.name,
                opaque_types,
                adapter_bodies,
            ));
        } else {
            impl_builder.add_method(&gen_instance_method(
                method,
                mapper,
                true,
                &typ.name,
                opaque_types,
                core_import,
                adapter_bodies,
            ));
        }
    }
    for method in &statics {
        if method.is_async {
            impl_builder.add_method(&gen_async_static_method(method, mapper, opaque_types));
        } else {
            impl_builder.add_method(&gen_static_method(method, mapper, opaque_types, typ, core_import));
        }
    }

    impl_builder.build()
}

/// Generate a PHP struct, adding `serde::Serialize` and `serde::Deserialize` when serde is available.
/// All structs need Deserialize (not just those with Named params) because
/// structs with from_json may reference other structs that also need Deserialize.
/// Serialize is needed for the serde bridge `From<BindingType> for CoreType` used
/// by enum-tainted types (types with enum-Named fields that PHP maps to String).
///
/// When `php_namespace` is provided, a separate `#[php(name = "Namespace\\ClassName")]` attribute
/// is generated alongside the plain `#[php_class]` so that ext-php-rs 0.15+ registers the class
/// in the correct PHP namespace (e.g. `Kreuzcrawl\CrawlConfig` instead of global `CrawlConfig`).
/// Note: `#[php_class(name = "...")]` was removed in ext-php-rs 0.15; the two-attribute form is required.
pub(crate) fn gen_php_struct(
    typ: &TypeDef,
    mapper: &PhpMapper,
    cfg: &RustBindingConfig<'_>,
    php_namespace: Option<&str>,
    enum_names: &AHashSet<String>,
) -> String {
    // Build the php_class attributes: with namespace → plain #[php_class] + #[php(name = "Ns\\ClassName")],
    // without → use the config's struct_attrs unchanged.
    // ext-php-rs 0.15+ uses a separate #[php] attr for the name; #[php_class(<args>)] is no longer supported.
    let php_name_attr: String;
    let struct_attrs_override: Vec<&str>;
    let effective_struct_attrs: &[&str] = if let Some(ns) = php_namespace {
        // In the generated Rust source file, backslashes in string literals must be escaped.
        // The namespace string contains literal '\' separators (e.g. "Html\To\Markdown\Rs"),
        // so we double them so the generated code compiles: "Html\\To\\Markdown\\Rs\\ClassName".
        let ns_escaped = ns.replace('\\', "\\\\");
        php_name_attr = format!("php(name = \"{}\\\\{}\")", ns_escaped, typ.name);
        struct_attrs_override = vec!["php_class", php_name_attr.as_str()];
        &struct_attrs_override
    } else {
        cfg.struct_attrs
    };

    // Per-field attribute callback: add `php(prop)` for scalar-compatible fields so that
    // ext-php-rs 0.15 exposes them as PHP properties automatically.  Non-scalar fields get
    // no automatic attribute; instead a `#[php(getter)]` method is generated separately in
    // `gen_struct_methods`.
    let field_attrs_fn = |field: &FieldDef| -> Vec<String> {
        let mut attrs = if is_php_prop_scalar_with_enums(&field.ty, enum_names) {
            // Use php(rename) to keep snake_case naming consistent with getter properties.
            // Without this, ext-php-rs auto-converts to camelCase for #[php(prop)] fields.
            vec![format!("php(prop, name = \"{}\")", field.name)]
        } else {
            vec![]
        };
        // Non-optional Duration fields are stored as Option<i64> when has_serde is enabled
        // (option_duration_on_defaults). When None, serde serializes them as JSON null, but
        // the core Duration field uses a custom duration_ms deserializer that rejects null.
        // Skip-serializing None ensures the field is omitted so the core uses its default.
        if cfg.has_serde && matches!(field.ty, TypeRef::Duration) && !field.optional {
            attrs.push("serde(skip_serializing_if = \"Option::is_none\")".to_string());
        }
        attrs
    };

    if cfg.has_serde {
        // Build a modified config that also derives Serialize + Deserialize, and adds
        // #[serde(default)] so from_json() works with partial JSON (missing fields use
        // their Default values instead of failing deserialization).
        let mut extra_derives: Vec<&str> = cfg.struct_derives.to_vec();
        extra_derives.push("serde::Serialize");
        extra_derives.push("serde::Deserialize");
        let mut serde_struct_attrs: Vec<&str> = effective_struct_attrs.to_vec();
        serde_struct_attrs.push("serde(default)");
        let modified_cfg = RustBindingConfig {
            struct_attrs: &serde_struct_attrs,
            field_attrs: cfg.field_attrs,
            struct_derives: &extra_derives,
            method_block_attr: cfg.method_block_attr,
            constructor_attr: cfg.constructor_attr,
            static_attr: cfg.static_attr,
            function_attr: cfg.function_attr,
            enum_attrs: cfg.enum_attrs,
            enum_derives: cfg.enum_derives,
            needs_signature: cfg.needs_signature,
            signature_prefix: cfg.signature_prefix,
            signature_suffix: cfg.signature_suffix,
            core_import: cfg.core_import,
            async_pattern: cfg.async_pattern,
            has_serde: cfg.has_serde,
            type_name_prefix: cfg.type_name_prefix,
            option_duration_on_defaults: cfg.option_duration_on_defaults,
            opaque_type_names: cfg.opaque_type_names,
            skip_impl_constructor: cfg.skip_impl_constructor,
            cast_uints_to_i32: cfg.cast_uints_to_i32,
            cast_large_ints_to_f64: cfg.cast_large_ints_to_f64,
            named_non_opaque_params_by_ref: cfg.named_non_opaque_params_by_ref,
        };
        generators::gen_struct_with_per_field_attrs(typ, mapper, &modified_cfg, field_attrs_fn)
    } else {
        let modified_cfg = RustBindingConfig {
            struct_attrs: effective_struct_attrs,
            ..*cfg
        };
        generators::gen_struct_with_per_field_attrs(typ, mapper, &modified_cfg, field_attrs_fn)
    }
}

/// Generate ext-php-rs methods for a struct.
#[allow(dead_code)]
pub(crate) fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &PhpMapper,
    has_serde: bool,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    enums: &[EnumDef],
) -> String {
    gen_struct_methods_impl(
        typ,
        mapper,
        has_serde,
        core_import,
        opaque_types,
        enum_names,
        enums,
        &[], // exclude_functions: empty by default
    )
}

#[allow(clippy::too_many_arguments)]
pub fn gen_struct_methods_with_exclude(
    typ: &TypeDef,
    mapper: &PhpMapper,
    has_serde: bool,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    enums: &[EnumDef],
    exclude_functions: &[String],
) -> String {
    gen_struct_methods_impl(
        typ,
        mapper,
        has_serde,
        core_import,
        opaque_types,
        enum_names,
        enums,
        exclude_functions,
    )
}

#[allow(clippy::too_many_arguments)]
fn gen_struct_methods_impl(
    typ: &TypeDef,
    mapper: &PhpMapper,
    has_serde: bool,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    enums: &[EnumDef],
    exclude_functions: &[String],
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);
    impl_builder.add_attr("php_impl");

    if !typ.fields.is_empty() {
        let has_named_params = typ
            .fields
            .iter()
            .any(|f| !is_php_prop_scalar_with_enums(&f.ty, enum_names));
        // When has_serde and the struct has defaults, always emit from_json so callers can
        // use partial JSON. PHP enum fields map to String in the binding; their Rust-native
        // defaults (e.g. BrowserMode::Auto) are not valid in the generated binding code, so
        // a PHP kwargs __construct would fail to compile for any struct with enum-typed fields.
        let use_from_json = has_serde && (has_named_params || typ.has_default);
        if use_from_json {
            let constructor = "#[php(name = \"from_json\")]\npub fn from_json(json: String) -> PhpResult<Self> {\n    \
                 serde_json::from_str(&json)\n        \
                 .map_err(|e| PhpException::default(e.to_string()))\n\
                 }"
            .to_string();
            impl_builder.add_method(&constructor);
        } else if has_named_params {
            let constructor = format!(
                "pub fn __construct() -> PhpResult<Self> {{\n    \
                 Err(PhpException::default(\"Not implemented: constructor for {} requires complex params\".to_string()))\n\
                 }}",
                typ.name
            );
            impl_builder.add_method(&constructor);
        } else {
            let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);
            if typ.has_default {
                // kwargs-style constructor: all fields optional with defaults (no serde, no Named fields)
                let config_method = alef_codegen::config_gen::gen_php_kwargs_constructor(typ, &map_fn);
                impl_builder.add_method(&config_method);
            } else {
                // Normal positional constructor
                let (param_list, _, assignments) = constructor_parts(&typ.fields, &map_fn);
                let constructor = format!(
                    "pub fn __construct({param_list}) -> Self {{\n    \
                     Self {{ {assignments} }}\n\
                     }}"
                );
                impl_builder.add_method(&constructor);
            }
        }
    }

    // Generate #[php(getter)] methods for non-scalar fields so PHP can access them as
    // $obj->fieldName.  Scalar fields already have #[php(prop)] on the struct field itself.
    for field in &typ.fields {
        if field.cfg.is_some() {
            continue;
        }
        let effective_ty = &field.ty;
        if !is_php_prop_scalar_with_enums(effective_ty, enum_names) {
            let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);
            let rust_return_type = if field.optional {
                mapper.optional(&mapper.map_type(&field.ty))
            } else {
                map_fn(&field.ty)
            };
            let getter_method = format!(
                "#[php(getter)]\npub fn get_{field_name}(&self) -> {ret} {{\n    self.{field_name}.clone()\n}}",
                field_name = field.name,
                ret = rust_return_type,
            );
            impl_builder.add_method(&getter_method);

            // Note: setters for Named/Vec/Map fields are not generated because
            // ext-php-rs doesn't support &T: FromZval for #[php(setter)] parameters.
            // Config types with complex fields should be constructed via fromJson().
        }
    }

    // Non-opaque structs don't have adapter bodies — adapters apply to opaque types only.
    let empty_adapter_bodies: alef_adapters::AdapterBodies = Default::default();

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        if method.is_async {
            impl_builder.add_method(&gen_async_instance_method(
                method,
                mapper,
                false,
                &typ.name,
                opaque_types,
                &empty_adapter_bodies,
            ));
        } else {
            impl_builder.add_method(&gen_instance_method_non_opaque(
                method,
                mapper,
                typ,
                core_import,
                opaque_types,
                enums,
            ));
        }
    }
    for method in &statics {
        // Skip methods that are in the exclusion list
        if exclude_functions.contains(&method.name) {
            continue;
        }
        if method.is_async {
            impl_builder.add_method(&gen_async_static_method(method, mapper, opaque_types));
        } else {
            impl_builder.add_method(&gen_static_method(method, mapper, opaque_types, typ, core_import));
        }
    }

    impl_builder.build()
}

/// Generate PHP enum constants (enums as string constants).
pub(crate) fn gen_enum_constants(enum_def: &EnumDef) -> String {
    let mut lines = vec![format!("// {} enum values", enum_def.name)];

    for variant in &enum_def.variants {
        let const_name = format!("{}_{}", enum_def.name.to_uppercase(), variant.name.to_uppercase());
        lines.push(format!("pub const {}: &str = \"{}\";", const_name, variant.name));
    }

    lines.join("\n")
}

/// Return true if an enum is a "tagged data enum" — has a serde tag AND at least one variant
/// with named fields. These are lowered to flat PHP classes rather than string constants.
pub(crate) fn is_tagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|v| !v.fields.is_empty())
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
    use heck::ToSnakeCase as _;
    if alef_codegen::conversions::is_tuple_variant(&variant.fields) {
        let base = variant.name.to_snake_case();
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
    use std::fmt::Write as _;
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");

    let php_attrs: String = if let Some(ns) = php_namespace {
        let ns_escaped = ns.replace('\\', "\\\\");
        let php_name_attr = format!("php(name = \"{}\\\\{}\")", ns_escaped, enum_def.name);
        format!("#[php_class]\n#[{php_name_attr}]")
    } else {
        "#[php_class]".to_string()
    };

    let mut out = String::new();
    writeln!(out, "{php_attrs}").ok();
    writeln!(out, "#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]").ok();
    writeln!(out, "pub struct {} {{", enum_def.name).ok();
    // Discriminator field
    writeln!(out, "    #[php(prop, name = \"{tag_field}\")]").ok();
    writeln!(out, "    #[serde(rename = \"{tag_field}\")]").ok();
    writeln!(out, "    pub {tag_field}_tag: String,").ok();

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
                writeln!(out, "    #[serde(skip_serializing_if = \"Option::is_none\")]").ok();
                writeln!(out, "    pub {flat_name}: {field_ty},").ok();
            }
        }
    }
    out.push('}');
    out
}

/// Generate `#[php_impl]` accessor methods and a `from_json` constructor for the flat data enum.
pub(crate) fn gen_flat_data_enum_methods(enum_def: &EnumDef, mapper: &PhpMapper) -> String {
    use std::fmt::Write as _;
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
    writeln!(out, "{}", impl_builder.build()).ok();
    out
}

/// Returns the serde-renamed tag string for a variant.
fn variant_tag_value(variant: &EnumVariant, enum_def: &EnumDef) -> String {
    if let Some(rename) = &variant.serde_rename {
        return rename.clone();
    }
    if let Some(rename_all) = &enum_def.serde_rename_all {
        return apply_rename_all(&variant.name, rename_all);
    }
    variant.name.clone()
}

fn apply_rename_all(name: &str, strategy: &str) -> String {
    use heck::{ToKebabCase, ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
    match strategy {
        "lowercase" => name.to_lowercase(),
        "UPPERCASE" => name.to_uppercase(),
        "camelCase" => name.to_lower_camel_case(),
        "PascalCase" => name.to_upper_camel_case(),
        "snake_case" => name.to_snake_case(),
        "SCREAMING_SNAKE_CASE" => name.to_shouty_snake_case(),
        "kebab-case" => name.to_kebab_case(),
        _ => name.to_string(),
    }
}

/// Generate `From<core::DataEnum> for PhpDataEnum` and `From<PhpDataEnum> for core::DataEnum`
/// for a tagged data enum lowered to a flat PHP class.
pub(crate) fn gen_flat_data_enum_from_impls(enum_def: &EnumDef, core_import: &str) -> String {
    use alef_core::ir::{PrimitiveType, TypeRef};
    use std::fmt::Write as _;
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
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
    writeln!(out, "impl From<{core_path}> for {binding_name} {{").ok();
    writeln!(out, "    fn from(val: {core_path}) -> Self {{").ok();
    writeln!(out, "        match val {{").ok();
    for variant in &enum_def.variants {
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            // No variant fields: only the tag is set; all Option fields default to None.
            // `..Default::default()` is only needed when the struct has other fields.
            if all_flat_fields.is_empty() {
                writeln!(
                    out,
                    "            {core_path}::{name} => Self {{ {tag_field}_tag: \"{tag_val}\".to_string() }},",
                    name = variant.name,
                )
                .ok();
            } else {
                writeln!(
                    out,
                    "            {core_path}::{name} => Self {{ {tag_field}_tag: \"{tag_val}\".to_string(), ..Default::default() }},",
                    name = variant.name,
                ).ok();
            }
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
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
            if is_tuple {
                write!(out, "            {core_path}::{}({pattern}) => Self {{", variant.name).ok();
            } else {
                write!(
                    out,
                    "            {core_path}::{}{{ {pattern} }} => Self {{",
                    variant.name
                )
                .ok();
            }
            write!(out, " {tag_field}_tag: \"{tag_val}\".to_string(),").ok();
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
                write!(out, " {flat_name}: {expr},").ok();
            }
            // Omit `..Default::default()` when this variant's fields cover every flat struct
            // field — the struct update would have no effect and triggers `clippy::needless_update`.
            let variant_flat_names: std::collections::BTreeSet<String> =
                (0..variant.fields.len()).map(|i| flat_field_name(variant, i)).collect();
            if variant_flat_names == all_flat_fields {
                writeln!(out, " }},").ok();
            } else {
                writeln!(out, " ..Default::default() }},").ok();
            }
        }
    }
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

    writeln!(out).ok();

    // --- binding → core: match on the tag field to reconstruct the correct variant ---
    // We use tag-value matching rather than serde round-trip to avoid serde field rename
    // mismatches between the flat struct (uses Rust snake_case names) and the core type
    // (may have #[serde(rename = "camelCase")] on individual variant fields).
    writeln!(out, "impl From<{binding_name}> for {core_path} {{").ok();
    writeln!(out, "    fn from(val: {binding_name}) -> Self {{").ok();
    writeln!(out, "        match val.{tag_field}_tag.as_str() {{").ok();
    for variant in &enum_def.variants {
        let tag_val = variant_tag_value(variant, enum_def);
        if variant.fields.is_empty() {
            writeln!(out, "            \"{tag_val}\" => {core_path}::{},", variant.name).ok();
        } else {
            let is_tuple = alef_codegen::conversions::is_tuple_variant(&variant.fields);
            if is_tuple {
                write!(out, "            \"{tag_val}\" => {core_path}::{}(", variant.name).ok();
            } else {
                write!(out, "            \"{tag_val}\" => {core_path}::{}{{", variant.name).ok();
            }
            if is_tuple {
                // Tuple variant: positional syntax uses `, ` separators.
                let exprs: Vec<String> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(idx, f)| flat_enum_binding_to_core_field_expr(f, &flat_field_name(variant, idx)))
                    .collect();
                write!(out, " {}", exprs.join(", ")).ok();
                writeln!(out, " ),").ok();
            } else {
                // Struct variant: `field_name: <expr>,` for each field.
                for (idx, f) in variant.fields.iter().enumerate() {
                    let flat_name = flat_field_name(variant, idx);
                    let expr = flat_enum_binding_to_core_field_expr(f, &flat_name);
                    write!(out, " {flat_name}: {expr},").ok();
                }
                writeln!(out, " }},").ok();
            }
        }
    }
    // Fallback to first variant (with all fields defaulted) for unrecognised tags.
    if let Some(first) = enum_def.variants.first() {
        if first.fields.is_empty() {
            writeln!(out, "            _ => {core_path}::{},", first.name).ok();
        } else if alef_codegen::conversions::is_tuple_variant(&first.fields) {
            write!(out, "            _ => {core_path}::{}(", first.name).ok();
            let parts: Vec<String> = first
                .fields
                .iter()
                .map(|f| {
                    if f.is_boxed {
                        "Box::new(Default::default())".to_string()
                    } else {
                        "Default::default()".to_string()
                    }
                })
                .collect();
            write!(out, " {}", parts.join(", ")).ok();
            writeln!(out, " ),").ok();
        } else {
            write!(out, "            _ => {core_path}::{}{{", first.name).ok();
            for f in &first.fields {
                let default_expr = if f.is_boxed {
                    format!("{name}: Box::new(Default::default()),", name = f.name)
                } else {
                    format!("{name}: Default::default(),", name = f.name)
                };
                write!(out, " {default_expr}").ok();
            }
            writeln!(out, " }},").ok();
        }
    }
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();

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
fn flat_enum_core_to_binding_field_expr(f: &alef_core::ir::FieldDef, bound_var: &str) -> String {
    use alef_core::ir::{PrimitiveType, TypeRef};

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
fn flat_enum_binding_to_core_field_expr(f: &alef_core::ir::FieldDef, flat_name: &str) -> String {
    use alef_core::ir::{PrimitiveType, TypeRef};

    if f.sanitized {
        return if f.is_boxed {
            "Box::new(Default::default())".to_string()
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
