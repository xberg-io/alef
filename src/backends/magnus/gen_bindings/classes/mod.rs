//! Struct and enum code generators for the Magnus (Ruby) backend.

use crate::codegen::builder::ImplBuilder;
use crate::codegen::generators;
use crate::codegen::shared::{binding_fields, function_params};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use ahash::AHashSet;

use crate::backends::magnus::type_map::MagnusMapper;

use super::functions::{ASYNC_RETURN_IS_FALLIBLE, gen_magnus_unimplemented_body};
use super::method_result_wrap::non_opaque_method_result_wrap;

/// Check whether a struct has a `content` field of type `String` or `Option<String>`.
/// When true, a `to_s` method should be generated so Ruby callers can use `result.to_s`
/// to retrieve the primary markdown output without explicitly calling `.content`.
pub(super) fn has_content_string_field(typ: &TypeDef) -> bool {
    binding_fields(&typ.fields).any(|f| {
        if f.name != "content" {
            return false;
        }
        matches!(&f.ty, TypeRef::String)
            || matches!(&f.ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String))
    })
}

/// Check if a field contains a bridge handle that cannot be safely passed across thread boundaries.
fn is_thread_unsafe_field(field: &FieldDef, trait_bridges: &[TraitBridgeConfig]) -> bool {
    crate::codegen::generators::trait_bridge::is_bridge_handle_type_ref(&field.ty, trait_bridges)
}

/// Generate an opaque Magnus-wrapped struct with inner Arc or Arc<Mutex<>>.
///
/// `opaque_struct.rs.jinja` renders the struct declaration AND its three companion impls
/// (`IntoValueFromNative`, `magnus::TryConvert`, `TryConvertOwned`) as one multi-item block. A
/// `#[cfg(...)]` attribute only binds to the single item immediately following it, so `typ.cfg`
/// is threaded into the template itself and repeated before every item there -- gating the
/// returned string at the call site (a single `#[cfg]` prepended to the whole blob) would gate
/// only the struct declaration and leave the impls unconditionally referencing a type the core
/// crate never compiled in. ~keep
pub(super) fn gen_opaque_struct(typ: &TypeDef, core_import: &str, module_name: &str) -> String {
    let class_path = format!("{}::{}", module_name, typ.name);
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let needs_mutex = crate::codegen::generators::type_needs_mutex(typ);

    crate::backends::magnus::template_env::render(
        "opaque_struct.rs.jinja",
        minijinja::context! {
            struct_name => &typ.name,
            class_path => &class_path,
            core_path => &core_path,
            needs_mutex => needs_mutex,
            cfg => typ.cfg.as_deref(),
        },
    )
}

/// Generate Magnus methods for an opaque struct (delegates to self.inner).
///
/// `streaming_method_names` lists method names whose default async-stub emission
/// should be skipped — the streaming module emits a dedicated, hand-rolled
/// implementation for those methods (yielding to a Ruby block / returning an
/// Enumerator) and registers it separately.
pub(super) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    core_import: &str,
    streaming_method_names: &AHashSet<String>,
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);

    let needs_mutex = crate::codegen::generators::type_needs_mutex(typ);

    for method in &typ.methods {
        if !method.is_static {
            if streaming_method_names.contains(&method.name) {
                continue;
            }
            if method.is_async {
                impl_builder.add_method(&gen_opaque_async_instance_method(
                    typ,
                    method,
                    mapper,
                    &typ.name,
                    opaque_types,
                    mutex_types,
                    core_import,
                    needs_mutex,
                ));
            } else {
                impl_builder.add_method(&gen_opaque_instance_method(
                    typ,
                    method,
                    mapper,
                    &typ.name,
                    opaque_types,
                    mutex_types,
                    core_import,
                    needs_mutex,
                ));
            }
        }
    }

    impl_builder.build()
}

/// Map a param type for a Magnus wrapper-fn signature. A `TypeRef::Bytes` param must decode from a
/// Ruby `String` to honor the `.rbs` contract (`rbs_type` maps `Bytes` -> `String`), so the wrapper
/// takes a `magnus::RString`; `build_method_preamble` then copies it into a `Vec<u8>` before the core
/// call. Every other type uses the default Rust mapping.
fn magnus_param_signature_type(ty: &TypeRef, mapper: &MagnusMapper) -> String {
    match ty {
        TypeRef::Bytes => "magnus::RString".to_string(),
        _ => mapper.map_type(ty),
    }
}

