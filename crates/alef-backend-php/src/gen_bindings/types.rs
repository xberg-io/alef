use crate::type_map::PhpMapper;
use ahash::AHashSet;
use alef_codegen::builder::ImplBuilder;
use alef_codegen::generators::{self, RustBindingConfig};
use alef_codegen::shared::{constructor_parts, partition_methods};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, FieldDef, TypeDef, TypeRef};

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

/// Generate ext-php-rs methods for an opaque struct (delegates to self.inner).
pub(crate) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
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
            ));
        } else {
            impl_builder.add_method(&gen_instance_method(method, mapper, true, &typ.name, opaque_types));
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
        if is_php_prop_scalar_with_enums(&field.ty, enum_names) {
            // Use php(rename) to keep snake_case naming consistent with getter properties.
            // Without this, ext-php-rs auto-converts to camelCase for #[php(prop)] fields.
            vec![format!("php(prop, name = \"{}\")", field.name)]
        } else {
            vec![]
        }
    };

    if cfg.has_serde {
        // Build a modified config that also derives Serialize + Deserialize.
        let mut extra_derives: Vec<&str> = cfg.struct_derives.to_vec();
        extra_derives.push("serde::Serialize");
        extra_derives.push("serde::Deserialize");
        let modified_cfg = RustBindingConfig {
            struct_attrs: effective_struct_attrs,
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

/// Return true if a TypeRef contains a Named type (another struct/class that
/// ext-php-rs cannot deserialize from a PHP value as an owned parameter).
pub(crate) fn type_ref_has_named(ty: &alef_core::ir::TypeRef) -> bool {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_has_named(inner),
        TypeRef::Map(k, v) => type_ref_has_named(k) || type_ref_has_named(v),
        _ => false,
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
        let has_named_params = typ.fields.iter().any(|f| type_ref_has_named(&f.ty));
        if has_named_params {
            if has_serde {
                let constructor = "pub fn from_json(json: String) -> PhpResult<Self> {\n    \
                     serde_json::from_str(&json)\n        \
                     .map_err(|e| PhpException::default(e.to_string()))\n\
                     }"
                .to_string();
                impl_builder.add_method(&constructor);
            } else {
                let constructor = format!(
                    "pub fn __construct() -> PhpResult<Self> {{\n    \
                     Err(PhpException::default(\"Not implemented: constructor for {} requires complex params\".to_string()))\n\
                     }}",
                    typ.name
                );
                impl_builder.add_method(&constructor);
            }
        } else {
            let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);
            if typ.has_default {
                // kwargs-style constructor: all fields optional with defaults
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

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        if method.is_async {
            impl_builder.add_method(&gen_async_instance_method(
                method,
                mapper,
                false,
                &typ.name,
                opaque_types,
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
