use super::constructors::gen_constructor_with_renames;
use super::instance::gen_method;
use super::static_methods::gen_static_method;
use crate::codegen::generators::{AdapterBodies, RustBindingConfig};
use crate::codegen::shared::partition_methods;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::TypeDef;
use ahash::AHashSet;

/// Generate a full methods impl block (non-opaque types).
pub fn gen_impl_block(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    adapter_bodies: &AdapterBodies,
    opaque_types: &AHashSet<String>,
) -> String {
    gen_impl_block_with_renames(typ, mapper, cfg, adapter_bodies, opaque_types, None)
}

/// Like `gen_impl_block` but with field renames for keyword escaping in the constructor.
pub fn gen_impl_block_with_renames(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    adapter_bodies: &AdapterBodies,
    opaque_types: &AHashSet<String>,
    field_renames: Option<&std::collections::HashMap<String, String>>,
) -> String {
    let (instance, statics) = partition_methods(&typ.methods);
    let has_emittable_instance = instance
        .iter()
        .any(|m| !m.sanitized || adapter_bodies.contains_key(&format!("{}.{}", typ.name, m.name)));
    let has_emittable_statics = statics
        .iter()
        .any(|m| !m.sanitized || adapter_bodies.contains_key(&format!("{}.{}", typ.name, m.name)));
    if !has_emittable_instance && !has_emittable_statics && typ.fields.is_empty() {
        return String::new();
    }

    let prefixed_name = format!("{}{}", cfg.type_name_prefix, typ.name);
    let mut out = String::with_capacity(2048);

    // `#[staticmethod] pub fn new(...)` and would conflict with a `#[new]` constructor).
    let has_explicit_static_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");
    if !typ.fields.is_empty() && !cfg.skip_impl_constructor && !has_explicit_static_new {
        out.push_str(&gen_constructor_with_renames(typ, mapper, cfg, field_renames));
        out.push_str("\n\n");
    }

    let empty_mutex_types: AHashSet<String> = AHashSet::new();
    for m in &instance {
        let adapter_key = format!("{}.{}", typ.name, m.name);
        if m.sanitized && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
        if cfg.skip_methods_when_not_delegatable
            && !adapter_bodies.contains_key(&adapter_key)
            && !crate::codegen::shared::can_auto_delegate(m, opaque_types)
        {
            continue;
        }
        out.push_str(&gen_method(
            m,
            mapper,
            cfg,
            typ,
            false,
            opaque_types,
            &empty_mutex_types,
            adapter_bodies,
        ));
        out.push_str("\n\n");
    }

    for m in &statics {
        let adapter_key = format!("{}.{}", typ.name, m.name);
        if m.sanitized && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
        if cfg.skip_methods_when_not_delegatable
            && !adapter_bodies.contains_key(&adapter_key)
            && !crate::codegen::shared::can_auto_delegate(m, opaque_types)
        {
            continue;
        }
        out.push_str(&gen_static_method(
            m,
            mapper,
            cfg,
            typ,
            adapter_bodies,
            opaque_types,
            &empty_mutex_types,
        ));
        out.push_str("\n\n");
    }

    let trimmed = out.trim_end();
    let content = trimmed.to_string();

    crate::codegen::template_env::render(
        "generators/methods/impl_block.jinja",
        minijinja::context! {
            block_attr => cfg.method_block_attr,
            prefixed_name => prefixed_name,
            content => content,
        },
    )
}
