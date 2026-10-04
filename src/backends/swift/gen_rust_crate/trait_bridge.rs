//! Emits the Rust-side trait bridge wrapper and trampolines.
//!
//! Each configured `TraitBridgeConfig` entry gets:
//!   - an `extern "Rust"` block with `{Trait}Box` + one free trampoline fn per method
//!   - a `pub struct {Trait}Box(pub Box<dyn Trait + Send + Sync>)` definition
//!   - one `pub fn {trait_snake}_call_{method}(this: &{Trait}Box, …)` trampoline per method
//!
//! [`SwiftBridgeGenerator`] implements [`crate::codegen::generators::trait_bridge::TraitBridgeGenerator`]
//! for the inbound plugin registration pattern (Swift implements a Rust trait). The
//! `gen_unregistration_fn` and `gen_clear_fn` overrides emit swift-bridge–visible `pub fn`
//! wrappers that delegate to the host crate's `unregister_*` / `clear_*` registry entry
//! points.

use crate::backends::swift::gen_rust_crate::type_bridge::bridge_type;
use crate::codegen::conversions::helpers::{apply_param_newtype_to_core, is_explicit_newtype};
use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;

/// Swift-specific trait bridge generator.
///
/// The Swift inbound plugin pattern (Swift class implements a Rust trait) is
/// primarily handled by the `plugin_inbound` module, which emits `extern "Swift"`
/// shims, wrapper structs, and the `Plugin` / trait impls. This generator
/// provides the [`TraitBridgeGenerator`] contract so that `gen_unregistration_fn`
/// and `gen_clear_fn` can be called uniformly from the plugin inbound emitter.
///
/// `gen_registration_fn` returns an empty string because registration requires
/// a `Swift{Trait}Box` argument whose type is only available in the inbound
/// `extern "Rust"` context; that function is emitted by `plugin_inbound` directly.
pub struct SwiftBridgeGenerator;

impl TraitBridgeGenerator for SwiftBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "swift_bridge::opaque"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![]
    }

    fn gen_sync_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_async_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_constructor(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    /// Returns an empty string. Registration requires a `Swift{Trait}Box` argument
    /// whose type is only available inside the `extern "Rust"` block produced by
    /// `plugin_inbound::emit_extern_block_for_inbound_registration`; that function
    /// emits the `register_*` entry point directly.
    fn gen_registration_fn(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    /// Emit a `pub fn {name}(name: String) -> Result<(), String>` that unregisters
    /// a previously-registered plugin by name.
    ///
    /// The function calls into the configured registry directly — consistent with
    /// how `register_*` calls the registry in `plugin_inbound::emit_inbound_wrapper`.
    ///
    /// Returns an empty string when `spec.bridge_config.unregister_fn` is `None`
    /// or when `spec.bridge_config.registry_getter` is not set (no registry to
    /// call into).
    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let Some(registry_getter) = spec.bridge_config.registry_getter.as_deref() else {
            return String::new();
        };
        let trait_name = &spec.trait_def.name;
        format!(
            "/// Unregister a previously-registered `{trait_name}` plugin by name.\n\
             pub fn {unregister_fn}(name: String) -> Result<(), String> {{\n\
             \x20\x20\x20\x20let registry = {registry_getter}();\n\
             \x20\x20\x20\x20let mut guard = registry.write();\n\
             \x20\x20\x20\x20guard.remove(&name).map_err(|e| e.to_string())\n\
             }}\n"
        )
    }

    /// Emit a `pub fn {name}() -> Result<(), String>` that clears all registered
    /// plugins of this type. Typically used in test teardown.
    ///
    /// The function calls the configured host clear function so host-managed recovery
    /// semantics are preserved.
    ///
    /// Returns an empty string when `spec.bridge_config.clear_fn` is `None`.
    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        let trait_name = &spec.trait_def.name;
        crate::backends::swift::template_env::render(
            "trait_clear_forwarder.rs.jinja",
            minijinja::context! {
                trait_name => trait_name,
                clear_fn => clear_fn,
                host_path => host_path,
            },
        )
    }
}

