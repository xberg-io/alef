//! Method-level emission for a generated producer crate: ABI types, function-pointer types and
//! the `extern "C"` wrappers that call the implementation through the contract trait.

use crate::codegen::component::{ComponentMethodIr, ComponentParamIr, WireType};
use anyhow::Result;
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::fmt::Write as _;

fn rust_abi_type(ty: &WireType, output: bool) -> String {
    match ty {
        WireType::Unit => "()".into(),
        WireType::Bool | WireType::U8 => "u8".into(),
        WireType::U16 => "u16".into(),
        WireType::U32 => "u32".into(),
        WireType::U64 => "u64".into(),
        WireType::I8 => "i8".into(),
        WireType::I16 => "i16".into(),
        WireType::I32 | WireType::Enum(_) => "i32".into(),
        WireType::I64 => "i64".into(),
        WireType::F32 => "f32".into(),
        WireType::F64 => "f64".into(),
        WireType::Utf8 | WireType::Char | WireType::Path | WireType::Bytes if output => "AlefOwnedBuffer".into(),
        WireType::Utf8 | WireType::Char | WireType::Path => "AlefStr".into(),
        WireType::Bytes => "AlefSlice".into(),
        other => panic!("unsupported producer wire type: {other:?}"),
    }
}

pub(super) fn rust_function_type(contract: &str, method: &ComponentMethodIr) -> String {
    let mut params = vec!["*mut c_void".to_string()];
    params.extend(method.params.iter().map(|param| rust_abi_type(&param.ty, false)));
    if method.is_async {
        params.push(format!("{contract}{}Completion", method.name.to_upper_camel_case()));
        params.push("*mut c_void".into());
        params.push("*mut AlefTaskV1".into());
    } else {
        if method.result != WireType::Unit {
            params.push(format!("*mut {}", rust_abi_type(&method.result, true)));
        }
        params.push("*mut AlefOwnedBuffer".into());
    }
    format!("unsafe extern \"C\" fn({}) -> AlefStatus", params.join(", "))
}

pub(super) fn emit_async_callback_alias(out: &mut String, contract: &str, method: &ComponentMethodIr) -> Result<()> {
    let result = if method.result == WireType::Unit {
        String::new()
    } else {
        format!(", {}", rust_abi_type(&method.result, true))
    };
    writeln!(
        out,
        "type {contract}{}Completion = unsafe extern \"C\" fn(*mut c_void, AlefStatus{result}, AlefOwnedBuffer);",
        method.name.to_upper_camel_case()
    )?;
    Ok(())
}

pub(super) fn emit_method_wrapper(
    out: &mut String,
    trait_path: &str,
    prefix: &str,
    contract: &str,
    method: &ComponentMethodIr,
) -> Result<()> {
    let method_name = method.name.to_snake_case();
    write!(
        out,
        "unsafe extern \"C\" fn {prefix}_component_{method_name}(instance: *mut c_void"
    )?;
    for param in &method.params {
        write!(
            out,
            ", {}: {}",
            param.name.to_snake_case(),
            rust_abi_type(&param.ty, false)
        )?;
    }
    if method.is_async {
        write!(
            out,
            ", completion: {contract}{}Completion, completion_context: *mut c_void, out_task: *mut AlefTaskV1",
            method.name.to_upper_camel_case()
        )?;
    } else {
        if method.result != WireType::Unit {
            write!(out, ", out_result: *mut {}", rust_abi_type(&method.result, true))?;
        }
        out.push_str(", out_error: *mut AlefOwnedBuffer");
    }
    out.push_str(") -> AlefStatus {\n");
    out.push_str("    // SAFETY: the host passes null or the live handle create_component returned.\n");
    writeln!(
        out,
        "    let Some(instance) = (unsafe {{ {prefix}_component_ref(instance) }}) else {{ return AlefStatus::INVALID_ARGUMENT; }};"
    )?;
    if !method.is_async && method.result != WireType::Unit {
        out.push_str("    if out_result.is_null() { return AlefStatus::INVALID_ARGUMENT; }\n");
    }
    for param in &method.params {
        emit_input_conversion(out, param)?;
    }
    let args = method
        .params
        .iter()
        .map(|param| format!("{}_value", param.name.to_snake_case()))
        .collect::<Vec<_>>()
        .join(", ");
    if method.is_async {
        emit_async_body(out, trait_path, &method_name, &args, method)?;
    } else {
        writeln!(
            out,
            "    let call = catch_unwind(AssertUnwindSafe(|| {trait_path}::{method_name}(&**instance, {args})));"
        )?;
        emit_sync_match(out, method)?;
        out.push_str("}\n\n");
    }
    Ok(())
}

