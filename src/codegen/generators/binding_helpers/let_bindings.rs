use crate::core::ir::{ParamDef, TypeRef};
use ahash::AHashSet;

/// Generate let bindings for non-opaque Named params, converting them to core types.
pub fn gen_named_let_bindings_pub(params: &[ParamDef], opaque_types: &AHashSet<String>, core_import: &str) -> String {
    gen_named_let_bindings(params, opaque_types, core_import)
}

/// Like `gen_named_let_bindings_pub` but for augmented params where non-optional Named params
/// with defaults have been promoted to `Option<T>` in the binding signature.
///
/// Augmented optional params (original `optional=false`, augmented to `optional=true`) must use
/// the "promoted" template (`unwrap_or_default().into()`) rather than the "optional" template
/// (`map(Into::into)`) because the call-site still uses `&param_core` (non-optional borrow from
/// original params). The optional_ref template produces `Option<&T>` which cannot satisfy `&T`.
///
/// Naturally optional params (original `optional=true`) continue using the optional template.
pub fn gen_named_let_bindings_with_augmented(
    augmented_params: &[ParamDef],
    original_params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    gen_named_let_bindings_inner_augmented(augmented_params, original_params, opaque_types, core_import)
}

/// Like `gen_named_let_bindings_pub` but without optional-promotion semantics.
/// Use this for backends (e.g. WASM) that do not promote non-optional params to `Option<T>`.
pub fn gen_named_let_bindings_no_promote(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    gen_named_let_bindings_inner(params, opaque_types, core_import, false)
}

pub(in crate::codegen::generators) fn gen_named_let_bindings(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    gen_named_let_bindings_inner(params, opaque_types, core_import, true)
}

/// Variant of `gen_named_let_bindings` for backends where Named non-opaque params
/// are passed by reference (`&T`) in the function signature (e.g. extendr).
/// Uses `.clone().into()` instead of `.into()` to convert the borrowed value.
pub(in crate::codegen::generators) fn gen_named_let_bindings_by_ref(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut bindings = String::new();
    for (idx, p) in params.iter().enumerate() {
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                let core_type_path = format!("{core_import}::{name}");
                let binding = if p.optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/named_let_binding_by_ref_optional.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_type_path => &core_type_path,
                        },
                    )
                } else {
                    crate::codegen::template_env::render(
                        "binding_helpers/named_let_binding_by_ref_simple.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_type_path => &core_type_path,
                        },
                    )
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())) => {
                let binding = if p.optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_named_let_binding_by_ref_optional.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else {
                    let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
                    if promoted {
                        crate::codegen::template_env::render(
                            "binding_helpers/vec_named_let_binding_by_ref_promoted.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        )
                    } else {
                        crate::codegen::template_env::render(
                            "binding_helpers/vec_named_let_binding_by_ref_simple.jinja",
                            minijinja::context! {
                                name => &p.name,
                            },
                        )
                    }
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                let binding = if p.optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_string_refs_binding_optional.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_string_refs_binding_simple.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            _ => {}
        }
    }
    bindings
}

fn gen_named_let_bindings_inner(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
    promote: bool,
) -> String {
    let mut bindings = String::new();
    for (idx, p) in params.iter().enumerate() {
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                let promoted = promote && crate::codegen::shared::is_promoted_optional(params, idx);
                let core_type_path = format!("{}::{}", core_import, name);
                let binding = if p.optional {
                    if p.is_ref {
                        crate::codegen::template_env::render(
                            "binding_helpers/named_let_binding_optional_ref.jinja",
                            minijinja::context! {
                                name => &p.name,
                                core_type_path => &core_type_path,
                            },
                        )
                    } else {
                        crate::codegen::template_env::render(
                            "binding_helpers/named_let_binding_optional.jinja",
                            minijinja::context! {
                                name => &p.name,
                                core_type_path => &core_type_path,
                            },
                        )
                    }
                } else if promoted {
                    crate::codegen::template_env::render(
                        "binding_helpers/named_let_binding_promoted.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_type_path => &core_type_path,
                            is_mut => p.is_mut,
                        },
                    )
                } else {
                    crate::codegen::template_env::render(
                        "binding_helpers/named_let_binding_simple.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_type_path => &core_type_path,
                            is_mut => p.is_mut,
                        },
                    )
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())) => {
                let promoted = promote && crate::codegen::shared::is_promoted_optional(params, idx);
                let binding = if p.optional && p.is_ref {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_named_let_binding_optional.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else if p.optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_named_let_binding_optional_no_ref.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else if promoted {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_named_let_binding_promoted.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_named_let_binding_simple.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                let binding = if p.optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_string_refs_binding_optional.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_string_refs_binding_simple.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() =>
            {
                let template = if p.optional {
                    "binding_helpers/sanitized_vec_string_filter_optional.jinja"
                } else {
                    "binding_helpers/sanitized_vec_string_filter_simple.jinja"
                };
                bindings.push_str(&crate::codegen::template_env::render(
                    template,
                    minijinja::context! {
                        name => &p.name,
                    },
                ));
            }
            _ => {}
        }
    }
    bindings
}