/// Build let-binding preamble for non-opaque Named ref params and Vec<String> ref params.
/// Emits `let {name}_core: core::Type = {name}.into();` for Named non-opaque is_ref params,
/// and `let {name}_refs: Vec<&str> = ...;` for Vec<String>/Vec<Char> is_ref params.
fn build_method_preamble(
    params: &[crate::core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut out = String::new();
    for p in params {
        if p.sanitized {
            continue;
        }
        match &p.ty {
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => {
                let core_path = format!("{}::{}", core_import, n);
                if p.optional {
                    out.push_str(&crate::backends::magnus::template_env::render(
                        "method_optional_named_ref_preamble.rs.jinja",
                        minijinja::context! {
                            param_name => &p.name,
                            core_path => &core_path,
                        },
                    ));
                    out.push_str("        ");
                } else {
                    out.push_str(&crate::backends::magnus::template_env::render(
                        "method_named_ref_preamble.rs.jinja",
                        minijinja::context! {
                            param_name => &p.name,
                            core_path => &core_path,
                        },
                    ));
                    out.push_str("        ");
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) && p.is_ref => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    let core_inner_ty = format!("{core_import}::{name}");
                    let vec_ty = format!("Vec<{core_inner_ty}>");
                    if p.optional {
                        out.push_str(&crate::backends::magnus::template_env::render(
                            "method_optional_named_vec_binding.rs.jinja",
                            minijinja::context! {
                                param_name => &p.name,
                                vec_ty => &vec_ty,
                            },
                        ));
                        out.push_str("        ");
                    } else {
                        out.push_str(&crate::backends::magnus::template_env::render(
                            "method_named_vec_binding.rs.jinja",
                            minijinja::context! {
                                param_name => &p.name,
                                vec_ty => &vec_ty,
                            },
                        ));
                        out.push_str("        ");
                    }
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                if p.optional {
                    out.push_str(&crate::backends::magnus::template_env::render(
                        "method_optional_string_vec_ref_preamble.rs.jinja",
                        minijinja::context! {
                            param_name => &p.name,
                        },
                    ));
                    out.push_str("        ");
                } else {
                    out.push_str(&crate::backends::magnus::template_env::render(
                        "method_string_vec_ref_preamble.rs.jinja",
                        minijinja::context! {
                            param_name => &p.name,
                        },
                    ));
                    out.push_str("        ");
                }
            }
            TypeRef::Bytes if !p.optional => {
                out.push_str(&crate::backends::magnus::template_env::render(
                    "method_bytes_ref_preamble.rs.jinja",
                    minijinja::context! {
                        param_name => &p.name,
                    },
                ));
                out.push_str("        ");
            }
            _ => {}
        }
    }
    out
}

/// Generate an opaque sync instance method for Magnus (delegates to self.inner).
#[allow(clippy::too_many_arguments)]
fn gen_opaque_instance_method(
    typ: &TypeDef,
    method: &MethodDef,
    mapper: &MagnusMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    core_import: &str,
    needs_mutex: bool,
) -> String {
    use crate::codegen::shared;
    let params = function_params(&method.params, &|ty| magnus_param_signature_type(ty, mapper));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let is_ref_mut_receiver = matches!(method.receiver, Some(crate::core::ir::ReceiverKind::RefMut));
    let can_delegate = !method.sanitized
        && (!is_ref_mut_receiver || needs_mutex)
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && shared::is_delegatable_param(&p.ty, opaque_types))
        && shared::is_delegatable_return(&method.return_type);

    let body = if can_delegate {
        let preamble = build_method_preamble(&method.params, opaque_types, core_import);
        let needs_let_bindings = !preamble.is_empty();
        let call_args = if needs_let_bindings {
            generators::gen_call_args_with_let_bindings_json_str(&method.params, opaque_types)
        } else {
            generators::gen_call_args(&method.params, opaque_types)
        };
        let refs_preamble = preamble;
        let is_owned_receiver = matches!(method.receiver, Some(ReceiverKind::Owned));
        let has_mut_methods = typ
            .methods
            .iter()
            .any(|m| matches!(m.receiver.as_ref(), Some(ReceiverKind::RefMut)));
        let inner_access = if is_owned_receiver {
            "self.inner.as_ref().clone()".to_string()
        } else if has_mut_methods {
            "self.inner.lock().unwrap()".to_string()
        } else {
            "self.inner".to_string()
        };
        let core_call = format!("{inner_access}.{}({})", method.name, call_args);
        if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!(
                    "{refs_preamble}{core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok(())"
                )
            } else {
                let wrap = generators::wrap_return_with_mutex(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    mutex_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                );
                format!(
                    "{refs_preamble}let result = {core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok({wrap})"
                )
            }
        } else {
            let wrapped = generators::wrap_return_with_mutex(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                mutex_types,
                true,
                method.returns_ref,
                method.returns_cow,
            );
            format!("{refs_preamble}{wrapped}")
        }
    } else {
        gen_magnus_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };
    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n    "
    } else {
        ""
    };
    // A gated method keeps its gate: the scaffolded Ruby extension crate declares every referenced
    // cfg feature as a default-on passthrough (`scaffold::languages::ruby`), so `rustc` resolves
    // it, exactly as `prepend_cfg` already arranges for top-level functions. The matching
    // `define_method` statement in `functions::module_init` carries the identical gate — emitting
    // one without the other leaves `method!(Type::name, n)` naming a fn that was compiled out. ~keep
    let method_cfg = method.rust_cfg_attribute();
    format!(
        "{method_cfg}{trait_allow}fn {}(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    }}",
        method.name
    )
}

