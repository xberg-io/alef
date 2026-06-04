//! Service-API codegen for the PyO3 backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rs`** — Rust pyo3 glue that wraps each registered Python
//!    callable as `Arc<dyn <HandlerContractDef::trait_name>>` via an async
//!    callback bridge, builds the core service via the owner type's
//!    registration and run entrypoints, and exposes a `#[pyfunction]` entry
//!    point.
//!
//! 2. **`service.py`** — An idiomatic Python class mirroring the service's
//!    constructor, configurator methods, and registration decorators, with a
//!    `run(...)` method that delegates to the native extension.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointKind, HandlerContractDef, RegistrationDef, RegistrationVariantStyle, ServiceDef, TypeRef,
};
use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use std::collections::BTreeSet;
use std::path::PathBuf;

// ───────────────────────────────────────────────────────────────── helpers ──

/// Convert a `TypeRef` to a simple Python type annotation string.
fn python_type_annotation(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "str".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::F32 | PrimitiveType::F64 => "float".to_owned(),
                _ => "int".to_owned(),
            }
        }
        TypeRef::Bytes => "bytes".to_owned(),
        TypeRef::Optional(inner) => format!("{} | None", python_type_annotation(inner)),
        TypeRef::Vec(inner) => format!("list[{}]", python_type_annotation(inner)),
        TypeRef::Map(k, v) => format!("dict[{}, {}]", python_type_annotation(k), python_type_annotation(v)),
        TypeRef::Unit => "None".to_owned(),
        TypeRef::Named(n) => n.clone(),
        TypeRef::Json => "object".to_owned(),
        TypeRef::Path => "str".to_owned(),
        TypeRef::Duration => "float".to_owned(),
    }
}

/// Find the `HandlerContractDef` by trait name in the surface.
fn find_contract<'a>(api: &'a ApiSurface, trait_name: &str) -> Option<&'a HandlerContractDef> {
    api.handler_contracts.iter().find(|c| c.trait_name == trait_name)
}

