use crate::backends::swift::gen_bindings::boxes::{swift_adapter_conversions, swift_box_ffi_type, swift_box_params};
use crate::backends::swift::naming::{swift_rust_shim_ident as swift_ident, swift_source_ident as swift_case_ident};
use crate::core::backend::GeneratedFile;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use std::collections::HashSet;
use std::path::PathBuf;

pub(super) fn find_swift_bridge_out_dir(binding_crate_name: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let workspace_root = std::iter::once(cwd.clone())
        .chain(cwd.ancestors().skip(1).map(|p| p.to_path_buf()))
        .take(8)
        .find(|p| p.join("Cargo.lock").exists())?;
    let target = workspace_root.join("target");

    let crate_prefix = format!("{binding_crate_name}-");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;

    for profile in ["release", "debug"] {
        let build_dir = target.join(profile).join("build");
        let entries = match std::fs::read_dir(&build_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with(&crate_prefix) {
                continue;
            }
            let out = entry.path().join("out");
            let marker = out.join("SwiftBridgeCore.swift");
            if !marker.exists() {
                continue;
            }
            let mtime = std::fs::metadata(&marker)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                best = Some((mtime, out));
            }
        }
    }
    best.map(|(_, p)| p)
}

pub(super) fn emit_swift_bridge_files(
    crate_name: &str,
    binding_crate_name: &str,
    package_root: &std::path::Path,
) -> anyhow::Result<Option<Vec<GeneratedFile>>> {
    let out_dir = match find_swift_bridge_out_dir(binding_crate_name) {
        Some(d) => d,
        None => {
            let sources_rust_bridge_c = package_root.join("Sources").join("RustBridgeC");
            let header_path = sources_rust_bridge_c.join("RustBridgeC.h");

            if let Ok(existing) = std::fs::read_to_string(&header_path) {
                if existing.contains("Concatenates SwiftBridgeCore.h") {
                    return Ok(None);
                }
            }
            let minimal_header = format!(
                "#ifndef RUST_BRIDGE_C_H\n\
                 #define RUST_BRIDGE_C_H\n\
                 \n\
                 // Placeholder header for the RustBridgeC SwiftPM target.\n\
                 // Run `cargo build -p {binding_crate_name}` and re-run `alef generate` to populate.\n\
                 // The typedefs below are the minimum required for SwiftBridgeCore.swift\n\
                 // to compile before the full cargo build has been run.\n\
                 \n\
                 #include <stdint.h>\n\
                 #include <stdbool.h>\n\
                 \n\
                 typedef struct RustStr {{ uint8_t* const start; uintptr_t len; }} RustStr;\n\
                 typedef struct __private__FfiSlice {{ void* const start; uintptr_t len; }} __private__FfiSlice;\n\
                 typedef struct __private__OptionU8 {{ uint8_t val; bool is_some; }} __private__OptionU8;\n\
                 typedef struct __private__OptionI8 {{ int8_t val; bool is_some; }} __private__OptionI8;\n\
                 typedef struct __private__OptionU16 {{ uint16_t val; bool is_some; }} __private__OptionU16;\n\
                 typedef struct __private__OptionI16 {{ int16_t val; bool is_some; }} __private__OptionI16;\n\
                 typedef struct __private__OptionU32 {{ uint32_t val; bool is_some; }} __private__OptionU32;\n\
                 typedef struct __private__OptionI32 {{ int32_t val; bool is_some; }} __private__OptionI32;\n\
                 typedef struct __private__OptionU64 {{ uint64_t val; bool is_some; }} __private__OptionU64;\n\
                 typedef struct __private__OptionI64 {{ int64_t val; bool is_some; }} __private__OptionI64;\n\
                 typedef struct __private__OptionUsize {{ uintptr_t val; bool is_some; }} __private__OptionUsize;\n\
                 typedef struct __private__OptionIsize {{ intptr_t val; bool is_some; }} __private__OptionIsize;\n\
                 typedef struct __private__OptionF32 {{ float val; bool is_some; }} __private__OptionF32;\n\
                 typedef struct __private__OptionF64 {{ double val; bool is_some; }} __private__OptionF64;\n\
                 typedef struct __private__OptionBool {{ bool val; bool is_some; }} __private__OptionBool;\n\
                 \n\
                 #endif /* RUST_BRIDGE_C_H */\n"
            );
            return Ok(Some(vec![GeneratedFile {
                path: header_path,
                content: minimal_header,
                generated_header: false,
            }]));
        }
    };

    let core_swift_src = out_dir.join("SwiftBridgeCore.swift");
    let crate_swift_src = out_dir
        .join(binding_crate_name)
        .join(format!("{binding_crate_name}.swift"));
    let core_h_src = out_dir.join("SwiftBridgeCore.h");
    let crate_h_src = out_dir.join(binding_crate_name).join(format!("{binding_crate_name}.h"));

    for p in [&core_swift_src, &crate_swift_src, &core_h_src, &crate_h_src] {
        if !p.exists() {
            return Ok(None);
        }
    }

    let core_swift = std::fs::read_to_string(&core_swift_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", core_swift_src.display()))?;
    let crate_swift = std::fs::read_to_string(&crate_swift_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", crate_swift_src.display()))?;
    let core_h = std::fs::read_to_string(&core_h_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", core_h_src.display()))?;
    let crate_h = std::fs::read_to_string(&crate_h_src)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", crate_h_src.display()))?;

    let core_swift_content = make_swift_bridge_ref_ptr_public(&append_rust_string_ref_to_string_extension(
        &add_retroactive_to_imported_protocol_conformances(&prepend_rust_bridge_c_import(&core_swift)),
    ));
    let crate_swift_content = make_swift_bridge_ref_ptr_public(&prepend_rust_bridge_c_import(&crate_swift));

    let rust_bridge_c_h = format!(
        "#ifndef RUST_BRIDGE_C_H\n\
         #define RUST_BRIDGE_C_H\n\
         \n\
         // Auto-generated by alef — do not edit by hand.\n\
         // Concatenates SwiftBridgeCore.h and {binding_crate_name}.h produced by\n\
         // `cargo build -p {binding_crate_name}` via swift_bridge_build.\n\
         \n\
         {core_h}\n\
         {crate_h}\n\
         #endif /* RUST_BRIDGE_C_H */\n"
    );

    let sources_rust_bridge = package_root.join("Sources").join("RustBridge");
    let sources_rust_bridge_c = package_root.join("Sources").join("RustBridgeC");
    let _ = crate_name;
    let files = vec![
        GeneratedFile {
            path: sources_rust_bridge.join("SwiftBridgeCore.swift"),
            content: core_swift_content,
            generated_header: false,
        },
        GeneratedFile {
            path: sources_rust_bridge.join(format!("{binding_crate_name}.swift")),
            content: crate_swift_content,
            generated_header: false,
        },
        GeneratedFile {
            path: sources_rust_bridge_c.join("RustBridgeC.h"),
            content: rust_bridge_c_h,
            generated_header: false,
        },
    ];
    Ok(Some(files))
}

pub(super) fn emit_inbound_protocols(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_types: &HashSet<String>,
    out: &mut String,
) {
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        if bridge_cfg.exclude_languages.iter().any(|l| l == "swift") {
            continue;
        }
        let trait_name = &bridge_cfg.trait_name;
        let type_alias = match bridge_cfg.type_alias.as_deref() {
            Some(a) => a,
            None => continue,
        };
        let Some(options_type) = bridge_cfg.options_type.as_deref() else {
            continue;
        };
        let Some(field) = bridge_cfg.resolved_options_field() else {
            continue;
        };
        let result_type_name = bridge_cfg.result_type.as_deref();
        let protocol_return_type = result_type_name.unwrap_or("Void");

        let Some(trait_def) = api.types.iter().find(|t| t.is_trait && t.name == *trait_name) else {
            continue;
        };

        let result_enum = result_type_name.and_then(|name| api.enums.iter().find(|e| e.name == name));
        let box_name = format!("Swift{trait_name}Box");
        let adapter_name = format!("_{trait_name}ProtocolAdapter");
        let protocol_name = format!("{trait_name}Protocol");
        let delegate_protocol_name = format!("_Swift{trait_name}BoxDelegate");
        let factory_fn = format!(
            "make{}{}",
            trait_name.to_upper_camel_case(),
            type_alias.to_upper_camel_case()
        );

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_protocol_open.swift.jinja",
            minijinja::context! { protocol_name => &protocol_name, },
        ));
        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let method_camel = method_snake.to_lower_camel_case();
            let params = swift_protocol_params(method, exclude_types);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_protocol_method.swift.jinja",
                minijinja::context! {
                    method_name => &method_camel,
                    params => &params,
                    return_type => protocol_return_type,
                },
            ));
        }
        out.push_str("}\n\n");

        let default_case = result_enum
            .and_then(|en| en.variants.iter().find(|v| v.fields.is_empty()))
            .map(|v| swift_case_ident(&v.name.to_lower_camel_case()));
        let default_case_doc = default_case.as_ref().map(|case| format!(".{case}"));
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_protocol_default_open.swift.jinja",
            minijinja::context! {
                protocol_name => &protocol_name,
                default_case => default_case_doc.as_deref(),
            },
        ));
        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let method_camel = method_snake.to_lower_camel_case();
            let underscore_params = swift_protocol_underscore_params(method, exclude_types);
            let (return_type, body) = if let Some(default_case) = &default_case {
                (Some(protocol_return_type), format!("return .{default_case}"))
            } else {
                (None, String::new())
            };
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_protocol_default_method.swift.jinja",
                minijinja::context! {
                    method_name => &method_camel,
                    params => &underscore_params,
                    return_type => return_type,
                    body => &body,
                },
            ));
        }
        out.push_str("}\n\n");

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_adapter_open.swift.jinja",
            minijinja::context! {
                adapter_name => &adapter_name,
                delegate_protocol_name => &delegate_protocol_name,
                protocol_name => &protocol_name,
            },
        ));
        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let method_camel = method_snake.to_lower_camel_case();
            let delegate_method = swift_ident(&method_camel);
            let delegate_params = swift_box_params(method);
            let (conversion_lines, call_args) = swift_adapter_conversions(method, exclude_types);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_adapter_method_open.swift.jinja",
                minijinja::context! {
                    method_name => &delegate_method,
                    params => &delegate_params,
                },
            ));
            for line in &conversion_lines {
                out.push_str(&crate::backends::swift::template_env::render(
                    "swift_forwarder_conversion_line.swift.jinja",
                    minijinja::context! { line => line, },
                ));
            }
            let result_json = if let Some(result_type_name) = result_type_name.filter(|_| result_enum.is_some()) {
                format!(
                    "        return {}_toJson(inner.{method_camel}({call_args}))\n",
                    result_type_name.to_snake_case()
                )
            } else {
                let call = if call_args.is_empty() {
                    format!("inner.{method_camel}()")
                } else {
                    format!("inner.{method_camel}({call_args})")
                };
                crate::backends::swift::template_env::render(
                    "swift_bridge_adapter_void_return.swift.jinja",
                    minijinja::context! { call => &call, },
                )
            };
            out.push_str(&result_json);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_adapter_method_close.swift.jinja",
                minijinja::context! {},
            ));
        }
        out.push_str("}\n\n");

        if let Some(en) = result_enum {
            let result_type_name = en.name.as_str();
            let fn_name = format!("{}_toJson", result_type_name.to_snake_case());
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_result_helper_open.swift.jinja",
                minijinja::context! {
                    result_type_name => result_type_name,
                    function_name => &fn_name,
                },
            ));
            for variant in &en.variants {
                let variant_name = &variant.name;
                let swift_case = swift_case_ident(&variant_name.to_lower_camel_case());
                if variant.fields.is_empty() {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "swift_bridge_result_unit_case.swift.jinja",
                        minijinja::context! {
                            swift_case => &swift_case,
                            variant_name => variant_name,
                        },
                    ));
                } else if variant.is_tuple && variant.fields.len() == 1 {
                    out.push_str(&crate::backends::swift::template_env::render(
                        "swift_bridge_result_newtype_case.swift.jinja",
                        minijinja::context! {
                            swift_case => &swift_case,
                            variant_name => variant_name,
                        },
                    ));
                }
            }
            out.push_str("    }\n}\n\n");
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_bridge_json_escape_helper.swift.jinja",
                minijinja::context! {},
            ));
            out.push('\n');
        }

        let opts_snake = options_type.to_snake_case();
        let options_fn = format!("{opts_snake}FromJsonWith{}", field.to_upper_camel_case()).to_lower_camel_case();
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_factory.swift.jinja",
            minijinja::context! {
                protocol_name => &protocol_name,
                type_alias => type_alias,
                options_fn => &options_fn,
                factory_fn => &factory_fn,
                box_name => &box_name,
                adapter_name => &adapter_name,
            },
        ));
        out.push('\n');

        out.push_str(&crate::backends::swift::template_env::render(
            "swift_bridge_options_forwarder.swift.jinja",
            minijinja::context! {
                options_type => options_type,
                type_alias => type_alias,
                options_fn => &options_fn,
                field => &field,
            },
        ));
        out.push('\n');
    }
}

