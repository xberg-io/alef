//! Forwarder emission for free functions that return a host-native capsule (e.g. `Language`)
//! type. Split out of `forwarders.rs` (at the file-size cap) to keep the cap addition for the
//! `&mut T` writeback fix (`writeback.rs`) from pushing it further over. ~keep

use super::*;

/// Returns the capsule config if this function returns a capsule-eligible type.
pub(super) fn swift_capsule_return_config<'a>(
    func: &FunctionDef,
    capsule_types: &'a std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> Option<&'a crate::core::config::HostCapsuleTypeConfig> {
    if let TypeRef::Named(name) = &func.return_type {
        capsule_types.get(name.as_str())
    } else {
        None
    }
}

/// Emit a synchronous free function that returns a host-native capsule (Language) type.
///
/// The C FFI returns the host runtime's raw grammar pointer as a usize (0 = error sentinel).
/// The wrapper reconstructs OpaquePointer via OpaquePointer(bitPattern:) and then constructs
/// the host `Language` using the expression from `capsule_cfg.construct_expr`.
///
/// `capsule_cfg.host_type` and `capsule_cfg.construct_expr` are required; missing values
/// produce a `// ALEF ERROR:` comment in the generated output.
pub(super) fn emit_capsule_free_function_forwarder(
    func: &FunctionDef,
    swift_name: &str,
    capsule_cfg: &crate::core::config::HostCapsuleTypeConfig,
    out: &mut String,
) {
    let host_type = match capsule_cfg.required_host_type("Language", "swift") {
        Ok(t) => t.to_string(),
        Err(e) => {
            out.push_str(&format!("// ALEF ERROR: {e}\n"));
            return;
        }
    };

    if !func.doc.is_empty() {
        emit_doc_comment(&func.doc, "", out);
    }

    let mut sig_params: Vec<String> = Vec::new();
    let mut c_args: Vec<String> = Vec::new();
    for param in &func.params {
        let swift_param_name = swift_ident(&param.name.to_lower_camel_case());
        let swift_ty = if param.optional { "String?" } else { "String" };
        let param_default = if param.optional { " = nil" } else { "" };
        sig_params.push(format!("{swift_param_name}: {swift_ty}{param_default}"));
        c_args.push(swift_param_name);
    }
    let sig = sig_params.join(", ");

    let is_fallible = func.error_type.is_some();
    let throws_clause = if is_fallible { " throws" } else { "" };
    let return_clause = if is_fallible {
        format!(" -> {host_type}")
    } else {
        format!(" -> {host_type}?")
    };

    let c_call = format!("RustBridge.{swift_name}({})", c_args.join(", "));
    let construct = match capsule_cfg.construct_required("cLang", "Language", "swift") {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("// ALEF ERROR: {e}\n"));
            return;
        }
    };
    let nil_error = format!(
        "NSError(domain: \"alef.capsule\", code: 1, userInfo: [NSLocalizedDescriptionKey: \"Capsule function returned null: {swift_name}\"])"
    );

    let body = if is_fallible {
        format!(
            "let addr = {c_call}\n    guard addr != 0, let cLang = OpaquePointer(bitPattern: addr) else {{ throw {nil_error} }}\n    return {construct}"
        )
    } else {
        format!(
            "let addr = {c_call}\n    guard addr != 0, let cLang = OpaquePointer(bitPattern: addr) else {{ return nil }}\n    return {construct}"
        )
    };

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_sync_forwarder.swift.jinja",
        minijinja::context! {
            function_name => swift_name,
            params => &sig,
            throws_clause => throws_clause,
            return_clause => &return_clause,
            conversion_lines => "",
            body => body,
        },
    ));
}

/// Emit an async free function that returns a host-native capsule (Language) type.
///
/// The C FFI returns the host runtime's raw grammar pointer as a usize (0 = error sentinel).
/// The wrapper reconstructs OpaquePointer via OpaquePointer(bitPattern:) and then constructs
/// the host `Language` from it.
pub(super) fn emit_async_capsule_free_function_forwarder(
    func: &FunctionDef,
    swift_name: &str,
    capsule_cfg: &crate::core::config::HostCapsuleTypeConfig,
    out: &mut String,
) {
    let host_type = match capsule_cfg.required_host_type("Language", "swift") {
        Ok(t) => t.to_string(),
        Err(e) => {
            out.push_str(&format!("// ALEF ERROR: {e}\n"));
            return;
        }
    };

    if !func.doc.is_empty() {
        emit_doc_comment(&func.doc, "", out);
    }

    let mut sig_params: Vec<String> = Vec::new();
    let mut c_args: Vec<String> = Vec::new();
    for param in &func.params {
        let swift_param_name = swift_ident(&param.name.to_lower_camel_case());
        let swift_ty = if param.optional { "String?" } else { "String" };
        let param_default = if param.optional { " = nil" } else { "" };
        sig_params.push(format!("{swift_param_name}: {swift_ty}{param_default}"));
        c_args.push(swift_param_name);
    }
    let sig = sig_params.join(", ");
    let return_clause = format!(" -> {host_type}");

    let c_call = format!("RustBridge.{swift_name}({})", c_args.join(", "));
    let construct = match capsule_cfg.construct_required("cLang", "Language", "swift") {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("// ALEF ERROR: {e}\n"));
            return;
        }
    };
    let nil_error = format!(
        "NSError(domain: \"alef.capsule\", code: 1, userInfo: [NSLocalizedDescriptionKey: \"Capsule function returned null: {swift_name}\"])"
    );

    let body = format!(
        "        let addr = try await {c_call}\n        guard addr != 0, let cLang = OpaquePointer(bitPattern: addr) else {{ throw {nil_error} }}\n        return {construct}\n"
    );

    out.push_str(&super::render_async_forwarder(
        swift_name,
        &sig,
        &return_clause,
        "",
        &body,
    ));
}
