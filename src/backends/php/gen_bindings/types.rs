use crate::adapters::AdapterBodies;
use crate::backends::php::type_map::PhpMapper;
use crate::codegen::builder::ImplBuilder;
use crate::codegen::generators::{self, RustBindingConfig};
use crate::codegen::naming::{pascal_to_snake, wire_variant_value};
use crate::codegen::shared::{binding_fields, partition_methods};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::{ToLowerCamelCase, ToPascalCase};

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
///
/// This function is public so that `alef-e2e` can determine which fields require
/// `->getCamelCase()` getter-method syntax vs. `->camelCase` property syntax.
pub fn is_php_prop_scalar(ty: &TypeRef, enum_names: &AHashSet<String>) -> bool {
    is_php_prop_scalar_with_enums(ty, enum_names)
}

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

fn serde_default_fn_name(type_name: &str, field_name: &str) -> String {
    format!("{}_{}", pascal_to_snake(type_name), pascal_to_snake(field_name))
}

fn supports_serde_default_fn(field: &FieldDef) -> bool {
    use crate::core::ir::DefaultValue;

    matches!(
        (&field.typed_default, &field.ty),
        (
            Some(DefaultValue::BoolLiteral(_)),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool)
        ) | (
            // String literals or enum-variant defaults on a String field can be synthesized as
            // String::from(literal). Named fields are skipped: the binding wraps the core enum
            // as a PHP-friendly struct, so a String-returning default fn would not type-check
            // against the wrapped Named field. Without the attr, serde falls back to the type's
            // own Default impl, which the core supplies.
            Some(DefaultValue::StringLiteral(_) | DefaultValue::EnumVariant(_)),
            TypeRef::String
        ) | (
            Some(DefaultValue::IntLiteral(_)),
            TypeRef::Primitive(
                crate::core::ir::PrimitiveType::U8
                    | crate::core::ir::PrimitiveType::U16
                    | crate::core::ir::PrimitiveType::U32
                    | crate::core::ir::PrimitiveType::U64
                    | crate::core::ir::PrimitiveType::I8
                    | crate::core::ir::PrimitiveType::I16
                    | crate::core::ir::PrimitiveType::I32
                    | crate::core::ir::PrimitiveType::I64
                    | crate::core::ir::PrimitiveType::Usize
                    | crate::core::ir::PrimitiveType::Isize
            )
        ) | (
            Some(DefaultValue::FloatLiteral(_)),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64)
        )
    )
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