pub(super) fn already_emitted_top_level_names(api: &ApiSurface) -> HashSet<String> {
    let mut names: HashSet<String> = HashSet::new();
    for func in &api.functions {
        if func.is_async {
            continue;
        }
        let first = func.params.first().map(|p| &p.ty);
        let is_bytes_or_path = matches!(first, Some(TypeRef::Bytes) | Some(TypeRef::Path));
        if !is_bytes_or_path {
            continue;
        }
        if convenience_name_shadows_bridge(func) {
            continue;
        }
        let swift_inner = swift_ident(&func.name.to_lower_camel_case());
        let wrapper_name = if swift_inner.ends_with("Sync") {
            swift_inner[..swift_inner.len() - 4].to_string()
        } else {
            swift_inner
        };
        names.insert(wrapper_name);
    }
    names
}

pub(super) fn emit_ref_property_extensions(api: &ApiSurface) -> Option<(String, String)> {
    let eligible_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && !t.methods.is_empty())
        .collect();

    if eligible_types.is_empty() {
        return None;
    }

    let mut content = String::new();
    content.push_str("import RustBridge\n\n");
    content.push_str("// MARK: - Property-access ergonomics for e2e tests\n");
    content.push_str("//\n");
    content.push_str("// This file provides computed-property aliases for methods on swift-bridge-generated types,\n");
    content.push_str("// allowing callers to write `result.mimeType` rather than `result.mimeType()`.\n");
    content.push_str("// These extensions are especially useful in e2e test assertions where the alef\n");
    content.push_str("// fixture generator emits property-access syntax.\n");
    content.push_str("//\n");
    content.push_str("// Although these are primarily for test convenience, they are part of the public API\n");
    content.push_str("// and can be used in production code for more ergonomic access to generated ref types.\n");

    let mut has_any_extensions = false;

    for ty in eligible_types {
        let mut type_has_extensions = false;
        let mut type_content = String::new();
        for method in &ty.methods {
            if method.is_async || method.is_static || method.binding_excluded {
                continue;
            }
            if !matches!(&method.return_type, TypeRef::String) || method.params.is_empty() {
                continue;
            }
            if !method.params.iter().all(|p| is_extension_param_bridgeable(&p.ty, api)) {
                continue;
            }

            if !type_has_extensions {
                type_content.push('\n');
                type_content.push_str(&crate::backends::swift::template_env::render(
                    "swift_ref_extension_open.swift.jinja",
                    minijinja::context! { type_name => &ty.name, },
                ));
                type_has_extensions = true;
            } else {
                type_content.push('\n');
            }

            let camel = method.name.to_lower_camel_case();
            type_content.push_str(&crate::backends::swift::template_env::render(
                "swift_ref_string_alias_property.swift.jinja",
                minijinja::context! {
                    method_name => &camel,
                    property_name => &camel,
                },
            ));
        }
        if type_has_extensions {
            type_content.push_str("}\n");
            type_content.push_str(&crate::backends::swift::template_env::render(
                "swift_ref_extension_inheritance_comment.swift.jinja",
                minijinja::context! {
                    type_name => &ty.name,
                },
            ));
            content.push_str(&type_content);
            has_any_extensions = true;
        }
    }

    if has_any_extensions {
        Some(("RustBridgeRefExtensions.swift".to_string(), content))
    } else {
        None
    }
}