/// Generate an opaque async instance method for Magnus (block on runtime, delegates to self.inner).
#[allow(clippy::too_many_arguments)]
fn gen_opaque_async_instance_method(
    typ: &TypeDef,
    method: &MethodDef,
    mapper: &MagnusMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    core_import: &str,
    needs_mutex: bool,
) -> String {
    use crate::codegen::shared;
    let params = function_params(&method.params, &|ty| magnus_param_signature_type(ty, mapper));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, ASYNC_RETURN_IS_FALLIBLE);

    let is_ref_mut_receiver = matches!(method.receiver, Some(crate::core::ir::ReceiverKind::RefMut));
    let can_delegate = !method.sanitized
        && (!is_ref_mut_receiver || needs_mutex)
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && shared::is_delegatable_param(&p.ty, opaque_types))
        && shared::is_delegatable_return(&method.return_type);

    let body = if can_delegate {
        let preamble = build_method_preamble(&method.params, opaque_types, core_import);
        let needs_let_bindings = !preamble.is_empty();
        let call_args = if needs_let_bindings {
            generators::gen_call_args_with_let_bindings_json_str(&method.params, opaque_types)
        } else {
            generators::gen_call_args(&method.params, opaque_types)
        };
        let refs_preamble = preamble;
        let has_mut_methods = typ
            .methods
            .iter()
            .any(|m| matches!(m.receiver.as_ref(), Some(ReceiverKind::RefMut)));
        let inner_setup = if has_mut_methods {
            "let inner = self.inner.lock().unwrap();\n        ".to_string()
        } else {
            "let inner = self.inner.clone();\n        ".to_string()
        };
        let core_call = format!("inner.{}({})", method.name, call_args);
        let result_wrap = generators::wrap_return_with_mutex(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            mutex_types,
            true,
            method.returns_ref,
            method.returns_cow,
        );
        if method.error_type.is_some() {
            format!(
                "{refs_preamble}{inner_setup}// 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep\n        \
                 // extraction future (a nested archive member, a multi-stage OCR pipeline), and a\n        \
                 // stack overflow aborts the process with SIGBUS instead of raising a catchable panic.\n        \
                 const ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;\n        \
                 let rt = tokio::runtime::Builder::new_multi_thread()\n            \
                     .enable_all()\n            \
                     .thread_stack_size(ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES)\n            \
                     .build()\n            \
                     .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 Ok({result_wrap})"
            )
        } else {
            format!(
                "{refs_preamble}{inner_setup}// 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep\n        \
                 // extraction future (a nested archive member, a multi-stage OCR pipeline), and a\n        \
                 // stack overflow aborts the process with SIGBUS instead of raising a catchable panic.\n        \
                 const ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;\n        \
                 let rt = tokio::runtime::Builder::new_multi_thread()\n            \
                     .enable_all()\n            \
                     .thread_stack_size(ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES)\n            \
                     .build()\n            \
                     .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ {core_call}.await }});\n        \
                 {result_wrap}"
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &method.return_type,
            &format!("{}_async", method.name),
            ASYNC_RETURN_IS_FALLIBLE,
        )
    };
    // See `gen_opaque_instance_method`. The registration side names `{method}_async`, and
    // `module_init` gates that statement on this same `method.cfg`. ~keep
    let method_cfg = method.rust_cfg_attribute();
    format!(
        "{method_cfg}fn {}_async(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    \
         }}",
        method.name
    )
}