/// Generate ext-php-rs methods for an opaque struct, excluding streaming methods.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gen_opaque_struct_methods_with_exclude(
    api: &crate::core::ir::ApiSurface,
    typ: &TypeDef,
    mapper: &PhpMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    adapter_bodies: &AdapterBodies,
    mutex_types: &AHashSet<String>,
    streaming_method_keys: &AHashSet<String>,
    trait_bridges: &[crate::core::config::TraitBridgeConfig],
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);
    impl_builder.add_attr("php_impl");

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        let method_key = format!("{}.{}", typ.name, method.name);
        if streaming_method_keys.contains(&method_key) {
            continue;
        }

        if method.is_async {
            impl_builder.add_method(&gen_async_instance_method(
                method,
                mapper,
                true,
                &typ.name,
                opaque_types,
                &mapper.enum_names,
                core_import,
                adapter_bodies,
                mutex_types,
            ));
        } else {
            impl_builder.add_method(&gen_instance_method(
                method,
                mapper,
                true,
                &typ.name,
                opaque_types,
                &mapper.enum_names,
                core_import,
                adapter_bodies,
                mutex_types,
            ));
        }
    }

    // Emit streaming methods (which come from adapters, not the IR methods list).
    // These return Vec<String> (chunks) to be wrapped by the PHP-side Generator.
    for streaming_key in streaming_method_keys.iter() {
        if streaming_key.starts_with(&format!("{}.", typ.name)) {
            if let Some(body) = adapter_bodies.get(streaming_key) {
                let method_name = streaming_key.strip_prefix(&format!("{}.", typ.name)).unwrap_or("");
                if !method_name.is_empty() {
                    // Find the original method to get its parameter information
                    let orig_method = instance.iter().find(|m| m.name == method_name);

                    let params_str = if let Some(method) = orig_method {
                        super::helpers::gen_php_function_params(&method.params, mapper, opaque_types, &AHashSet::new())
                    } else {
                        String::new()
                    };

                    // The adapter body already includes the parameter conversions via
                    // core_let_bindings_cloned, so we just use it as-is.
                    let method_code = format!(
                        "    #[php(name = \"{}\")]\n    \
                         pub fn {}(&self{}{}) -> std::result::Result<Vec<String>, ext_php_rs::exception::PhpException> {{\n    \
                         {}\n    \
                         }}",
                        method_name.to_lower_camel_case(),
                        method_name,
                        if params_str.is_empty() { "" } else { ", " },
                        params_str,
                        body
                    );
                    impl_builder.add_method(&method_code);
                }
            }
        }
    }
    for method in &statics {
        let method_key = format!("{}.{}", typ.name, method.name);
        if streaming_method_keys.contains(&method_key) {
            continue;
        }

        if method.is_async {
            impl_builder.add_method(&gen_async_static_method(method, mapper, opaque_types));
        } else {
            impl_builder.add_method(&gen_static_method(
                method,
                mapper,
                opaque_types,
                typ,
                core_import,
                mutex_types,
            ));
        }
    }

    // Emit from_php_object for trait bridge type aliases (e.g., VisitorHandle)
    for bridge in trait_bridges {
        if let Some(ref type_alias) = bridge.type_alias {
            if type_alias == &typ.name {
                // Generate the from_php_object static method that wraps a PHP object
                // The bridge struct is named Php<TraitName>Bridge (e.g., PhpSyntaxWalkerBridge)
                let bridge_struct_name = format!("Php{}Bridge", bridge.trait_name.to_pascal_case().replace('-', ""));
                // Use the full path to the trait from the core crate (e.g., sample_crate::visitor::SyntaxWalker)
                let _trait_path = format!(
                    "{}::visitor::{}",
                    core_import,
                    bridge.trait_name.split("::").last().unwrap_or(&bridge.trait_name)
                );
                // The inner field wraps VisitorHandle (which is Arc<Mutex<dyn SyntaxWalker + Send>>)
                // VisitorHandle is a type alias: Arc<Mutex<dyn SyntaxWalker + Send>>
                // We need to create Arc<VisitorHandle>, so wrap Arc<Mutex<>> in Arc
                let handle_path =
                    crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge, core_import);
                let method_code = format!(
                    "    #[php(name = \"from_php_object\")]\n    \
                     pub fn from_php_object(obj: &mut ext_php_rs::types::ZendObject) -> ext_php_rs::prelude::PhpResult<Self> {{\n    \
                     use ext_php_rs::prelude::*;\n    \
                     let bridge = {}::new(obj);\n    \
                     let visitor_handle: {handle_path} = std::sync::Arc::new(std::sync::Mutex::new(bridge));\n    \
                     Ok(Self {{ inner: std::sync::Arc::new(visitor_handle) }})\n    \
                     }}\n",
                    bridge_struct_name
                );
                impl_builder.add_method(&method_code);
            }
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
/// in the correct PHP namespace (e.g. `SampleCrawler\CrawlConfig` instead of global `CrawlConfig`).
/// Note: `#[php_class(name = "...")]` was removed in ext-php-rs 0.15; the two-attribute form is required.
pub(crate) fn gen_php_struct(
    typ: &TypeDef,
    mapper: &PhpMapper,
    cfg: &RustBindingConfig<'_>,
    php_namespace: Option<&str>,
    enum_names: &AHashSet<String>,
    lang_rename_all: &str,
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

    // Per-field attribute callback: add `php(prop)` ONLY for fields whose Rust type
    // implements ext-php-rs's `Prop<'_>` trait. The blanket `Prop` impls cover
    // primitives, String, Option<scalar>, Vec<primitive|enum>, etc., but NOT
    // Option<CustomStruct>, Vec<CustomStruct>, Map, Json, Bytes, or external types.
    // Emitting `#[php(prop)]` on unsupported types fails to compile with E0277.
    // Non-prop fields are accessed via `#[php(getter)]` methods generated separately
    // in `gen_struct_methods`.
    let field_attrs_fn = |field: &FieldDef| -> Vec<String> {
        let mut attrs = if is_php_prop_scalar_with_enums(&field.ty, enum_names) {
            // Convert field names to lowerCamelCase for PHP (e.g., mime_type -> mimeType)
            let php_name = crate::codegen::naming::to_php_name(&field.name);
            vec![format!("php(prop, name = \"{}\")", php_name)]
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
        if cfg.has_serde && typ.has_default && !field.optional && supports_serde_default_fn(field) {
            let fn_name = serde_default_fn_name(&typ.name, &field.name);
            attrs.push(format!("serde(default = \"crate::serde_defaults::{fn_name}\")"));
        }
        // Enum-backed String fields (PHP maps unit enums to plain `String`) default to "" via
        // `String::default()`, but the core enum doesn't accept `""` as a valid variant. Skip
        // serializing the empty string so the core deserializer falls back to the enum's own
        // `Default` (which always corresponds to a real variant).
        if cfg.has_serde {
            let enum_backed_string = match &field.ty {
                TypeRef::Named(n) if enum_names.contains(n) => true,
                TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n)),
                _ => false,
            };
            if enum_backed_string {
                if field.optional {
                    attrs.push("serde(skip_serializing_if = \"Option::is_none\")".to_string());
                } else {
                    attrs.push("serde(skip_serializing_if = \"String::is_empty\")".to_string());
                }
            }
        }
        attrs
    };

    if cfg.has_serde {
        // Build a modified config that also derives Serialize + Deserialize, and adds
        // #[serde(default)] so from_json() works with partial JSON (missing fields use
        // their Default values instead of failing deserialization).
        //
        // When `cfg.emit_delegating_default_impl` is true AND `typ.has_default` is true,
        // the shared struct generator suppresses the auto `#[derive(Default)]` and appends
        // a delegating `impl Default { fn default() -> Self { <core::Type as Default>::default().into() } }`.
        // This preserves the core type's custom defaults (e.g. `max_redirects: 10`) instead
        // of falling back to primitive zeros that would later be propagated back to core via
        // `From<BindingType>`, silently overwriting the semantic defaults.
        let mut extra_derives: Vec<&str> = cfg.struct_derives.to_vec();
        extra_derives.push("serde::Serialize");
        extra_derives.push("serde::Deserialize");
        let has_custom_core_default = typ.has_default;
        if !has_custom_core_default {
            extra_derives.push("Default");
        }
        let mut serde_struct_attrs: Vec<&str> = effective_struct_attrs.to_vec();
        // Wire-case is sourced from the per-language registry
        // (`ResolvedCrateConfig::serde_rename_all_for_language`) so all bindings agree
        // on a single source of truth.  PHP defaults to camelCase to match PSR-12.
        let serde_default_attr = format!("serde(default, rename_all = \"{lang_rename_all}\")");
        serde_struct_attrs.push(serde_default_attr.as_str());
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
            lossy_skip_types: cfg.lossy_skip_types,
            serializable_opaque_type_names: cfg.serializable_opaque_type_names,
            never_skip_cfg_field_names: cfg.never_skip_cfg_field_names,
            emit_delegating_default_impl: has_custom_core_default,
            skip_methods_when_not_delegatable: cfg.skip_methods_when_not_delegatable,
        };
        // The shared struct generator handles the delegating `impl Default` automatically
        // when `emit_delegating_default_impl` is enabled and `typ.has_default` is true.
        generators::gen_struct_with_per_field_attrs(typ, mapper, &modified_cfg, field_attrs_fn)
    } else {
        // Without serde, no `#[serde(default)]` is applied — the binding's `Default` impl
        // is never invoked from a partial-JSON path so the delegating impl is unnecessary.
        let modified_cfg = RustBindingConfig {
            struct_attrs: effective_struct_attrs,
            emit_delegating_default_impl: false,
            ..*cfg
        };
        generators::gen_struct_with_per_field_attrs(typ, mapper, &modified_cfg, field_attrs_fn)
    }
}

