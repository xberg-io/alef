use crate::backends::swift::naming::swift_rust_shim_ident as swift_ident;
use crate::core::backend::GeneratedFile;
use crate::core::config::{BridgeBinding, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::HashSet;

pub(super) fn emit_inbound_box_files(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    rust_bridge_dir: &std::path::Path,
) -> Vec<GeneratedFile> {
    let mut files = Vec::new();
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.bind_via != BridgeBinding::OptionsField {
            continue;
        }
        if bridge_cfg.exclude_languages.iter().any(|l| l == "swift") {
            continue;
        }
        let trait_name = &bridge_cfg.trait_name;

        let Some(trait_def) = api.types.iter().find(|t| t.is_trait && t.name == *trait_name) else {
            continue;
        };

        let box_name = format!("Swift{trait_name}Box");
        let delegate_protocol_name = format!("_Swift{trait_name}BoxDelegate");

        let mut content = String::new();
        content.push_str(&crate::backends::swift::template_env::render(
            "swift_inbound_box_preamble.swift.jinja",
            minijinja::context! { trait_name => trait_name, },
        ));
        content.push_str(&crate::backends::swift::template_env::render(
            "swift_inbound_box_delegate_protocol_open.swift.jinja",
            minijinja::context! {
                box_name => &box_name,
                trait_name => trait_name,
                delegate_protocol_name => &delegate_protocol_name,
            },
        ));
        for method in &trait_def.methods {
            let method_camel = swift_ident(&method.name.to_lower_camel_case());
            let delegate_params = swift_box_params(method);
            content.push_str(&crate::backends::swift::template_env::render(
                "swift_inbound_box_delegate_method.swift.jinja",
                minijinja::context! {
                    method_name => &method_camel,
                    params => &delegate_params,
                },
            ));
        }
        content.push_str("}\n\n");
        content.push_str(&crate::backends::swift::template_env::render(
            "swift_inbound_box_class_open.swift.jinja",
            minijinja::context! {
                box_name => &box_name,
                delegate_protocol_name => &delegate_protocol_name,
            },
        ));

        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let shim_name = format!("alef_{method_snake}");
            let box_params = swift_box_params_keyword(method);
            let delegate_call_args = swift_box_delegate_call_args(method);
            let method_camel = swift_ident(&method.name.to_lower_camel_case());
            content.push_str(&crate::backends::swift::template_env::render(
                "swift_inbound_box_method.swift.jinja",
                minijinja::context! {
                    shim_name => &shim_name,
                    params => &box_params,
                    method_name => &method_camel,
                    args => &delegate_call_args,
                },
            ));
        }
        content.push_str("}\n");

        files.push(GeneratedFile {
            path: rust_bridge_dir.join(format!("Swift{trait_name}Box.swift")),
            content,
            generated_header: false,
        });
    }
    files
}