/// Recursively collect `Named` type references into `out`.
fn collect_named_types(ty: &TypeRef, out: &mut BTreeSet<String>) {
    match ty {
        TypeRef::Named(n) => {
            out.insert(n.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types(k, out);
            collect_named_types(v, out);
        }
        _ => {}
    }
}

/// Walk every `TypeRef` reachable from a service definition's user-facing surface
/// (constructor, configurators, registration metadata, entrypoints).
fn collect_service_named_types(service: &ServiceDef, out: &mut BTreeSet<String>) {
    for p in &service.constructor.params {
        collect_named_types(&p.ty, out);
    }
    for m in &service.configurators {
        for p in &m.params {
            collect_named_types(&p.ty, out);
        }
    }
    for r in &service.registrations {
        for p in &r.metadata_params {
            collect_named_types(&p.ty, out);
        }
    }
    for e in &service.entrypoints {
        for p in &e.params {
            collect_named_types(&p.ty, out);
        }
    }
}

/// Collect runtime-referenced type names from registration variants. Variants
/// reference enum classes (e.g. `Method.Get`) inside the wrapper construction,
/// which must be imported at runtime — TYPE_CHECKING is not sufficient.
fn collect_variant_runtime_types(service: &ServiceDef, out: &mut BTreeSet<String>) {
    for r in &service.registrations {
        for variant in &r.variants {
            if let Some(wc) = &variant.wrapper_call {
                // The wrapper type itself is constructed at runtime.
                out.insert(wc.wrapper_type_name.clone());
                // Each Fixed arg's value_expr like `crate::Method::Get` resolves to
                // `Method.Get` in Python — the bare `Method` class needs a runtime
                // import. Take the second-to-last `::` segment as the enum class.
                for arg in &wc.args {
                    if let crate::core::ir::WrapperConstructorArg::Fixed { value_expr, .. } = arg {
                        let segments: Vec<&str> = value_expr.split("::").collect();
                        if segments.len() >= 2 {
                            out.insert(segments[segments.len() - 2].to_owned());
                        }
                    }
                }
            }
        }
    }
}

/// Emit a Python docstring at the given column indent. Single-line if `text`
/// has no newline; otherwise multi-line with the closing `"""` on its own line
/// and every body line re-indented to match the opening, so the output passes
/// pydocstyle/ruff `D207`/`D209`.
fn format_docstring(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    let pad = " ".repeat(indent);
    if !trimmed.contains('\n') {
        return format!("{pad}\"\"\"{trimmed}\"\"\"\n");
    }
    let mut lines = trimmed.lines();
    let first = lines.next().unwrap_or("");
    let mut out = format!("{pad}\"\"\"{first}\n");
    for line in lines {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&pad);
    out.push_str("\"\"\"\n");
    out
}

// ─────────────────────────────────────────────────────────── Python output ──

/// Generate the idiomatic Python service class (`service.py`).
///
/// Produces a Python module containing one class per service.  Each class
/// exposes:
/// - A constructor mirroring [`ServiceDef::constructor`].
/// - Configurator methods from [`ServiceDef::configurators`].
/// - Decorator-style registration helpers from [`ServiceDef::registrations`].
/// - A `run(...)` method derived from the first [`EntrypointKind::Run`]
///   entrypoint.
pub(super) fn gen_service_py(api: &ApiSurface, module_name: &str) -> String {
    let mut out = String::new();

    // Aggregate every `Named` type referenced across services' user-facing
    // surface so we can emit a single TYPE_CHECKING import block.
    let mut named_types: BTreeSet<String> = BTreeSet::new();
    let mut runtime_types: BTreeSet<String> = BTreeSet::new();
    for service in &api.services {
        collect_service_named_types(service, &mut named_types);
        collect_variant_runtime_types(service, &mut runtime_types);
    }
    // Runtime types take precedence — drop them from the TYPE_CHECKING-only set.
    for n in &runtime_types {
        named_types.remove(n);
    }
    let any_registrations = api.services.iter().any(|s| !s.registrations.is_empty());

    out.push_str("\"\"\"Idiomatic service API: builders, decorators, and the App wrapper.\"\"\"\n\n");
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("from typing import TYPE_CHECKING, Any\n\n");
    // The native extension is a submodule of the package (e.g. `pkg._pkg`), so import it
    // relatively — a bare `import _pkg` would not resolve at runtime.
    out.push_str(&format!("from . import {module_name}\n"));

    // Variant constructors reference runtime types (e.g. RouteBuilder, Method),
    // so emit those as a normal import — TYPE_CHECKING is not enough.
    if !runtime_types.is_empty() {
        let joined = runtime_types.iter().cloned().collect::<Vec<_>>().join(", ");
        out.push_str(&format!("from .{module_name} import {joined}\n"));
    }

    // Emit a TYPE_CHECKING block for annotation-only imports so the file
    // passes ruff `F821`, `TC003`, and import-sort checks without paying the
    // runtime import cost.
    if any_registrations || !named_types.is_empty() {
        out.push('\n');
        out.push_str("if TYPE_CHECKING:\n");
        // Mirror ruff's import-sort policy: stdlib group, then local group,
        // separated by a blank line inside the TYPE_CHECKING block.
        if any_registrations {
            out.push_str("    from collections.abc import Callable\n");
            if !named_types.is_empty() {
                out.push('\n');
            }
        }
        if !named_types.is_empty() {
            let joined = named_types.iter().cloned().collect::<Vec<_>>().join(", ");
            out.push_str(&format!("    from .{module_name} import {joined}\n"));
        }
    }
    // Two blank lines before the first class (PEP8 / ruff-format).
    out.push_str("\n\n");

    for service in &api.services {
        gen_service_class(&mut out, service, api, module_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, api: &ApiSurface, module_name: &str) {
    let class_name = &service.name;

    out.push_str(&format!("class {class_name}:\n"));
    if !service.doc.is_empty() {
        out.push_str(&format_docstring(&service.doc, 4));
        out.push('\n');
    }

    // __init__
    {
        let ctor = &service.constructor;
        let mut init_params = vec!["self".to_owned()];
        let mut init_args = Vec::new();
        for p in &ctor.params {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                init_params.push(format!("{}: {} | None = None", p.name, annotation));
            } else {
                init_params.push(format!("{}: {}", p.name, annotation));
            }
            init_args.push(p.name.clone());
        }

        let param_sig = init_params.join(", ");
        out.push_str(&format!("    def __init__({param_sig}) -> None:\n"));
        if !ctor.doc.is_empty() {
            out.push_str(&format_docstring(&ctor.doc, 8));
        }
        // Stored state for registrations — also serves as a non-empty body so
        // we never need a stray `pass` statement (ruff `PIE790`).
        out.push_str("        self._registrations: list[tuple[Any, ...]] = []\n");
        for arg in &init_args {
            out.push_str(&format!("        self._{arg} = {arg}\n"));
        }
        out.push('\n');
    }

    // Configurator methods
    for method in &service.configurators {
        let mut params = vec!["self".to_owned()];
        for p in &method.params {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} | None = None", p.name, annotation));
            } else {
                params.push(format!("{}: {}", p.name, annotation));
            }
        }
        let param_sig = params.join(", ");
        let method_name = &method.name;
        // With `from __future__ import annotations` the return type is a
        // string at runtime, so we don't need to quote the self-class name
        // (ruff `UP037`).
        out.push_str(&format!("    def {method_name}({param_sig}) -> {class_name}:\n"));
        if !method.doc.is_empty() {
            out.push_str(&format_docstring(&method.doc, 8));
        }
        for p in &method.params {
            out.push_str(&format!("        self._{} = {}\n", p.name, p.name));
        }
        out.push_str("        return self\n\n");
    }

    // Registration methods as decorator-style helpers
    for reg in &service.registrations {
        gen_registration_method(out, reg, service, api, module_name);
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let mut params = vec!["self".to_owned()];
        for p in &ep.params {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("{}: {} | None = None", p.name, annotation));
            } else {
                params.push(format!("{}: {}", p.name, annotation));
            }
        }
        let param_sig = params.join(", ");
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&format!("    def {ep_name}({param_sig}) -> None:\n"));
                if !ep.doc.is_empty() {
                    out.push_str(&format_docstring(&ep.doc, 8));
                }
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                out.push_str(&format!("        {module_name}.{native_fn}(self._registrations"));
                for p in &ep.params {
                    out.push_str(&format!(", {}", p.name));
                }
                out.push_str(")\n\n");
            }
            EntrypointKind::Finalize => {
                out.push_str(&format!("    def {ep_name}({param_sig}) -> Any:\n"));
                if !ep.doc.is_empty() {
                    out.push_str(&format_docstring(&ep.doc, 8));
                }
                let native_fn = format!("{service_snake}_{ep_name}", service_snake = class_name.to_snake_case());
                out.push_str(&format!("        return {module_name}.{native_fn}(self._registrations"));
                for p in &ep.params {
                    out.push_str(&format!(", {}", p.name));
                }
                out.push_str(")\n\n");
            }
        }
    }
}

/// Generate a wrapper constructor call for a variant if wrapper_call is present.
/// Returns the Python code to construct the wrapper (e.g., "builder = RouteBuilder(Method.GET, path)")
/// or None if no wrapper_call exists.
fn build_wrapper_constructor_expr(variant: &crate::core::ir::RegistrationVariant) -> Option<String> {
    let wc = variant.wrapper_call.as_ref()?;
    let mut call_args = Vec::new();

    for arg in &wc.args {
        match arg {
            crate::core::ir::WrapperConstructorArg::Fixed {
                param_name: _,
                value_expr,
            } => {
                // Convert a Rust enum path like `my_crate::Method::Get` into the
                // Python form `Method.GET`: take the last two `::` segments
                // (the enum type name and the variant name) and apply the same
                // SHOUTY_SNAKE_CASE rename the pyclass codegen emits via
                // `#[pyo3(name = "…")]`. Non-`::` values pass through verbatim
                // — the library author owns them.
                let segments: Vec<&str> = value_expr.split("::").collect();
                if segments.len() >= 2 {
                    let class = segments[segments.len() - 2];
                    let variant_name = segments[segments.len() - 1].to_shouty_snake_case();
                    call_args.push(format!("{class}.{variant_name}"));
                } else {
                    call_args.push(value_expr.clone());
                }
            }
            crate::core::ir::WrapperConstructorArg::Free { param } => {
                call_args.push(param.name.clone());
            }
        }
    }

    // PyO3 opaque wrappers expose a `new` classmethod (emitted by gen_bindings/types.rs)
    // rather than a `__init__` constructor, so build `WrapperType.new(...)` when the IR
    // names a constructor method. The bare `WrapperType(...)` form remains for backends
    // that bind a true `__init__`.
    let call_expr = if wc.constructor_method.is_empty() || wc.constructor_method == "__init__" {
        format!("{}({})", wc.wrapper_type_name, call_args.join(", "))
    } else {
        format!(
            "{}.{}({})",
            wc.wrapper_type_name,
            wc.constructor_method,
            call_args.join(", ")
        )
    };
    Some(format!("{} = {}", wc.metadata_param, call_expr))
}