/// Emit the `extern "Rust"` block for a trait bridge.
///
/// Declares an opaque `{Trait}Box` type plus one free trampoline function per method:
/// `fn {trait_snake}_call_{method}(this: &{Trait}Box, args…) -> ret`.
/// All parameter/return types are flattened to swift-bridge-safe types (primitives,
/// String, `Vec<leaf>`). Complex types (Named, Optional, Map, `Vec<non-leaf>`) are JSON-bridged.
pub fn emit_extern_block_for_trait_bridge(trait_def: &TypeDef, visible_type_names: &HashSet<&str>) -> String {
    let mut block = String::new();
    block.push_str("    extern \"Rust\" {\n");
    block.push_str(&crate::backends::swift::template_env::render(
        "trait_extern_type.jinja",
        minijinja::context! {
            trait_name => &trait_def.name,
        },
    ));

    let trait_snake = heck::AsSnakeCase(trait_def.name.as_str()).to_string();

    block.push_str(&crate::backends::swift::template_env::render(
        "trait_phantom_fn.jinja",
        minijinja::context! {
            trait_name => &trait_def.name,
            trait_snake => &trait_snake,
        },
    ));

    for method in &trait_def.methods {
        if method.has_default_impl {
            continue;
        }

        let method_name = method.name.to_snake_case();
        let fn_name = format!("{trait_snake}_call_{method_name}");

        let mut params = vec!["this: &".to_string() + &format!("{}Box", trait_def.name)];
        for p in &method.params {
            let bridge_ty = bridge_type_for_trait_method(&p.ty, visible_type_names);
            let bridge_ty = if p.optional {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = p.name.to_snake_case();
            params.push(format!("{name}: {bridge_ty}"));
        }

        let return_ty = if method.error_type.is_some() {
            "String".to_string()
        } else {
            bridge_type_for_trait_method(&method.return_type, visible_type_names)
        };

        let params_str = params.join(", ");
        block.push_str(&crate::backends::swift::template_env::render(
            "trait_method_fn.jinja",
            minijinja::context! {
                fn_name => &fn_name,
                params => &params_str,
                return_type => &return_ty,
            },
        ));
    }

    block.push_str("    }\n\n");
    block
}

/// Emit the Rust wrapper struct and trampoline functions for a trait bridge.
///
/// Emits:
/// - `pub struct {Trait}Box(pub Box<dyn source_crate::path::Trait + Send + Sync>);`
/// - For each method: a `pub fn {trait_snake}_call_{method}(this: &{Trait}Box, …) -> ret`
///   that delegates to `this.0.{method}(…)`.
/// - Async methods block on a current-thread Tokio runtime (same as async function shims).
///
/// `visible_type_names` must contain all type names (structs + enums) that have swift-bridge
/// wrapper newtypes in the generated lib.rs. Named return types NOT in this set (e.g. excluded
/// types like `InternalDocument`) are serialised to JSON rather than wrapped in a nonexistent
/// struct or enum.
///
/// `enum_names` must be EVERY enum, not just the fieldless ones: it selects `{t}::from(..)` over
/// the tuple-struct call `{t}(..)`, and `enums::emit_enum_wrapper` emits `impl From<core>` for a
/// data-carrying enum too (its data variants are mirrored as fieldless ones).
pub fn emit_trait_bridge_wrapper(
    trait_def: &TypeDef,
    source_crate: &str,
    enum_names: &HashSet<&str>,
    visible_type_names: &HashSet<&str>,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = String::new();
    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name.as_str()).to_string();

    let trait_path = if trait_def.rust_path.is_empty() {
        format!("{source_crate}::{trait_name}")
    } else {
        trait_def.rust_path.replace('-', "_")
    };

    out.push_str(&crate::backends::swift::template_env::render(
        "trait_struct.jinja",
        minijinja::context! {
            trait_name => trait_name,
            trait_path => &trait_path,
        },
    ));

    out.push_str(&crate::backends::swift::template_env::render(
        "trait_phantom_impl.jinja",
        minijinja::context! {
            trait_name => trait_name,
            trait_snake => &trait_snake,
        },
    ));

    for method in &trait_def.methods {
        if method.has_default_impl {
            continue;
        }

        let method_name = method.name.to_snake_case();
        let fn_name = format!("{trait_snake}_call_{method_name}");

        let mut sig_params = vec![format!("this: &{trait_name}Box")];
        for p in &method.params {
            let bridge_ty = bridge_type_for_trait_method(&p.ty, visible_type_names);
            let bridge_ty = if p.optional {
                format!("Option<{bridge_ty}>")
            } else {
                bridge_ty
            };
            let name = p.name.to_snake_case();
            let needs_mut = p.is_mut;
            if needs_mut {
                sig_params.push(format!("mut {name}: {bridge_ty}"));
            } else {
                sig_params.push(format!("{name}: {bridge_ty}"));
            }
        }
        let sig_params_str = sig_params.join(", ");

        let return_ty = if method.error_type.is_some() {
            "String".to_string()
        } else {
            bridge_type_for_trait_method(&method.return_type, visible_type_names)
        };

        let mut pre_call_bindings = Vec::new();
        let call_args: Vec<String> = method
            .params
            .iter()
            .map(|p| trait_call_arg(p, visible_type_names, type_paths, &mut pre_call_bindings))
            .collect();
        let call_args_str = call_args.join(", ");
        let source_call = format!("this.0.{method_name}({call_args_str})");

        let bindings = if pre_call_bindings.is_empty() {
            String::new()
        } else {
            pre_call_bindings.join("\n") + "\n"
        };
        let body = format!(
            "{}{}",
            bindings,
            emit_trait_method_body(method, &source_call, &return_ty, enum_names, visible_type_names)
        );

        out.push_str(&crate::backends::swift::template_env::render(
            "trait_method_impl.jinja",
            minijinja::context! {
                fn_name => &fn_name,
                params => &sig_params_str,
                return_type => &return_ty,
                body => &body,
            },
        ));
    }

    out
}

