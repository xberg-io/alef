/// For a variant-wrapper opaque type (one whose `is_variant_wrapper` flag is set),
/// emit a `#[napi(constructor)] pub fn new_constructor(...)` on the prefixed binding
/// struct so that `new WrapperType(args)` JS constructor-syntax resolves at runtime.
///
/// napi-rs allows multiple `#[napi] impl` blocks. The static `new` is already
/// emitted by `gen_opaque_struct_methods` as a `#[napi]` static method; this
/// companion uses a distinct Rust fn name (`new_constructor`) to avoid the
/// duplicate-`fn new` conflict while mapping to the same JS `new Class()` path
/// via `#[napi(constructor)]`.
///
/// Returns `None` when the wrapper has no `new` method in the IR (or its receiver
/// is not `None`), silently skipping rather than panicking.
pub(super) fn napi_variant_wrapper_constructor(
    typ: &crate::core::ir::TypeDef,
    mapper: &crate::backends::napi::type_map::NapiMapper,
    core_import: &str,
    prefix: &str,
) -> Option<String> {
    use crate::codegen::type_mapper::TypeMapper as _;
    let ctor = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())?;
    let map_fn = |t: &crate::core::ir::TypeRef| mapper.map_type(t);
    let sig_params = crate::codegen::shared::function_params(&ctor.params, &map_fn);

    let call_args = ctor
        .params
        .iter()
        .map(|p| {
            let mapped_type = map_fn(&p.ty);

            let core_type_name = match &p.ty {
                crate::core::ir::TypeRef::Named(name) => name.as_str(),
                _ => "",
            };

            let needs_conversion =
                !core_type_name.is_empty() && mapped_type.starts_with(&mapper.prefix) && !mapped_type.contains("::");

            if needs_conversion {
                format!("{}.into()", p.name)
            } else {
                p.name.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let struct_name = format!("{prefix}{}", typ.name);
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let new_call = if call_args.is_empty() {
        format!("{core_path}::new()")
    } else {
        format!("{core_path}::new({call_args})")
    };
    let inner_expr = if crate::codegen::generators::type_needs_mutex(typ) {
        format!("std::sync::Arc::new(std::sync::Mutex::new({new_call}))")
    } else {
        format!("std::sync::Arc::new({new_call})")
    };
    let body = format!("Self {{ inner: {inner_expr} }}");
    let fn_sig = if sig_params.is_empty() {
        "pub fn new_constructor() -> Self".to_string()
    } else {
        format!("pub fn new_constructor({sig_params}) -> Self")
    };
    Some(format!(
        "#[napi]\nimpl {struct_name} {{\n    #[napi(constructor)]\n    {fn_sig} {{\n        {body}\n    }}\n}}\n",
    ))
}

/// For an explicitly-opaque type with `has_default` that is treated as opaque in NAPI,
/// emit a `#[napi(constructor)] pub fn new_constructor() -> Self` to enable JS `new ClassName()` syntax.
/// Uses a distinct Rust fn name (`new_constructor`) to avoid a duplicate-`fn new` conflict
/// with the static `#[napi] pub fn new()` method already emitted by `gen_opaque_struct_methods`.
/// This is a simple wrapper around the Rust `new()` method that returns a default instance.
pub(super) fn napi_default_constructor(
    typ: &crate::core::ir::TypeDef,
    _mapper: &crate::backends::napi::type_map::NapiMapper,
    core_import: &str,
    prefix: &str,
) -> Option<String> {
    typ.methods
        .iter()
        .find(|m| m.name == "new" && m.receiver.is_none() && m.params.is_empty())?;

    let struct_name = format!("{prefix}{}", typ.name);
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let inner_expr = if crate::codegen::generators::type_needs_mutex(typ) {
        format!("std::sync::Arc::new(std::sync::Mutex::new({core_path}::new()))")
    } else {
        format!("std::sync::Arc::new({core_path}::new())")
    };

    let constructor = format!(
        "#[napi]\nimpl {struct_name} {{\n    #[napi(constructor)]\n    pub fn new_constructor() -> Self {{\n        Self {{ inner: {inner_expr} }}\n    }}\n}}\n"
    );

    Some(constructor)
}