/// Compute the shared metadata tuple string and the set of consumed param names
/// for a registration variant. Used by both emission forms so the logic is not
/// duplicated.
fn variant_meta_tuple(variant: &crate::core::ir::RegistrationVariant, base_reg: &RegistrationDef) -> (String, String) {
    let wrapper_consumed: BTreeSet<&str> = if let Some(wc) = &variant.wrapper_call {
        let mut s = BTreeSet::new();
        s.insert(wc.metadata_param.as_str());
        for arg in &wc.args {
            match arg {
                crate::core::ir::WrapperConstructorArg::Fixed { param_name, .. } => {
                    s.insert(param_name.as_str());
                }
                crate::core::ir::WrapperConstructorArg::Free { param } => {
                    s.insert(param.name.as_str());
                }
            }
        }
        s
    } else {
        BTreeSet::new()
    };
    let overridden: BTreeSet<&str> = variant.overrides.iter().map(|o| o.param_name.as_str()).collect();
    let mut meta_items: Vec<String> = Vec::new();
    if let Some(wc) = &variant.wrapper_call {
        meta_items.push(wc.metadata_param.clone());
    }
    for p in &base_reg.metadata_params {
        if wrapper_consumed.contains(p.name.as_str()) || overridden.contains(p.name.as_str()) {
            continue;
        }
        meta_items.push(p.name.clone());
    }
    let base_method = base_reg.method.clone();
    let meta_tuple = if meta_items.is_empty() {
        "()".to_owned()
    } else if meta_items.len() == 1 {
        format!("({},)", meta_items[0])
    } else {
        format!("({})", meta_items.join(", "))
    };
    (base_method, meta_tuple)
}

/// Emit the direct verb-decorator form: `def get(self, path, handler) -> ClassName`.
///
/// Used when `style` is `VerbDecorator` or `Hybrid`.
fn emit_direct_method(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    class_name: &str,
    free_params_sig: &[String],
    meta_tuple: &str,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;

    let params_sig = if free_params_sig.is_empty() {
        "self, handler: Callable[..., Any]".to_owned()
    } else {
        format!("self, {}, handler: Callable[..., Any]", free_params_sig.join(", "))
    };

    out.push_str(&format!("    def {variant_name}({params_sig}) -> {class_name}:\n"));

    if let Some(doc) = &variant.doc {
        out.push_str(&format_docstring(doc, 8));
    } else {
        out.push_str(&format!(
            "        \"\"\"Register a handler for the {variant_name} variant.\"\"\"\n"
        ));
    }

    if let Some(wrapper_expr) = build_wrapper_constructor_expr(variant) {
        out.push_str(&format!("        {}\n", wrapper_expr));
    }

    out.push_str(&format!(
        "        self._registrations.append((\"{base_method}\", {meta_tuple}, handler))\n"
    ));
    out.push_str("        return self\n\n");
}

/// Emit the builder/decorator-factory form: `def get_decorator(self, path) -> Callable`.
///
/// Used when `style` is `Builder` or `Hybrid`.
fn emit_decorator_factory(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    free_params_sig: &[String],
    meta_tuple: &str,
) {
    let variant_name = &variant.name;
    let base_method = &base_reg.method;
    let decorator_name = format!("{variant_name}_decorator");

    let params_sig_no_handler = if free_params_sig.is_empty() {
        "self".to_owned()
    } else {
        format!("self, {}", free_params_sig.join(", "))
    };

    out.push_str(&format!(
        "    def {decorator_name}({params_sig_no_handler}) -> Callable[[Callable[..., Any]], Callable[..., Any]]:\n"
    ));
    if let Some(doc) = &variant.doc {
        let decorator_doc = format!("Decorator form for {}", doc.trim_start());
        out.push_str(&format_docstring(&decorator_doc, 8));
    } else {
        out.push_str(&format!(
            "        \"\"\"Decorator form for the {variant_name} variant.\"\"\"\n"
        ));
    }

    if let Some(wrapper_expr) = build_wrapper_constructor_expr(variant) {
        out.push_str(&format!("        {}\n", wrapper_expr));
    }

    out.push('\n');
    out.push_str("        def _decorator(fn: Callable[..., Any]) -> Callable[..., Any]:\n");
    out.push_str(&format!(
        "            self._registrations.append((\"{base_method}\", {meta_tuple}, fn))\n"
    ));
    out.push_str("            return fn\n");
    out.push('\n');
    out.push_str("        return _decorator\n\n");
}

/// Emit a registration variant (shortcut method) for the given variant definition.
///
/// Which forms are emitted depends on [`RegistrationVariant::style`]:
/// - [`RegistrationVariantStyle::VerbDecorator`] — only the direct method form
///   (`def get(self, path, handler)`).
/// - [`RegistrationVariantStyle::Builder`] — only the decorator-factory form
///   (`def get_decorator(self, path) -> Callable`).
/// - [`RegistrationVariantStyle::Hybrid`] (default) — both forms.
fn gen_registration_variant(
    out: &mut String,
    variant: &crate::core::ir::RegistrationVariant,
    base_reg: &RegistrationDef,
    _service: &ServiceDef,
    class_name: &str,
) {
    // Build the free params (non-fixed) for the variant signature
    let mut free_params_sig = Vec::new();
    for param in &variant.signature_params {
        let annotation = python_type_annotation(&param.ty);
        if param.optional {
            free_params_sig.push(format!("{}: {} | None = None", param.name, annotation));
        } else {
            free_params_sig.push(format!("{}: {}", param.name, annotation));
        }
    }

    let (_base_method, meta_tuple) = variant_meta_tuple(variant, base_reg);

    match variant.style {
        RegistrationVariantStyle::VerbDecorator => {
            emit_direct_method(out, variant, base_reg, class_name, &free_params_sig, &meta_tuple);
        }
        RegistrationVariantStyle::Builder => {
            emit_decorator_factory(out, variant, base_reg, &free_params_sig, &meta_tuple);
        }
        RegistrationVariantStyle::Hybrid => {
            emit_direct_method(out, variant, base_reg, class_name, &free_params_sig, &meta_tuple);
            emit_decorator_factory(out, variant, base_reg, &free_params_sig, &meta_tuple);
        }
    }
}