pub(in crate::backends::swift) fn emit_function_param_box_files(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    rust_bridge_dir: &std::path::Path,
    excluded_types: &HashSet<String>,
) -> Vec<GeneratedFile> {
    use crate::backends::swift::gen_bindings::plugin_marshal::{
        swift_shim_param_decode, swift_shim_param_ffi_type, swift_shim_return_ffi_type, swift_shim_return_marshal,
    };

    let mut files = Vec::new();
    let helpers_content = format!(
        "{}\n{}",
        crate::core::hash::SELF_MARKING_HEADER_LINE,
        r#"// swift-format-ignore-file

import Foundation
import RustBridge

// MARK: - JSON Envelope

/// JSON envelope for Box method returns. Carries `Ok(T)` as `{"ok": <serialised T>}`
/// and `Err(String)` as `{"err": "<message>"}`.
///
/// Visibility is `internal` (default) — sibling Box files in the same `RustBridge` SwiftPM
/// target call these helpers. `private` would scope them file-local and break that.
enum InboundEnvelope<T: Encodable>: Encodable {
  case ok(T)
  case err(String)

  enum CodingKeys: String, CodingKey { case ok, err }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    switch self {
    case .ok(let value): try container.encode(value, forKey: .ok)
    case .err(let message): try container.encode(message, forKey: .err)
    }
  }
}

/// Encode a successful `()` result as `{"ok":null}`.
func encodeOkVoidEnvelope() -> String {
  return "{\"ok\":null}"
}

/// Encode a successful `T: Encodable` result as `{"ok": <T>}`.
func encodeOkEnvelope<T: Encodable>(_ value: T) -> String {
  do {
    let payload = InboundEnvelope.ok(value)
    let data = try JSONEncoder().encode(payload)
    return String(data: data, encoding: .utf8) ?? "{\"err\":\"swift: invalid utf8 in envelope\"}"
  } catch {
    return encodeErrEnvelope("swift: failed to encode ok envelope: \(error)")
  }
}

/// Encode a failure as `{"err": "<message>"}`.
func encodeErrEnvelope(_ message: String) -> String {
  let escaped = message.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(
    of: "\"", with: "\\\"")
  return "{\"err\":\"\(escaped)\"}"
}

/// Decode a JSON-encoded payload into a `Decodable` type.
func decodeJson<T: Decodable>(_ json: String, as type: T.Type) throws -> T {
  let data = json.data(using: .utf8) ?? Data()
  return try JSONDecoder().decode(type, from: data)
}
"#
    );

    files.push(GeneratedFile {
        path: rust_bridge_dir.join("ZSwiftPluginHelpers.swift"),
        content: helpers_content.to_string(),
        generated_header: false,
    });

    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.bind_via != BridgeBinding::FunctionParam {
            continue;
        }
        if bridge_cfg.exclude_languages.iter().any(|l| l == "swift") {
            continue;
        }
        let trait_name = &bridge_cfg.trait_name;

        let Some(trait_def) = api.types.iter().find(|t| t.is_trait && t.name == *trait_name) else {
            continue;
        };

        let box_name = format!("Swift{trait_name}Box");
        let bridge_protocol_name = format!("Swift{trait_name}Bridge");

        let mut content = String::new();
        content.push_str(crate::core::hash::SELF_MARKING_HEADER_LINE);
        content.push('\n');
        content.push_str("// swift-format-ignore-file\n");
        content.push_str("// This file contains generated FFI glue for plugin Box classes.\n\n");
        content.push_str("import Foundation\n");
        content.push_str("import RustBridge\n\n");

        content.push_str(&crate::backends::swift::template_env::render(
            "swift_function_param_box_class_open.swift.jinja",
            minijinja::context! {
                box_name => &box_name,
                bridge_protocol_name => &bridge_protocol_name,
            },
        ));
        content.push_str(&crate::backends::swift::template_env::render(
            "swift_function_param_box_plugin_shims.swift.jinja",
            minijinja::context! {},
        ));

        let bridge_exclude_types =
            crate::backends::swift::gen_bindings::trait_bridge::excluded_named_type_bridge_policy(
                trait_def,
                excluded_types,
            );

        for method in &trait_def.methods {
            let method_snake = method.name.to_snake_case();
            let shim_name = format!("alef_{method_snake}");
            let method_camel = swift_ident(&method.name.to_lower_camel_case());

            let mut param_sig = String::new();
            for (idx, param) in method.params.iter().enumerate() {
                if idx > 0 {
                    param_sig.push_str(", ");
                }
                let external = param.name.to_snake_case();
                let internal = swift_ident(&param.name.to_lower_camel_case());
                let ffi_type = swift_shim_param_ffi_type(&param.ty, param.optional);
                param_sig.push_str(&crate::backends::swift::template_env::render(
                    "swift_function_param_box_param_signature.swift.jinja",
                    minijinja::context! {
                        external => &external,
                        internal => &internal,
                        ffi_type => &ffi_type,
                        uses_external_label => external != internal,
                    },
                ));
            }

            let return_ffi_type = swift_shim_return_ffi_type(method);

            content.push_str(&crate::backends::swift::template_env::render(
                "swift_function_param_box_method_open.swift.jinja",
                minijinja::context! {
                    shim_name => &shim_name,
                    params => &param_sig,
                    return_type => &return_ffi_type,
                },
            ));

            let mut setup_lines = Vec::new();
            let mut call_args = Vec::new();
            for param in &method.params {
                let param_camel = swift_ident(&param.name.to_lower_camel_case());
                let decode = swift_shim_param_decode(&param_camel, &param.ty, param.optional, &bridge_exclude_types);
                setup_lines.extend(decode.setup);
                call_args.push(format!("{}: {}", param.name.to_lower_camel_case(), decode.expr));
            }

            let has_throwing_param = setup_lines.iter().any(|s| s.contains("JSONDecoder"));
            let method_throws = method.error_type.is_some();
            if has_throwing_param {
                let mut setup_body = String::new();
                for setup_line in &setup_lines {
                    setup_body.push_str(&crate::backends::swift::template_env::render(
                        "swift_function_param_box_setup_line.swift.jinja",
                        minijinja::context! {
                            indent => "            ",
                            line => setup_line,
                        },
                    ));
                }

                let call_args_str = call_args.join(", ");
                let bridge_call = format!("bridge.{method_camel}({call_args_str})");
                let body = if method_throws {
                    match &method.return_type {
                        TypeRef::Unit => crate::backends::swift::template_env::render(
                            "swift_function_param_box_throwing_unit.swift.jinja",
                            minijinja::context! {
                                indent => "            ",
                                bridge_call => &bridge_call,
                            },
                        ),
                        _ => crate::backends::swift::template_env::render(
                            "swift_function_param_box_throwing_result.swift.jinja",
                            minijinja::context! {
                                indent => "            ",
                                bridge_call => &bridge_call,
                            },
                        ),
                    }
                } else {
                    let return_lines = swift_shim_return_marshal(method, &bridge_call);
                    let mut body = String::new();
                    for line in return_lines {
                        body.push_str(&crate::backends::swift::template_env::render(
                            "swift_function_param_box_setup_line.swift.jinja",
                            minijinja::context! {
                                indent => "            ",
                                line => &line,
                            },
                        ));
                    }
                    body
                };
                content.push_str(&crate::backends::swift::template_env::render(
                    "swift_function_param_box_catching_body.swift.jinja",
                    minijinja::context! {
                        setup_lines => setup_body,
                        body => body,
                    },
                ));
            } else {
                for setup_line in &setup_lines {
                    content.push_str(&crate::backends::swift::template_env::render(
                        "swift_function_param_box_setup_line.swift.jinja",
                        minijinja::context! {
                            indent => "        ",
                            line => setup_line,
                        },
                    ));
                }

                let call_args_str = call_args.join(", ");
                let bridge_call = format!("bridge.{method_camel}({call_args_str})");
                let return_lines = swift_shim_return_marshal(method, &bridge_call);
                for line in return_lines {
                    content.push_str(&crate::backends::swift::template_env::render(
                        "swift_function_param_box_setup_line.swift.jinja",
                        minijinja::context! {
                            indent => "        ",
                            line => &line,
                        },
                    ));
                }
            }

            content.push_str(&crate::backends::swift::template_env::render(
                "swift_function_param_box_method_close.swift.jinja",
                minijinja::context! {},
            ));
        }

        content.push_str("}\n");
        files.push(GeneratedFile {
            path: rust_bridge_dir.join(format!("Swift{trait_name}Box.swift")),
            content,
            generated_header: false,
        });
    }

    files
}