/// Bridge type for trait method parameters and returns.
/// Optional and vector containers remain native recursively. Maps, excluded named leaves, and
/// the shared `Optional<Vec<excluded Named>>` exception cross as one JSON string while preserving
/// the outer optional as `Option<String>`.
///
/// `visible_type_names` contains the set of Named types that have generated
/// swift-bridge newtype wrappers in lib.rs. Named types outside this set
/// (e.g. excluded internal types like `InternalDocument`) are JSON-bridged as
/// `String` rather than referencing a nonexistent wrapper newtype.
fn bridge_type_for_trait_method(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> String {
    if matches!(ty, TypeRef::Optional(_)) && trait_type_is_json_blob(ty, visible_type_names) {
        return "Option<String>".to_string();
    }
    if trait_type_is_json_blob(ty, visible_type_names) {
        return "String".to_string();
    }
    match ty {
        TypeRef::Optional(inner) => format!("Option<{}>", bridge_type_for_trait_method(inner, visible_type_names)),
        TypeRef::Vec(inner) => format!("Vec<{}>", bridge_type_for_trait_method(inner, visible_type_names)),
        _ => bridge_type(ty),
    }
}

fn trait_type_is_json_blob(ty: &TypeRef, visible_type_names: &HashSet<&str>) -> bool {
    trait_type_is_single_json_blob(ty, |name| !visible_type_names.contains(name))
}

pub(crate) fn trait_type_is_single_json_blob(ty: &TypeRef, named_is_json: impl Fn(&str) -> bool) -> bool {
    match ty {
        TypeRef::Map(_, _) => true,
        TypeRef::Named(name) => named_is_json(name),
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Vec(elem) if matches!(elem.as_ref(), TypeRef::Named(name) if named_is_json(name)))
        }
        _ => false,
    }
}