fn gen_registration_method(
    out: &mut String,
    reg: &RegistrationDef,
    service: &ServiceDef,
    api: &ApiSurface,
    _module_name: &str,
) {
    let method_name = &reg.method;
    let class_name = &service.name;

    // Find the contract to get wire-type doc info
    let _contract = find_contract(api, &reg.callback_contract);

    // Build metadata param signature (excluding the callback param)
    let mut meta_params: Vec<String> = reg
        .metadata_params
        .iter()
        .map(|p| {
            let annotation = python_type_annotation(&p.ty);
            if p.optional {
                format!("{}: {} | None = None", p.name, annotation)
            } else {
                format!("{}: {}", p.name, annotation)
            }
        })
        .collect();
    meta_params.insert(0, "self".to_owned());

    // Decorator factory form: `def method(self, *meta_params) -> Callable`
    // This lets the user write:
    //   @app.register(meta1, meta2)
    //   async def handler(request): ...
    let meta_sig = meta_params.join(", ");

    out.push_str(&format!(
        "    def {method_name}({meta_sig}) -> Callable[[Callable[..., Any]], Callable[..., Any]]:\n"
    ));
    if !reg.doc.is_empty() {
        out.push_str(&format_docstring(&reg.doc, 8));
    }

    // Collect metadata param names for the closure
    let meta_names: Vec<&str> = reg.metadata_params.iter().map(|p| p.name.as_str()).collect();
    let meta_tuple = if meta_names.is_empty() {
        "()".to_owned()
    } else if meta_names.len() == 1 {
        format!("({},)", meta_names[0])
    } else {
        format!("({})", meta_names.join(", "))
    };

    // PEP8 / ruff-format: nested function definitions inside a method body
    // get a leading and trailing blank line so they read as a logical block.
    out.push('\n');
    out.push_str("        def _decorator(fn: Callable[..., Any]) -> Callable[..., Any]:\n");
    out.push_str(&format!(
        "            self._registrations.append((\"{method_name}\", {meta_tuple}, fn))\n"
    ));
    out.push_str("            return fn\n");
    out.push('\n');
    out.push_str("        return _decorator\n\n");

    // Also expose a plain (non-decorator) register variant for direct use:
    // `app.register_handler(meta1, meta2, handler=fn)`
    let direct_name = format!("register_{method_name}");
    if direct_name != *method_name {
        // Only add when the name differs (avoid collision if method is already named "register_*")
        out.push_str(&format!(
            "    def {direct_name}({meta_sig}, {}: Callable[..., Any]) -> {class_name}:\n",
            reg.callback_param,
        ));
        out.push_str(&format!(
            "        \"\"\"Register a {method_name} callback directly.\"\"\"\n"
        ));
        out.push_str(&format!(
            "        self._registrations.append((\"{method_name}\", {meta_tuple}, {}))\n",
            reg.callback_param,
        ));
        out.push_str("        return self\n\n");
    }

    // Emit registration variants (shortcuts for common patterns)
    for variant in &reg.variants {
        gen_registration_variant(out, variant, reg, service, class_name);
    }
}

// ──────────────────────────────────────────────────────────────── Rust glue ──

/// Generate the Rust pyo3 glue module (`service.rs`).
///
/// For each service this emits:
/// - A `Py{ContractName}Bridge` struct that wraps a `Py<PyAny>` callable and
///   `impl`s the handler contract trait, using pyo3_async_runtimes for async
///   callables and `spawn_blocking` for sync ones.
/// - A `#[pyfunction]` `{snake_service}_{entrypoint}` that accepts the
///   collected registrations list (a Python `list[tuple[str, tuple, Callable]]`)
///   and any entrypoint params, builds the native service, and drives it.
pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let core_import = config.core_import_name();
    let mut out = String::new();

    // File-level allow attributes to keep clippy happy in generated code
    out.push_str("#![allow(clippy::too_many_arguments, clippy::unused_async)]\n\n");
    out.push_str("use pyo3::prelude::*;\n");
    out.push_str("use pyo3::types::{PyList, PyTuple, PyString};\n");
    out.push_str("use std::sync::Arc;\n");
    out.push_str("use serde_json;\n");
    out.push_str("use std::future::Future;\n");
    out.push_str("use std::pin::Pin;\n");
    out.push_str("use axum::http::Request;\n");
    out.push_str("use axum::body::Body;\n\n");

    // Emit one handler bridge per unique handler contract referenced by any registration.
    // Skip non-object-safe traits (WebSocketHandler, SseEventProducer) which use RPITIT.
    let referenced_contracts: Vec<&HandlerContractDef> = {
        let mut names: Vec<&str> = api
            .services
            .iter()
            .flat_map(|s| s.registrations.iter())
            .map(|r| r.callback_contract.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .iter()
            .filter_map(|n| find_contract(api, n))
            .filter(|c| {
                // PyO3 pyo3 backend cannot generate bridges for non-object-safe traits.
                // WebSocketHandler and SseEventProducer use RPITIT (impl Trait return type).
                c.trait_name != "WebSocketHandler" && c.trait_name != "SseEventProducer"
            })
            .collect()
    };

    for contract in &referenced_contracts {
        gen_handler_bridge(&mut out, contract, &core_import);
    }

    // Emit one pyfunction per service × entrypoint
    for service in &api.services {
        for ep in &service.entrypoints {
            gen_run_pyfunction(&mut out, service, ep, api, &core_import);
        }
    }

    out
}