/// Generate a Magnus-wrapped struct definition using the shared TypeMapper.
///
/// `delegate_deserialize` must only be true when the caller has already confirmed (via the same
/// "core→binding convertible" set that gates `gen_from_core_to_binding_filtered`) that
/// `From<core::Type> for {{ struct_name }}` will also be emitted this run -- see
/// `crate::codegen::generators::struct_wants_deserialize_delegation`'s doc comment for why this
/// function cannot safely make that call on its own.
///
/// `struct_def.rs.jinja` renders the struct declaration AND its companion impls
/// (`IntoValueFromNative`, `magnus::TryConvert`, `TryConvertOwned`, and -- when
/// `delegate_deserialize` -- a delegating `Deserialize`) as one multi-item block. A `#[cfg(...)]`
/// attribute only binds to the single item immediately following it, so `typ.cfg` is threaded
/// into the template itself (repeated before every item there) and applied again here to the
/// separately-appended delegating-`Deserialize` impl -- gating the returned string at the call
/// site (a single `#[cfg]` prepended to the whole blob) would gate only the struct declaration
/// and leave the impls unconditionally referencing a type the core crate never compiled in. ~keep
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &MagnusMapper,
    module_name: &str,
    core_import: &str,
    generates_default: bool,
    trait_bridges: &[TraitBridgeConfig],
    delegate_deserialize: bool,
) -> String {
    let class_path = format!("{}::{}", module_name, typ.name);

    let filtered_fields: Vec<FieldDef> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| !is_thread_unsafe_field(f, trait_bridges))
        .cloned()
        .collect();

    let fields: Vec<minijinja::Value> = filtered_fields
        .iter()
        .map(|field| {
            let field_type = if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
                mapper.optional(&mapper.map_type(&field.ty))
            } else {
                mapper.map_type(&field.ty)
            };
            // ~keep Delegates to the same predicate the free `fn default_timeout()` emitter
            // gates on (`super::field_wants_default_timeout`), so this attribute can never
            // reference a function the module decided not to generate — see that function's
            // doc comment for the disagreement this closes.
            let wants_default_timeout = super::field_wants_default_timeout(field);
            minijinja::context! {
                name => &field.name,
                field_type => &field_type,
                wants_default_timeout => wants_default_timeout,
            }
        })
        .collect();

    let typ_cfg = typ.cfg.as_deref();
    let mut rendered = crate::backends::magnus::template_env::render(
        "struct_def.rs.jinja",
        minijinja::context! {
            struct_name => &typ.name,
            class_path => &class_path,
            fields => &fields,
            has_default => typ.has_default,
            generates_default => generates_default,
            delegate_deserialize => delegate_deserialize,
            cfg => typ_cfg,
        },
    );
    if delegate_deserialize {
        let deserialize_impl = generators::gen_delegating_deserialize_impl(typ, core_import, "", &[]);
        rendered.push_str(&super::prepend_cfg(typ_cfg, deserialize_impl));
    }
    rendered
}