fn convenience_name_shadows_bridge(func: &FunctionDef) -> bool {
    let swift_name = swift_ident(&func.name.to_lower_camel_case());
    let wrapper_name = if swift_name.ends_with("Sync") {
        swift_name[..swift_name.len() - 4].to_string()
    } else {
        swift_name.clone()
    };
    wrapper_name == swift_name
}

fn swift_protocol_params(method: &MethodDef, exclude_types: &HashSet<String>) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let name = p.name.to_lower_camel_case();
            let ty = swift_inbound_type(&p.ty, p.optional, exclude_types);
            format!("_ {name}: {ty}")
        })
        .collect();
    params.join(", ")
}

fn swift_protocol_underscore_params(method: &MethodDef, exclude_types: &HashSet<String>) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let name = p.name.to_lower_camel_case();
            let ty = swift_inbound_type(&p.ty, p.optional, exclude_types);
            format!("_ _{name}: {ty}")
        })
        .collect();
    params.join(", ")
}

fn swift_inbound_type(ty: &TypeRef, optional: bool, exclude_types: &HashSet<String>) -> String {
    use crate::core::ir::PrimitiveType;
    let inner = match ty {
        TypeRef::Named(name) if exclude_types.contains(name) => "String".to_string(),
        TypeRef::Named(name) => name.clone(),
        TypeRef::String => "String".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "Bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "Int32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "Int64".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "Int".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "Int".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "Float".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "Double".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "Int8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "Int16".to_string(),
        TypeRef::Vec(inner) => format!("RustVec<{}>", swift_box_ffi_type(inner, false)),
        TypeRef::Optional(inner) => return format!("{}?", swift_inbound_type(inner, false, exclude_types)),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Bytes => "RustVec<UInt8>".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "Double".to_string(),
        TypeRef::Map(_, _) => "String".to_string(),
    };
    if optional { format!("{inner}?") } else { inner }
}