/// Emit the `Py{ContractName}Bridge` struct + trait impl.
///
/// Pattern mirrors the proven hand-written handler.rs: detect whether the
/// Python callable is a coroutine function; if so await it via
/// pyo3_async_runtimes; otherwise call it synchronously inside
/// `spawn_blocking` to avoid blocking the async executor.
fn gen_handler_bridge(out: &mut String, contract: &HandlerContractDef, core_import: &str) {
    let trait_name = &contract.trait_name;
    let bridge_name = format!("Py{}Bridge", trait_name.to_upper_camel_case());
    let dispatch_name = &contract.dispatch.name;

    // Determine wire types — use plain serde_json::Value, not re-exported from core
    let req_type = contract.wire_request_type.as_deref().unwrap_or("serde_json::Value");
    let resp_type = contract.wire_response_type.as_deref().unwrap_or("serde_json::Value");

    // Special handling: if the wire type includes the core import prefix, strip it
    let req_type = if req_type.contains("::") {
        req_type.split("::").last().unwrap_or(req_type)
    } else {
        req_type
    };
    let resp_type = if resp_type.contains("::") {
        resp_type.split("::").last().unwrap_or(resp_type)
    } else {
        resp_type
    };

    // Leading dispatch parameters the bridge ignores (e.g. a foreign framework type the
    // contract's dispatch method receives but the wire bridge does not consume). Their concrete
    // types cannot be reconstructed from the sanitized surface, so the library supplies them
    // verbatim via `dispatch_extra_params`. Each is emitted as a `, {decl}` prefix argument.
    let extra_param: String = contract
        .dispatch_extra_params
        .iter()
        .map(|p| format!(", {p}"))
        .collect();
    let wire_name = contract.wire_param_name.as_deref().unwrap_or("request");

    out.push_str(&format!(
        "/// Generated pyo3 bridge for the `{trait_name}` contract.\n\
         ///\n\
         /// Wraps a Python callable (sync or async) so it can be used\n\
         /// as `Arc<dyn {trait_name}>` from Rust async code.\n\
         pub struct {bridge_name} {{\n    \
             callable: Py<PyAny>,\n    \
             is_async: bool,\n\
         }}\n\n"
    ));

    out.push_str(&format!(
        "impl {bridge_name} {{\n    \
             /// Create a bridge from a Python callable.\n    \
             pub fn new(py: Python<'_>, callable: &Bound<'_, PyAny>) -> PyResult<Self> {{\n        \
                 let is_async = py\n            \
                     .import(\"inspect\")?\n            \
                     .call_method1(\"iscoroutinefunction\", (callable,))?\n            \
                     .is_truthy()\n            \
                     .unwrap_or(false);\n        \
                 Ok(Self {{\n            \
                     callable: callable.clone().unbind(),\n            \
                     is_async,\n        \
                 }})\n    \
             }}\n\
         }}\n\n"
    ));

    // Safety: The bridge holds a Py<PyAny> (GIL-independent handle) and a bool.
    // Both are Send + Sync once the GIL is not held.
    out.push_str(&format!(
        "// SAFETY: Py<PyAny> is Send+Sync when we never alias it without the GIL.\n\
         unsafe impl Send for {bridge_name} {{}}\n\
         unsafe impl Sync for {bridge_name} {{}}\n\n"
    ));

    // Trait impl — returns a boxed future directly without async_trait
    // Use proper module paths for serde_json::Value since it's not re-exported from core_import
    let req_path = if req_type == "Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{req_type}")
    };
    let resp_path = if resp_type == "Value" {
        "serde_json::Value".to_string()
    } else {
        format!("{core_import}::{resp_type}")
    };

    // The future's `Output` is the contract dispatch's real return type when the library
    // supplies one (`dispatch_return_type`); otherwise the bridge yields the wire response
    // wrapped in a boxed-error `Result`. When a `response_adapter` is configured, the inner
    // fallible computation produces the wire `Result` and the adapter converts it into the
    // dispatch return type — keeping the generator ignorant of the library's response model.
    let box_err = "Box<dyn std::error::Error + Send + Sync>";
    let wire_output = format!("Result<{resp_path}, {box_err}>");
    let output_type = contract
        .dispatch_return_type
        .clone()
        .unwrap_or_else(|| wire_output.clone());
    let tail = match &contract.response_adapter {
        Some(adapter) => format!("{adapter}(outcome)"),
        None => "outcome".to_string(),
    };

    out.push_str(&format!(
        "impl {core_import}::{trait_name} for {bridge_name} {{\n    \
             fn {dispatch_name}(\n        \
                 &self{extra_param},\n        \
                 {wire_name}: {req_path},\n    \
             ) -> Pin<Box<dyn Future<Output = {output_type}> + Send + '_>> {{\n        \
                 // Acquire Python GIL in a thread-safe context before entering the async block.\n        \
                 // Py<PyAny> holds a GIL-independent reference that can be used outside the GIL.\n        \
                 let callable = pyo3::Python::attach(|py| self.callable.clone_ref(py));\n        \
                 let is_async = self.is_async;\n\n        \
                 Box::pin(async move {{\n            \
                     let outcome: {wire_output} = async move {{\n                \
                         // Serialize the request to a Python-friendly dict via serde_json\n                \
                         let req_json = serde_json::to_string(&{wire_name})\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?;\n\n                \
                         let raw_result = if is_async {{\n                    \
                             // Async callable: hand off to pyo3_async_runtimes so it drives\n                    \
                             // the Python event loop without blocking the Tokio executor.\n                    \
                             let future = pyo3::Python::attach(|py| -> PyResult<_> {{\n                        \
                                 let req_obj = py.import(\"json\")?.call_method1(\"loads\", (&req_json,))?;\n                        \
                                 let coro = callable.call1(py, (req_obj,))?;\n                        \
                                 pyo3_async_runtimes::tokio::into_future(coro.into_bound(py))\n                    \
                             }})\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?;\n                    \
                             let py_result = future.await\n                        \
                                 .map_err(|e| Box::new(e) as {box_err})?;\n                    \
                             pyo3::Python::attach(|py| {{\n                        \
                                 let json_mod = py.import(\"json\")?;\n                        \
                                 let json_str: String = json_mod\n                            \
                                     .call_method1(\"dumps\", (py_result.bind(py),))?\n                            \
                                     .extract()?;\n                        \
                                 Ok::<String, PyErr>(json_str)\n                    \
                             }})\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?\n                \
                         }} else {{\n                    \
                             // Sync callable: run in a blocking thread so we never hold the GIL\n                    \
                             // on the async executor.\n                    \
                             tokio::task::spawn_blocking(move || {{\n                        \
                                 pyo3::Python::attach(|py| {{\n                            \
                                     let req_obj = py.import(\"json\")?.call_method1(\"loads\", (&req_json,))?;\n                            \
                                     let result = callable.call1(py, (req_obj,))?;\n                            \
                                     let json_mod = py.import(\"json\")?;\n                            \
                                     let json_str: String = json_mod\n                                \
                                         .call_method1(\"dumps\", (result.bind(py),))?\n                                \
                                         .extract()?;\n                            \
                                     Ok::<String, PyErr>(json_str)\n                        \
                                 }})\n                    \
                             }})\n                    \
                             .await\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?\n                \
                         }};\n\n                \
                         // Deserialize the JSON result back into the wire response DTO.\n                \
                         let response: {resp_path} = serde_json::from_str(&raw_result)\n                    \
                             .map_err(|e| Box::new(e) as {box_err})?;\n                \
                         Ok(response)\n            \
                     }}\n            \
                     .await;\n\n            \
                     {tail}\n        \
                 }})\n    \
             }}\n\
         }}\n\n"
    ));
}

