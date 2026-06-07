use crate::codegen::generators::binding_helpers::{
    apply_return_newtype_unwrap, gen_async_body, gen_call_args, gen_call_args_cfg, gen_call_args_with_let_bindings,
    gen_lossy_binding_to_core_fields, gen_lossy_binding_to_core_fields_mut, gen_named_let_bindings_pub,
    gen_serde_let_bindings, gen_unimplemented_body, has_named_params, is_simple_non_opaque_param,
    wrap_return_with_mutex_mapped,
};
use crate::codegen::generators::{AdapterBodies, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::{function_params, function_sig_defaults, partition_methods};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;

/// Returns true when `name` matches a known trait method that would trigger
/// `clippy::should_implement_trait`.
pub fn is_trait_method_name(name: &str) -> bool {
    crate::codegen::generators::TRAIT_METHOD_NAMES.contains(&name)
}

/// Generate a constructor method.
pub fn gen_constructor(typ: &TypeDef, mapper: &dyn TypeMapper, cfg: &RustBindingConfig) -> String {
    gen_constructor_with_renames(typ, mapper, cfg, None)
}

/// Like `gen_constructor` but with field renames for keyword escaping.
pub fn gen_constructor_with_renames(
    typ: &TypeDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    field_renames: Option<&std::collections::HashMap<String, String>>,
) -> String {
    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);

    // For types with has_default, generate optional kwargs-style constructor
    let (param_list, sig_defaults, assignments) = if typ.has_default {
        crate::codegen::shared::config_constructor_parts_with_renames_and_cfg_restore(
            &typ.fields,
            &map_fn,
            cfg.option_duration_on_defaults,
            field_renames,
            cfg.never_skip_cfg_field_names,
        )
    } else {
        crate::codegen::shared::constructor_parts_with_renames_and_cfg_restore(
            &typ.fields,
            &map_fn,
            field_renames,
            cfg.never_skip_cfg_field_names,
        )
    };

    crate::codegen::template_env::render(
        "generators/methods/constructor.jinja",
        minijinja::context! {
            has_too_many_args => typ.fields.len() > 7,
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_defaults => sig_defaults,
            signature_suffix => cfg.signature_suffix,
            constructor_attr => cfg.constructor_attr,
            param_list => param_list,
            assignments => assignments,
        },
    )
}