#[allow(clippy::too_many_arguments)]
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
    mutex_types: &AHashSet<String>,
) -> String {
    gen_struct_methods_impl(
        typ,
        mapper,
        has_serde,
        core_import,
        opaque_types,
        enum_names,
        enums,
        &[],              // exclude_functions: empty by default
        &AHashSet::new(), // bridge_type_aliases: empty by default
        &[],              // never_skip_cfg_field_names: empty by default
        mutex_types,
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
    bridge_type_aliases: &AHashSet<String>,
    never_skip_cfg_field_names: &[String],
    mutex_types: &AHashSet<String>,
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
        bridge_type_aliases,
        never_skip_cfg_field_names,
        mutex_types,
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
    bridge_type_aliases: &AHashSet<String>,
    _never_skip_cfg_field_names: &[String],
    mutex_types: &AHashSet<String>,
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);
    impl_builder.add_attr("php_impl");

    // When the type already has an explicit static `new()` method in its IR, do not emit a
    // field-based `#[php(constructor)]` — the static method will be emitted as a named
    // constructor and would conflict (duplicate `new` definitions) with the auto-generated one.
    let has_explicit_static_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");

    if !has_explicit_static_new && !typ.fields.is_empty() {
        let has_named_params = typ
            .fields
            .iter()
            .any(|f| !is_php_prop_scalar_with_enums(&f.ty, enum_names));
        // When has_serde and the struct has defaults, always emit from_json so callers can
        // use partial JSON. PHP enum fields map to String in the binding; their Rust-native
        // defaults (e.g. BrowserMode::Auto) are not valid in the generated binding code, so
        // a PHP kwargs __construct would fail to compile for any struct with enum-typed fields.
        let has_field_defaults = typ
            .fields
            .iter()
            .any(|field| field.default.is_some() || field.typed_default.is_some());
        let use_from_json = has_serde && (has_named_params || typ.has_default || has_field_defaults);
        if use_from_json {
            let constructor = "#[php(name = \"from_json\")]\npub fn from_json(json: String) -> PhpResult<Self> {\n    \
                 serde_json::from_str(&json)\n        \
                 .map_err(|e| PhpException::default(e.to_string()))\n\
                 }"
            .to_string();
            impl_builder.add_method(&constructor);

            // Also generate a #[php(constructor)] for named construction.
            // Include parameters for all scalar/Vec fields (required and optional).
            // Omit complex optional fields (they default to None).
            fn field_can_be_param(
                ty: &crate::core::ir::TypeRef,
                enum_names: &AHashSet<String>,
                opaque_types: &AHashSet<String>,
            ) -> bool {
                match ty {
                    crate::core::ir::TypeRef::Vec(inner) => {
                        // Vec<NonOpaqueCustomType> cannot be a constructor param (requires error handling for FromZval)
                        match inner.as_ref() {
                            crate::core::ir::TypeRef::Named(name) => {
                                // Only allow if it's opaque or an enum (which map to String)
                                opaque_types.contains(name.as_str()) || enum_names.contains(name.as_str())
                            }
                            // Vec<serde_json::Value> does not implement FromZval; skip.
                            crate::core::ir::TypeRef::Json => false,
                            _ => true, // Vec<primitive>, Vec<String>, etc.
                        }
                    }
                    crate::core::ir::TypeRef::Bytes => true,
                    crate::core::ir::TypeRef::Optional(inner) => {
                        // Optional scalar/Vec can be a param; optional complex cannot
                        field_can_be_param(inner, enum_names, opaque_types)
                    }
                    _ => is_php_prop_scalar_with_enums(ty, enum_names),
                }
            }

            // Only generate constructor if there's at least one representable required field (otherwise from_json is simpler)
            let has_representable_required = typ
                .fields
                .iter()
                .filter(|f| !f.binding_excluded)
                .any(|f| !f.optional && field_can_be_param(&f.ty, enum_names, opaque_types));

            if has_representable_required {
                // Build parameter lines using gen_php_function_params logic for proper type conversions
                // For Vec<NonOpaqueCustomType>, this converts to &ZendHashTable
                let param_defs: Vec<crate::core::ir::ParamDef> = typ
                    .fields
                    .iter()
                    .filter(|f| !f.binding_excluded)
                    // cfg-gated fields are absent from the binding struct — skip them so they
                    // don't appear as constructor parameters or in the struct literal.
                    .filter(|f| f.cfg.is_none())
                    .filter(|f| field_can_be_param(&f.ty, enum_names, opaque_types))
                    .map(|f| {
                        let php_param_name = crate::codegen::naming::to_php_name(&f.name);
                        // Non-optional Duration fields are stored as `Option<i64>` in the
                        // binding when `has_serde` is enabled on a `has_default` type
                        // (option_duration_on_defaults). The constructor signature must
                        // match the field type or the struct init will fail to type-check.
                        let optional =
                            f.optional || (has_serde && typ.has_default && matches!(f.ty, TypeRef::Duration));
                        crate::core::ir::ParamDef {
                            name: php_param_name,
                            ty: f.ty.clone(),
                            optional,
                            default: None,
                            is_ref: false,
                            is_mut: false,
                            newtype_wrapper: None,
                            sanitized: false,
                            original_type: None,
                            typed_default: None,
                            map_is_ahash: false,
                            map_key_is_cow: false,
                            vec_inner_is_ref: false,
                            map_is_btree: false,
                            core_wrapper: crate::core::ir::CoreWrapper::None,
                        }
                    })
                    .collect();

                let param_lines =
                    super::helpers::gen_php_function_params(&param_defs, mapper, opaque_types, &AHashSet::new());

                // Generate let bindings for Vec<NonOpaqueCustomType> fields
                let mut let_bindings = String::new();
                for f in typ
                    .fields
                    .iter()
                    .filter(|f| !f.binding_excluded)
                    .filter(|f| f.cfg.is_none())
                    .filter(|f| field_can_be_param(&f.ty, enum_names, opaque_types))
                {
                    if let TypeRef::Vec(inner) = &f.ty {
                        if let TypeRef::Named(name) = inner.as_ref() {
                            if !opaque_types.contains(name.as_str()) && !enum_names.contains(name.as_str()) {
                                // Vec<NonOpaqueCustomType> parameter needs conversion from ZendHashTable.
                                // Use the struct template (FromZval) for both optional and non-optional.
                                let php_param_name = crate::codegen::naming::to_php_name(&f.name);
                                let_bindings.push_str(&crate::backends::php::template_env::render(
                                    "php_vec_named_struct_let_binding.jinja",
                                    minijinja::context! {
                                        php_name => php_param_name.as_str(),
                                        core_import => core_import,
                                        struct_name => name.as_str(),
                                        is_optional => f.optional,
                                    },
                                ));
                            }
                        }
                    }
                }

                let param_init = typ
                    .fields
                    .iter()
                    // Skip `binding_excluded` fields entirely — they are absent from
                    // the binding struct, so any reference in the Self literal would
                    // produce `struct X has no field named Y`. cfg-gated fields, by
                    // contrast, stay in the binding struct (their presence is feature-
                    // controlled) so the Self literal must still initialize them.
                    .filter(|f| !f.binding_excluded)
                    .map(|f| {
                        let php_param_name = crate::codegen::naming::to_php_name(&f.name);
                        if f.cfg.is_some() {
                            // cfg-gated fields are core-only: no constructor parameter.
                            return format!("{}: Default::default()", f.name);
                        }
                        if field_can_be_param(&f.ty, enum_names, opaque_types) {
                            // Check if this needs let-binding conversion
                            if let TypeRef::Vec(inner) = &f.ty {
                                if let TypeRef::Named(name) = inner.as_ref() {
                                    if !opaque_types.contains(name.as_str()) && !enum_names.contains(name.as_str()) {
                                        // Use the _core binding
                                        return format!("{}: {}_core", f.name, php_param_name);
                                    }
                                }
                            }
                            // Bytes: param is PhpBytes (PHP-side); field is Vec<u8>. Unwrap.
                            let is_bytes = matches!(&f.ty, TypeRef::Bytes)
                                || matches!(&f.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes));
                            if is_bytes {
                                if f.optional {
                                    return format!("{}: {}.map(|b| b.0)", f.name, php_param_name);
                                }
                                return format!("{}: {}.0", f.name, php_param_name);
                            }
                            // Params that are in the constructor
                            format!("{}: {}", f.name, php_param_name)
                        } else {
                            // Complex fields default to None/Default
                            format!("{}: Default::default()", f.name)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let named_constructor = format!(
                    "#[php(constructor)]\npub fn new(\n{param_lines}\n) -> Self {{\n    \
                     {let_bindings}Self {{ {param_init} }}\n\
                     }}"
                );
                impl_builder.add_method(&named_constructor);
            }
        } else if has_named_params {
            let constructor = format!(
                "pub fn __construct() -> PhpResult<Self> {{\n    \
                 Err(PhpException::default(\"Not implemented: constructor for {} requires complex params\".to_string()))\n\
                 }}",
                typ.name
            );
            impl_builder.add_method(&constructor);
        } else {
            let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
            if typ.has_default {
                // kwargs-style constructor: all fields optional with defaults (no serde, no Named fields)
                let config_method = crate::codegen::config_gen::gen_php_kwargs_constructor(typ, &map_fn);
                impl_builder.add_method(&config_method);
            } else {
                // Named constructor for non-Default types. Generate a factory method
                // decorated with #[php(constructor)] that accepts named parameters.
                // Use gen_php_function_params for proper Vec<NonOpaqueCustomType> handling
                let param_defs: Vec<crate::core::ir::ParamDef> = typ
                    .fields
                    .iter()
                    .filter(|f| !f.binding_excluded)
                    // cfg-gated fields are absent from the binding struct.
                    .filter(|f| f.cfg.is_none())
                    .map(|f| crate::core::ir::ParamDef {
                        name: f.name.clone(),
                        ty: f.ty.clone(),
                        optional: f.optional,
                        default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                        sanitized: false,
                        original_type: None,
                        typed_default: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
                        vec_inner_is_ref: false,
                        map_is_btree: false,
                        core_wrapper: crate::core::ir::CoreWrapper::None,
                    })
                    .collect();

                let param_lines =
                    super::helpers::gen_php_function_params(&param_defs, mapper, opaque_types, &AHashSet::new());

                // Generate let bindings for Vec<NonOpaqueCustomType> fields
                let mut let_bindings = String::new();
                for f in binding_fields(&typ.fields).filter(|f| f.cfg.is_none()) {
                    if let TypeRef::Vec(inner) = &f.ty {
                        if let TypeRef::Named(name) = inner.as_ref() {
                            if !opaque_types.contains(name.as_str()) && !enum_names.contains(name.as_str()) {
                                // Vec<NonOpaqueCustomType> parameter needs conversion from ZendHashTable.
                                // Use the struct template (FromZval) for both optional and non-optional.
                                let php_param_name = crate::codegen::naming::to_php_name(&f.name);
                                let_bindings.push_str(&crate::backends::php::template_env::render(
                                    "php_vec_named_struct_let_binding.jinja",
                                    minijinja::context! {
                                        php_name => php_param_name.as_str(),
                                        core_import => core_import,
                                        struct_name => name.as_str(),
                                        is_optional => f.optional,
                                    },
                                ));
                            }
                        }
                    }
                }

                let param_init = typ
                    .fields
                    .iter()
                    // Skip `binding_excluded` fields entirely — they are absent from
                    // the binding struct, so any reference in the Self literal would
                    // produce `struct X has no field named Y`. cfg-gated fields, by
                    // contrast, stay in the binding struct (their presence is feature-
                    // controlled) so the Self literal must still initialize them.
                    .filter(|f| !f.binding_excluded)
                    .map(|f| {
                        let php_param_name = crate::codegen::naming::to_php_name(&f.name);
                        if f.cfg.is_some() {
                            return format!("{}: Default::default()", f.name);
                        }
                        // Check if this needs let-binding conversion
                        if let TypeRef::Vec(inner) = &f.ty {
                            if let TypeRef::Named(name) = inner.as_ref() {
                                if !opaque_types.contains(name.as_str()) && !enum_names.contains(name.as_str()) {
                                    // Use the _core binding
                                    return format!("{}: {}_core", f.name, php_param_name);
                                }
                            }
                        }
                        // Default: use php parameter name (camelCase) for the value
                        format!("{}: {}", f.name, php_param_name)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let constructor = format!(
                    "#[php(constructor)]\npub fn new(\n{param_lines}\n) -> Self {{\n    \
                     {let_bindings}Self {{ {param_init} }}\n\
                     }}"
                );
                impl_builder.add_method(&constructor);
            }
        }
    }

    // Note: Clone is derived automatically and works correctly for both Arc<T> and Arc<Mutex<T>>
    // since Arc::clone() and Mutex::clone() both increment refcounts without wrapping.

    // Generate getter methods for all fields (both scalar and non-scalar).
    //
    // Scalar fields have `#[php(prop)]` on the struct field itself which exposes them as
    // PHP properties, but readonly class DTOs also need getter methods for the e2e tests
    // which call methods like `$result->getContent()` rather than accessing properties.
    //
    // Historical note: this code used to emit `#[php(getter)] pub fn get_camelCase(...)` so
    // PHP could access `$obj->camelCase` as a magic property. But ext-php-rs-derive 0.11.7
    // (the latest release compatible with ext-php-rs 0.15.4) leaves `get_method_props` as
    // `todo!()` in its derive macro — so `#[php(getter)]` registers ONLY as a property
    // accessor (the runtime callable method is never registered) AND the property accessor
    // path itself is broken for non-scalar return types, raising a fatal "Call to undefined
    // method" error at every site.
    //
    // The approach emits a regular `pub fn` with a `get_snake_case` Rust ident, which
    // ext-php-rs surfaces as a callable PHP method named `getCamelCase()`. Matches the
    // `alef-e2e/src/codegen/php.rs` field-access dispatch which emits the `->getCamelCase()`
    // shape for all fields.
    //
    // Cfg-gated fields stay in the binding struct (gen_struct keeps them with #[serde(skip)])
    // so PHP also needs a getter to access them — do not skip them here.
    for field in binding_fields(&typ.fields) {
        let _effective_ty = &field.ty;
        // Use a snake_case Rust ident — ext-php-rs's `#[php_impl]` macro auto-converts
        // `get_camel_case` (snake_case Rust) to `getCamelCase()` (camelCase PHP method).
        // The previous v0.16.55 attempt at a camelCase Rust ident was not surfaced
        // correctly by ext-php-rs (the resulting PHP method dispatch failed at runtime
        // with "Call to undefined method getCamelCase()").
        let getter_ident = format!("get_{}", field.name);

        // Untagged data enums and `TypeRef::Json` both map to `serde_json::Value` in
        // the binding struct, but ext-php-rs has no IntoZval impl for `serde_json::Value`.
        // Emit a JSON-string getter (Option<String>) so PHP can introspect the serialized
        // form, while the actual round-trip through `from_json` uses the Value field directly.
        // Map<_, Json> and Optional<Map<_, Json>> are caught here too — ext-php-rs 0.15.12+
        // tightened HashMap IntoZval bounds to require V: IntoZval, which Value does not impl.
        fn ty_is_or_wraps_json(t: &TypeRef) -> bool {
            match t {
                TypeRef::Json => true,
                TypeRef::Optional(inner) | TypeRef::Vec(inner) => ty_is_or_wraps_json(inner),
                TypeRef::Map(_, v) => matches!(v.as_ref(), TypeRef::Json),
                _ => false,
            }
        }
        let is_json_field = ty_is_or_wraps_json(&field.ty);
        if ty_references_untagged_data_enum(&field.ty, &mapper.untagged_data_enum_names) || is_json_field {
            let body = if field.optional {
                format!(
                    "self.{name}.as_ref().and_then(|v| serde_json::to_string(v).ok())",
                    name = field.name
                )
            } else {
                format!("serde_json::to_string(&self.{name}).ok()", name = field.name)
            };
            let getter_method = format!("pub fn {getter_ident}(&self) -> Option<String> {{\n    {body}\n}}");
            impl_builder.add_method(&getter_method);
            continue;
        }
        let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
        // Don't double-wrap Optional: if the field's IR type is already Optional<T>,
        // it maps to `Option<T>` — wrapping again with mapper.optional() would yield
        // `Option<Option<T>>`, which doesn't match the storage field's actual type.
        let mapped = mapper.map_type(&field.ty);
        let already_optional = matches!(field.ty, TypeRef::Optional(_));
        // The PHP struct emitter always enables option_duration_on_defaults, so Duration
        // fields on Default structs are stored as Option<i64> in the binding struct.
        // The getter return type must mirror the storage type — apply the same condition
        // (`typ.has_default && !field.optional && Duration`) so the getter declares
        // Option<i64> and not bare i64.
        let force_optional = typ.has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
        let rust_return_type = if (field.optional || force_optional) && !already_optional {
            mapper.optional(&mapped)
        } else {
            map_fn(&field.ty)
        };

        // For Option<NonOpaqueNamed>, ext-php-rs's IntoZval impl may not handle
        // the conversion to PHP null correctly. Explicitly unwrap and map through
        // .into() conversion, which is what php_wrap_return does for returns.
        // For Option<NonOpaqueNamed>, ext-php-rs's IntoZval impl may not handle
        // the conversion to PHP null correctly. Explicitly unwrap and map through
        // .into() conversion, which is what php_wrap_return does for returns.
        // The IR type may be either:
        // 1. Optional<Named> — field.optional=true, field.ty=Optional(Named), already_optional=true
        // 2. Named with optional flag set — field.optional=true, field.ty=Named, already_optional=false
        // In case (2), the getter returns Option<T> due to the condition on line 790.
        let is_optional_named = match &field.ty {
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str()))
            }
            TypeRef::Named(n) if field.optional && !already_optional => !opaque_types.contains(n.as_str()),
            _ => false,
        };
        let body = if is_optional_named {
            format!("self.{name}.clone().map(Into::into)", name = field.name)
        } else {
            format!("self.{name}.clone()", name = field.name)
        };

        let getter_method = format!(
            "pub fn {getter_ident}(&self) -> {ret} {{\n    {body}\n}}",
            body = body,
            ret = rust_return_type,
        );
        impl_builder.add_method(&getter_method);

        // Note: setters for Named/Vec/Map fields are not generated because
        // ext-php-rs doesn't support &T: FromZval for #[php(setter)] parameters.
        // Config types with complex fields should be constructed via fromJson().
    }

    // Non-opaque structs don't have adapter bodies — adapters apply to opaque types only.
    let empty_adapter_bodies: crate::adapters::AdapterBodies = Default::default();

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        if method.is_async {
            impl_builder.add_method(&gen_async_instance_method(
                method,
                mapper,
                false,
                &typ.name,
                opaque_types,
                &mapper.enum_names,
                core_import,
                &empty_adapter_bodies,
                &AHashSet::new(),
            ));
        } else {
            impl_builder.add_method(&gen_instance_method_non_opaque(
                method,
                mapper,
                typ,
                core_import,
                opaque_types,
                enums,
                bridge_type_aliases,
                mutex_types,
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
            impl_builder.add_method(&gen_static_method(
                method,
                mapper,
                opaque_types,
                typ,
                core_import,
                mutex_types,
            ));
        }
    }

    // Generate wither methods for opaque-type / bridge-type fields. These let PHP callers set
    // a single trait-bridge field on an existing struct instance. PHP can't construct opaque
    // handles via the generated constructor because they're filtered out of constructor params.
    //
    // Walk raw `typ.fields` (not `binding_fields()`): trait-bridge fields are often marked
    // binding_excluded so they don't appear in the constructor / from_json builder, but the
    // struct still carries the field and the wither is the only way to set it from PHP.
    let all_opaque_types: AHashSet<String> = opaque_types.iter().chain(bridge_type_aliases.iter()).cloned().collect();
    for field in typ.fields.iter() {
        // Trait-bridge / opaque fields lose their `Option<>` wrapper during IR extraction
        // (they're inherently optional handles) but the generated struct re-wraps them in
        // `Option<T>`. Match both shapes so the wither emits for either IR form.
        let bridge_inner: Option<&str> = match &field.ty {
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(name) if all_opaque_types.contains(name.as_str()) => Some(name.as_str()),
                _ => None,
            },
            TypeRef::Named(name) if all_opaque_types.contains(name.as_str()) => Some(name.as_str()),
            _ => None,
        };
        if let Some(inner_name) = bridge_inner {
            let wither_name = format!("with_{}", field.name);
            let param_name = crate::codegen::naming::to_php_name(&field.name);
            let mapped_inner_type = mapper.map_type(&TypeRef::Named(inner_name.to_string()));
            // ext-php-rs heap-allocates PHP objects: both `self` and trait-bridge args must be
            // passed by reference. Emit `&self -> Self` for chainable fluent calls, and accept
            // the bridge handle as `&mut Inner` then clone it into the new instance.
            let wither_method = format!(
                "pub fn {wither_name}(&self, {param_name}: &mut {mapped_inner_type}) -> Self {{\n    \
                 let mut next = self.clone();\n    \
                 next.{field_name} = Some({param_name}.clone());\n    \
                 next\n\
                 }}",
                field_name = field.name,
            );
            impl_builder.add_method(&wither_method);
        }
    }

    impl_builder.build()
}

/// Generate a `#[php_class]` with class constants for unit-variant enums.
///
/// Emits a PHP-visible class that allows consumers to reference enum values as
/// `EnumName::VARIANT_NAME` at runtime.
pub(crate) fn gen_enum_constants(enum_def: &EnumDef, php_namespace: Option<&str>) -> String {
    let mut lines = vec![];

    // Emit the #[php_class] decorator with optional namespace.
    if let Some(ns) = php_namespace {
        lines.push(format!("#[php_class(namespace = \"{}\")]", ns));
    } else {
        lines.push("#[php_class]".to_string());
    }

    // Emit the PHP class struct (with no fields — exists only for constants).
    lines.push(format!("pub struct {} {{}}", enum_def.name));
    lines.push(String::new());

    // Emit the #[php_impl] block with class constants.
    lines.push(format!("#[php_impl]"));
    lines.push(format!("impl {} {{", enum_def.name));

    for variant in &enum_def.variants {
        let const_name = variant.name.to_uppercase();
        lines.push(format!("    pub const {}: &str = \"{}\";", const_name, variant.name));
    }

    lines.push("}".to_string());

    lines.join("\n")
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

/// Generate `#[php_impl]` accessor methods and a `from_json` constructor for the flat data enum.
pub(crate) fn gen_flat_data_enum_methods(enum_def: &EnumDef, mapper: &PhpMapper) -> String {
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
    // Fallback for unrecognised tags: delegate to `<CorePath>::default()`.
    // This respects the core type's custom `impl Default` (e.g. `EmbeddingModelType` returns
    // `Preset { name: "balanced" }`) AND the `#[default]` variant attribute when present.
    // The prior path constructed the `#[default]`-marked or first variant with field-level
    // `Default::default()` defaults, which produced invalid sentinel values like
    // `Preset { name: "" }` for enums whose `Default` is custom rather than derived.
    out.push_str(&crate::backends::php::template_env::render(
        "php_flat_enum_default_fallback_match_arm.jinja",
        minijinja::context! {
            core_path => &core_path,
        },
    ));
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