/// Generate Magnus methods for a struct.
pub(super) fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    _generates_default: bool,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);

    if !typ.fields.is_empty() {
        let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);

        let filtered_fields: Vec<FieldDef> = typ
            .fields
            .iter()
            .filter(|f| !f.binding_excluded)
            .filter(|f| !is_thread_unsafe_field(f, trait_bridges))
            .cloned()
            .collect();

        if !filtered_fields.is_empty() {
            let mut filtered_typ = typ.clone();
            filtered_typ.fields = filtered_fields.clone();
            let config_method = crate::codegen::config_gen::gen_magnus_kwargs_constructor(&filtered_typ, &map_fn);
            impl_builder.add_method(&config_method);
        }
    }

    let mut emitted_field_names: AHashSet<&str> = AHashSet::default();
    for field in binding_fields(&typ.fields) {
        if is_thread_unsafe_field(field, trait_bridges) {
            continue;
        }
        emitted_field_names.insert(field.name.as_str());
        impl_builder.add_method(&gen_field_accessor(field, mapper));
    }

    for method in &typ.methods {
        if !method.is_static {
            // A field accessor and a sync instance method with the same name both mint
            // `fn {name}(&self)` in this one impl block, which rustc rejects with `E0592`.
            // The accessor is emitted first and wins. Async methods are exempt: they are
            // named `{name}_async`, so they cannot collide. ~keep
            if !method.is_async && emitted_field_names.contains(method.name.as_str()) {
                continue;
            }
            if method.is_async {
                impl_builder.add_method(&gen_async_instance_method(
                    method,
                    mapper,
                    typ,
                    opaque_types,
                    core_import,
                ));
            } else {
                impl_builder.add_method(&gen_instance_method(method, mapper, typ, opaque_types, core_import));
            }
        }
    }

    if has_content_string_field(typ) {
        let content_field = binding_fields(&typ.fields).find(|f| f.name == "content").unwrap();
        let is_optional = matches!(&content_field.ty, TypeRef::Optional(_)) || content_field.optional;
        let body = if is_optional {
            "self.content.clone().unwrap_or_default()".to_string()
        } else {
            "self.content.clone()".to_string()
        };
        impl_builder.add_method(&format!(
            "#[allow(clippy::should_implement_trait)]\n    fn to_s(&self) -> String {{\n        {body}\n    }}"
        ));
    }

    impl_builder.build()
}

/// Generate a field accessor method.
fn gen_field_accessor(field: &FieldDef, mapper: &MagnusMapper) -> String {
    let return_type = if field.optional {
        let inner_ty = match &field.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            ty => ty,
        };
        mapper.optional(&mapper.map_type(inner_ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let body = if is_primitive_copy(&field.ty) {
        format!("self.{}", field.name)
    } else {
        format!("self.{}.clone()", field.name)
    };

    let allow_attr = if field.name.starts_with("from_") || field.name.starts_with("to_") {
        "#[allow(clippy::wrong_self_convention)]\n    "
    } else {
        ""
    };

    format!(
        "{allow_attr}fn {}(&self) -> {} {{\n        {}\n    }}",
        field.name, return_type, body
    )
}

/// Check if a type is a Copy type (primitives and unit).
fn is_primitive_copy(ty: &crate::core::ir::TypeRef) -> bool {
    matches!(
        ty,
        crate::core::ir::TypeRef::Primitive(_) | crate::core::ir::TypeRef::Unit
    )
}

/// Generate an instance method binding for a non-opaque struct.
fn gen_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    typ: &TypeDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::codegen::shared;
    let params = function_params(&method.params, &|ty| magnus_param_signature_type(ty, mapper));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let needs_mut_receiver = method.receiver == Some(ReceiverKind::RefMut);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let field_conversions = if needs_mut_receiver {
            generators::gen_lossy_binding_to_core_fields_mut(typ, core_import, false, opaque_types, false, false, &[])
        } else {
            generators::gen_lossy_binding_to_core_fields(typ, core_import, false, opaque_types, false, false, &[])
        };
        let core_call = format!("core_self.{}({})", method.name, call_args);
        let result_wrap = non_opaque_method_result_wrap(method);
        if method.error_type.is_some() {
            format!(
                "{field_conversions}let result = {core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok(result{result_wrap})"
            )
        } else {
            format!("{field_conversions}{core_call}{result_wrap}")
        }
    } else {
        gen_magnus_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };
    let allow_attr = if !can_delegate {
        "#[allow(unused_variables)]\n    "
    } else {
        ""
    };
    let self_recv = if needs_mut_receiver { "&mut self" } else { "&self" };
    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n    "
    } else {
        ""
    };
    // See `gen_opaque_instance_method` for why the gate is re-emitted rather than filtered. ~keep
    let method_cfg = method.rust_cfg_attribute();
    format!(
        "{method_cfg}{trait_allow}{allow_attr}fn {}({self_recv}, {params}) -> {return_annotation} {{\n        \
         {body}\n    }}",
        method.name
    )
}