/// Generate an instance method.
///
/// When `is_opaque` is true, generates delegation to `self.inner` via Arc clone
/// instead of converting self to core type.
///
/// `opaque_types` is the set of opaque type names, used for correct return wrapping.
/// `mutex_types` is the subset of opaque types whose `inner` field is `Arc<Mutex<T>>`;
/// method dispatch uses `.lock().unwrap()` for these types.
#[allow(clippy::too_many_arguments)]
pub fn gen_method(
    method: &MethodDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    typ: &TypeDef,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    adapter_bodies: &AdapterBodies,
) -> String {
    let type_name = &typ.name;
    // Use the full rust_path (with hyphens replaced by underscores) for core type references
    let core_type_path = typ.rust_path.replace('-', "_");

    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
    let params = function_params(&method.params, &map_fn);
    let return_type = mapper.map_type(&method.return_type);
    let ret = mapper.wrap_return(&return_type, method.error_type.is_some());

    let core_import = cfg.core_import;

    // When non-opaque Named params have is_ref=true, or Vec<String> params have is_ref=true,
    // we need let bindings so the converted/intermediate value outlives the borrow.
    // Use has_named_params which covers both Named types and Vec<String> with is_ref=true.
    let has_ref_named_params = has_named_params(&method.params, opaque_types);
    let (call_args, ref_let_bindings) = if has_ref_named_params {
        (
            gen_call_args_with_let_bindings(&method.params, opaque_types),
            gen_named_let_bindings_pub(&method.params, opaque_types, core_import),
        )
    } else if cfg.cast_uints_to_i32 || cfg.cast_large_ints_to_f64 {
        // Use cast-aware call args for backends that remap numeric types (e.g. extendr).
        (
            gen_call_args_cfg(
                &method.params,
                opaque_types,
                cfg.cast_uints_to_i32,
                cfg.cast_large_ints_to_f64,
            ),
            String::new(),
        )
    } else {
        (gen_call_args(&method.params, opaque_types), String::new())
    };

    let is_owned_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned));
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));

    // Detect non-opaque RefMut methods that can use the functional clone-mutate-return pattern.
    // These cannot use &mut self in frozen PyO3 classes or immutable WASM structs, so instead
    // we generate: clone self to core, apply mutation, convert back to Self.
    // Conditions: non-opaque, RefMut receiver, no trait source (trait methods need special handling),
    // all params delegatable (Named types are allowed — gen_call_args handles them via .into()),
    // and not sanitized.
    let is_functional_ref_mut = !is_opaque
        && is_ref_mut_receiver
        && !method.sanitized
        && method.trait_source.is_none()
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types));

    // Methods from trait impls can't be called on Arc<dyn Trait> through deref.
    // Skip these unless there's an adapter body that can handle them.
    let is_trait_method = method.trait_source.is_some();

    // Whether this opaque type uses Arc<Mutex<T>> for interior mutability.
    let self_needs_mutex = is_opaque && mutex_types.contains(type_name.as_str());

    // Auto-delegate opaque methods: unwrap Arc for params, wrap Arc for returns.
    // Owned receivers require the type to implement Clone (builder pattern).
    // RefMut receivers normally can't be delegated on Arc<T>, but Arc<Mutex<T>> allows
    // &mut T via .lock().unwrap(), so mutex types CAN delegate RefMut methods.
    // Trait methods can't be delegated on opaque types (Arc deref doesn't expose trait methods).
    // Async methods are allowed — gen_async_body handles them below.
    let opaque_can_delegate = is_opaque
        && !method.sanitized
        && (!is_ref_mut_receiver || self_needs_mutex)
        && !is_trait_method
        && (!is_owned_receiver || typ.is_clone)
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_opaque_delegatable_type(&p.ty))
        && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type);

    // Build the core call expression: opaque types delegate to self.inner directly,
    // non-opaque types convert self to core type first.
    // For mutex types, acquire the lock before calling the method.
    let make_core_call = |method_name: &str| -> String {
        if is_opaque {
            if is_owned_receiver {
                // Owned receiver: clone out of Arc/Mutex to get an owned value.
                // For Mutex types, lock first then clone the inner value.
                if self_needs_mutex {
                    format!("self.inner.lock().unwrap().clone().{method_name}({call_args})")
                } else {
                    format!("(*self.inner).clone().{method_name}({call_args})")
                }
            } else if self_needs_mutex {
                // Mutex type: lock to get &mut T (works for both &self and &mut self methods).
                format!("self.inner.lock().unwrap().{method_name}({call_args})")
            } else {
                format!("self.inner.{method_name}({call_args})")
            }
        } else {
            format!("{core_type_path}::from(self.clone()).{method_name}({call_args})")
        }
    };

    // For async opaque methods, we clone the Arc before moving into the future.
    // For mutex types, the cloned Arc<Mutex<T>> is locked inside the async block.
    let make_async_core_call = |method_name: &str| -> String {
        if is_opaque {
            if self_needs_mutex {
                format!("inner.lock().unwrap().{method_name}({call_args})")
            } else {
                format!("inner.{method_name}({call_args})")
            }
        } else {
            format!("{core_type_path}::from(self.clone()).{method_name}({call_args})")
        }
    };

    // Generate the body: convert self to core type, call method, convert result back
    //
    // For opaque types, wrap the return value appropriately:
    //   - Named(self) → Self { inner: Arc::new(result) }
    //   - Named(other) → OtherType::from(result)
    //   - primitives/String/Vec/Unit → pass through
    let result_expr = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
    let async_result_wrap = if is_opaque {
        wrap_return_with_mutex_mapped(
            &result_expr,
            &method.return_type,
            type_name,
            opaque_types,
            mutex_types,
            is_opaque,
            method.returns_ref,
            method.returns_cow,
            mapper,
        )
    } else {
        // For non-opaque types, only use From conversion if the return type is simple
        // enough. Named return types may not have a From impl.
        match &method.return_type {
            TypeRef::Named(_) | TypeRef::Json => format!("{result_expr}.into()"),
            _ => result_expr.clone(),
        }
    };

    // Explicit adapter bodies always take precedence over auto-generated delegation —
    // they are user overrides that capture intentional non-default behavior.
    let adapter_key_inner = format!("{}.{}", type_name, method.name);
    let adapter_override = adapter_bodies.get(&adapter_key_inner).cloned();

    let body = if let Some(adapter_body) = adapter_override {
        adapter_body
    } else if !opaque_can_delegate {
        if cfg.has_serde
            && is_opaque
            && !method.sanitized
            && !is_trait_method
            && has_named_params(&method.params, opaque_types)
            && method.error_type.is_some()
            && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type)
        {
            // Serde-based param conversion for opaque methods with non-opaque Named params.
            // NOTE: Only executed when has_serde=true, ensuring serde_json calls are gated.
            let err_conv = match cfg.async_pattern {
                AsyncPattern::Pyo3FutureIntoPy => {
                    ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                }
                AsyncPattern::NapiNativeAsync => {
                    ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                }
                AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                AsyncPattern::TokioBlockOn => {
                    ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))"
                }
                _ => ".map_err(|e| e.to_string())",
            };
            let serde_bindings =
                gen_serde_let_bindings(&method.params, opaque_types, cfg.core_import, err_conv, "        ");
            let serde_call_args = gen_call_args_with_let_bindings(&method.params, opaque_types);
            let core_call = if self_needs_mutex {
                format!("self.inner.lock().unwrap().{}({serde_call_args})", method.name)
            } else {
                format!("self.inner.{}({serde_call_args})", method.name)
            };
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{serde_bindings}{core_call}{err_conv}?;\n        Ok(())")
            } else {
                let wrap = wrap_return_with_mutex_mapped(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    mutex_types,
                    is_opaque,
                    method.returns_ref,
                    method.returns_cow,
                    mapper,
                );
                format!("{serde_bindings}let result = {core_call}{err_conv}?;\n        Ok({wrap})")
            }
        } else if is_functional_ref_mut {
            // Functional clone-mutate-return pattern for non-opaque RefMut methods.
            // PyO3 frozen classes and WASM structs don't support &mut self, so instead:
            //   1. Convert binding self to a mutable core type.
            //   2. Call the mutating core method (which changes core_self in place).
            //   3. Convert the mutated core type back to the binding type and return Self.
            //
            // The generated signature uses &self -> Self (or -> Result<Self, E> if fallible),
            // making the method work correctly with immutable binding wrappers.
            let field_conversions = gen_lossy_binding_to_core_fields_mut(
                typ,
                cfg.core_import,
                cfg.option_duration_on_defaults,
                opaque_types,
                cfg.cast_uints_to_i32,
                cfg.cast_large_ints_to_f64,
                cfg.lossy_skip_types,
            );
            let core_call = format!("core_self.{}({call_args})", method.name);
            if method.error_type.is_some() {
                let err_conv = match cfg.async_pattern {
                    AsyncPattern::Pyo3FutureIntoPy => {
                        ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                    }
                    AsyncPattern::NapiNativeAsync => {
                        ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                    }
                    AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                    AsyncPattern::TokioBlockOn => {
                        ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))"
                    }
                    _ => ".map_err(|e| e.to_string())",
                };
                format!("{field_conversions}{core_call}{err_conv}?;\n        Ok(core_self.into())")
            } else {
                format!("{field_conversions}{core_call};\n        core_self.into()")
            }
        } else if !is_opaque
            && !method.sanitized
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && is_simple_non_opaque_param(&p.ty))
            && crate::codegen::shared::is_delegatable_return(&method.return_type)
        {
            // Non-opaque delegation: construct core type field-by-field, call method, convert back.
            // Sanitized fields use Default::default() (lossy but functional for builder pattern).
            let is_ref_mut = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));
            let field_conversions = if is_ref_mut {
                gen_lossy_binding_to_core_fields_mut(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            } else {
                gen_lossy_binding_to_core_fields(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            };
            let core_call = format!("core_self.{}({call_args})", method.name);
            let newtype_suffix = if method.return_newtype_wrapper.is_some() {
                ".0"
            } else {
                ""
            };
            let result_wrap = match &method.return_type {
                // When returns_cow=true the core returns Cow<'_, T>: call .into_owned() to
                // obtain an owned T before the binding→core From conversion.
                // When returns_ref=true (or &T / Cow<'_, T> via the old flag), same treatment.
                TypeRef::Named(n) if n == type_name && (method.returns_cow || method.returns_ref) => {
                    ".into_owned().into()".to_string()
                }
                TypeRef::Named(_) if method.returns_cow || method.returns_ref => ".into_owned().into()".to_string(),
                TypeRef::Named(n) if n == type_name => ".into()".to_string(),
                TypeRef::Named(_) => ".into()".to_string(),
                TypeRef::String => {
                    if method.returns_ref {
                        ".to_owned()".to_string()
                    } else {
                        String::new()
                    }
                }
                TypeRef::Path => {
                    if method.returns_ref {
                        ".to_owned()".to_string()
                    } else {
                        ".to_string_lossy().to_string()".to_string()
                    }
                }
                // Bytes: binding uses Vec<u8>. Always use .to_vec() which works for both
                // &Bytes and owned Bytes (avoids &Bytes→Vec<u8> From trait issues).
                TypeRef::Bytes => ".to_vec()".to_string(),
                // Optional<Named>: when core returns Option<&T>, need .map(|v| v.clone().into())
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                    if method.returns_ref {
                        ".map(|v| v.clone().into())".to_string()
                    } else {
                        ".map(Into::into)".to_string()
                    }
                }
                // Optional<String>: when core returns Option<&str>, need .map(|v| v.to_owned())
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Bytes) => {
                    if method.returns_ref {
                        ".map(|v| v.to_owned())".to_string()
                    } else {
                        String::new()
                    }
                }
                // Primitive return: cast when the binding uses a different numeric type.
                // R maps u8/u16/u32/i8/i16 → i32 and u64/i64/usize/isize/f32 → f64.
                TypeRef::Primitive(p) => {
                    use crate::codegen::conversions::helpers::{needs_f64_cast, needs_i32_cast};
                    if cfg.cast_uints_to_i32 && needs_i32_cast(p) {
                        " as i32".to_string()
                    } else if cfg.cast_large_ints_to_f64 && needs_f64_cast(p) {
                        " as f64".to_string()
                    } else {
                        String::new()
                    }
                }
                // Optional<Primitive>: cast inside map when the binding uses a different type.
                TypeRef::Optional(inner) => {
                    if let TypeRef::Primitive(p) = inner.as_ref() {
                        use crate::codegen::conversions::helpers::{needs_f64_cast, needs_i32_cast};
                        if cfg.cast_uints_to_i32 && needs_i32_cast(p) {
                            ".map(|v| v as i32)".to_string()
                        } else if cfg.cast_large_ints_to_f64 && needs_f64_cast(p) {
                            ".map(|v| v as f64)".to_string()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                }
                // Map: when core returns &BTreeMap (returns_ref=true), the binding map type
                // (e.g. HashMap) may differ. Collect via into_iter to coerce to the target type.
                TypeRef::Map(_, _) => {
                    if method.returns_ref {
                        ".iter().map(|(k, v)| (k.clone(), v.clone())).collect()".to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            };
            if method.error_type.is_some() {
                let err_conv = match cfg.async_pattern {
                    AsyncPattern::Pyo3FutureIntoPy => {
                        ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                    }
                    AsyncPattern::NapiNativeAsync => {
                        ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                    }
                    AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                    _ => ".map_err(|e| e.to_string())",
                };
                format!(
                    "{field_conversions}let result = {core_call}{err_conv}?;\n        Ok(result{newtype_suffix}{result_wrap})"
                )
            } else {
                format!("{field_conversions}{core_call}{newtype_suffix}{result_wrap}")
            }
        } else if is_opaque
            && !method.sanitized
            && (!is_ref_mut_receiver || self_needs_mutex)
            && (!is_owned_receiver || typ.is_clone)
            && method.error_type.is_none()
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && crate::codegen::shared::is_opaque_delegatable_type(&p.ty))
            && matches!(&method.return_type, TypeRef::Named(n) if n == type_name)
        {
            // Builder pattern for opaque types: method returns Self without error type.
            // Delegate to core method and wrap result back in Self { inner: Arc::new(...) }.
            let core_call = if is_owned_receiver {
                if self_needs_mutex {
                    format!("self.inner.lock().unwrap().clone().{}({call_args})", method.name)
                } else {
                    format!("(*self.inner).clone().{}({call_args})", method.name)
                }
            } else if self_needs_mutex {
                format!("self.inner.lock().unwrap().{}({call_args})", method.name)
            } else {
                format!("self.inner.{}({call_args})", method.name)
            };
            let unwrapped = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            let arc_expr = if self_needs_mutex {
                format!("Arc::new(std::sync::Mutex::new({unwrapped}))")
            } else {
                format!("Arc::new({unwrapped})")
            };
            format!("Self {{ inner: {arc_expr} }}")
        } else if !is_opaque
            && !method.sanitized
            && !is_ref_mut_receiver
            && (!is_owned_receiver || typ.is_clone)
            && method.error_type.is_none()
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && is_simple_non_opaque_param(&p.ty))
            && matches!(&method.return_type, TypeRef::Named(n) if n == type_name)
        {
            // Builder pattern for non-opaque types: method returns Self without error type.
            // Construct core type field-by-field, call method, convert result back via .into().
            let is_ref_mut = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));
            let field_conversions = if is_ref_mut {
                gen_lossy_binding_to_core_fields_mut(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            } else {
                gen_lossy_binding_to_core_fields(
                    typ,
                    cfg.core_import,
                    cfg.option_duration_on_defaults,
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                    cfg.lossy_skip_types,
                )
            };
            let core_call = format!("core_self.{}({call_args})", method.name);
            let newtype_suffix = if method.return_newtype_wrapper.is_some() {
                ".0"
            } else {
                ""
            };
            let result_wrap = if method.returns_cow || method.returns_ref {
                ".into_owned().into()"
            } else {
                ".into()"
            };
            format!("{field_conversions}{core_call}{newtype_suffix}{result_wrap}")
        } else {
            gen_unimplemented_body(
                &method.return_type,
                &format!("{type_name}.{}", method.name),
                method.error_type.is_some(),
                cfg,
                &method.params,
                opaque_types,
            )
        }
    } else if method.is_async {
        let inner_clone_line = if is_opaque {
            "let inner = self.inner.clone();\n        "
        } else {
            ""
        };
        let core_call_str = make_async_core_call(&method.name);
        gen_async_body(
            &core_call_str,
            cfg,
            method.error_type.is_some(),
            &async_result_wrap,
            is_opaque,
            inner_clone_line,
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = make_core_call(&method.name);
        if method.error_type.is_some() {
            // Backend-specific error conversion
            let err_conv = match cfg.async_pattern {
                AsyncPattern::Pyo3FutureIntoPy => {
                    ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                }
                AsyncPattern::NapiNativeAsync => {
                    ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                }
                AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                _ => ".map_err(|e| e.to_string())",
            };
            if is_opaque {
                if matches!(method.return_type, TypeRef::Unit) {
                    // Unit return: avoid let_unit_value by not binding the result
                    format!("{core_call}{err_conv}?;\n        Ok(())")
                } else {
                    let wrap = wrap_return_with_mutex_mapped(
                        &result_expr,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        mutex_types,
                        is_opaque,
                        method.returns_ref,
                        method.returns_cow,
                        mapper,
                    );
                    format!("let result = {core_call}{err_conv}?;\n        Ok({wrap})")
                }
            } else {
                format!("{core_call}{err_conv}")
            }
        } else if is_opaque {
            let unwrapped_call = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            wrap_return_with_mutex_mapped(
                &unwrapped_call,
                &method.return_type,
                type_name,
                opaque_types,
                mutex_types,
                is_opaque,
                method.returns_ref,
                method.returns_cow,
                mapper,
            )
        } else {
            core_call
        }
    };
    let adapter_key = format!("{}.{}", type_name, method.name);
    let has_adapter = adapter_bodies.contains_key(&adapter_key);

    // Prepend let bindings for non-opaque Named ref params (needed for borrow lifetime).
    // Skip when an adapter body is used: the adapter body is self-contained and already
    // includes its own parameter conversions (via core_let_bindings). Prepending the
    // normal {name}_core bindings would produce a duplicate .into() call on a moved value
    // (E0382 use of moved value).
    let body = if ref_let_bindings.is_empty() || has_adapter {
        body
    } else {
        format!("{ref_let_bindings}{body}")
    };

    let needs_py = method.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;

    // When an async PyO3 method could not be auto-delegated (e.g. sanitized params,
    // non-delegatable return type, or trait methods), the body was computed for a
    // synchronous context but the generated signature will be
    // `PyResult<Bound<'py, PyAny>>`. Return `Err` directly — wrapping in
    // `future_into_py` would cause E0283 because the async block only returns `Err`
    // and Rust cannot infer the generic `T` parameter.
    let body = if needs_py && !opaque_can_delegate && !has_adapter {
        let err_msg = format!("Not implemented: {type_name}.{}", method.name);
        // Suppress unused parameter warnings — params are not used in the stub body.
        let suppress = if method.params.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
            if names.len() == 1 {
                format!("let _ = {};\n        ", names[0])
            } else {
                format!("let _ = ({});\n        ", names.join(", "))
            }
        };
        format!("{suppress}Err(pyo3::exceptions::PyNotImplementedError::new_err(\"{err_msg}\"))")
    } else {
        body
    };
    let self_param = match (needs_py, params.is_empty()) {
        (true, true) => "&self, py: Python<'py>",
        (true, false) => "&self, py: Python<'py>, ",
        (false, true) => "&self",
        (false, false) => "&self, ",
    };

    // For async PyO3 methods, override return type to PyResult<Bound<'py, PyAny>>
    // and add the 'py lifetime generic on the method name.
    // For functional RefMut methods, override to Self (or Result<Self, E>) because the
    // generated body clones self, applies the mutation, and returns the updated value.
    let ret = if needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else if is_functional_ref_mut {
        mapper.wrap_return("Self", method.error_type.is_some())
    } else {
        ret
    };
    let method_lifetime = if needs_py { "<'py>" } else { "" };

    // Wrap long signature if necessary
    let (sig_start, sig_params, sig_end) = if self_param.len() + params.len() > 100 {
        let wrapped_params = method
            .params
            .iter()
            .map(|p| {
                let ty = if p.optional {
                    format!("Option<{}>", mapper.map_type(&p.ty))
                } else {
                    mapper.map_type(&p.ty)
                };
                format!("{}: {}", p.name, ty)
            })
            .collect::<Vec<_>>()
            .join(",\n        ");
        let py_param = if needs_py { "\n        py: Python<'py>," } else { "" };
        (
            format!(
                "pub fn {}{method_lifetime}(\n        &self,{}\n        ",
                method.name, py_param
            ),
            wrapped_params,
            "\n    ) -> ".to_string(),
        )
    } else {
        (
            format!("pub fn {}{method_lifetime}({}", method.name, self_param),
            params,
            ") -> ".to_string(),
        )
    };

    let total_params = method.params.len() + 1 + if needs_py { 1 } else { 0 };
    let sig_defaults = if cfg.needs_signature {
        function_sig_defaults(&method.params)
    } else {
        String::new()
    };

    crate::codegen::template_env::render(
        "generators/methods/method_signature.jinja",
        minijinja::context! {
            has_too_many_arguments => total_params > 7,
            has_missing_errors_doc => method.error_type.is_some(),
            has_should_implement_trait => is_trait_method_name(&method.name),
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_defaults => sig_defaults,
            signature_suffix => cfg.signature_suffix,
            sig_start => sig_start,
            sig_params => sig_params,
            sig_end => sig_end,
            ret => ret,
            body => body,
        },
    )
}

