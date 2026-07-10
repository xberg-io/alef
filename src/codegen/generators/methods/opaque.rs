use super::instance::gen_method;
use super::static_methods::gen_static_method;
use crate::codegen::generators::{AdapterBodies, RustBindingConfig};
use crate::codegen::shared::partition_methods;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::TypeDef;
use ahash::AHashSet;

/// Generate a full impl block for an opaque type, delegating methods to `self.inner`.
///
/// `opaque_types` is the set of type names that are opaque wrappers (use `Arc<inner>`).
/// This is needed so that return-type wrapping uses the correct pattern for cross-type returns.
/// `mutex_types` is the subset of opaque types whose inner field uses `Arc<Mutex<T>>`;
/// method dispatch uses `.lock().unwrap()` for these types.
pub fn gen_opaque_impl_block(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    adapter_bodies: &AdapterBodies,
) -> String {
    let (instance, statics) = partition_methods(&typ.methods);
    let has_emittable_instance = instance
        .iter()
        .any(|m| !m.sanitized || adapter_bodies.contains_key(&format!("{}.{}", typ.name, m.name)));
    let has_emittable_statics = statics
        .iter()
        .any(|m| !m.sanitized || adapter_bodies.contains_key(&format!("{}.{}", typ.name, m.name)));
    if !has_emittable_instance && !has_emittable_statics {
        return String::new();
    }

    let mut out = String::with_capacity(2048);
    let prefixed_name = format!("{}{}", cfg.type_name_prefix, typ.name);

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
            true,
            opaque_types,
            mutex_types,
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
            mutex_types,
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

/// Generate a custom opaque-handle constructor from a [`ClientConstructorConfig`].
///
/// Emits a `pub fn new(params…) -> Result<Self, ErrType>` method body suitable
/// for inclusion inside an `impl TypeName { … }` block.
///
/// * `constructor_attr` — optional attribute line placed immediately before the
///   `pub fn new` line (e.g. `"#[new]"` for PyO3, `"#[napi(constructor)]"` for
///   NAPI-RS, or `""` to emit no attribute).
pub fn gen_opaque_constructor(
    ctor: &ClientConstructorConfig,
    type_name: &str,
    core_import: &str,
    constructor_attr: &str,
) -> String {
    let source_path = if core_import.is_empty() {
        type_name.to_string()
    } else {
        format!("{core_import}::{type_name}")
    };

    let params_str = if ctor.params.is_empty() {
        String::new()
    } else {
        ctor.params
            .iter()
            .map(|p| format!("{}: {}", p.name, p.ty))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let body = ctor
        .body
        .replace("{type_name}", type_name)
        .replace("{source_path}", &source_path);

    let err_ty = ctor.error_type.as_deref().unwrap_or("String");

    let attr_prefix = if constructor_attr.is_empty() {
        String::new()
    } else {
        format!("    {constructor_attr}\n")
    };

    format!("{attr_prefix}    pub fn new({params_str}) -> Result<Self, {err_ty}> {{\n        {body}\n    }}\n")
}