/// Like `gen_named_let_bindings_inner` but aware of augmented params.
///
/// When `augmented_params[idx].optional = true` but `original_params[idx].optional = false`,
/// the param was augmented (it has a default). Such params must use the "promoted" template
/// (`unwrap_or_default().into()`) because the call-site emits `&param_core` (non-optional borrow
/// from the original params). The optional_ref template produces `Option<&T>` which doesn't
/// satisfy `&T`.
fn gen_named_let_bindings_inner_augmented(
    augmented_params: &[ParamDef],
    original_params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    let mut bindings = String::new();
    for (idx, p) in augmented_params.iter().enumerate() {
        let is_augmented_optional = p.optional && original_params.get(idx).map(|orig| !orig.optional).unwrap_or(false);
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                let core_type_path = format!("{}::{}", core_import, name);
                let binding = if is_augmented_optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/named_let_binding_promoted.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_type_path => &core_type_path,
                            is_mut => p.is_mut,
                        },
                    )
                } else if p.optional {
                    if p.is_ref {
                        crate::codegen::template_env::render(
                            "binding_helpers/named_let_binding_optional_ref.jinja",
                            minijinja::context! {
                                name => &p.name,
                                core_type_path => &core_type_path,
                            },
                        )
                    } else {
                        crate::codegen::template_env::render(
                            "binding_helpers/named_let_binding_optional.jinja",
                            minijinja::context! {
                                name => &p.name,
                                core_type_path => &core_type_path,
                            },
                        )
                    }
                } else {
                    let promoted = crate::codegen::shared::is_promoted_optional(augmented_params, idx);
                    if promoted {
                        crate::codegen::template_env::render(
                            "binding_helpers/named_let_binding_promoted.jinja",
                            minijinja::context! {
                                name => &p.name,
                                core_type_path => &core_type_path,
                                is_mut => p.is_mut,
                            },
                        )
                    } else {
                        crate::codegen::template_env::render(
                            "binding_helpers/named_let_binding_simple.jinja",
                            minijinja::context! {
                                name => &p.name,
                                core_type_path => &core_type_path,
                                is_mut => p.is_mut,
                            },
                        )
                    }
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())) => {
                let binding = if p.optional && p.is_ref {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_named_let_binding_optional.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else if p.optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_named_let_binding_optional_no_ref.jinja",
                        minijinja::context! {
                            name => &p.name,
                        },
                    )
                } else {
                    let promoted = crate::codegen::shared::is_promoted_optional(augmented_params, idx);
                    let template = if promoted {
                        "binding_helpers/vec_named_let_binding_promoted.jinja"
                    } else {
                        "binding_helpers/vec_named_let_binding_simple.jinja"
                    };
                    crate::codegen::template_env::render(template, minijinja::context! { name => &p.name })
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                let binding = if p.optional {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_string_refs_binding_optional.jinja",
                        minijinja::context! { name => &p.name },
                    )
                } else {
                    crate::codegen::template_env::render(
                        "binding_helpers/vec_string_refs_binding_simple.jinja",
                        minijinja::context! { name => &p.name },
                    )
                };
                bindings.push_str(&binding);
                bindings.push_str("\n    ");
            }
            _ => {}
        }
    }
    bindings
}

/// Generate serde-based let bindings for non-opaque Named params.
/// Serializes binding types to JSON and deserializes to core types.
/// Used when From impls don't exist (e.g., types with sanitized fields).
/// `indent` is the whitespace prefix for each generated line (e.g., "    " for functions, "        " for methods).
/// NOTE: This function should only be called when `cfg.has_serde` is true.
/// The caller (functions.rs, methods.rs) must gate the call behind a `has_serde` check.
pub fn gen_serde_let_bindings(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
    err_conv: &str,
    indent: &str,
) -> String {
    let mut bindings = String::new();
    for (idx, p) in params.iter().enumerate() {
        let promoted = crate::codegen::shared::is_promoted_optional(params, idx);
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                let core_path = format!("{}::{}", core_import, name);
                if p.optional {
                    bindings.push_str(&crate::codegen::template_env::render(
                        "binding_helpers/serde_named_let_binding_optional.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_path => core_path,
                            err_conv => err_conv,
                            indent => indent,
                        },
                    ));
                } else if promoted {
                    bindings.push_str(&crate::codegen::template_env::render(
                        "binding_helpers/serde_named_let_binding_promoted.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_path => core_path,
                            err_conv => err_conv,
                            indent => indent,
                        },
                    ));
                } else {
                    bindings.push_str(&crate::codegen::template_env::render(
                        "binding_helpers/serde_named_let_binding_simple.jinja",
                        minijinja::context! {
                            name => &p.name,
                            core_path => core_path,
                            err_conv => err_conv,
                            indent => indent,
                        },
                    ));
                }
            }
            TypeRef::Vec(inner) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if !opaque_types.contains(name.as_str()) {
                        let core_path = format!("{}::{}", core_import, name);
                        if p.optional {
                            bindings.push_str(&crate::codegen::template_env::render(
                                "binding_helpers/serde_vec_named_optional.jinja",
                                minijinja::context! {
                                    name => &p.name,
                                    core_path => core_path,
                                    err_conv => err_conv,
                                    indent => indent,
                                },
                            ));
                        } else {
                            bindings.push_str(&crate::codegen::template_env::render(
                                "binding_helpers/serde_vec_named_simple.jinja",
                                minijinja::context! {
                                    name => &p.name,
                                    core_path => core_path,
                                    err_conv => err_conv,
                                    indent => indent,
                                },
                            ));
                        }
                    }
                } else if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() {
                    let template = if p.optional {
                        "binding_helpers/serde_sanitized_vec_string_optional.jinja"
                    } else {
                        "binding_helpers/serde_sanitized_vec_string_simple.jinja"
                    };
                    bindings.push_str(&crate::codegen::template_env::render(
                        template,
                        minijinja::context! {
                            name => &p.name,
                            err_conv => err_conv,
                            indent => indent,
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    bindings
}
