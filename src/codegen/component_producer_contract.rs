//! Per-contract blocks of a generated producer crate: lifecycle functions, the method table,
//! the static descriptor and the `#[no_mangle]` entry point.

use super::methods::{emit_async_callback_alias, emit_method_wrapper, rust_function_type};
use crate::codegen::component::{ComponentContractIr, ResolvedProvidedContract, entry_point_symbol};
use anyhow::Result;
use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use std::fmt::Write as _;

/// Per-contract identifier names shared by every emitter for one contract block.
struct ContractNames {
    prefix: String,
    screaming_prefix: String,
    implementation_alias: String,
    contract_name: String,
    api_name: String,
}

impl ContractNames {
    fn new(contract: &ComponentContractIr) -> Self {
        let contract_name = contract.name.to_upper_camel_case();
        Self {
            prefix: contract.name.to_snake_case(),
            screaming_prefix: contract.name.to_shouty_snake_case(),
            implementation_alias: format!("{contract_name}ComponentImplementation"),
            api_name: format!("{contract_name}ApiV{}", contract.interface_version),
            contract_name,
        }
    }
}

pub(super) fn emit_contract_block(
    out: &mut String,
    provided: &ResolvedProvidedContract,
    feature_hash: &str,
) -> Result<()> {
    let names = ContractNames::new(&provided.contract);
    emit_lifecycle_functions(out, &names, &provided.implementation)?;
    emit_contract_table_and_entry(out, &names, provided, feature_hash)
}

/// Emit the implementation type alias and the `create`/`destroy`/`component_ref`
/// functions the descriptor's function pointers below point back into.
fn emit_lifecycle_functions(out: &mut String, names: &ContractNames, implementation: &str) -> Result<()> {
    let ContractNames {
        prefix,
        implementation_alias,
        ..
    } = names;
    writeln!(out, "type {implementation_alias} = {implementation};\n")?;
    // Instances are shared across host threads through raw pointers; a non-`Sync`
    // implementation would be silent UB, so reject it at compile time.
    writeln!(
        out,
        "const _: fn() = || {{\n    fn assert_send_sync<T: Send + Sync>() {{}}\n    assert_send_sync::<{implementation_alias}>();\n}};\n"
    )?;
    writeln!(
        out,
        "unsafe fn {prefix}_component_ref<'a>(instance: *mut c_void) -> Option<&'a Arc<{implementation_alias}>> {{"
    )?;
    out.push_str(
        "    // SAFETY: callers pass null or a handle `create_component` returned that the host has not\n    // destroyed yet, which points at a live `Arc` of the implementation.\n",
    );
    writeln!(
        out,
        "    unsafe {{ instance.cast::<Arc<{implementation_alias}>>().as_ref() }}"
    )?;
    out.push_str("}\n\n");
    writeln!(
        out,
        "unsafe extern \"C\" fn {prefix}_create_component(host: *const AlefHostApiV1, out_instance: *mut *mut c_void, out_error: *mut AlefOwnedBuffer) -> AlefStatus {{"
    )?;
    out.push_str("    if out_instance.is_null() { return AlefStatus::INVALID_ARGUMENT; }\n");
    out.push_str("    // SAFETY: the host passes null or a pointer to its host API for the duration of this call.\n");
    out.push_str("    let host = unsafe { alef_component_abi::HostApi::from_raw(host) };\n");
    writeln!(
        out,
        "    // Prefers `FromHost::from_host(&host)` when the implementation provides it, else `Default`.\n    match catch_unwind(AssertUnwindSafe(|| Arc::new(alef_component_abi::construct_component!({implementation_alias}, &host)))) {{"
    )?;
    out.push_str("        Ok(instance) => {\n");
    out.push_str("            // SAFETY: out_instance is non-null and the host passes it writable for this call.\n");
    out.push_str("            unsafe { *out_instance = Box::into_raw(Box::new(instance)).cast() };\n");
    out.push_str("            AlefStatus::OK\n        }\n");
    out.push_str("        Err(payload) => {\n");
    out.push_str("            if !out_error.is_null() {\n");
    out.push_str("                // SAFETY: out_error is non-null and the host passes it writable for this call.\n");
    out.push_str("                unsafe { *out_error = error_buffer(format!(\"component constructor panicked: {}\", panic_message(&*payload))) };\n");
    out.push_str("            }\n            AlefStatus::INTERNAL_ERROR\n        }\n    }\n}\n\n");
    writeln!(
        out,
        "unsafe extern \"C\" fn {prefix}_destroy_component(instance: *mut c_void) {{"
    )?;
    out.push_str("    if !instance.is_null() {\n        let _ = catch_unwind(AssertUnwindSafe(|| {\n");
    out.push_str("            // SAFETY: a non-null handle is the `Box<Arc<_>>` create_component leaked, and the host\n            // destroys each handle exactly once.\n");
    writeln!(
        out,
        "            drop(unsafe {{ Box::from_raw(instance.cast::<Arc<{implementation_alias}>>()) }});"
    )?;
    out.push_str("        }));\n    }\n}\n\n");
    Ok(())
}