fn emit_async_body(
    out: &mut String,
    trait_path: &str,
    method_name: &str,
    args: &str,
    method: &ComponentMethodIr,
) -> Result<()> {
    out.push_str("    if out_task.is_null() { return AlefStatus::INVALID_ARGUMENT; }\n");
    out.push_str("    let instance = Arc::clone(instance);\n");
    out.push_str("    let cancelled = Arc::new(AtomicBool::new(false));\n");
    out.push_str("    // SAFETY: out_task is non-null and the host passes it writable for this call.\n");
    out.push_str("    unsafe { *out_task = cancel_flag_task(Arc::clone(&cancelled)) };\n");
    out.push_str("    // Raw pointers are not `Send`; the worker thread rebuilds the context from its address.\n");
    out.push_str("    let completion_context = completion_context.expose_provenance();\n");
    out.push_str("    std::thread::spawn(move || {\n");
    out.push_str(
        "        let completion_context = std::ptr::with_exposed_provenance_mut::<c_void>(completion_context);\n",
    );
    out.push_str("        if cancelled.load(Ordering::Acquire) {\n");
    emit_completion(out, method, "AlefStatus::CANCELLED", None, 12)?;
    out.push_str("            return;\n        }\n");
    writeln!(
        out,
        "        let call = catch_unwind(AssertUnwindSafe(|| futures::executor::block_on({trait_path}::{method_name}(&*instance, {args}))));"
    )?;
    emit_async_match(out, method)?;
    out.push_str("    });\n    AlefStatus::PENDING\n}\n\n");
    Ok(())
}

fn emit_input_conversion(out: &mut String, param: &ComponentParamIr) -> Result<()> {
    let name = param.name.to_snake_case();
    match &param.ty {
        WireType::Utf8 => {
            writeln!(
                out,
                "    if {name}.ptr.is_null() && {name}.len != 0 {{ return AlefStatus::INVALID_ARGUMENT; }}"
            )?;
            writeln!(
                out,
                "    // SAFETY: a non-empty string argument points at `len` bytes the host keeps readable for this call.\n    let {name}_bytes: &[u8] = if {name}.len == 0 {{ &[] }} else {{ unsafe {{ std::slice::from_raw_parts({name}.ptr.cast::<u8>(), {name}.len) }} }};"
            )?;
            writeln!(
                out,
                "    let Ok({name}_value) = std::str::from_utf8({name}_bytes) else {{ return AlefStatus::INVALID_ARGUMENT; }};"
            )?;
            if !param.borrowed {
                writeln!(out, "    let {name}_value = {name}_value.to_owned();")?;
            }
        }
        WireType::Char | WireType::Path => unreachable!("validated as unsupported by producer"),
        WireType::Bytes => {
            writeln!(
                out,
                "    if {name}.ptr.is_null() && {name}.len != 0 {{ return AlefStatus::INVALID_ARGUMENT; }}"
            )?;
            writeln!(
                out,
                "    // SAFETY: a non-empty byte argument points at `len` bytes the host keeps readable for this call.\n    let {name}_value: &[u8] = if {name}.len == 0 {{ &[] }} else {{ unsafe {{ std::slice::from_raw_parts({name}.ptr, {name}.len) }} }};"
            )?;
            if !param.borrowed {
                writeln!(out, "    let {name}_value = {name}_value.to_vec();")?;
            }
        }
        WireType::Bool => writeln!(out, "    let {name}_value = {name} != 0;")?,
        WireType::Unit => writeln!(out, "    let {name}_value = ();")?,
        _ => writeln!(out, "    let {name}_value = {name};")?,
    }
    Ok(())
}