pub(super) fn swift_box_ffi_type(ty: &TypeRef, optional: bool) -> String {
    use crate::core::ir::PrimitiveType;
    let inner = match ty {
        TypeRef::String | TypeRef::Named(_) | TypeRef::Path | TypeRef::Json | TypeRef::Map(_, _) => {
            "RustString".to_string()
        }
        TypeRef::Optional(inner) => return format!("{}?", swift_box_ffi_type(inner, false)),
        TypeRef::Vec(inner) => format!("RustVec<{}>", swift_box_ffi_type(inner, false)),
        TypeRef::Primitive(PrimitiveType::Usize) | TypeRef::Primitive(PrimitiveType::Isize) => "UInt".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "Bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "UInt32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "UInt64".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "Int32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "Int64".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "Float".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "Double".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "UInt8".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "Int8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "UInt16".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "Int16".to_string(),
        TypeRef::Bytes => "RustVec<UInt8>".to_string(),
        TypeRef::Char => "Character".to_string(),
        TypeRef::Duration => "Double".to_string(),
        TypeRef::Unit => "Void".to_string(),
    };
    if optional { format!("{inner}?") } else { inner }
}

pub(super) fn swift_box_params(method: &crate::core::ir::MethodDef) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let name = swift_ident(&p.name.to_lower_camel_case());
            let ty = swift_box_ffi_type(&p.ty, p.optional);
            format!("_ {name}: {ty}")
        })
        .collect();
    params.join(", ")
}