/// Emit the `#[pyfunction]` entry point for one service × entrypoint.
///
/// The function:
/// 1. Accepts the registrations list (`list[tuple[str, tuple, Callable]]`).
/// 2. Constructs the native service owner via its constructor (zero-arg form
///    since constructor params were already captured at `__init__` time and
///    are not yet threaded through — a deliberate first-pass simplification).
/// 3. Iterates registrations, wraps each callable in the appropriate bridge,
///    and calls the owner's registration method.
/// 4. Calls the owner's entrypoint (blocking if `Run`, awaiting via Tokio if async).
fn gen_run_pyfunction(
    out: &mut String,
    service: &ServiceDef,
    ep: &crate::core::ir::EntrypointDef,
    api: &ApiSurface,
    core_import: &str,
) {
    let service_snake = service.name.to_snake_case();
    let fn_name = format!("{service_snake}_{}", ep.method);
    let owner_path = &service.rust_path;
    let ep_method = &ep.method;

    // Build the function signature: registrations + entrypoint params
    let mut rust_params = vec![
        "_py: Python<'_>".to_owned(),
        "registrations: &Bound<'_, PyList>".to_owned(),
    ];
    for p in &ep.params {
        let rust_ty = typeref_to_rust_type(&p.ty, core_import);
        rust_params.push(format!("{}: {}", p.name, rust_ty));
    }
    let param_sig = rust_params.join(", ");

    out.push_str(&format!(
        "/// Drive `{owner_path}::{ep_method}` from Python.\n\
         ///\n\
         /// Each entry in `registrations` is a `(method_name, metadata_tuple, callable)` triple\n\
         /// produced by the Python service class.\n\
         #[pyfunction]\n\
         pub fn {fn_name}({param_sig}) -> PyResult<()> {{\n"
    ));

    // Build the owner instance via its constructor
    let ctor_call = build_ctor_call(service, owner_path, core_import);
    out.push_str(&format!("    let mut owner = {ctor_call};\n\n"));

    // Iterate registrations and dispatch
    out.push_str("    for entry in registrations.iter() {\n");
    out.push_str("        let tuple: &Bound<'_, PyTuple> = entry.downcast()?;\n");
    out.push_str("        let method_name: String = tuple.get_item(0)?.extract()?;\n");
    out.push_str("        let callable = tuple.get_item(2)?;\n\n");

    // Dispatch on method name
    out.push_str("        match method_name.as_str() {\n");
    for reg in &service.registrations {
        let reg_method = &reg.method;
        let contract_name = &reg.callback_contract;

        if let Some(contract) = find_contract(api, contract_name) {
            let bridge_name = format!("Py{}Bridge", contract.trait_name.to_upper_camel_case());
            // Extract metadata params from the tuple (index 1 is the metadata sub-tuple)
            let meta_count = reg.metadata_params.len();

            out.push_str(&format!("            \"{reg_method}\" => {{\n"));
            out.push_str(&format!(
                "                let bridge = {bridge_name}::new(_py, &callable)?;\n"
            ));
            out.push_str(&format!(
                "                let handler: Arc<dyn {core_import}::{contract_name}> = Arc::new(bridge);\n"
            ));

            if meta_count > 0 {
                // Bind the metadata item to a local first — `tuple.get_item(1)?` is a temporary
                // and `.downcast()` borrows from it, so chaining would drop it while borrowed.
                out.push_str("                let meta_item = tuple.get_item(1)?;\n");
                out.push_str("                let meta: &Bound<'_, PyTuple> = meta_item.downcast()?;\n");
                for (i, meta_param) in reg.metadata_params.iter().enumerate() {
                    // A metadata param whose type is a generated opaque binding type is a
                    // `#[pyclass]` wrapping `inner: Arc<core>`. pyo3 can only extract the BINDING
                    // pyclass, not the core type the owner method expects — so extract the binding
                    // type and unwrap `.inner` to core. (`service` is a descendant of the crate
                    // root where the pyclass is defined, so the private `inner` field is in scope.)
                    let opaque_named = match &meta_param.ty {
                        TypeRef::Named(n) => api
                            .types
                            .iter()
                            .find(|t| &t.name == n && !t.is_trait && t.is_opaque)
                            .map(|_| n.clone()),
                        _ => None,
                    };
                    if let Some(name) = opaque_named {
                        out.push_str(&format!(
                            "                let {pname}_binding: crate::{name} = meta.get_item({i})?.extract()?;\n",
                            pname = meta_param.name,
                        ));
                        out.push_str(&format!(
                            "                let {pname}: {core_import}::{name} = (*{pname}_binding.inner).clone();\n",
                            pname = meta_param.name,
                        ));
                    } else {
                        let rust_ty = typeref_to_rust_type(&meta_param.ty, core_import);
                        out.push_str(&format!(
                            "                let {}: {} = meta.get_item({i})?.extract()?;\n",
                            meta_param.name, rust_ty,
                        ));
                    }
                }
                let meta_args: Vec<String> = reg.metadata_params.iter().map(|p| p.name.clone()).collect();
                out.push_str(&format!(
                    "                owner.{reg_method}({}, handler)\n",
                    meta_args.join(", ")
                ));
            } else {
                out.push_str(&format!("                owner.{reg_method}(handler)\n"));
            }

            // Handle error if the registration is fallible
            if reg.error_type.is_some() {
                out.push_str(
                    "                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n",
                );
            } else {
                out.push_str("                    ;\n");
            }
            out.push_str("            }\n");
        }
    }
    out.push_str("            _ => {\n");
    out.push_str(
        "                return Err(pyo3::exceptions::PyValueError::new_err(\n                    \
         format!(\"unknown registration method: {method_name}\"),\n                ));\n",
    );
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    // Call the entrypoint
    let ep_call = build_ep_call(ep, service, core_import);
    out.push_str(&ep_call);

    out.push_str("    Ok(())\n}\n\n");
}