fn trait_json_to_core_expr(
    expr: &str,
    ty: &TypeRef,
    visible_type_names: &HashSet<&str>,
    type_paths: &std::collections::HashMap<String, String>,
) -> Option<String> {
    if let TypeRef::Optional(inner) = ty
        && trait_type_is_json_blob(ty, visible_type_names)
    {
        let native_ty = trait_native_type(inner, type_paths);
        return Some(format!(
            "({expr}).map(|json| serde_json::from_str::<{native_ty}>(&json).expect(\"valid JSON bridge value\"))"
        ));
    }
    if trait_type_is_json_blob(ty, visible_type_names) {
        let native_ty = match ty {
            TypeRef::Named(name) => type_paths
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone())
                .replace('-', "_"),
            _ => crate::backends::swift::gen_rust_crate::type_bridge::swift_bridge_rust_type(ty),
        };
        return Some(format!(
            "serde_json::from_str::<{native_ty}>(&{expr}).expect(\"valid JSON bridge value\")"
        ));
    }
    match ty {
        TypeRef::Optional(inner) => trait_json_to_core_expr("value", inner, visible_type_names, type_paths)
            .map(|converted| format!("({expr}).map(|value| {converted})")),
        TypeRef::Vec(inner) => trait_json_to_core_expr("value", inner, visible_type_names, type_paths)
            .map(|converted| format!("({expr}).into_iter().map(|value| {converted}).collect::<Vec<_>>()")),
        _ => None,
    }
}

fn trait_native_type(ty: &TypeRef, type_paths: &std::collections::HashMap<String, String>) -> String {
    match ty {
        TypeRef::Named(name) => type_paths
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone())
            .replace('-', "_"),
        TypeRef::Optional(inner) => format!("Option<{}>", trait_native_type(inner, type_paths)),
        TypeRef::Vec(inner) => format!("Vec<{}>", trait_native_type(inner, type_paths)),
        TypeRef::Map(key, value) => format!(
            "std::collections::HashMap<{}, {}>",
            trait_native_type(key, type_paths),
            trait_native_type(value, type_paths)
        ),
        _ => crate::backends::swift::gen_rust_crate::type_bridge::swift_bridge_rust_type(ty),
    }
}

fn trait_core_to_json_expr(expr: &str, ty: &TypeRef, visible_type_names: &HashSet<&str>) -> Option<String> {
    if matches!(ty, TypeRef::Optional(_)) && trait_type_is_json_blob(ty, visible_type_names) {
        return Some(format!(
            "({expr}).map(|value| serde_json::to_string(&value).expect(\"serializable return\"))"
        ));
    }
    if trait_type_is_json_blob(ty, visible_type_names) {
        return Some(format!(
            "serde_json::to_string(&({expr})).expect(\"serializable return\")"
        ));
    }
    match ty {
        TypeRef::Optional(inner) => trait_core_to_json_expr("value", inner, visible_type_names)
            .map(|converted| format!("({expr}).map(|value| {converted})")),
        TypeRef::Vec(inner) => trait_core_to_json_expr("value", inner, visible_type_names)
            .map(|converted| format!("({expr}).into_iter().map(|value| {converted}).collect::<Vec<_>>()")),
        _ => None,
    }
}