fn append_rust_string_ref_to_string_extension(content: &str) -> String {
    const MARKER: &str = "// alef: RustStringRef.toString() shim";
    if let Some(idx) = content.find(MARKER) {
        let mut head = content[..idx].to_string();
        while head.ends_with('\n') {
            head.pop();
        }
        head.push('\n');
        head
    } else {
        content.to_string()
    }
}

fn make_swift_bridge_ref_ptr_public(content: &str) -> String {
    content
        .replace(
            "    var ptr: UnsafeMutableRawPointer",
            "    public var ptr: UnsafeMutableRawPointer",
        )
        .replace("    var isOwned: Bool = true", "    public var isOwned: Bool = true")
}

fn add_retroactive_to_imported_protocol_conformances(content: &str) -> String {
    const TARGETS: &[(&str, &str)] = &[
        (
            "extension RustStr: Identifiable",
            "extension RustStr: @retroactive Identifiable",
        ),
        (
            "extension RustStr: Equatable",
            "extension RustStr: @retroactive Equatable",
        ),
    ];
    let mut out = content.to_string();
    for (from, to) in TARGETS {
        out = out.replace(from, to);
    }
    out
}

fn prepend_rust_bridge_c_import(content: &str) -> String {
    const IMPORT: &str = "import RustBridgeC";
    const IGNORE: &str = "// swift-format-ignore-file";
    let head: Vec<&str> = content.lines().take(5).collect();
    let has_import = head.iter().any(|l| l.trim() == IMPORT);
    let has_ignore = head.iter().any(|l| l.trim() == IGNORE);
    match (has_import, has_ignore) {
        (true, true) => content.to_string(),
        (true, false) => format!("{IGNORE}\n{content}"),
        (false, true) => format!("{IMPORT}\n\n{content}"),
        (false, false) => format!("{IGNORE}\n{IMPORT}\n\n{content}"),
    }
}

fn is_extension_param_bridgeable(ty: &TypeRef, api: &ApiSurface) -> bool {
    match ty {
        TypeRef::Named(n) if n.starts_with("Result") || n == "Result" => false,
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Path
        | TypeRef::Bytes
        | TypeRef::Duration
        | TypeRef::Unit => true,
        TypeRef::Named(n) => {
            if let Some(enum_def) = api.enums.iter().find(|e| &e.name == n) {
                enum_def.has_serde
            } else {
                true
            }
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => is_extension_param_bridgeable(inner, api),
        TypeRef::Map(..) | TypeRef::Char | TypeRef::Json => false,
    }
}
