use super::trait_names::is_trait_method_name;
use crate::codegen::generators::binding_helpers::{
    apply_return_newtype_unwrap, gen_async_body, gen_call_args, gen_call_args_cfg, gen_call_args_with_let_bindings,
    gen_lossy_binding_to_core_fields, gen_lossy_binding_to_core_fields_mut, gen_named_let_bindings_pub,
    gen_serde_let_bindings, gen_unimplemented_body, has_named_params, is_simple_non_opaque_param,
    wrap_return_with_mutex_mapped,
};
use crate::codegen::generators::{AdapterBodies, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::{function_params, function_sig_defaults};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;

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
                // Optional<Vec<Named>>: per-element From<core> conversion.
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Primitive(p) => {
                        use crate::codegen::conversions::helpers::{needs_f64_cast, needs_i32_cast};
                        if cfg.cast_uints_to_i32 && needs_i32_cast(p) {
                            ".map(|v| v as i32)".to_string()
                        } else if cfg.cast_large_ints_to_f64 && needs_f64_cast(p) {
                            ".map(|v| v as f64)".to_string()
                        } else {
                            String::new()
                        }
                    }
                    // Option<Vec<Named>>: convert each inner element through Into::into.
                    TypeRef::Vec(vec_inner) if matches!(vec_inner.as_ref(), TypeRef::Named(_)) => {
                        if method.returns_ref {
                            ".as_ref().map(|v| v.iter().map(|x| x.clone().into()).collect())".to_string()
                        } else {
                            ".map(|v| v.into_iter().map(Into::into).collect())".to_string()
                        }
                    }
                    _ => String::new(),
                },
                // Map: when core returns &BTreeMap (returns_ref=true), the binding map type
                // (e.g. HashMap) may differ. Collect via into_iter to coerce to the target type.
                TypeRef::Map(_, _) => {
                    if method.returns_ref {
                        ".iter().map(|(k, v)| (k.clone(), v.clone())).collect()".to_string()
                    } else {
                        String::new()
                    }
                }
                // Vec<Named>: core returns Vec<core::T> but binding expects Vec<wrapper::T>.
                // Apply the binding's From<core> conversion to each element via Into::into.
                // For `&[T]` (returns_ref=true) use `.iter()` to avoid clippy::into_iter_on_ref.
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                    if method.returns_ref {
                        ".iter().map(|v| v.clone().into()).collect()".to_string()
                    } else {
                        ".into_iter().map(Into::into).collect()".to_string()
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
        let mut inner_clone_line = if is_opaque {
            "let inner = self.inner.clone();\n        ".to_string()
        } else {
            String::new()
        };
        // For async Pyo3 functions with let_bindings, move them inside the async block
        // so temporary borrows (e.g., Vec<&str> from Vec<String>) extend to when the
        // future executes, not just when the binding function returns.
        if cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy && !ref_let_bindings.is_empty() {
            inner_clone_line.push_str(&ref_let_bindings);
        }
        let core_call_str = make_async_core_call(&method.name);
        gen_async_body(
            &core_call_str,
            cfg,
            method.error_type.is_some(),
            &async_result_wrap,
            is_opaque,
            &inner_clone_line,
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
    // NOTE: For async Pyo3 methods with bindings, the bindings are moved INSIDE the async
    // block via the inner_clone_line parameter to gen_async_body(), so they should NOT
    // be prepended here (they would be duplicated).
    let body = if ref_let_bindings.is_empty() || has_adapter {
        body
    } else if method.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy {
        // Bindings already moved inside async block, don't prepend
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
