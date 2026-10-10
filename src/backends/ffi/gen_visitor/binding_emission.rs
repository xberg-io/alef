use heck::ToPascalCase;

use crate::backends::ffi::gen_visitor::callback_specs::{callback_specs_from_trait, gen_struct_fields};
use crate::backends::ffi::gen_visitor::context::{
    context_field_specs, gen_context_inits, gen_context_setup, gen_context_struct_fields, gen_result_constants,
    gen_result_decode_arms,
};
use crate::backends::ffi::gen_visitor::legacy_conversion::{
    named_type_ref, visitor_function_spec, visitor_options_param,
};
use crate::backends::ffi::gen_visitor::protocol::VisitorProtocol;
use crate::backends::ffi::gen_visitor::visitor_refs::{gen_impl_methods, gen_visitor_ref_methods};
use crate::backends::ffi::template_env::render;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::FunctionDef;

#[cfg(test)]
pub fn gen_visitor_bindings(
    prefix: &str,
    core_import: &str,
    embed_visitor_in_options: bool,
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: Option<&TraitBridgeConfig>,
    function: Option<&FunctionDef>,
) -> String {
    gen_visitor_bindings_with_api(
        prefix,
        core_import,
        embed_visitor_in_options,
        trait_def,
        bridge_cfg,
        function,
        None,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn gen_visitor_bindings_with_api(
    prefix: &str,
    core_import: &str,
    embed_visitor_in_options: bool,
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: Option<&TraitBridgeConfig>,
    function: Option<&FunctionDef>,
    api: Option<&crate::core::ir::ApiSurface>,
    emit_legacy_options_setter: bool,
) -> String {
    let pascal_prefix = prefix.to_pascal_case();
    let Some(api) = api else {
        tracing::warn!("gen_visitor_bindings(ffi): visitor callbacks require API metadata");
        return String::new();
    };
    let Some(bridge_cfg) = bridge_cfg else {
        tracing::warn!("gen_visitor_bindings(ffi): visitor callbacks require trait_bridge metadata");
        return String::new();
    };
    let Some(protocol) = VisitorProtocol::from_api(api, bridge_cfg) else {
        return String::new();
    };
    let Some(context_def) = api.types.iter().find(|type_def| type_def.name == protocol.context_type) else {
        return String::new();
    };
    let Some(result_metadata) = crate::codegen::visitor_result::visitor_result_metadata(api, bridge_cfg) else {
        tracing::warn!(
            "gen_visitor_bindings(ffi): trait bridge `{}` result_type metadata is required",
            bridge_cfg.trait_name
        );
        return String::new();
    };
    let default_result = format!("{}::{}", protocol.result_path, result_metadata.default_variant.name);
    let result_decode_arms = gen_result_decode_arms(&result_metadata, &default_result);
    let specs = callback_specs_from_trait(trait_def, Some(bridge_cfg));
    if specs.is_empty() {
        tracing::warn!(
            "gen_visitor_bindings(ffi): trait `{}` has no `{}`/`{}` visitor callback methods, skipping visitor callbacks",
            trait_def.name,
            protocol.context_type,
            protocol.result_type
        );
        return String::new();
    }
    let callback_count = specs.len();
    let trait_path = trait_def.rust_path.replace('-', "_");
    let trait_name = &trait_def.name;
    let options_type = function
        .and_then(|func| visitor_options_param(func, Some(bridge_cfg)))
        .and_then(|param| named_type_ref(&param.ty))
        .or(bridge_cfg.options_type.as_deref());
    let Some(options_type) = options_type else {
        tracing::warn!(
            "gen_visitor_bindings(ffi): visitor callbacks require a configured or IR-derived options type, skipping visitor callbacks"
        );
        return String::new();
    };
    let options_field = bridge_cfg.resolved_options_field().unwrap_or("visitor");
    let options_path = format!("{core_import}::{options_type}");

    let context_fields = context_field_specs(context_def, api);
    if context_fields.is_empty() {
        tracing::warn!(
            "gen_visitor_bindings(ffi): context_type `{}` has no FFI-compatible fields",
            protocol.context_type
        );
        return String::new();
    }
    let result_constants = gen_result_constants(prefix, &result_metadata);
    let context_struct_fields = gen_context_struct_fields(&context_fields);
    let context_setup = gen_context_setup(&context_fields);
    let context_inits = gen_context_inits(&context_fields);
    let struct_fields = gen_struct_fields(&specs, &pascal_prefix);
    let impl_methods = gen_impl_methods(&specs, &pascal_prefix, core_import, &protocol, &default_result);
    let visitor_ref_methods = gen_visitor_ref_methods(&specs, &pascal_prefix, &protocol, &default_result);
    let legacy_options_setter = if emit_legacy_options_setter {
        gen_legacy_options_setter(
            prefix,
            &pascal_prefix,
            &options_path,
            options_field,
            &trait_path,
            &visitor_ref_methods,
        )
    } else {
        String::new()
    };

    let visitor_function = function.and_then(|func| {
        visitor_function_spec(
            prefix,
            func,
            core_import,
            Some(bridge_cfg),
            embed_visitor_in_options,
            options_field,
        )
    });
    let context_path = protocol.context_path.clone();
    let result_path = protocol.result_path.clone();

    let mut out = format!(
        r#"// ---------------------------------------------------------------------------
// Visitor / callback FFI — {callback_count} {trait_name} methods
// ---------------------------------------------------------------------------

{result_constants}

/// Opaque context passed to every C callback.
///
/// Fields reflect `{context_type}` from the Rust core. All string pointers are
/// valid only for the duration of the callback invocation.
#[repr(C)]
pub struct {pascal_prefix}Context {{
{context_struct_fields}
}}

/// C-facing callback struct for the visitor pattern.
///
/// Populate the function-pointer fields you care about; leave the rest null.
/// The `user_data` pointer is forwarded unchanged to every callback — use it
/// to thread your own context through the conversion.
///
/// # Field order
///
/// The field order matches the Go binding's expected C layout exactly.
///
/// # Callback return protocol
///
/// Callbacks return an `i32` visit-result code.  When the code is
/// a string-payload variant, the callback must also write a heap-allocated,
/// null-terminated string into `*out_custom` and set `*out_len` to its byte
/// length (excluding the null terminator). The Rust side will read the string
/// and then call `free()` on the pointer.
///
/// For all other codes `out_custom` and `out_len` are not written.
///
/// # Callback signatures
///
/// All callbacks share the same leading parameters:
/// ```c
/// fn(ctx, user_data, out_custom, out_len, ...) -> i32
/// ```
/// followed by method-specific parameters documented on each field.
#[repr(C)]
pub struct {pascal_prefix}VisitorCallbacks {{
    /// Arbitrary caller context forwarded to every callback.
    pub user_data: *mut std::ffi::c_void,
    /// Releases callback-owned strings with the allocator that created them.
    pub free_string: Option<unsafe extern "C" fn(*mut std::ffi::c_char)>,
{struct_fields}}}

// SAFETY: The `user_data` pointer is the caller's responsibility. We require
// callers to uphold thread-safety themselves (i.e. not share a visitor across
// threads without synchronisation). The callbacks themselves are `extern "C"`
// and therefore inherently `Send`.
unsafe impl Send for {pascal_prefix}VisitorCallbacks {{}}
// SAFETY: see Send impl above; the callbacks struct is effectively a POD vtable.
unsafe impl Sync for {pascal_prefix}VisitorCallbacks {{}}

/// Opaque handle wrapping a `{pascal_prefix}VisitorCallbacks` and implementing
/// the Rust `{trait_name}` trait.
///
/// Allocate with `{prefix}_visitor_create` and release with `{prefix}_visitor_free`.
/// The handle must NOT outlive the `{pascal_prefix}VisitorCallbacks` it was created from.
pub struct {pascal_prefix}Visitor {{
    callbacks: {pascal_prefix}VisitorCallbacks,
    /// CString storage for tag names / parent tags that we pass back to C.
    /// RefCell is used for interior mutability; it is Send (Vec<CString> is Send) and
    /// the outer Arc<Mutex> serialises all access, so Sync is not required on RefCell itself.
    _tag_scratch: std::cell::RefCell<Vec<std::ffi::CString>>,
}}

// SAFETY: {pascal_prefix}Visitor is only accessed through the outer Arc<Mutex<dyn {trait_name} + Send>>
// which serialises access. The `user_data` pointer is the caller's responsibility.
unsafe impl Send for {pascal_prefix}Visitor {{}}
// SAFETY: see Send impl above; Sync is safe because all mutation goes through Mutex.
unsafe impl Sync for {pascal_prefix}Visitor {{}}

impl std::fmt::Debug for {pascal_prefix}Visitor {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        f.debug_struct("{pascal_prefix}Visitor").finish_non_exhaustive()
    }}
}}

/// Map a visit-result integer code + optional custom string pointer back to
/// the Rust result enum.
///
/// # Safety
///
/// `custom_ptr` must be either null or a pointer to a heap-allocated
/// null-terminated string that this function will take ownership of (freeing
/// it after reading).
unsafe fn decode_visit_result(
    code: i32,
    custom_ptr: *mut std::ffi::c_char,
    custom_len: usize,
    free_string: Option<unsafe extern "C" fn(*mut std::ffi::c_char)>,
) -> {result_path} {{
    use {result_path} as VisitorResult;
    match code {{
{result_decode_arms}
    }}
}}

/// Build a temporary `{pascal_prefix}Context` from a Rust `{context_type}`, invoke
/// the provided callback, and decode the result.
///
/// The context passed to the C callback is only valid for the duration
/// of this function call.
unsafe fn call_with_ctx<F>(
    ctx: &{context_path},
    free_string: Option<unsafe extern "C" fn(*mut std::ffi::c_char)>,
    callback: F,
) -> {result_path}
where
    F: FnOnce(
        *const {pascal_prefix}Context,
        *mut *mut std::ffi::c_char,
        *mut usize,
    ) -> i32,
{{
{context_setup}

    let c_ctx = {pascal_prefix}Context {{
{context_inits}
    }};

    let mut out_custom: *mut std::ffi::c_char = std::ptr::null_mut();
    let mut out_len: usize = 0;

    let code = callback(&c_ctx, &mut out_custom, &mut out_len);

    // SAFETY: the callback supplies a buffer valid for out_len bytes and its matching destructor.
    unsafe {{ decode_visit_result(code, out_custom, out_len, free_string) }}
}}

/// Convert an `Option<&str>` to a C pointer: non-null CString when `Some`, null when `None`.
///
/// Returns `(ptr, Option<CString>)` — the `Option<CString>` must be kept alive
/// until after the pointer is consumed by the callback.
fn opt_str_to_c(s: Option<&str>) -> (*const std::ffi::c_char, Option<std::ffi::CString>) {{
    match s {{
        Some(val) => match std::ffi::CString::new(val) {{
            Ok(cs) => {{
                let ptr = cs.as_ptr();
                (ptr, Some(cs))
            }}
            Err(_) => (std::ptr::null(), None),
        }},
        None => (std::ptr::null(), None),
    }}
}}

impl {trait_path} for {pascal_prefix}Visitor {{
{impl_methods}}}

/// Create a new visitor handle from a callbacks struct.
///
/// The returned handle must be freed with `{prefix}_visitor_free`.
/// The `{pascal_prefix}VisitorCallbacks` struct is **copied** into the handle;
/// the caller may free it after this call returns.
///
/// Returns null on allocation failure.
///
/// # Safety
///
/// `callbacks` must point to a valid, fully initialised `{pascal_prefix}VisitorCallbacks`.
/// `user_data` (embedded in the struct) must remain valid and accessible from
/// any thread that calls `{prefix}_convert_with_visitor` until after
/// `{prefix}_visitor_free` is called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_visitor_create(
    callbacks: *const {pascal_prefix}VisitorCallbacks,
) -> AlefHandle {{
    catch_ffi_panic(0, || {{
    if callbacks.is_null() {{
        return 0;
    }}
    // SAFETY: caller guarantees the pointer is valid.
    let cbs = unsafe {{ callbacks.read() }};
    let visitor = {pascal_prefix}Visitor {{
        callbacks: cbs,
        _tag_scratch: std::cell::RefCell::new(Vec::new()),
    }};
    match insert_handle(visitor) {{
        Ok(handle) => handle,
        Err(error) => {{ set_handle_error(&error); 0 }}
    }}
    }})
}}

