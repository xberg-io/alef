use super::constructors::gen_go_opaque_constructor;
use super::functions::{
    gen_adapter_wrapper, gen_capsule_function_wrapper, gen_convert_with_visitor_wrapper, gen_function_wrapper,
};
use super::methods::{gen_method_wrapper, gen_streaming_method_wrapper};
use super::types::{
    gen_config_options, gen_enum_type, gen_last_error_helper, gen_opaque_type, gen_opaque_type_free_only,
    gen_ptr_helper, gen_struct_type, gen_unmarshal_bytes_helper, is_passthrough_raw_message_enum, is_tuple_field,
};
use crate::codegen::naming::go_type_name;
use crate::core::config::{AdapterPattern, ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};
use std::collections::HashSet;

/// Strip trailing whitespace from every line and ensure the file ends with a single newline.
pub(super) fn strip_trailing_whitespace(content: &str) -> String {
    let mut result: String = content
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Run `gofmt -s` on generated Go code. Falls back to the original if gofmt is unavailable.
pub(super) fn format_go_code(code: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let child = Command::new("gofmt")
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut c) => {
            if let Some(ref mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(code.as_bytes());
            }
            match c.wait_with_output() {
                Ok(output) if output.status.success() => {
                    String::from_utf8(output.stdout).unwrap_or_else(|_| code.to_string())
                }
                _ => code.to_string(),
            }
        }
        Err(_) => code.to_string(),
    }
}

/// Returns true if a `TypeRef::Named` type comes from `api.enums` (either unit or data enum)
/// and therefore does not have `_from_json`/`_to_json`/`_free` FFI helpers.
///
/// Only types in `api.types` (non-opaque struct types) have these helpers in the C header.
pub(super) fn is_ffi_enum_type(name: &str, ffi_enum_names: &HashSet<String>) -> bool {
    ffi_enum_names.contains(name)
}

/// Returns true if a function references a DATA enum type (from `api.enums`) as a parameter type
/// or return type, for which the FFI header lacks `_from_json`/`_to_json`/`_free` helpers.
///
/// Unit-variant enums (in `ffi_param_enum_names`) can be marshaled to/from i32 and do NOT cause
/// skipping. Data enums without those helpers cannot be generated correctly and must be skipped.
fn uses_ffi_enum_type(
    func_params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    ffi_enum_names: &HashSet<String>,
    ffi_param_enum_names: &HashSet<String>,
    opaque_names: &std::collections::HashSet<&str>,
) -> bool {
    let named_is_problem =
        |n: &str| is_ffi_enum_type(n, ffi_enum_names) && !ffi_param_enum_names.contains(n) && !opaque_names.contains(n);
    let return_uses = match return_type {
        TypeRef::Named(n) => named_is_problem(n),
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if named_is_problem(n)),
        _ => false,
    };
    if return_uses {
        return true;
    }
    func_params.iter().any(|p| match &p.ty {
        TypeRef::Named(n) => named_is_problem(n),
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if named_is_problem(n)),
        _ => false,
    })
}

/// Returns true if a type reference mentions any excluded type.
fn references_excluded_type(ty: &TypeRef, exclude_types: &HashSet<String>) -> bool {
    exclude_types.iter().any(|name| ty.references_named(name))
}

/// Returns true if any parameter or return type mentions an excluded type.
fn signature_references_excluded_type(
    params: &[crate::core::ir::ParamDef],
    return_type: &TypeRef,
    exclude_types: &HashSet<String>,
) -> bool {
    references_excluded_type(return_type, exclude_types)
        || params
            .iter()
            .any(|param| references_excluded_type(&param.ty, exclude_types))
}

pub(super) fn find_options_bridge_function<'a>(
    api: &'a ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
) -> Option<&'a crate::core::ir::FunctionDef> {
    api.functions
        .iter()
        .find(|func| options_bridge_function_matches(func, bridge_cfg))
}

fn options_bridge_function_matches(func: &crate::core::ir::FunctionDef, bridge_cfg: &TraitBridgeConfig) -> bool {
    let Some(options_type) = bridge_cfg.options_type.as_deref() else {
        return false;
    };
    func.params
        .iter()
        .any(|param| type_ref_named_type(&param.ty) == Some(options_type))
}

fn type_ref_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => type_ref_named_type(inner),
        _ => None,
    }
}