/// Build the Rust constructor call for the service owner.
fn build_ctor_call(service: &ServiceDef, owner_path: &str, _core_import: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        // For a first-pass implementation where constructor params are not
        // yet threaded through, fall back to Default if available; otherwise
        // use new() with zero-value placeholders.
        // Callers can always extend by adding constructor params to the pyfunction
        // signature in a follow-up pass.
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
fn build_ep_call(ep: &crate::core::ir::EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");
    // Bind non-Unit returns to `_` so the unwrapped value (after `?`-propagation) doesn't
    // trigger `unused_must_use` for `Result`-returning entrypoints like `into_router`.
    let bind = if matches!(ep.return_type, TypeRef::Unit) {
        ""
    } else {
        "let _ = "
    };

    if ep.is_async {
        // Drive the async entrypoint on the Tokio runtime that pyo3_async_runtimes
        // already configured. The GIL is released for the duration of the (potentially
        // long-running, blocking) entrypoint so host callbacks invoked from within it can
        // re-acquire the GIL — holding it here would deadlock any callback that needs it.
        format!(
            "    {bind}_py.detach(|| {{\n        \
             pyo3_async_runtimes::tokio::get_runtime().block_on(owner.{ep_method}({args_str}))\n    \
             }})\n        \
             .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n"
        )
    } else if ep.error_type.is_some() {
        format!(
            "    {bind}owner.{ep_method}({args_str})\n        \
             .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;\n"
        )
    } else {
        format!("    {bind}owner.{ep_method}({args_str});\n")
    }
}

/// Map a `TypeRef` to a Rust type string for use in generated function signatures.
fn typeref_to_rust_type(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "u8".to_owned(),
                PrimitiveType::U16 => "u16".to_owned(),
                PrimitiveType::U32 => "u32".to_owned(),
                PrimitiveType::U64 => "u64".to_owned(),
                PrimitiveType::I8 => "i8".to_owned(),
                PrimitiveType::I16 => "i16".to_owned(),
                PrimitiveType::I32 => "i32".to_owned(),
                PrimitiveType::I64 => "i64".to_owned(),
                PrimitiveType::F32 => "f32".to_owned(),
                PrimitiveType::F64 => "f64".to_owned(),
                PrimitiveType::Usize => "usize".to_owned(),
                PrimitiveType::Isize => "isize".to_owned(),
            }
        }
        TypeRef::Bytes => "Vec<u8>".to_owned(),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_rust_type(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_rust_type(k, core_import),
            typeref_to_rust_type(v, core_import)
        ),
        TypeRef::Unit => "()".to_owned(),
        TypeRef::Named(n) => format!("{core_import}::{n}"),
        TypeRef::Json => "serde_json::Value".to_owned(),
        TypeRef::Path => "std::path::PathBuf".to_owned(),
        TypeRef::Duration => "std::time::Duration".to_owned(),
    }
}

// ──────────────────────────────────────────────────────── public entry point ──