/// Generate an async instance method binding for Magnus (block on runtime).
fn gen_async_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    typ: &TypeDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::codegen::shared;
    let params = function_params(&method.params, &|ty| magnus_param_signature_type(ty, mapper));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, ASYNC_RETURN_IS_FALLIBLE);

    let can_delegate = !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let field_conversions =
            generators::gen_lossy_binding_to_core_fields(typ, core_import, false, opaque_types, false, false, &[]);
        let result_wrap = non_opaque_method_result_wrap(method);
        if method.error_type.is_some() {
            format!(
                "{field_conversions}// 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep\n        \
                 // extraction future (a nested archive member, a multi-stage OCR pipeline), and a\n        \
                 // stack overflow aborts the process with SIGBUS instead of raising a catchable panic.\n        \
                 const ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;\n        \
                 let rt = tokio::runtime::Builder::new_multi_thread()\n            \
                     .enable_all()\n            \
                     .thread_stack_size(ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES)\n            \
                     .build()\n            \
                     .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ core_self.{name}({call_args}).await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 Ok(result{result_wrap})",
                name = method.name
            )
        } else {
            format!(
                "{field_conversions}// 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep\n        \
                 // extraction future (a nested archive member, a multi-stage OCR pipeline), and a\n        \
                 // stack overflow aborts the process with SIGBUS instead of raising a catchable panic.\n        \
                 const ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;\n        \
                 let rt = tokio::runtime::Builder::new_multi_thread()\n            \
                     .enable_all()\n            \
                     .thread_stack_size(ASYNC_METHOD_RUNTIME_STACK_SIZE_BYTES)\n            \
                     .build()\n            \
                     .map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ core_self.{name}({call_args}).await }});\n        \
                 result{result_wrap}",
                name = method.name
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &method.return_type,
            &format!("{}_async", method.name),
            ASYNC_RETURN_IS_FALLIBLE,
        )
    };
    // See `gen_opaque_instance_method`. The registration side names `{method}_async`, and
    // `module_init` gates that statement on this same `method.cfg`. ~keep
    let method_cfg = method.rust_cfg_attribute();
    format!(
        "{method_cfg}fn {}_async(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    \
         }}",
        method.name
    )
}

mod gen_enum;

pub(super) use gen_enum::{data_enum_variant_constructor_registrations, gen_data_enum_variant_constructors, gen_enum};

/// Generate a From impl for binding → core conversion that excludes thread-unsafe fields.
///
/// Fields whose type references a bridge handle (e.g. `VisitorHandle`) are dropped via
/// `ConversionConfig::exclude_types`, which filters at codegen time. The previous
/// post-processing line filter broke when the IR's `cfg` was stripped for active
/// features, leaving the field present and emitted into the From body.
pub(super) fn gen_from_binding_to_core_filtered(
    typ: &TypeDef,
    core_import: &str,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    if !binding_fields(&typ.fields).any(|field| is_thread_unsafe_field(field, trait_bridges)) {
        return crate::codegen::conversions::gen_from_binding_to_core(typ, core_import);
    }

    let exclude_owned: Vec<String> = trait_bridges
        .iter()
        .filter_map(|bridge| bridge.type_alias.clone())
        .collect();
    let cfg = crate::codegen::conversions::ConversionConfig {
        exclude_types: exclude_owned.as_slice(),
        ..Default::default()
    };
    crate::codegen::conversions::gen_from_binding_to_core_cfg(typ, core_import, &cfg)
}

/// Generate a From impl for core → binding conversion that excludes thread-unsafe fields.
/// Mirrors `gen_from_binding_to_core_filtered` for the opposite direction.
pub(super) fn gen_from_core_to_binding_filtered(
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    if !binding_fields(&typ.fields).any(|field| is_thread_unsafe_field(field, trait_bridges)) {
        return crate::codegen::conversions::gen_from_core_to_binding(typ, core_import, opaque_types);
    }

    let exclude_owned: Vec<String> = trait_bridges
        .iter()
        .filter_map(|bridge| bridge.type_alias.clone())
        .collect();
    let cfg = crate::codegen::conversions::ConversionConfig {
        exclude_types: exclude_owned.as_slice(),
        opaque_types: Some(opaque_types),
        ..Default::default()
    };
    crate::codegen::conversions::gen_from_core_to_binding_cfg(typ, core_import, opaque_types, &cfg)
}

