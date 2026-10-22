//! Emits the swift-bridge wrapper newtype structs for IR struct types.
//!
//! `emit_type_wrapper` produces:
//!   - `pub struct T(pub SourceT)` newtype
//!   - `impl T { pub fn new(…) → T }` constructor
//!   - `impl T { pub fn field(&self) → BridgeType }` getters
//!
//! Enum wrappers live in `enums.rs`.

use crate::backends::swift::gen_rust_crate::bridge_error::result_type;
use crate::backends::swift::gen_rust_crate::default_construction::{
    EnumKinds, emit_default_construction_body, emit_direct_field_inits,
};
use crate::backends::swift::gen_rust_crate::extern_block::{constructor_fields, has_constructor_extern};
use crate::backends::swift::gen_rust_crate::type_bridge::{bridge_type, needs_json_bridge};
use crate::codegen::generators::type_paths::resolve_type_path;
use crate::core::ir::{TypeDef, TypeRef};
use crate::core::keywords::swift_ident;
use heck::ToSnakeCase;
use std::collections::{HashMap, HashSet};

use super::getters::emit_getters;

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_type_wrapper(
    ty: &TypeDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    unit_enum_names: &HashSet<&str>,
    reconstructible_enum_names: &HashSet<&str>,
    no_serde_names: &HashSet<&str>,
    first_class_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &HashSet<&str>,
) -> String {
    let mut out = String::new();
    let source_path = resolve_type_path(&ty.name, source_crate, type_paths);
    if let Some(cfg) = ty.cfg.as_deref() {
        out.push_str(&format!("#[cfg({cfg})]\n"));
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "struct_newtype.jinja",
        crate::alef_context! {
            name => &ty.name,
            source_path => &source_path,
            has_lifetime_params => ty.has_lifetime_params,
        },
    ));

    if !ty.fields.is_empty() {
        if let Some(cfg) = ty.cfg.as_deref() {
            out.push_str(&format!("#[cfg({cfg})]\n"));
        }
        out.push_str(&crate::backends::swift::template_env::render(
            "impl_header.jinja",
            crate::alef_context! {
                name => &ty.name,
            },
        ));

        let mut constructors = vec![("new", std::borrow::Cow::Borrowed(ty))];
        if !ty.legacy_constructor_fields().is_empty() {
            let mut legacy = ty.clone();
            legacy.fields = ty
                .legacy_constructor_fields()
                .iter()
                .map(|name| {
                    ty.fields
                        .iter()
                        .find(|field| &field.name == name)
                        .expect("validated constructor field")
                        .clone()
                })
                .collect();
            constructors[0].1 = std::borrow::Cow::Owned(legacy);
            constructors.push(("new_with_fields", std::borrow::Cow::Borrowed(ty)));
        }
        for (constructor_name, constructor_type) in &constructors {
            out.push_str(&emit_constructor(
                constructor_type,
                constructor_name,
                &source_path,
                type_paths,
                enum_names,
                unit_enum_names,
                reconstructible_enum_names,
                first_class_names.contains(ty.name.as_str()),
                no_serde_names,
                exclude_fields,
                configured_features,
            ));
        }

        emit_getters(
            ty,
            type_paths,
            enum_names,
            unit_enum_names,
            no_serde_names,
            first_class_names,
            exclude_fields,
            configured_features,
            &mut out,
        );

        out.push_str("}\n");
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn emit_constructor(
    ty: &TypeDef,
    constructor_name: &str,
    source_path: &str,
    type_paths: &HashMap<String, String>,
    enum_names: &HashSet<&str>,
    unit_enum_names: &HashSet<&str>,
    reconstructible_enum_names: &HashSet<&str>,
    parent_first_class: bool,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &HashSet<&str>,
) -> String {
    let enum_kinds = EnumKinds::with_reconstructible(enum_names, unit_enum_names, reconstructible_enum_names);
    if !has_constructor_extern(ty, exclude_fields, configured_features, enum_kinds, parent_first_class) {
        return String::new();
    }
    let fields = constructor_fields(ty, exclude_fields, configured_features);
    let params: Vec<_> = fields
        .iter()
        .map(|field| {
            let mapped = bridge_type(&field.ty);
            let mapped = if field.optional && !needs_json_bridge(&field.ty) {
                format!("Option<{mapped}>")
            } else {
                mapped
            };
            format!("{}: {mapped}", swift_ident(&field.name.to_snake_case()))
        })
        .collect();
    let needs_default = constructor_needs_default(ty, &fields);
    let mut out = String::new();
    if !needs_default && ty.has_default {
        out.push_str("    #[allow(clippy::needless_update)]\n");
    }
    out.push_str(&crate::backends::swift::template_env::render(
        "fn_new_signature.jinja",
        crate::alef_context! {
            params => params.join(", "), name => &ty.name, constructor_name => constructor_name,
        },
    ));
    out.push_str(&constructor_initialization(
        ty,
        source_path,
        type_paths,
        enum_kinds,
        no_serde_names,
        exclude_fields,
        configured_features,
        needs_default,
    ));
    out.push_str("    }\n");
    out
}

fn constructor_needs_default(ty: &TypeDef, fields: &[&crate::core::ir::FieldDef]) -> bool {
    let all_primitive = fields.iter().all(|field| matches!(field.ty, TypeRef::Primitive(_)));
    let has_vec = fields.iter().any(|field| {
        matches!(&field.ty, TypeRef::Vec(inner) if !matches!(inner.as_ref(), TypeRef::Primitive(_) | TypeRef::Bytes))
    });
    let has_string = !ty.has_serde
        && fields.iter().any(|field| {
            matches!(
                field.ty,
                TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Char
            )
        });
    !all_primitive
        && (ty.has_serde
            || has_vec
            || has_string
            || ty.has_stripped_cfg_fields
            || fields
                .iter()
                .any(|field| needs_json_bridge(&field.ty) || matches!(field.ty, TypeRef::Named(_))))
}

#[allow(clippy::too_many_arguments)]
fn constructor_initialization(
    ty: &TypeDef,
    source_path: &str,
    type_paths: &HashMap<String, String>,
    enum_kinds: EnumKinds<'_, '_>,
    no_serde_names: &HashSet<&str>,
    exclude_fields: &HashSet<String>,
    configured_features: &HashSet<&str>,
    needs_default: bool,
) -> String {
    if needs_default && ty.has_default {
        return emit_default_construction_body(
            ty,
            source_path,
            type_paths,
            enum_kinds,
            no_serde_names,
            exclude_fields,
            configured_features,
        );
    }
    let field_inits = emit_direct_field_inits(
        ty,
        type_paths,
        enum_kinds,
        no_serde_names,
        exclude_fields,
        configured_features,
    );
    let mut out = crate::backends::swift::template_env::render(
        "struct_literal_open.jinja",
        crate::alef_context! {
            name => &ty.name, source_path => source_path,
        },
    );
    for init in &field_inits {
        out.push_str(init);
        out.push_str(",\n");
    }
    if ty.has_default {
        out.push_str("            ..Default::default()\n");
    }
    out.push_str("        })\n");
    out
}

/// Emit a `pub fn create_<type_name>(api_key: String, base_url: Option<String>) -> Result<TypeName, AlefBridgeError>`
/// constructor shim for an opaque type that exposes methods.
///
/// The source crate must provide `<TypeName>::new(api_key, base_url)` or a compatible constructor.
/// This mirrors the common stateful-client constructor pattern.
///
/// When the source crate's constructor signature differs
/// `DefaultClient::new(ClientConfig, Option<&str>)`), the caller can supply a
/// custom body via `[crates.<crate>.swift] client_constructor_body."TypeName" = "..."`
/// in alef.toml. The custom body is interpolated verbatim, with `{type_name}` and
/// `{source_path}` placeholders available.
pub(crate) fn emit_type_constructor_shim(
    ty: &TypeDef,
    source_crate: &str,
    type_paths: &HashMap<String, String>,
    custom_body: Option<&str>,
) -> String {
    let type_snake = ty.name.to_snake_case();
    let fn_name = format!("create_{type_snake}");
    let type_name = &ty.name;
    let source_path = resolve_type_path(type_name, source_crate, type_paths);

    let cfg_prefix = ty.cfg.as_deref().map(|c| format!("#[cfg({c})]\n")).unwrap_or_default();

    if let Some(body) = custom_body {
        let interpolated = body
            .replace("{type_name}", type_name)
            .replace("{source_path}", &source_path);
        // The configured body returns the constructor's own `Result`. It is wrapped in an inner
        // function so a body that already maps its error to a `String` still compiles, and the
        // error type of the inner function is whatever the body returns. ~keep
        return format!(
            concat!(
                "{cfg_prefix}pub fn {fn_name}(api_key: String, base_url: Option<String>)",
                " -> {result_ty} {{\n",
                "    fn build(api_key: String, base_url: Option<String>)",
                " -> Result<{type_name}, impl std::fmt::Display + 'static> {{\n",
                "{interpolated}\n",
                "    }}\n",
                "    build(api_key, base_url).map_err(alef_bridge_error)\n",
                "}}\n"
            ),
            cfg_prefix = cfg_prefix,
            fn_name = fn_name,
            result_ty = result_type(type_name),
            type_name = type_name,
            interpolated = interpolated,
        );
    }

    format!(
        "{cfg_prefix}pub fn {fn_name}(api_key: String, base_url: Option<String>) -> {result_ty} {{\n    \
         {source_path}::new(api_key, base_url)\n        \
         .map_err(alef_bridge_error)\n        \
         .map({type_name})\n}}\n",
        cfg_prefix = cfg_prefix,
        result_ty = result_type(type_name),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata, TypeRef};

    fn transparent_string_wrapper(containers: Vec<NewtypeContainer>) -> String {
        NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "sample::SecretString",
            "from",
            "into_inner",
            containers,
        )])
    }

    #[test]
    fn transparent_string_field_converts_at_constructor_and_getter_boundaries() {
        let ty = TypeDef {
            name: "Credentials".to_string(),
            rust_path: "sample::Credentials".to_string(),
            has_default: true,
            has_serde: true,
            fields: vec![FieldDef {
                name: "secret".to_string(),
                ty: TypeRef::String,
                newtype_wrapper: Some(transparent_string_wrapper(vec![])),
                ..Default::default()
            }],
            ..Default::default()
        };

        let output = emit_type_wrapper(
            &ty,
            "sample",
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(
            output.contains("__target.secret = sample::SecretString::from(secret);"),
            "{output}"
        );
        assert!(output.contains("self.0.secret.clone()).into_inner()"), "{output}");
    }

    #[test]
    fn transparent_string_map_field_converts_at_getter_boundary() {
        let ty = TypeDef {
            name: "Credentials".to_string(),
            rust_path: "sample::Credentials".to_string(),
            has_default: true,
            has_serde: true,
            fields: vec![FieldDef {
                name: "lookup".to_string(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                newtype_wrapper: Some(transparent_string_wrapper(vec![NewtypeContainer::MapValue])),
                ..Default::default()
            }],
            ..Default::default()
        };

        let output = emit_type_wrapper(
            &ty,
            "sample",
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(output.contains("self.0.lookup.clone()).into_iter()"), "{output}");
    }

    #[test]
    fn wrapper_constructor_filters_cfg_gated_fields() {
        let fields = vec![
            FieldDef {
                version: Default::default(),
                name: "field_a".to_string(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                optional: false,
                default: None,
                doc: "".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: Default::default(),
                vec_inner_core_wrapper: Default::default(),
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
                name: "field_b".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: "".to_string(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: Some("feature = \"heuristics\"".to_string()),
                typed_default: None,
                core_wrapper: Default::default(),
                vec_inner_core_wrapper: Default::default(),
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                sensitive: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
        ];

        let ty = TypeDef {
            name: "TestType".to_string(),
            rust_path: "test::TestType".to_string(),
            original_rust_path: "".to_string(),
            fields,
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: "".to_string(),
            cfg: None,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        };

        let exclude_fields = std::collections::HashSet::new();
        let mut configured_features = std::collections::HashSet::new();
        configured_features.insert("pdf");

        let output = emit_type_wrapper(
            &ty,
            "test_crate",
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &exclude_fields,
            &configured_features,
        );

        assert!(output.contains("pub struct TestType"));
        assert!(!output.contains("field_b: String"));
        assert!(output.contains("field_a: u32"));
    }

    fn primitive_field(name: &str) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
            optional: false,
            ..Default::default()
        }
    }

    fn primitive_only_type(has_default: bool) -> crate::core::ir::TypeDef {
        crate::core::ir::TypeDef {
            name: "SampleLimits".to_string(),
            rust_path: "test::SampleLimits".to_string(),
            fields: vec![primitive_field("depth"), primitive_field("width")],
            is_clone: true,
            has_default,
            ..Default::default()
        }
    }

    fn emit_wrapper(ty: &crate::core::ir::TypeDef) -> String {
        emit_type_wrapper(
            ty,
            "test_crate",
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
    }

    /// Forward-compatibility: the direct struct-literal constructor for a
    /// has_default core type must end with `..Default::default()`, so an
    /// additive core field falls back to its default instead of failing E0063
    /// until the bindings are regenerated.
    #[test]
    fn wrapper_constructor_direct_literal_with_default_spreads() {
        let output = emit_wrapper(&primitive_only_type(true));

        assert!(
            output.contains("..Default::default()"),
            "has_default core type must get the spread trailer in the direct-literal \
             constructor; got:\n{output}"
        );
        assert!(
            output.contains("#[allow(clippy::needless_update)]"),
            "the spread over a fully-mirrored literal needs the needless_update allow; \
             got:\n{output}"
        );
    }

    /// Companion: without a core `Default` impl the spread cannot compile (E0277) —
    /// the exhaustive literal must stay as-is.
    #[test]
    fn wrapper_constructor_direct_literal_without_default_keeps_exhaustive_literal() {
        let output = emit_wrapper(&primitive_only_type(false));

        assert!(
            output.contains("pub fn new("),
            "primitive-only DTOs keep their direct constructor regardless of Default; \
             got:\n{output}"
        );
        assert!(
            !output.contains("..Default::default()"),
            "the spread trailer must not be emitted when the core type has no Default \
             impl; got:\n{output}"
        );
        assert!(
            !output.contains("#[allow(clippy::needless_update)]"),
            "needless_update allow must not be emitted when no spread; got:\n{output}"
        );
    }
}