/// Free a visitor handle previously returned by `{prefix}_visitor_create`.
///
/// After this call the pointer is invalid and must not be used.
///
/// # Safety
///
/// `visitor` must have been returned by `{prefix}_visitor_create`, or be null.
/// Passing a null pointer is safe and has no effect.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_visitor_free(visitor: AlefHandle) {{
    catch_ffi_panic((), || {{
    if visitor != 0
        && let Err(error) = remove_handle::<{pascal_prefix}Visitor>(visitor)
    {{
        set_handle_error(&error);
    }}
    }})
}}
{legacy_options_setter}"#,
        prefix = prefix,
        pascal_prefix = pascal_prefix,
        callback_count = callback_count,
        trait_name = trait_name,
        context_type = protocol.context_type,
        context_path = context_path,
        result_constants = result_constants,
        result_path = result_path,
        result_decode_arms = result_decode_arms,
        context_struct_fields = context_struct_fields,
        context_setup = context_setup,
        context_inits = context_inits,
        trait_path = trait_path,
        struct_fields = struct_fields,
        impl_methods = impl_methods,
        legacy_options_setter = legacy_options_setter,
    );

    if let Some(visitor_function) = visitor_function {
        out.push_str(&render(
            "ffi_visitor_with_callback_function.jinja",
            crate::alef_context! {
                prefix,
                with_visitor_fn_name => visitor_function.fn_name,
                pascal_prefix,
                trait_path,
                visitor_ref_methods,
                params => visitor_function.ffi_params,
                param_conversions => visitor_function.param_conversions,
                return_type => visitor_function.return_type,
                call => visitor_function.call,
            },
        ));
    }

    out
}