pub(super) fn swift_box_params_keyword(method: &crate::core::ir::MethodDef) -> String {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let external = p.name.to_snake_case();
            let internal = swift_ident(&p.name.to_lower_camel_case());
            let ty = swift_box_ffi_type(&p.ty, p.optional);
            if external == internal {
                format!("{internal}: {ty}")
            } else {
                format!("{external} {internal}: {ty}")
            }
        })
        .collect();
    params.join(", ")
}

pub(super) fn swift_adapter_conversions(
    method: &crate::core::ir::MethodDef,
    exclude_types: &HashSet<String>,
) -> (Vec<String>, String) {
    let mut setup_lines: Vec<String> = Vec::new();
    let call_args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let snake = swift_ident(&p.name.to_lower_camel_case());
            match &p.ty {
                TypeRef::Named(name) if exclude_types.contains(name) => {
                    if p.optional {
                        format!("{snake}?.toString()")
                    } else {
                        format!("{snake}.toString()")
                    }
                }
                TypeRef::Named(name) => {
                    let from_json = format!("{name}FromJson").to_lower_camel_case();
                    let local = format!("{snake}Decoded");
                    if p.optional {
                        setup_lines.push(format!(
                            "let {local}: {name}? = {snake}.flatMap {{ try? {from_json}($0.toString()) }}"
                        ));
                    } else {
                        // The primary decode uses the caller's JSON; "{}" is only a fallback for
                        // types whose fields are all optional/defaulted. A bare `try!` on that
                        // fallback would crash the whole process the day the type gains a
                        // required field, so both attempts are `try?` and a failed pair returns
                        // an empty result instead of aborting. ~keep
                        setup_lines.push(format!(
                            "let {local}: {name}? = (try? {from_json}({snake}.toString())) ?? (try? {from_json}(\"{{}}\"))"
                        ));
                        setup_lines.push(format!("guard let {local} = {local} else {{ return \"{{}}\" }}"));
                    }
                    local
                }
                TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Map(_, _) => {
                    if p.optional {
                        format!("{snake}?.toString()")
                    } else {
                        format!("{snake}.toString()")
                    }
                }
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(name) if exclude_types.contains(name) => format!("{snake}.flatMap {{ $0.toString() }}"),
                    TypeRef::Named(name) => {
                        let from_json = format!("{name}FromJson").to_lower_camel_case();
                        let local = format!("{snake}Decoded");
                        setup_lines.push(format!(
                            "let {local}: {name}? = {snake}.flatMap {{ try? {from_json}($0.toString()) }}"
                        ));
                        local
                    }
                    TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Map(_, _) => {
                        format!("{snake}?.toString()")
                    }
                    _ => snake,
                },
                _ => snake,
            }
        })
        .collect();
    (setup_lines, call_args.join(", "))
}

pub(super) fn swift_box_delegate_call_args(method: &crate::core::ir::MethodDef) -> String {
    method
        .params
        .iter()
        .map(|p| swift_ident(&p.name.to_lower_camel_case()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{MethodDef, ParamDef};

    /// Regression test: a required `Named`-type param used to fall back to `try!` when both
    /// the caller's JSON and the "{}" placeholder failed to decode, crashing the whole process
    /// the moment the type gained a required field. The fallback must degrade to an empty
    /// result instead of aborting, and `try!` must never appear in the generated conversion. ~keep
    #[test]
    fn required_named_param_decode_failure_returns_early_instead_of_crashing() {
        let method = MethodDef {
            name: "visit_text".to_string(),
            params: vec![ParamDef {
                name: "ctx".to_string(),
                ty: TypeRef::Named("NodeContext".to_string()),
                ..ParamDef::default()
            }],
            ..MethodDef::default()
        };

        let (setup_lines, call_args) = swift_adapter_conversions(&method, &HashSet::new());
        let rendered = setup_lines.join("\n");

        assert!(
            !rendered.contains("try!"),
            "a force-try can crash the process on a benign decode failure: {rendered}"
        );
        assert!(
            rendered.contains("guard let ctxDecoded = ctxDecoded else { return \"{}\" }"),
            "expected a guarded early return on total decode failure, got:\n{rendered}"
        );
        assert!(
            rendered.contains("let ctxDecoded: NodeContext? = (try? nodeContextFromJson(ctx.toString())) ?? \
                                (try? nodeContextFromJson(\"{}\"))"),
            "expected both decode attempts to use `try?`, got:\n{rendered}"
        );
        assert_eq!(call_args, "ctxDecoded");
    }
}