/// Build the call-site argument expression for a trait method parameter.
/// JSON-bridged params are deserialized; Path params are converted to PathBuf/Path;
/// Named types visible in the bridge are passed through wrapper newtypes (extract `.0`);
/// Named types NOT in `visible_type_names` (excluded internal types) are JSON-bridged as `String`
/// at the boundary and deserialised here back to the source type.
pub(crate) fn trait_call_arg(
    p: &crate::core::ir::ParamDef,
    visible_type_names: &HashSet<&str>,
    type_paths: &std::collections::HashMap<String, String>,
    pre_call_bindings: &mut Vec<String>,
) -> String {
    let name = p.name.to_snake_case();

    if p.newtype_wrapper.as_deref().is_some_and(is_explicit_newtype) {
        let binding_expr = if p.optional {
            trait_json_to_core_expr("value", &p.ty, visible_type_names, type_paths)
                .map(|converted| format!("({name}).map(|value| {converted})"))
                .unwrap_or_else(|| name.clone())
        } else {
            trait_json_to_core_expr(&name, &p.ty, visible_type_names, type_paths).unwrap_or_else(|| name.clone())
        };
        let converted = apply_param_newtype_to_core(&binding_expr, p).expect("explicit newtype metadata checked above");
        if p.is_ref {
            let bound = format!("__{name}_newtype");
            let mutability = if p.is_mut { "mut " } else { "" };
            pre_call_bindings.push(format!("    let {mutability}{bound} = {converted};"));
            if p.optional {
                return if p.is_mut {
                    format!("{bound}.as_mut()")
                } else {
                    format!("{bound}.as_ref()")
                };
            }
            return if p.is_mut {
                format!("&mut {bound}")
            } else {
                format!("&{bound}")
            };
        }
        return converted;
    }

    let json_conversion = if p.optional {
        trait_json_to_core_expr("value", &p.ty, visible_type_names, type_paths)
            .map(|converted| format!("({name}).map(|value| {converted})"))
    } else {
        trait_json_to_core_expr(&name, &p.ty, visible_type_names, type_paths)
    };
    if let Some(deser) = json_conversion {
        if p.is_ref {
            let bound = format!("__{name}_json");
            let mutability = if p.is_mut { "mut " } else { "" };
            pre_call_bindings.push(format!("    let {mutability}{bound} = {deser};"));
            return if p.optional {
                if p.is_mut {
                    format!("{bound}.as_mut()")
                } else {
                    format!("{bound}.as_ref()")
                }
            } else if p.is_mut {
                format!("&mut {bound}")
            } else {
                format!("&{bound}")
            };
        }
        return deser;
    }

    if matches!(p.ty, TypeRef::Path) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(std::path::Path::new)");
            }
            return format!("{name}.map(std::path::PathBuf::from)");
        }
        if p.is_ref {
            return format!("std::path::Path::new(&{name})");
        }
        return format!("std::path::PathBuf::from({name})");
    }

    if let TypeRef::Named(named) = &p.ty
        && !visible_type_names.contains(named.as_str())
    {
        let qualified = type_paths
            .get(named.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| named.clone());
        let deser = if p.optional {
            format!("{name}.map(|json| serde_json::from_str::<{qualified}>(&json).expect(\"valid JSON for {name}\"))")
        } else {
            format!("serde_json::from_str::<{qualified}>(&{name}).expect(\"valid JSON for {name}\")")
        };
        if p.is_ref {
            return if p.optional {
                format!("({deser}).as_ref()")
            } else {
                format!("&{deser}")
            };
        }
        return deser;
    }

    if matches!(p.ty, TypeRef::Named(_)) {
        if p.optional {
            if p.is_ref {
                return format!("{name}.as_ref().map(|w| &w.0)");
            }
            return format!("{name}.map(|w| w.0)");
        }
        if p.is_mut {
            return format!("&mut {name}.0");
        }
        if p.is_ref {
            return format!("&{name}.0");
        }
        return format!("{name}.0");
    }

    if p.is_ref {
        match &p.ty {
            TypeRef::Bytes | TypeRef::String if p.optional => return format!("{name}.as_deref()"),
            TypeRef::Char if p.optional => return format!("{name}.as_ref()"),
            TypeRef::Bytes | TypeRef::String | TypeRef::Char => return format!("&{name}"),
            TypeRef::Vec(_) if p.optional => return format!("{name}.as_deref()"),
            TypeRef::Vec(_) => return format!("{name}.as_slice()"),
            _ => return format!("&{name}"),
        }
    }
    name
}