fn gen_legacy_options_setter(
    prefix: &str,
    pascal_prefix: &str,
    options_path: &str,
    options_field: &str,
    trait_path: &str,
    visitor_ref_methods: &str,
) -> String {
    let _ = (trait_path, visitor_ref_methods);
    format!(
        r#"
/// Attach a visitor to an options handle before calling `{prefix}_convert`.
///
/// The visitor will be invoked during conversion via the normal `{prefix}_convert` path.
/// This function consumes the `visitor` handle. Do not free or reuse it after attachment.
///
/// Passing `null` for either argument is a no-op.
///
/// # Safety
///
/// `options` must be a non-null pointer returned by `{prefix}_conversion_options_from_json`,
/// valid for write access.  `visitor` must be a non-null pointer returned by
/// `{prefix}_visitor_create`, or null. Ownership transfers to `options` on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {prefix}_options_set_visitor_handle(
    options: AlefHandle,
    visitor: AlefHandle,
) {{
    catch_ffi_panic((), || {{
    if options == 0 || visitor == 0 || options == visitor {{
        if options == visitor && options != 0 {{ set_handle_error(&HandleError::AliasedHandle); }}
        return;
    }}
    if let Err(error) = with_handle::<{options_path}, _>(options, |_| ()) {{
        set_handle_error(&error);
        return;
    }}
    let visitor = match take_handle::<{pascal_prefix}Visitor>(visitor) {{
        Ok(value) => value,
        Err(error) => {{ set_handle_error(&error); return; }}
    }};
    if let Err(error) = with_handle_mut::<{options_path}, _>(options, |options_ref| {{
        options_ref.{options_field} = Some(std::sync::Arc::new(std::sync::Mutex::new(visitor)));
    }}) {{
        set_handle_error(&error);
    }}
    }})
}}"#,
    )
}