/// Returns the host capsule config when `func` returns a configured capsule type by value
/// (bare `Named`). Optional capsule returns fall through to the standard opaque path.
fn go_capsule_return_config<'a>(
    func: &crate::core::ir::FunctionDef,
    capsule_types: &'a std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> Option<&'a crate::core::config::HostCapsuleTypeConfig> {
    if let crate::core::ir::TypeRef::Named(name) = &func.return_type {
        capsule_types.get(name.as_str())
    } else {
        None
    }
}

/// Derive the Go package qualifier used in the body from a capsule `host_type`.
///
/// `host_type` looks like `*tree_sitter.Language` or `tree_sitter.Language`; the
/// qualifier is the identifier the generated body references (`tree_sitter`). This is
/// the alias the import must declare explicitly: when the import path's last element
/// (`go-tree-sitter`) differs from the package name (`tree_sitter`), an unaliased
/// import is stripped by `goimports` in cgo files (it cannot resolve the package name
/// from the path), breaking the build. An explicit alias is matched syntactically and
/// survives. Returns `None` when no qualifier can be derived (no alias needed).
fn go_capsule_import_alias(host_type: &str) -> Option<&str> {
    let bare = host_type.trim_start_matches(['*', '&', '[', ']', ' ']);
    bare.split_once('.').map(|(qualifier, _)| qualifier)
}