/// Generate all service-API files for the pyo3 backend.
///
/// Returns up to two `GeneratedFile`s per non-empty service list:
/// - `{output_dir}/service.rs`   — Rust pyo3 glue
/// - `{python_pkg}/service.py`   — idiomatic Python class
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("python"), &config.name, "crates/{name}-py/src/");
    let module_name = config.python_module_name();

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // Python wrapper
    let service_py = gen_service_py(api, &module_name);

    // Python package output base (same logic as generate_public_api)
    let output_base = config
        .python
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| PathBuf::from(&s.output))
        .unwrap_or_else(|| {
            let package_name = config.name.replace('-', "_");
            PathBuf::from(format!("packages/python/{}", package_name))
        });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("service.py"),
            content: service_py,
            generated_header: true,
        },
    ])
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
        ServiceDef, TypeRef,
    };

    /// Construct a minimal but realistic [`ApiSurface`] that exercises:
    /// - A service with a constructor, one configurator, one registration
    ///   (bound to an async handler contract), and Run + Finalize entrypoints.
    /// - One [`HandlerContractDef`] with wire request/response DTO names.
    fn make_fixture_surface() -> ApiSurface {
        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: "Create a new service owner.".to_owned(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let configurator = MethodDef {
            name: "with_timeout".to_owned(),
            params: vec![ParamDef {
                name: "timeout_ms".to_owned(),
                ty: TypeRef::Primitive(PrimitiveType::U64),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("TestService".to_owned()),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: "Set request timeout.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let registration = RegistrationDef {
            method: "add_handler".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "method".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: "Register a request handler for a path and method.".to_owned(),
            variants: vec![],
        };

        let run_ep = EntrypointDef {
            method: "run".to_owned(),
            kind: EntrypointKind::Run,
            is_async: true,
            params: vec![ParamDef {
                name: "addr".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            error_type: Some("ServiceError".to_owned()),
            doc: "Run the service.".to_owned(),
        };

        let finalize_ep = EntrypointDef {
            method: "into_router".to_owned(),
            kind: EntrypointKind::Finalize,
            is_async: false,
            params: vec![],
            return_type: TypeRef::Named("Router".to_owned()),
            error_type: None,
            doc: "Consume and convert into a router.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![configurator],
            registrations: vec![registration],
            entrypoints: vec![run_ep, finalize_ep],
            doc: "A test service owner.".to_owned(),
            cfg: None,
        };

        let dispatch_method = MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "request".to_owned(),
                ty: TypeRef::Named("RequestData".to_owned()),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("ResponseData".to_owned()),
            is_async: true,
            is_static: false,
            error_type: Some("HandlerError".to_owned()),
            doc: "Dispatch a request.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: dispatch_method,
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("ResponseData".to_owned()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: "Async trait for handling requests.".to_owned(),
        };

        ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![contract],
            ..ApiSurface::default()
        }
    }

    /// `gen_service_py` emits a class named after the service owner.
    #[test]
    fn python_output_contains_service_class() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("class TestService:"),
            "expected `class TestService:` in output:\n{output}"
        );
    }

    /// `gen_service_py` emits `__init__` with registration state initialisation.
    #[test]
    fn python_output_contains_init_with_registrations() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def __init__(self)"),
            "expected `def __init__(self)` in output:\n{output}"
        );
        assert!(
            output.contains("self._registrations"),
            "expected `self._registrations` in output:\n{output}"
        );
    }

    /// `gen_service_py` emits configurator methods that return `self`.
    #[test]
    fn python_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def with_timeout(self, timeout_ms: int)"),
            "expected `with_timeout` configurator:\n{output}"
        );
        assert!(
            output.contains("return self"),
            "expected `return self` in configurator:\n{output}"
        );
    }

    /// `gen_service_py` emits a decorator for the registration method.
    #[test]
    fn python_output_contains_registration_decorator() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def add_handler("),
            "expected `add_handler` registration method:\n{output}"
        );
        assert!(
            output.contains("def _decorator(fn"),
            "expected inner `_decorator` closure:\n{output}"
        );
        assert!(
            output.contains("self._registrations.append"),
            "expected `_registrations.append` in decorator:\n{output}"
        );
    }

    /// `gen_service_py` emits the `run` entrypoint.
    #[test]
    fn python_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def run(self"),
            "expected `def run(self` entrypoint:\n{output}"
        );
        assert!(
            output.contains("_my_crate.test_service_run("),
            "expected native call `_my_crate.test_service_run(` in run:\n{output}"
        );
    }

    /// `gen_service_py` emits registration variants with both method and decorator forms.
    #[test]
    fn python_output_contains_registration_variants() {
        let mut surface = make_fixture_surface();
        // Add a variant to the registration
        let variant = crate::core::ir::RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: Some(crate::core::ir::WrapperConstructorCall {
                metadata_param: "builder".to_owned(),
                wrapper_type_path: "mylib::RouteBuilder".to_owned(),
                wrapper_type_name: "RouteBuilder".to_owned(),
                constructor_method: "new".to_owned(),
                args: vec![
                    crate::core::ir::WrapperConstructorArg::Fixed {
                        param_name: "method".to_owned(),
                        value_expr: "mylib::Method::GET".to_owned(),
                    },
                    crate::core::ir::WrapperConstructorArg::Free {
                        param: ParamDef {
                            name: "path".to_owned(),
                            ty: TypeRef::String,
                            optional: false,
                            default: None,
                            ..ParamDef::default()
                        },
                    },
                ],
            }),
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET handler.".to_owned()),
            style: crate::core::ir::RegistrationVariantStyle::Hybrid,
        };
        surface.services[0].registrations[0].variants.push(variant);

        let output = gen_service_py(&surface, "_my_crate");
        // Check for variant method form
        assert!(
            output.contains("def get(self, path: str, handler: Callable[..., Any])"),
            "expected `def get(self, path: str, handler)` method form:\n{output}"
        );
        // Check for variant decorator form
        assert!(
            output.contains("def get_decorator(self, path: str)"),
            "expected `def get_decorator(self, path: str)` decorator form:\n{output}"
        );
        // Check for wrapper constructor call (pyo3 opaque wrappers expose `.new()` classmethod)
        assert!(
            output.contains("builder = RouteBuilder.new(Method.GET, path)"),
            "expected wrapper constructor call with Method.GET:\n{output}"
        );
        // Wrapper-consumed params (path, method) must NOT appear in the metadata tuple
        assert!(
            output.contains("(\"add_handler\", (builder,), handler)"),
            "expected metadata tuple to contain only the constructed wrapper:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct PyRequestHandlerBridge"),
            "expected `PyRequestHandlerBridge` struct:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge trait impl.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for PyRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn Future<Output"),
            "expected dispatch method returning a boxed future:\n{output}"
        );
    }

    /// `gen_service_rs` emits the `#[pyfunction]` run entry point.
    #[test]
    fn rust_output_contains_pyfunction_run() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[pyfunction]"),
            "expected `#[pyfunction]` attribute:\n{output}"
        );
        assert!(
            output.contains("pub fn test_service_run("),
            "expected `test_service_run` function:\n{output}"
        );
    }

    /// `gen_service_rs` emits registration dispatch via `match method_name`.
    #[test]
    fn rust_output_contains_registration_dispatch() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("\"add_handler\""),
            "expected `\"add_handler\"` match arm:\n{output}"
        );
        assert!(
            output.contains("Arc<dyn my_crate::RequestHandler>"),
            "expected Arc wrapping of handler:\n{output}"
        );
    }

    /// Full `generate()` call returns two files when services are non-empty.
    #[test]
    fn generate_returns_two_files_for_non_empty_services() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert_eq!(files.len(), 2, "expected 2 generated files, got {}", files.len());
        let paths: Vec<&str> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(paths.contains(&"service.rs"), "expected service.rs in output");
        assert!(paths.contains(&"service.py"), "expected service.py in output");
    }

    /// Full `generate()` returns empty for a surface with no services.
    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_test_config() -> ResolvedCrateConfig {
        use crate::core::config::resolved::ResolvedCrateConfig;
        ResolvedCrateConfig {
            name: "my-crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }
}