/// Generate a Magnus-specific Default impl that delegates to the core type's Default.
/// This is used for structs with has_default=true to ensure proper defaults are used
/// instead of field-level Default::default() which may not match the core's semantics
/// (e.g., SecurityLimits uses 0 for usize fields but core defaults them to 500MB/100/10K).
pub(super) fn gen_magnus_default_impl(typ: &TypeDef, core_import: &str) -> String {
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    format!(
        "impl Default for {} {{\n    \
         fn default() -> Self {{\n        \
         {core_path}::default().into()\n    \
         }}\n}}\n",
        typ.name
    )
}

/// Generate an explicit Default impl for a binding struct using field-level defaults.
/// This is used when the struct has field-level defaults (e.g., from typed_default)
/// that don't match what the derived Default would produce. Uses the same defaults
/// as the kwargs constructor. Filters out thread-unsafe fields like the struct definition does.
///
/// `types_with_default` is the set of type names known to implement `Default` (via derive or
/// manual `impl`), i.e. every `TypeDef.name` in the surface with `has_default == true`. A field
/// with no default annotation of its own (`typed_default: None`, `default: None`) has no
/// legitimate default value to synthesize unless its own type provides one; the untyped
/// fallback table in `default_value_for_field` cannot see that distinction; it renders
/// `{Type}::default()` for any `Named` field regardless of whether `Type` actually has a
/// `Default` impl. One field having a real default (e.g. a `#[serde(default)]` string) does not
/// make every *other* required field in the same struct defaultable, so this function must bail
/// out of the whole impl rather than emit a call the target type cannot satisfy. ~keep
pub(super) fn gen_struct_default_impl_explicit(
    typ: &TypeDef,
    type_mapper: &dyn Fn(&TypeRef) -> String,
    trait_bridges: &[TraitBridgeConfig],
    types_with_default: &std::collections::HashSet<&str>,
) -> Option<String> {
    let filtered_fields: Vec<FieldDef> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded && !is_thread_unsafe_field(f, trait_bridges))
        .cloned()
        .collect();

    let is_update_struct = typ.name.ends_with("Update");

    let has_non_trivial_default = filtered_fields.iter().any(|field| {
        !matches!(&field.ty, TypeRef::Optional(_)) && (field.typed_default.is_some() || field.default.is_some())
    });

    if !has_non_trivial_default && !is_update_struct {
        return None;
    }

    let has_undefaultable_required_field = filtered_fields.iter().any(|field| {
        if matches!(&field.ty, TypeRef::Optional(_)) || field.optional {
            return false;
        }
        if field.typed_default.is_some() || field.default.is_some() {
            return false;
        }
        matches!(&field.ty, TypeRef::Named(name) if !types_with_default.contains(name.as_str()))
    });

    if has_undefaultable_required_field {
        return None;
    }

    let field_assignments: Vec<String> = filtered_fields
        .iter()
        .map(|field| {
            if matches!(&field.ty, TypeRef::Optional(_)) || field.optional {
                format!("{}: None", field.name)
            } else {
                let binding_type = if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
                    format!("Option<{}>", type_mapper(&field.ty))
                } else {
                    type_mapper(&field.ty)
                };

                let binding_ty = if binding_type == "String" && matches!(&field.ty, TypeRef::Json) {
                    TypeRef::String
                } else if binding_type == "String" {
                    match &field.ty {
                        TypeRef::String => TypeRef::String,
                        _ => field.ty.clone(),
                    }
                } else {
                    field.ty.clone()
                };

                // ~keep Must be the type-aware entry point, not `default_value_for_field`. This
                // impl exists precisely because the struct has field-level defaults that differ
                // from the derived Default, so the context-free fallback would emit
                // `Default::default()` here and defeat the whole purpose: `GridCell.default()`
                // returned row_span 0 while the kwargs constructor returned the real 1.
                let default_val = crate::codegen::config_gen::default_value_for_field_in_type(
                    &FieldDef {
                        ty: binding_ty,
                        ..field.clone()
                    },
                    "rust",
                    typ,
                );
                format!("{}: {}", field.name, default_val)
            }
        })
        .collect();

    Some(format!(
        "impl Default for {} {{\n    fn default() -> Self {{\n        Self {{\n            {},\n        }}\n    }}\n}}\n",
        typ.name,
        field_assignments.join(",\n            ")
    ))
}

#[cfg(test)]
mod tests;