fn emit_sync_match(out: &mut String, method: &ComponentMethodIr) -> Result<()> {
    out.push_str("    match call {\n");
    if method.fallible {
        out.push_str("        Ok(Ok(value)) => {\n");
        emit_sync_success(out, method)?;
        out.push_str("        }\n        Ok(Err(error)) => {\n");
        out.push_str("            if !out_error.is_null() {\n                // SAFETY: out_error is non-null and the host passes it writable for this call.\n                unsafe { *out_error = error_buffer(error) };\n            }\n");
        out.push_str("            AlefStatus::INTERNAL_ERROR\n        }\n");
    } else {
        out.push_str("        Ok(value) => {\n");
        emit_sync_success(out, method)?;
        out.push_str("        }\n");
    }
    out.push_str("        Err(payload) => {\n");
    out.push_str("            if !out_error.is_null() {\n                // SAFETY: out_error is non-null and the host passes it writable for this call.\n                unsafe { *out_error = error_buffer(format!(\"component method panicked: {}\", panic_message(&*payload))) };\n            }\n");
    out.push_str("            AlefStatus::INTERNAL_ERROR\n        }\n    }\n");
    Ok(())
}

fn emit_sync_success(out: &mut String, method: &ComponentMethodIr) -> Result<()> {
    if method.result != WireType::Unit {
        out.push_str("            // SAFETY: out_result was checked non-null above and the host passes it writable.\n");
        writeln!(
            out,
            "            unsafe {{ *out_result = {} }};",
            output_conversion(&method.result, "value")
        )?;
    }
    out.push_str("            AlefStatus::OK\n");
    Ok(())
}

fn emit_async_match(out: &mut String, method: &ComponentMethodIr) -> Result<()> {
    out.push_str("        match call {\n");
    if method.fallible {
        out.push_str("            Ok(Ok(value)) => {\n");
        emit_completion(out, method, "AlefStatus::OK", None, 16)?;
        out.push_str("            }\n            Ok(Err(error)) => {\n");
        emit_completion(out, method, "AlefStatus::INTERNAL_ERROR", Some("error"), 16)?;
        out.push_str("            }\n");
    } else {
        out.push_str("            Ok(value) => {\n");
        emit_completion(out, method, "AlefStatus::OK", None, 16)?;
        out.push_str("            }\n");
    }
    out.push_str("            Err(payload) => {\n");
    emit_completion(
        out,
        method,
        "AlefStatus::INTERNAL_ERROR",
        Some("format!(\"component method panicked: {}\", panic_message(&*payload))"),
        16,
    )?;
    out.push_str("            }\n        }\n");
    Ok(())
}

fn emit_completion(
    out: &mut String,
    method: &ComponentMethodIr,
    status: &str,
    error: Option<&str>,
    indent: usize,
) -> Result<()> {
    let pad = " ".repeat(indent);
    let result = if method.result == WireType::Unit {
        String::new()
    } else if status == "AlefStatus::OK" {
        format!(", {}", output_conversion(&method.result, "value"))
    } else {
        format!(", {}", zero_value(&method.result))
    };
    let error = error.map_or_else(
        || "AlefOwnedBuffer::EMPTY".to_string(),
        |value| format!("error_buffer({value})"),
    );
    writeln!(
        out,
        "{pad}// SAFETY: the host keeps completion_context valid until its completion runs, once.\n{pad}unsafe {{ completion(completion_context, {status}{result}, {error}) }};"
    )?;
    Ok(())
}

fn output_conversion(ty: &WireType, value: &str) -> String {
    match ty {
        WireType::Utf8 => format!("owned_buffer({value}.into_bytes())"),
        WireType::Bytes => format!("owned_buffer({value})"),
        WireType::Bool => format!("u8::from({value})"),
        WireType::Unit => "()".into(),
        _ => value.into(),
    }
}

fn zero_value(ty: &WireType) -> String {
    match ty {
        WireType::Utf8 | WireType::Char | WireType::Path | WireType::Bytes => "AlefOwnedBuffer::EMPTY".into(),
        WireType::F32 | WireType::F64 => "0.0".into(),
        WireType::Unit => "()".into(),
        _ => "0".into(),
    }
}