/// Emit the method table struct, its method wrappers, the static descriptor, and
/// the `#[no_mangle]` entry point that hands the descriptor to the host.
fn emit_contract_table_and_entry(
    out: &mut String,
    names: &ContractNames,
    provided: &ResolvedProvidedContract,
    feature_hash: &str,
) -> Result<()> {
    let ContractNames {
        prefix,
        contract_name,
        api_name,
        ..
    } = names;
    let contract = &provided.contract;

    for method in contract.methods.iter().filter(|method| method.is_async) {
        emit_async_callback_alias(out, contract_name, method)?;
    }
    writeln!(out, "#[repr(C)]\nstruct {api_name} {{")?;
    out.push_str("    struct_size: usize,\n");
    for method in &contract.methods {
        writeln!(
            out,
            "    {}: Option<{}>,",
            method.name.to_snake_case(),
            rust_function_type(contract_name, method)
        )?;
    }
    out.push_str("}\n\n");

    for method in &contract.methods {
        emit_method_wrapper(out, &contract.trait_path, prefix, contract_name, method)?;
    }

    writeln!(
        out,
        "static {screaming_prefix}_CONTRACT_API: {api_name} = {api_name} {{",
        screaming_prefix = names.screaming_prefix,
    )?;
    writeln!(out, "    struct_size: std::mem::size_of::<{api_name}>(),")?;
    for method in &contract.methods {
        let method_name = method.name.to_snake_case();
        writeln!(out, "    {method_name}: Some({prefix}_component_{method_name}),")?;
    }
    out.push_str("};\n\n");

    emit_descriptor_and_entrypoint(out, names, provided, feature_hash)
}

/// Emit the static `AlefComponentV1` descriptor and the `#[no_mangle]` entry
/// point the host calls to obtain a pointer to it.
fn emit_descriptor_and_entrypoint(
    out: &mut String,
    names: &ContractNames,
    provided: &ResolvedProvidedContract,
    feature_hash: &str,
) -> Result<()> {
    let ContractNames {
        prefix,
        screaming_prefix,
        api_name,
        ..
    } = names;
    let contract_hash = super::byte_array(&provided.contract_hash);

    out.push_str(
        "// SAFETY: every pointer in the descriptor refers to immutable process-lifetime data: the id and\n// version literals, the contract table static and functions.\n",
    );
    writeln!(
        out,
        "static {screaming_prefix}_COMPONENT: SyncStatic<AlefComponentV1> = unsafe {{\n    SyncStatic::new(AlefComponentV1 {{"
    )?;
    out.push_str("        struct_size: std::mem::size_of::<AlefComponentV1>(),\n");
    out.push_str("        abi_major: 1,\n        abi_minor: 0,\n");
    out.push_str(
        "        component_id: AlefStr { ptr: COMPONENT_ID.as_ptr().cast::<c_char>(), len: COMPONENT_ID.len() },\n",
    );
    out.push_str(
        "        component_version: AlefStr { ptr: COMPONENT_VERSION.as_ptr().cast::<c_char>(), len: COMPONENT_VERSION.len() },\n",
    );
    writeln!(out, "        contract_hash: {contract_hash},")?;
    writeln!(out, "        feature_set_hash: {feature_hash},")?;
    writeln!(
        out,
        "        contract: std::ptr::from_ref(&{screaming_prefix}_CONTRACT_API).cast::<c_void>(),"
    )?;
    writeln!(out, "        contract_size: std::mem::size_of::<{api_name}>(),")?;
    writeln!(
        out,
        "        create: Some({prefix}_create_component),\n        destroy: Some({prefix}_destroy_component),\n    }})\n}};\n"
    )?;
    out.push_str("/// Return this component's descriptor for the requested ABI version.\n");
    out.push_str("///\n/// # Safety\n");
    out.push_str("/// `out_component` must be null or point to writable storage for one descriptor pointer.\n");
    writeln!(out, "#[unsafe(no_mangle)]")?;
    writeln!(
        out,
        "pub unsafe extern \"C\" fn {}(requested_abi_major: u32, _host: *const AlefHostApiV1, out_component: *mut *const AlefComponentV1) -> AlefStatus {{",
        entry_point_symbol(&provided.contract.name)
    )?;
    out.push_str(
        "    if requested_abi_major != 1 || out_component.is_null() { return AlefStatus::INCOMPATIBLE_ABI; }\n",
    );
    writeln!(
        out,
        "    // SAFETY: out_component is non-null and writable per this function's contract, and\n    // {screaming_prefix}_COMPONENT is a static with process lifetime."
    )?;
    writeln!(
        out,
        "    unsafe {{ *out_component = {screaming_prefix}_COMPONENT.as_ptr() }};"
    )?;
    out.push_str("    AlefStatus::OK\n}\n\n");
    Ok(())
}