/// Emit the body of a trait method trampoline, handling sync vs async and error types.
///
/// `visible_type_names` is the union of all struct and enum names that have swift-bridge
/// wrapper newtypes in the generated lib.rs. Named return types not in this set (e.g.
/// excluded types like `InternalDocument`) are JSON-serialised rather than wrapped in a
/// struct that does not exist in the generated file.
pub(crate) fn emit_trait_method_body(
    method: &MethodDef,
    source_call: &str,
    _return_ty: &str,
    enum_names: &HashSet<&str>,
    visible_type_names: &HashSet<&str>,
) -> String {
    let async_uses_transparent_wrapper =
        method.return_newtype_wrapper.is_some() || method.params.iter().any(|param| param.newtype_wrapper.is_some());
    if method.is_async && async_uses_transparent_wrapper {
        return format!(
            "    compile_error!(\"alef cannot safely bridge async trait method `{}` through Swift; exclude the trait from Swift generation\");\n",
            method.name
        );
    }

    let wrap_return = |expr: String| -> String {
        if method.return_newtype_wrapper.is_some() {
            let converted = apply_return_newtype_unwrap(&expr, &method.return_newtype_wrapper);
            return trait_core_to_json_expr(&converted, &method.return_type, visible_type_names).unwrap_or(converted);
        }
        if let Some(converted) = trait_core_to_json_expr(&expr, &method.return_type, visible_type_names) {
            converted
        } else {
            match &method.return_type {
                TypeRef::String => format!("{expr}.to_string()"),
                TypeRef::Path => format!("{expr}.display().to_string()"),
                TypeRef::Named(name) => {
                    if !visible_type_names.contains(name.as_str()) {
                        format!("serde_json::to_string(&({expr})).expect(\"serializable return\")")
                    } else if enum_names.contains(name.as_str()) {
                        format!("{name}::from({expr})")
                    } else {
                        format!("{name}({expr})")
                    }
                }
                _ => expr,
            }
        }
    };

    let envelope_result_expr = |base: String| -> String {
        let ok_fragment = if matches!(method.return_type, TypeRef::Unit) {
            "\"null\"".to_string()
        } else {
            let converted = apply_return_newtype_unwrap("v", &method.return_newtype_wrapper);
            format!("serde_json::to_string(&({converted})).expect(\"serializable return\")")
        };

        format!(
            "match {base} {{\n\
             \x20\x20\x20\x20Ok(v) => format!(\"{{{{\\\"ok\\\": {{}}}}}}\", {ok_fragment}),\n\
             \x20\x20\x20\x20Err(e) => format!(\"{{{{\\\"err\\\": {{}}}}}}\", serde_json::to_string(&e.to_string()).expect(\"serializable error\")),\n\
             }}"
        )
    };

    if method.is_async {
        let await_expr = format!("{source_call}.await");
        if method.error_type.is_some() {
            let enveloped = envelope_result_expr(await_expr);
            format!("    crate::__alef_tokio_runtime().block_on(async {{ {enveloped} }})\n")
        } else {
            let inner = wrap_return(await_expr);
            format!("    crate::__alef_tokio_runtime().block_on(async {{ {inner} }})\n")
        }
    } else if method.error_type.is_some() {
        let enveloped = envelope_result_expr(source_call.to_string());
        format!("    {enveloped}\n")
    } else if method.returns_ref
        && matches!(&method.return_type, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String))
    {
        format!("    {source_call}.iter().map(|s| s.to_string()).collect()\n")
    } else {
        let wrapped = wrap_return(source_call.to_string());
        format!("    {wrapped}\n")
    }
}

#[cfg(test)]
mod transparent_string_tests {
    use super::*;
    use crate::core::ir::{NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata, ParamDef, ReceiverKind};