/// Generate a static method.
pub fn gen_static_method(
    method: &MethodDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    typ: &TypeDef,
    adapter_bodies: &AdapterBodies,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
) -> String {
    let type_name = &typ.name;
    // Use the full rust_path (with hyphens replaced by underscores) for core type references
    let core_type_path = typ.rust_path.replace('-', "_");
    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
    let params = function_params(&method.params, &map_fn);
    let return_type = mapper.map_type(&method.return_type);
    let ret = mapper.wrap_return(&return_type, method.error_type.is_some());

    let core_import = cfg.core_import;

    // Use let bindings when any non-opaque Named or Vec<Named> params exist.
    // This includes Vec<Named> without is_ref=true, which need element conversion.
    let use_let_bindings = has_named_params(&method.params, opaque_types);
    let (call_args, ref_let_bindings) = if use_let_bindings {
        (
            gen_call_args_with_let_bindings(&method.params, opaque_types),
            gen_named_let_bindings_pub(&method.params, opaque_types, core_import),
        )
    } else {
        (gen_call_args(&method.params, opaque_types), String::new())
    };

    // For lifetime-parameterized types, emit let bindings for String→Cow and Map→BTreeMap conversions.
    // These are needed when static methods of lifetime types receive binding-level String/HashMap
    // but the core methods expect Cow<'_, str> and BTreeMap (owned for binding wrapper context).
    let lifetime_bindings = if typ.has_lifetime_params {
        let mut bindings = String::new();
        for p in &method.params {
            match &p.ty {
                TypeRef::String => {
                    if p.optional {
                        bindings.push_str(&format!("let {}_converted = {}.map(Into::into);\n    ", p.name, p.name));
                    } else {
                        bindings.push_str(&format!(
                            "let {}_converted: std::borrow::Cow<'_, str> = {}.into();\n    ",
                            p.name, p.name
                        ));
                    }
                }
                TypeRef::Map(_, _) => {
                    // Map types: convert HashMap to BTreeMap (owned, since wrapper context has no lifetime)
                    bindings.push_str(&format!("let {}_converted = {}.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<std::collections::BTreeMap<_, _>>();\n    ", p.name, p.name));
                }
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                    bindings.push_str(&format!("let {}_converted = {}.map(Into::into);\n    ", p.name, p.name));
                }
                _ => {}
            }
        }
        bindings
    } else {
        String::new()
    };

    // Adjust call_args to use converted variables when lifetime bindings were emitted.
    // Special case: for borrowed methods of lifetime types, we need to call the owned variant instead
    // because the wrapper function can't provide the lifetime required by the borrowed variant.
    let is_borrowed_to_owned = method.name.contains("borrowed_attributes");
    let (call_args, method_name_override) = if !lifetime_bindings.is_empty() {
        let mut adjusted = call_args.clone();
        for p in &method.params {
            match &p.ty {
                TypeRef::Map(_, _) => {
                    // The original call arg for is_ref Map params is `&{name}`. When switching
                    // from borrowed→owned, the owned method takes an owned BTreeMap, so the `&`
                    // must be dropped. Replace `&{name}` → `{name}_converted` in that case,
                    // and `{name}` → `{name}_converted` when no ref prefix was generated.
                    if is_borrowed_to_owned && p.is_ref {
                        adjusted = adjusted.replace(&format!("&{}", p.name), &format!("{}_converted", p.name));
                    } else {
                        adjusted = adjusted.replace(&p.name.to_string(), &format!("{}_converted", p.name));
                    }
                }
                TypeRef::String => {
                    adjusted = adjusted.replace(&p.name.to_string(), &format!("{}_converted", p.name));
                }
                TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                    adjusted = adjusted.replace(&p.name.to_string(), &format!("{}_converted", p.name));
                }
                _ => {}
            }
        }
        // If calling a with_borrowed_* method, switch to with_owned_* since we have owned data
        let override_name = if is_borrowed_to_owned {
            Some(method.name.replace("borrowed", "owned"))
        } else {
            None
        };
        (adjusted, override_name)
    } else {
        (call_args, None)
    };

    // Update method name if needed (borrowed → owned for wrapper functions)
    let actual_method_name = method_name_override.as_deref().unwrap_or(&method.name);

    let can_delegate = crate::codegen::shared::can_auto_delegate(method, opaque_types);

    // Explicit adapter bodies always take precedence over auto-generated delegation —
    // they are user overrides that capture intentional non-default behavior.
    let adapter_key = format!("{}.{}", type_name, method.name);
    let adapter_override = adapter_bodies.get(&adapter_key).cloned();

    let body = if let Some(adapter_body) = adapter_override {
        adapter_body
    } else if !can_delegate {
        gen_unimplemented_body(
            &method.return_type,
            &format!("{type_name}::{}", method.name),
            method.error_type.is_some(),
            cfg,
            &method.params,
            opaque_types,
        )
    } else if method.is_async {
        let core_call = format!("{core_type_path}::{}({call_args})", actual_method_name);
        let return_wrap = format!("{return_type}::from(result)");
        gen_async_body(
            &core_call,
            cfg,
            method.error_type.is_some(),
            &return_wrap,
            false,
            "",
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_type_path}::{}({call_args})", actual_method_name);
        if method.error_type.is_some() {
            // Backend-specific error conversion
            let err_conv = match cfg.async_pattern {
                AsyncPattern::Pyo3FutureIntoPy => {
                    ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                }
                AsyncPattern::NapiNativeAsync => {
                    ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                }
                AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                _ => ".map_err(|e| e.to_string())",
            };
            // Wrap the Ok value if the return type needs conversion (e.g. PathBuf→String)
            let val_expr = apply_return_newtype_unwrap("val", &method.return_newtype_wrapper);
            let wrapped = wrap_return_with_mutex_mapped(
                &val_expr,
                &method.return_type,
                type_name,
                opaque_types,
                mutex_types,
                typ.is_opaque,
                method.returns_ref,
                method.returns_cow,
                mapper,
            );
            if wrapped == val_expr {
                format!("{core_call}{err_conv}")
            } else if wrapped == format!("{val_expr}.into()") {
                format!("{core_call}.map(Into::into){err_conv}")
            } else if let Some(type_path) = wrapped.strip_suffix(&format!("::from({val_expr})")) {
                format!("{core_call}.map({type_path}::from){err_conv}")
            } else {
                format!("{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            // Wrap return value for non-error case too (e.g. PathBuf→String)
            let unwrapped_call = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            wrap_return_with_mutex_mapped(
                &unwrapped_call,
                &method.return_type,
                type_name,
                opaque_types,
                mutex_types,
                typ.is_opaque,
                method.returns_ref,
                method.returns_cow,
                mapper,
            )
        }
    };
    // Prepend let bindings for non-opaque Named ref params and lifetime type conversions
    let body = if ref_let_bindings.is_empty() && lifetime_bindings.is_empty() {
        body
    } else {
        format!("{ref_let_bindings}{lifetime_bindings}{body}")
    };

    let static_needs_py = method.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;

    // For async PyO3 static methods, override return type and add lifetime generic.
    let ret = if static_needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        ret
    };
    let method_lifetime = if static_needs_py { "<'py>" } else { "" };

    // Wrap long signature if necessary
    let (sig_start, sig_params, sig_end) = if params.len() > 100 {
        let wrapped_params = method
            .params
            .iter()
            .map(|p| {
                let ty = if p.optional {
                    format!("Option<{}>", mapper.map_type(&p.ty))
                } else {
                    mapper.map_type(&p.ty)
                };
                format!("{}: {}", p.name, ty)
            })
            .collect::<Vec<_>>()
            .join(",\n        ");
        // For async PyO3, add py parameter
        if static_needs_py {
            (
                format!("pub fn {}{method_lifetime}(py: Python<'py>,\n        ", method.name),
                wrapped_params,
                "\n    ) -> ".to_string(),
            )
        } else {
            (
                format!("pub fn {}(\n        ", method.name),
                wrapped_params,
                "\n    ) -> ".to_string(),
            )
        }
    } else if static_needs_py {
        (
            format!("pub fn {}{method_lifetime}(py: Python<'py>, ", method.name),
            params,
            ") -> ".to_string(),
        )
    } else {
        (format!("pub fn {}(", method.name), params, ") -> ".to_string())
    };

    let total_params = method.params.len() + if static_needs_py { 1 } else { 0 };
    let sig_defaults = if cfg.needs_signature {
        function_sig_defaults(&method.params)
    } else {
        String::new()
    };
    let static_attr_str = if let Some(attr) = cfg.static_attr {
        format!("#[{attr}]")
    } else {
        String::new()
    };

    // For static methods, we need a variant of method_signature template
    // that handles static attributes. For now, build manually but use render for main block
    let mut out = String::with_capacity(1024);
    if total_params > 7 {
        out.push_str("    #[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() {
        out.push_str("    #[allow(clippy::missing_errors_doc)]\n");
    }
    if is_trait_method_name(&method.name) {
        out.push_str("    #[allow(clippy::should_implement_trait)]\n");
    }
    if !static_attr_str.is_empty() {
        out.push_str(&crate::codegen::template_env::render(
            "generators/methods/static_attr.jinja",
            minijinja::context! {
                static_attr_str => static_attr_str,
            },
        ));
    }
    if cfg.needs_signature {
        out.push_str(&crate::codegen::template_env::render(
            "generators/methods/signature_attr.jinja",
            minijinja::context! {
                signature_prefix => &cfg.signature_prefix,
                sig_defaults => sig_defaults,
                signature_suffix => &cfg.signature_suffix,
            },
        ));
    }
    out.push_str(&crate::codegen::template_env::render(
        "generators/methods/method_body.jinja",
        minijinja::context! {
            sig_start => sig_start,
            sig_params => sig_params,
            sig_end => sig_end,
            ret => ret,
            body => body,
        },
    ));
    out
}

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
    // Compute effective (non-sanitized or adapter-overridden) method counts for the early-return
    // check. Sanitized methods without adapters are skipped in the loops below, so they do not
    // contribute real content to the impl block.
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

    // Constructor — suppressed when the backend handles construction via a separate free
    // function (e.g. extendr kwargs constructor), when there are no fields, or when the
    // type already provides an explicit static `new()` method (which will be emitted as
    // `#[staticmethod] pub fn new(...)` and would conflict with a `#[new]` constructor).
    let has_explicit_static_new = typ.methods.iter().any(|m| m.is_static && m.name == "new");
    if !typ.fields.is_empty() && !cfg.skip_impl_constructor && !has_explicit_static_new {
        out.push_str(&gen_constructor_with_renames(typ, mapper, cfg, field_renames));
        out.push_str("\n\n");
    }

    // Instance methods
    let empty_mutex_types: AHashSet<String> = AHashSet::new();
    for m in &instance {
        // Skip sanitized methods that have no adapter override — they cannot be delegated
        // and emitting an unimplemented stub pollutes the public API with dead placeholders.
        // Adapter bodies are explicit overrides and always take precedence.
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

    // Static methods
    for m in &statics {
        // Skip sanitized static methods that have no adapter override.
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

    // Trim trailing newlines inside impl block
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
    // Compute effective (non-sanitized or adapter-overridden) method counts.
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

    // Instance methods — delegate to self.inner
    for m in &instance {
        // Skip sanitized methods that have no adapter override — they cannot be delegated
        // and emitting an unimplemented stub pollutes the public API with dead placeholders.
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

    // Static methods
    for m in &statics {
        // Skip sanitized static methods that have no adapter override.
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