/// Generate the complete Go binding file wrapping the C FFI layer.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_go_file(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    ffi_prefix: &str,
    pkg_name: &str,
    ffi_lib_name: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    go_output_dir: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    streaming_methods: &std::collections::HashMap<(String, String), String>,
    ffi_exclude_functions: &HashSet<String>,
    exclude_types: &HashSet<String>,
    value_only_types: &HashSet<String>,
    visitor_bridge_cfg: Option<&TraitBridgeConfig>,
) -> String {
    let mut header = String::with_capacity(2048);

    header.push_str(&hash::header(CommentStyle::DoubleSlash));
    header.push('\n');

    let depth = go_output_dir.trim_end_matches('/').matches('/').count() + 1;
    let to_root = "../".repeat(depth);

    header.push_str(&crate::backends::go::template_env::render(
        "package_doc_and_declaration.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            crate_name => &config.name,
        },
    ));
    header.push_str(&crate::backends::go::template_env::render(
        "cgo_preamble_binding.jinja",
        minijinja::context! {
            to_root => &to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_lib_name => ffi_lib_name,
            ffi_header => ffi_header,
        },
    ));
    header.push('\n');

    let mut body = String::with_capacity(8192);

    body.push_str(&gen_last_error_helper(ffi_prefix));
    body.push_str("\n\n");

    body.push_str(&gen_unmarshal_bytes_helper());
    body.push_str("\n\n");

    body.push_str(&gen_ptr_helper());
    body.push_str("\n\n");

    if !api.errors.is_empty() {
        body.push_str(&crate::codegen::error_gen::gen_go_sentinel_errors(&api.errors));
        body.push_str("\n\n");
        for error in &api.errors {
            body.push_str(&crate::codegen::error_gen::gen_go_error_struct(error, pkg_name));
            body.push_str("\n\n");
        }
    }

    let bridge_associated_types = config.bridge_associated_types();
    let visitor_types: std::collections::HashSet<&str> = if !bridge_param_names.is_empty() {
        bridge_associated_types.iter().map(|s| s.as_str()).collect()
    } else {
        std::collections::HashSet::new()
    };

    // Go type identifiers the loop below emits; disambiguates collisions via `go_free_function_name`. ~keep
    let reserved_type_names: HashSet<String> = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !visitor_types.contains(typ.name.as_str()) && !exclude_types.contains(&typ.name))
        .map(|typ| go_type_name(&typ.name))
        .chain(
            api.enums
                .iter()
                .filter(|e| !visitor_types.contains(e.name.as_str()) && !exclude_types.contains(&e.name))
                .map(|e| go_type_name(&e.name)),
        )
        .collect();

    let unit_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| {
            !exclude_types.contains(&e.name)
                && e.variants
                    .iter()
                    .all(|v| v.fields.is_empty() || v.fields.iter().all(is_tuple_field))
        })
        .filter(|e| !is_passthrough_raw_message_enum(e))
        .map(|e| e.name.as_str())
        .collect();
    let passthrough_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| is_passthrough_raw_message_enum(e))
        .filter(|e| !exclude_types.contains(&e.name))
        .map(|e| e.name.as_str())
        .collect();
    let text_types = &config.untagged_union_text_types;
    for enum_def in api
        .enums
        .iter()
        .filter(|e| !visitor_types.contains(e.name.as_str()) && !exclude_types.contains(&e.name))
    {
        body.push_str(&gen_enum_type(enum_def, text_types));
        body.push_str("\n\n");
    }

    let error_names: std::collections::HashSet<&str> = api.errors.iter().map(|e| e.name.as_str()).collect();

    let opaque_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .filter(|t| !exclude_types.contains(&t.name))
        .map(|t| t.name.as_str())
        .collect();

    let ffi_enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    let ffi_param_enum_names: HashSet<String> = api
        .enums
        .iter()
        .filter(|e| e.variants.iter().all(|v| v.fields.is_empty() && !v.is_tuple))
        .map(|e| e.name.clone())
        .collect();

    let data_enum_names: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| {
            !exclude_types.contains(&e.name)
                && e.variants
                    .iter()
                    .any(|v| !v.fields.is_empty() && v.fields.iter().any(|f| !is_tuple_field(f)))
        })
        .map(|e| e.name.as_str())
        .collect();

    let struct_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !exclude_types.contains(&t.name))
        .map(|t| t.name.as_str())
        .collect();

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !visitor_types.contains(typ.name.as_str()) && !exclude_types.contains(&typ.name))
    {
        if typ.is_opaque {
            if error_names.contains(typ.name.as_str()) {
                body.push_str(&gen_opaque_type_free_only(typ, ffi_prefix));
                body.push_str("\n\n");
            } else {
                body.push_str(&gen_opaque_type(typ, ffi_prefix));
                body.push_str("\n\n");
            }
            if let Some(ctor) = config.client_constructors.get(&typ.name) {
                body.push_str(&gen_go_opaque_constructor(typ, ffi_prefix, ctor));
                body.push_str("\n\n");
            }
        } else {
            body.push_str(&gen_struct_type(
                typ,
                &unit_enum_names,
                &passthrough_enum_names,
                &data_enum_names,
                &struct_names,
                &config.trait_bridges,
            ));
            body.push_str("\n\n");
            let empty_functional_options = vec![];
            let functional_options = config
                .go
                .as_ref()
                .map(|g| &g.functional_options)
                .unwrap_or(&empty_functional_options);
            if !typ.name.ends_with("Update") && functional_options.contains(&typ.name) {
                body.push_str(&gen_config_options(
                    typ,
                    &unit_enum_names,
                    &passthrough_enum_names,
                    &data_enum_names,
                    &config.trait_bridges,
                ));
                body.push_str("\n\n");
            }
        }
    }

    let go_capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig> =
        config.go.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();

    for func in api.functions.iter().filter(|f| {
        !ffi_exclude_functions.contains(&f.name)
            && !signature_references_excluded_type(&f.params, &f.return_type, exclude_types)
            && !uses_ffi_enum_type(
                &f.params,
                &f.return_type,
                &ffi_enum_names,
                &ffi_param_enum_names,
                &opaque_names,
            )
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, &config.trait_bridges)
    }) {
        if let Some(capsule_cfg) = go_capsule_return_config(func, &go_capsule_types) {
            body.push_str(&gen_capsule_function_wrapper(
                func,
                ffi_prefix,
                &opaque_names,
                &ffi_enum_names,
                &ffi_param_enum_names,
                capsule_cfg,
                &reserved_type_names,
            ));
            body.push_str("\n\n");
            continue;
        }
        if visitor_bridge_cfg.is_some_and(|bridge_cfg| options_bridge_function_matches(func, bridge_cfg)) {
            body.push_str(&gen_convert_with_visitor_wrapper(
                func,
                ffi_prefix,
                &opaque_names,
                value_only_types,
                visitor_bridge_cfg.expect("checked above"),
                &reserved_type_names,
            ));
            body.push_str("\n\n");
        } else {
            body.push_str(&gen_function_wrapper(
                func,
                ffi_prefix,
                &opaque_names,
                bridge_param_names,
                bridge_type_aliases,
                value_only_types,
                &ffi_enum_names,
                &ffi_param_enum_names,
                &reserved_type_names,
            ));
            body.push_str("\n\n");
        }
    }

    for adapter in &config.adapters {
        if !matches!(adapter.pattern, AdapterPattern::Streaming) {
            continue;
        }
        if adapter.owner_type.is_none() || adapter.item_type.is_none() {
            continue;
        }
        body.push_str(&gen_adapter_wrapper(adapter, pkg_name, &api.types));
        body.push_str("\n\n");
    }

    for typ in api
        .types
        .iter()
        .filter(|typ| !typ.is_trait && !exclude_types.contains(&typ.name))
    {
        if typ.is_opaque && error_names.contains(typ.name.as_str()) {
            continue;
        }
        if !typ.is_opaque && !typ.has_serde {
            continue;
        }
        for method in &typ.methods {
            if method.name == "default" {
                continue;
            }
            if typ.is_opaque
                && method.is_static
                && method.name != "new"
                && matches!(method.return_type, TypeRef::Named(_))
            {
                continue;
            }
            if let Some(item_type) = streaming_methods.get(&(typ.name.clone(), method.name.clone())) {
                body.push_str(&gen_streaming_method_wrapper(
                    typ,
                    method,
                    ffi_prefix,
                    item_type,
                    &data_enum_names,
                    &opaque_names,
                    value_only_types,
                    &ffi_enum_names,
                    &ffi_param_enum_names,
                ));
                body.push_str("\n\n");
                continue;
            }
            if ffi_exclude_functions.contains(&method.name) {
                continue;
            }
            if signature_references_excluded_type(&method.params, &method.return_type, exclude_types) {
                continue;
            }
            if uses_ffi_enum_type(
                &method.params,
                &method.return_type,
                &ffi_enum_names,
                &ffi_param_enum_names,
                &opaque_names,
            ) {
                continue;
            }
            body.push_str(&gen_method_wrapper(
                typ,
                method,
                ffi_prefix,
                &opaque_names,
                value_only_types,
                &ffi_enum_names,
                &ffi_param_enum_names,
            ));
            body.push_str("\n\n");
        }
    }

    let has_opaque_types = api.types.iter().any(|t| t.is_opaque);
    let has_sync_functions = api.functions.iter().any(|f| !f.is_async);
    let has_non_static_methods = api.types.iter().any(|t| t.methods.iter().any(|m| !m.is_static));
    let needs_json_and_unsafe = has_sync_functions || has_non_static_methods;

    let mut imports = vec!["fmt"];
    if needs_json_and_unsafe {
        imports.insert(0, "encoding/json");
        let has_runtime_usage = body.lines().any(|line| {
            if let Some(code_part) = line.split("//").next() {
                code_part.contains("runtime.")
            } else {
                false
            }
        });
        if has_runtime_usage {
            imports.push("runtime");
        }
        imports.push("unsafe");
    } else if has_opaque_types {
        imports.push("unsafe");
    }
    if !api.errors.is_empty() {
        imports.insert(1.min(imports.len()), "errors");
    }
    let capsule_emitted = go_capsule_types.values().any(|c| !c.package.is_empty())
        && api
            .functions
            .iter()
            .any(|f| go_capsule_return_config(f, &go_capsule_types).is_some());
    let mut capsule_imports: Vec<(Option<&str>, &str)> = Vec::new();
    if capsule_emitted {
        if !imports.contains(&"unsafe") {
            imports.push("unsafe");
        }
        for cfg in go_capsule_types.values().filter(|c| !c.package.is_empty()) {
            let path = cfg.package.as_str();
            if capsule_imports.iter().any(|(_, p)| *p == path) {
                continue;
            }
            capsule_imports.push((go_capsule_import_alias(&cfg.host_type), path));
        }
    }
    let mut import_lines: Vec<String> = imports.iter().map(|p| format!("\"{p}\"")).collect();
    for (alias, path) in &capsule_imports {
        let line = match alias {
            Some(alias) => format!("{alias} \"{path}\""),
            None => format!("\"{path}\""),
        };
        if !import_lines.contains(&line) {
            import_lines.push(line);
        }
    }
    let imports_str = crate::backends::go::template_env::render(
        "imports_basic.jinja",
        minijinja::context! {
            imports => import_lines,
        },
    );

    let mut out = String::with_capacity(header.len() + imports_str.len() + body.len());
    out.push_str(&header);
    out.push_str(&imports_str);
    out.push_str(&body);

    out
}