    fn wrapper(containers: Vec<NewtypeContainer>) -> String {
        NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
            "sample::SecretString",
            "from",
            "into_inner",
            containers,
        )])
    }

    fn returning_method(name: &str, is_async: bool, error_type: Option<&str>) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            return_type: TypeRef::String,
            return_newtype_wrapper: Some(wrapper(vec![])),
            is_async,
            error_type: error_type.map(str::to_string),
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }
    }

    #[test]
    fn trait_trampolines_convert_transparent_params_and_all_return_envelopes() {
        let mut borrowed = ParamDef {
            name: "borrowed".to_string(),
            ty: TypeRef::String,
            is_ref: true,
            ..Default::default()
        };
        borrowed.newtype_wrapper = Some(wrapper(vec![]));
        let mut lookup = ParamDef {
            name: "lookup".to_string(),
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            optional: true,
            ..Default::default()
        };
        lookup.newtype_wrapper = Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::MapValue]));
        let mut nested_map = ParamDef {
            name: "nested_map".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::String),
            ))),
            ..Default::default()
        };
        nested_map.newtype_wrapper = Some(wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::MapValue]));
        let mut map_list = ParamDef {
            name: "map_list".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::String),
            ))),
            ..Default::default()
        };
        map_list.newtype_wrapper = Some(wrapper(vec![NewtypeContainer::Vec, NewtypeContainer::MapValue]));
        let mut sync = returning_method("sync_secret", false, None);
        sync.params = vec![borrowed, lookup, nested_map, map_list];
        let trait_def = TypeDef {
            name: "SecretSource".to_string(),
            rust_path: "sample::SecretSource".to_string(),
            methods: vec![
                sync,
                returning_method("async_secret", true, None),
                returning_method("fallible_secret", false, Some("SecretError")),
                returning_method("async_fallible_secret", true, Some("SecretError")),
            ],
            ..Default::default()
        };

        let output = emit_trait_bridge_wrapper(
            &trait_def,
            "sample",
            &HashSet::new(),
            &HashSet::new(),
            &std::collections::HashMap::new(),
        );

        assert!(
            output.contains("let __borrowed_newtype = sample::SecretString::from(borrowed);"),
            "{output}"
        );
        assert!(output.contains("sync_secret(&__borrowed_newtype,"), "{output}");
        assert!(output.contains("lookup: Option<String>"), "{output}");
        assert!(
            output.contains("from_str::<std::collections::HashMap<String, String>>(&value)"),
            "{output}"
        );
        assert!(
            output.contains("map(|(key, value)| (key, sample::SecretString::from(value)))"),
            "{output}"
        );
        assert!(output.contains("nested_map: Option<String>"), "{output}");
        assert!(output.contains("map_list: Vec<String>"), "{output}");
        assert!(
            output.contains("(this.0.sync_secret(&__borrowed_newtype") && output.contains(")).into_inner()"),
            "{output}"
        );
        assert!(
            output.contains("cannot safely bridge async trait method `async_secret`"),
            "{output}"
        );
        assert!(
            output.contains("cannot safely bridge async trait method `async_fallible_secret`"),
            "{output}"
        );
        assert!(!output.contains("block_on("), "{output}");
    }

    #[test]
    fn transparent_return_conversion_is_inside_each_trait_result_branch() {
        let visible = HashSet::new();
        let enums = HashSet::new();

        let sync = returning_method("fallible_secret", false, Some("SecretError"));
        let sync_body = emit_trait_method_body(&sync, "source.fallible_secret()", "String", &enums, &visible);
        assert!(sync_body.contains("match source.fallible_secret()"), "{sync_body}");
        assert!(
            sync_body.contains("Ok(v) =>") && sync_body.contains("serde_json::to_string(&((v).into_inner()))"),
            "{sync_body}"
        );

        let async_method = returning_method("async_fallible_secret", true, Some("SecretError"));
        let async_body = emit_trait_method_body(
            &async_method,
            "source.async_fallible_secret()",
            "String",
            &enums,
            &visible,
        );
        assert!(
            async_body.contains("cannot safely bridge async trait method `async_fallible_secret`"),
            "{async_body}"
        );
        assert!(
            !async_body.contains("block_on(") && !async_body.contains(".await"),
            "{async_body}"
        );
    }

    #[test]
    fn fallible_json_returns_are_serialized_once_after_newtype_conversion() {
        let mut map_method = returning_method("map", false, Some("SecretError"));
        map_method.return_type = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        map_method.return_newtype_wrapper = Some(wrapper(vec![NewtypeContainer::MapValue]));
        let map_body = emit_trait_method_body(&map_method, "source.map()", "String", &HashSet::new(), &HashSet::new());
        assert!(map_body.contains("(v).into_iter()"), "{map_body}");
        assert!(!map_body.contains("to_string(&(serde_json::to_string"), "{map_body}");

        let excluded_method = MethodDef {
            name: "excluded".to_string(),
            return_type: TypeRef::Named("Hidden".to_string()),
            error_type: Some("SecretError".to_string()),
            ..Default::default()
        };
        let excluded_body = emit_trait_method_body(
            &excluded_method,
            "source.excluded()",
            "String",
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(excluded_body.contains("serde_json::to_string(&(v))"), "{excluded_body}");
        assert!(
            !excluded_body.contains("to_string(&(serde_json::to_string"),
            "{excluded_body}"
        );
    }

    #[test]
    fn non_wrapper_async_trait_method_keeps_existing_bridge_path() {
        let method = MethodDef {
            name: "plain_async".to_string(),
            return_type: TypeRef::String,
            is_async: true,
            ..Default::default()
        };
        let body = emit_trait_method_body(
            &method,
            "source.plain_async()",
            "String",
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(
            body.contains("block_on(async { source.plain_async().await.to_string() })"),
            "{body}"
        );
        assert!(!body.contains("compile_error!"), "{body}");
    }
}
