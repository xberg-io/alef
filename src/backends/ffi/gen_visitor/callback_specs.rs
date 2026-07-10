use crate::backends::ffi::gen_visitor::protocol::VisitorProtocol;
use crate::backends::ffi::template_env::render;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{PrimitiveType, TypeRef};

#[derive(serde::Serialize)]
pub(super) struct CallbackArgField {
    name: String,
    c_type: String,
}

/// The kind of a single callback parameter (beyond the common ctx/user_data/out
/// prefix that every callback shares).
pub(super) enum ParamKind {
    /// Required `*const c_char` — converted from `&str` via `CString::new`.
    Str(String),
    /// Optional `*const c_char` — converted from `Option<&str>` via `opt_str_to_c`.
    OptStr(String),
    /// `i32` — converted from `bool` via `i32::from`.
    Bool(String),
    /// `u32` — passed through directly.
    U32(String),
    /// `usize` — passed through directly.
    Usize(String),
    /// `*const *const c_char` + `usize` (cell_count) — special for table rows.
    CellSlice(String),
}

/// Specification for one visitor callback.
pub(crate) struct CallbackSpec {
    pub(super) name: String,
    pub(super) doc: String,
    pub(super) params: Vec<ParamKind>,
}

/// Build a `Vec<CallbackSpec>` from a trait's IR definition for the FFI backend.
///
/// Derives all FFI-specific fields (`ParamKind`) from `TypeRef` + `optional` flag.
/// Methods with unsupported parameter types are skipped with a warning.
/// Parameters whose type is `TypeRef::Named(_)` (the context threaded via FFI's
/// separate channel) are silently skipped — they do not become C parameters.
pub(crate) fn callback_specs_from_trait(
    trait_def: &crate::core::ir::TypeDef,
    bridge_cfg: Option<&TraitBridgeConfig>,
) -> Vec<CallbackSpec> {
    let Some(protocol) = VisitorProtocol::from_bridge_config("", bridge_cfg) else {
        eprintln!(
            "[alef] gen_visitor(ffi): visitor callbacks require configured context_type and result_type metadata"
        );
        return Vec::new();
    };
    let mut specs = Vec::with_capacity(trait_def.methods.len());
    'methods: for m in &trait_def.methods {
        if m.trait_source.is_some() {
            continue;
        }
        if !matches!(&m.return_type, TypeRef::Named(name) if name == &protocol.result_type) {
            eprintln!(
                "[alef] gen_visitor(ffi): skip method `{}` — visitor callbacks require `{}` return type",
                m.name, protocol.result_type
            );
            continue;
        }
        if !m
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Named(name) if name == &protocol.context_type))
        {
            eprintln!(
                "[alef] gen_visitor(ffi): skip method `{}` — visitor callbacks require `{}` parameter",
                m.name, protocol.context_type
            );
            continue;
        }
        let mut params = Vec::new();
        for p in &m.params {
            if matches!(&p.ty, TypeRef::Named(name) if name == &protocol.context_type) {
                continue;
            }
            let param_name = p.name.trim_start_matches('_').to_string();
            match (&p.ty, p.optional) {
                (TypeRef::String, false) => {
                    params.push(ParamKind::Str(param_name));
                }
                (TypeRef::String, true) => {
                    params.push(ParamKind::OptStr(param_name));
                }
                (TypeRef::Primitive(PrimitiveType::Bool), false) => {
                    params.push(ParamKind::Bool(param_name));
                }
                (
                    TypeRef::Primitive(
                        PrimitiveType::U32
                        | PrimitiveType::I32
                        | PrimitiveType::U16
                        | PrimitiveType::I16
                        | PrimitiveType::U8
                        | PrimitiveType::I8,
                    ),
                    false,
                ) => {
                    params.push(ParamKind::U32(param_name));
                }
                (TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::I64), false) => {
                    params.push(ParamKind::Usize(param_name));
                }
                (TypeRef::Vec(inner), false) => match inner.as_ref() {
                    TypeRef::String => {
                        params.push(ParamKind::CellSlice(param_name));
                    }
                    _ => {
                        eprintln!(
                            "[alef] gen_visitor(ffi): skip method `{}` — unsupported Vec param `{}`",
                            m.name, p.name
                        );
                        continue 'methods;
                    }
                },
                _ => {
                    eprintln!(
                        "[alef] gen_visitor(ffi): skip method `{}` — unsupported param `{}: {:?}`",
                        m.name, p.name, p.ty
                    );
                    continue 'methods;
                }
            }
        }
        specs.push(CallbackSpec {
            name: m.name.clone(),
            doc: m.doc.clone(),
            params,
        });
    }
    specs
}

/// Build the C `extern "C" fn(...)` signature parameters for one callback.
pub(super) fn callback_arg_fields(spec: &CallbackSpec, pascal_prefix: &str) -> Vec<CallbackArgField> {
    let mut fields = vec![
        CallbackArgField {
            name: "ctx".to_string(),
            c_type: format!("*const {pascal_prefix}Context"),
        },
        CallbackArgField {
            name: "user_data".to_string(),
            c_type: "*mut std::ffi::c_void".to_string(),
        },
    ];
    for p in &spec.params {
        match p {
            ParamKind::Str(n) | ParamKind::OptStr(n) => {
                fields.push(CallbackArgField {
                    name: n.clone(),
                    c_type: "*const std::ffi::c_char".to_string(),
                });
            }
            ParamKind::Bool(n) => fields.push(CallbackArgField {
                name: n.clone(),
                c_type: "i32".to_string(),
            }),
            ParamKind::U32(n) => fields.push(CallbackArgField {
                name: n.clone(),
                c_type: "u32".to_string(),
            }),
            ParamKind::Usize(n) => fields.push(CallbackArgField {
                name: n.clone(),
                c_type: "usize".to_string(),
            }),
            ParamKind::CellSlice(n) => {
                fields.push(CallbackArgField {
                    name: n.clone(),
                    c_type: "*const *const std::ffi::c_char".to_string(),
                });
                fields.push(CallbackArgField {
                    name: "cell_count".to_string(),
                    c_type: "usize".to_string(),
                });
            }
        }
    }
    fields.push(CallbackArgField {
        name: "out_custom".to_string(),
        c_type: "*mut *mut std::ffi::c_char".to_string(),
    });
    fields.push(CallbackArgField {
        name: "out_len".to_string(),
        c_type: "*mut usize".to_string(),
    });
    fields
}

/// Build sanitized doc lines for a callback field template.
fn callback_doc_lines(doc: &str) -> Vec<String> {
    doc.lines()
        .map(|line| line.trim_start_matches("///").trim_start().to_string())
        .collect()
}

/// Generate all `Option<unsafe extern "C" fn(...)>` struct fields.
pub(super) fn gen_struct_fields(specs: &[CallbackSpec], pascal_prefix: &str) -> String {
    let mut out = String::new();
    for spec in specs {
        let doc_lines = callback_doc_lines(&spec.doc);
        let params = callback_arg_fields(spec, pascal_prefix);
        out.push_str(&render(
            "ffi_visitor_callback_field.jinja",
            minijinja::context! {
                doc_lines,
                name => spec.name.as_str(),
                params,
            },
        ));
    }
    out
}
