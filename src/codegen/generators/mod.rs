use ahash::AHashMap;

pub mod binding_helpers;
pub mod dto_coercion;
pub mod enums;
pub mod functions;
pub mod methods;
pub mod structs;
pub mod trait_bridge;
pub mod type_paths;

/// Map of adapter-generated method/function bodies.
/// Key: "TypeName.method_name" for methods, "function_name" for free functions.
pub type AdapterBodies = AHashMap<String, String>;

/// Whether any enum payload is classified as sensitive for diagnostic representations. ~keep
pub fn enum_has_sensitive_representation(enum_def: &crate::core::ir::EnumDef) -> bool {
    enum_def
        .variants
        .iter()
        .any(|variant| variant.sensitive || variant.fields.iter().any(|field| field.sensitive))
}

/// Emit a `Debug` implementation that keeps ordinary fields useful and redacts sensitive ones. ~keep
pub fn gen_redacted_struct_debug_impl<'a>(
    type_name: &str,
    cfg: Option<&str>,
    fields: impl IntoIterator<Item = &'a crate::core::ir::FieldDef>,
) -> String {
    let fields: Vec<&crate::core::ir::FieldDef> = fields.into_iter().collect();
    if !fields.iter().any(|field| field.sensitive) {
        return String::new();
    }

    let mut out = String::new();
    if let Some(cfg) = cfg {
        out.push_str(&format!("#[cfg({cfg})]\n"));
    }
    out.push_str(&format!(
        "impl std::fmt::Debug for {type_name} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        let mut debug = f.debug_struct(\"{type_name}\");\n"
    ));
    for field in fields {
        if let Some(cfg) = &field.cfg {
            out.push_str(&format!("        #[cfg({cfg})]\n"));
        }
        let name = crate::codegen::naming::internal_rust_identifier(&field.name);
        if field.sensitive {
            out.push_str(&format!("        debug.field(\"{}\", &\"<redacted>\");\n", field.name));
        } else {
            out.push_str(&format!("        debug.field(\"{}\", &self.{name});\n", field.name));
        }
    }
    out.push_str("        debug.finish()\n    }\n}\n");
    out
}

/// Emit a representation-safe `Debug` implementation for a sensitive enum. ~keep
pub fn gen_redacted_enum_debug_impl(enum_def: &crate::core::ir::EnumDef) -> String {
    if !enum_has_sensitive_representation(enum_def) {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(&format!(
        "impl std::fmt::Debug for {} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        f.write_str(\"{}(<redacted>)\")\n    }}\n}}\n",
        enum_def.name, enum_def.name
    ));
    out
}

#[cfg(test)]
pub(crate) mod sensitive_runtime_test_support {
    use quote::quote;
    use std::path::{Path, PathBuf};

    pub fn assert_struct_debug_and_serde_round_trip(generated: &str, type_name: &str) {
        let source = runtime_probe_source(generated, type_name);
        compile_and_run(&source);
    }

    pub fn assert_enum_debug_and_serde_round_trip(generated: &str, type_name: &str) {
        let file = syn::parse_file(generated).expect("generated Rust must parse");
        let mut enumeration = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == type_name => Some(item.clone()),
                _ => None,
            })
            .expect("generated mirror enum must exist");
        enumeration.attrs.clear();
        for variant in &mut enumeration.variants {
            variant.attrs.clear();
            for field in &mut variant.fields {
                field.attrs.clear();
            }
        }
        let debug_impl = debug_impl(&file, type_name);
        let source = format!(
            r#"
#[derive(serde::Serialize, serde::Deserialize)]
{enumeration}
{debug_impl}

fn main() {{
    let secret = "planted-secret".to_string();
    let authentication = {type_name}::Bearer {{ token: secret.clone() }};
    let rendered = format!("{{authentication:?}}");
    assert!(!rendered.contains(&secret));
    let json = serde_json::to_string(&authentication).unwrap();
    assert!(json.contains(&secret));
    let decoded: {type_name} = serde_json::from_str(&json).unwrap();
    let {type_name}::Bearer {{ token }} = decoded;
    assert_eq!(token, secret);
}}
"#,
            enumeration = quote!(#enumeration),
            debug_impl = quote!(#debug_impl),
        );
        compile_and_run(&source);
    }

    fn runtime_probe_source(generated: &str, type_name: &str) -> String {
        let file = syn::parse_file(generated).expect("generated Rust must parse");
        let mut structure = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == type_name => Some(item.clone()),
                _ => None,
            })
            .expect("generated mirror struct must exist");
        structure.attrs.clear();
        for field in &mut structure.fields {
            field.attrs.clear();
        }
        let debug_impl = debug_impl(&file, type_name);
        format!(
            r#"
#[derive(serde::Serialize, serde::Deserialize)]
{structure}
{debug_impl}

fn main() {{
    let secret = "planted-secret".to_string();
    let credentials = {type_name} {{ label: "public".to_string(), token: secret.clone() }};
    let rendered = format!("{{credentials:?}}");
    assert!(rendered.contains("public"));
    assert!(!rendered.contains(&secret));
    let json = serde_json::to_string(&credentials).unwrap();
    assert!(json.contains(&secret));
    let decoded: {type_name} = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.token, secret);
}}
"#,
            structure = quote!(#structure),
            debug_impl = quote!(#debug_impl),
        )
    }

    fn debug_impl<'a>(file: &'a syn::File, type_name: &str) -> &'a syn::ItemImpl {
        file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Impl(item)
                    if item
                        .trait_
                        .as_ref()
                        .and_then(|(_, path, _)| path.segments.last())
                        .is_some_and(|segment| segment.ident == "Debug")
                        && matches!(item.self_ty.as_ref(), syn::Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == type_name)) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .expect("generated redacted Debug implementation must exist")
    }

    fn compile_and_run(source: &str) {
        let directory = tempfile::tempdir().expect("temporary Rust representation probe");
        let source_path = directory.path().join("probe.rs");
        let executable = directory.path().join(format!("probe{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&source_path, source).expect("representation probe source must be written");
        let dependencies = std::env::current_exe()
            .expect("test executable path")
            .parent()
            .expect("test dependency directory")
            .to_path_buf();
        let serde = dependency_rlib(&dependencies, "serde");
        let serde_json = dependency_rlib(&dependencies, "serde_json");
        let compile = std::process::Command::new("rustc")
            .args(["--edition=2024", "-L"])
            .arg(format!("dependency={}", dependencies.display()))
            .arg("--extern")
            .arg(format!("serde={}", serde.display()))
            .arg("--extern")
            .arg(format!("serde_json={}", serde_json.display()))
            .arg(&source_path)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("representation probe must compile");
        assert!(
            compile.status.success(),
            "representation probe failed to compile:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let output = std::process::Command::new(executable)
            .output()
            .expect("representation probe must run");
        assert!(
            output.status.success(),
            "representation probe failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn dependency_rlib(directory: &Path, crate_name: &str) -> PathBuf {
        let prefix = format!("lib{crate_name}-");
        let mut matches: Vec<PathBuf> = std::fs::read_dir(directory)
            .expect("test dependency directory must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".rlib"))
            })
            .collect();
        matches.sort();
        matches
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{crate_name} rlib must exist in {}", directory.display()))
    }
}

/// Async support pattern for the backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncPattern {
    /// No async support
    None,
    /// PyO3: pyo3_async_runtimes::tokio::future_into_py
    Pyo3FutureIntoPy,
    /// NAPI-RS: native async fn → auto-Promise
    NapiNativeAsync,
    /// wasm-bindgen: native async fn → auto-Promise
    WasmNativeAsync,
    /// Block on Tokio runtime (Ruby, PHP)
    TokioBlockOn,
}

/// Configuration for Rust binding code generation.
pub struct RustBindingConfig<'a> {
    /// Attrs applied to generated structs, e.g. `["pyclass(frozen)"]`.
    pub struct_attrs: &'a [&'a str],
    /// Attrs applied to each field, e.g. `["pyo3(get)"]`.
    pub field_attrs: &'a [&'a str],
    /// Derives applied to generated structs, e.g. `["Clone"]`.
    pub struct_derives: &'a [&'a str],
    /// Attr wrapping the impl block, e.g. `Some("pymethods")`.
    pub method_block_attr: Option<&'a str>,
    /// Attr placed on the constructor, e.g. `"#[new]"`.
    pub constructor_attr: &'a str,
    /// Attr placed on static methods, e.g. `Some("staticmethod")`.
    pub static_attr: Option<&'a str>,
    /// Attr placed on free functions, e.g. `"#[pyfunction]"`.
    pub function_attr: &'a str,
    /// Attrs applied to generated enums, e.g. `["pyclass(eq, eq_int)"]`.
    pub enum_attrs: &'a [&'a str],
    /// Derives applied to generated enums, e.g. `["Clone", "PartialEq"]`.
    pub enum_derives: &'a [&'a str],
    /// Whether the backend requires `#[pyo3(signature = (...))]`-style annotations.
    pub needs_signature: bool,
    /// Prefix for the signature annotation, e.g. `"#[pyo3(signature = ("`.
    pub signature_prefix: &'a str,
    /// Suffix for the signature annotation, e.g. `"))]"`.
    pub signature_suffix: &'a str,
    /// Core crate import path, e.g. `"sample_llm"`. Used to generate calls into core.
    pub core_import: &'a str,
    /// Async pattern supported by this backend.
    pub async_pattern: AsyncPattern,
    /// Whether serde/serde_json are available in the output crate's dependencies.
    /// When true, the generator can use serde-based param conversion and add `serde::Serialize` derives.
    /// When false, non-convertible Named params fall back to `gen_unimplemented_body`.
    pub has_serde: bool,
    /// Prefix for binding type names (e.g. "Js" for NAPI/WASM, "" for PyO3/PHP).
    /// Used in impl block targets: `impl {prefix}{TypeName}`.
    pub type_name_prefix: &'a str,
    /// When true, non-optional Duration fields on `has_default` types are emitted as
    /// `Option<u64>` in the binding struct so that unset fields fall back to the core
    /// type's `Default` implementation rather than `Duration::ZERO`.
    /// Used by PyO3 to prevent validation failures when `request_timeout` is unset.
    pub option_duration_on_defaults: bool,
    /// Opaque type names. Structs with non-optional fields of these types
    /// skip `Default`/`Serialize`/`Deserialize` derives since opaque wrappers don't impl them.
    pub opaque_type_names: &'a [String],
    /// When true, the impl block constructor (`fn new(...)`) is suppressed regardless of
    /// whether the type has fields. Useful for backends (e.g. extendr) that generate a
    /// separate kwargs-style free-function constructor instead of an in-class `new()`.
    pub skip_impl_constructor: bool,
    /// When true, small unsigned/signed ints (u8, u16, u32, i8, i16) are cast from i32 in
    /// `gen_lossy_binding_to_core_fields`. Used by the extendr backend where R maps small
    /// ints to i32.
    pub cast_uints_to_i32: bool,
    /// When true, large int/size types (u64, usize, isize) are cast from f64 in
    /// `gen_lossy_binding_to_core_fields`. Used by the extendr backend where R maps large
    /// ints to f64.
    pub cast_large_ints_to_f64: bool,
    /// When true, Named non-opaque struct parameters in free function signatures are emitted
    /// as `&T` (reference) instead of `T` (owned). Required for the extendr backend because
    /// `#[extendr]` only generates `TryFrom<&Robj> for &T`, not `for T`, so owned struct
    /// params cannot be passed through the FFI layer.
    pub named_non_opaque_params_by_ref: bool,
    /// Types that have no `From<BindingType>` impl (e.g. output-only flat data enums).
    /// When `gen_lossy_binding_to_core_fields` encounters a field whose `TypeRef::Named` type
    /// is in this slice, it emits `Default::default()` instead of `.clone().into()`.
    pub lossy_skip_types: &'a [String],
    /// Subset of `opaque_type_names` whose binding wrappers DO implement
    /// `serde::Serialize`/`Deserialize` (e.g. data-enum wrappers via `gen_pyo3_data_enum`,
    /// which emit forwarding impls delegating to the core type). Fields whose type
    /// references a name in this slice will NOT receive `#[serde(skip)]`, even when
    /// the name is also in `opaque_type_names`. Required so `from_json`/`to_json`
    /// round-trips on parent structs (e.g. `ChatCompletionRequest.messages: Vec<Message>`)
    /// don't silently drop the field to `Default::default()`.
    pub serializable_opaque_type_names: &'a [String],
    /// Field names that should NOT be skipped even if they are cfg-gated.
    /// Used when the binding crate enables the feature that gates the field,
    /// so the field must appear in the binding struct and From impl.
    /// Typically populated from trait bridge options field names.
    pub never_skip_cfg_field_names: &'a [String],
    /// When true and the core type has a custom `Default` impl (`typ.has_default == true`),
    /// the binding struct's auto-derived `Default` is suppressed and a delegating
    /// `impl Default for BindingType` is emitted that delegates to
    /// `<core::Type as Default>::default().into()`. This preserves the core type's custom
    /// default values (e.g. `max_redirects: 10`) instead of falling back to primitive
    /// Rust defaults (e.g. `max_redirects: 0`) when partial JSON missing the field is
    /// deserialised via a struct-level `#[serde(default)]`.
    ///
    /// Requires that `From<core::Type> for BindingType` is emitted for the type, which is
    /// the case for any non-opaque type that passes `can_generate_conversion(typ, &core_to_binding)`.
    /// This is also useful for host constructors that use `Self::default()` or
    /// `unwrap_or_default()` for omitted nested config fields.
    pub emit_delegating_default_impl: bool,

    /// When true, methods that cannot be auto-delegated AND have no adapter override are
    /// silently skipped from emitted impl blocks (mirroring the PHP backend's pattern)
    /// instead of emitting a `compile_error!` stub. Required for backends whose host
    /// language cannot meaningfully panic at startup (e.g. extendr's `#[extendr]` macro
    /// fails to expand around `compile_error!` bodies, breaking the whole crate).
    pub skip_methods_when_not_delegatable: bool,
    /// Remaps the leading crate segment in IR `rust_path` values when building the
    /// delegating `impl Default` body. When `core_crate_override` is set for a language,
    /// IR rust_paths still reference the original source crate; these remaps rewrite the
    /// Default body so it references the override crate instead.
    /// E.g. `[("mylib_core", "mylib_http")]` rewrites `mylib_core::T` → `mylib_http::T`.
    pub source_crate_remaps: &'a [(&'a str, &'a str)],
    /// When `Some(set)`, the delegating `impl Default` (and the corresponding suppression
    /// of `#[derive(Default)]`) is restricted to types whose name is in this set. Ensures
    /// that a delegating Default is only emitted when the matching `From<core::T>` impl
    /// will also be emitted. When `None`, all `has_default` types get the delegating impl
    /// (backward-compatible behaviour for backends that do not need the restriction).
    pub emit_delegating_default_for_types: Option<&'a ahash::AHashSet<String>>,
    /// When `Some(set)`, a struct whose name is in this set gets a hand-written `Deserialize`
    /// that delegates to the *core* type's own `Deserialize` -- honouring a real
    /// `#[serde(from/into/try_from/transparent)]` wire shape alef cannot otherwise reproduce --
    /// instead of the derived, field-by-field object `Deserialize`. Actually delegating still
    /// requires `structs::struct_wants_deserialize_delegation` to independently confirm the
    /// struct is field-sound for it (see that function); this set's only job is guaranteeing
    /// that `From<core::Type> for BindingType` is emitted for every type it names, so the
    /// delegating impl's `.into()` always compiles. Populate it with (a subset of) the same
    /// "core→binding convertible" set that already gates the backend's own
    /// `gen_from_core_to_binding_cfg` calls. `None` means the caller has not wired this, so no
    /// struct in this run gets delegation. ~keep
    pub delegate_deserialize_to_core_for_types: Option<&'a ahash::AHashSet<String>>,
    /// `{error}_to_py_err` converter function names already emitted for this crate's
    /// `api.errors` (see `crate::codegen::error_gen::gen_pyo3_error_converter`). When a free
    /// function's declared `error_type` has a matching converter here AND `async_pattern` is
    /// `AsyncPattern::Pyo3FutureIntoPy`, its `.map_err(...)` conversion routes through that
    /// converter instead of collapsing every error kind into `pyo3::exceptions::PyRuntimeError`
    /// (alef #452) -- the same converter the trait-bridge and capsule call sites already use, so
    /// `except {Variant}Error:` catches the right typed exception from a free function too.
    /// `None` (the default for every backend other than pyo3, and for any pyo3 call site that
    /// has not wired this) preserves today's generic conversion unchanged. ~keep
    pub error_converters: Option<&'a [String]>,
}

/// Method names that conflict with standard trait methods.
/// When a generated method has one of these names, we add
/// `#[allow(clippy::should_implement_trait)]` to suppress the lint.
pub(super) const TRAIT_METHOD_NAMES: &[&str] = &[
    "default", "from", "from_str", "into", "eq", "ne", "lt", "le", "gt", "ge", "add", "sub", "mul", "div", "rem",
    "neg", "not", "index", "deref",
];

pub use binding_helpers::{
    can_auto_delegate_function_with_named_let_bindings, gen_async_body, gen_call_args, gen_call_args_no_promote,
    gen_call_args_with_let_bindings, gen_call_args_with_let_bindings_json_str, gen_call_args_with_let_bindings_mutex,
    gen_call_args_with_let_bindings_mutex_json_str, gen_call_args_with_let_bindings_mutex_no_promote,
    gen_call_args_with_let_bindings_no_promote, gen_lossy_binding_to_core_fields, gen_lossy_binding_to_core_fields_mut,
    gen_named_let_bindings_no_promote, gen_named_let_bindings_pub, gen_named_let_bindings_with_augmented,
    gen_serde_let_bindings, gen_unimplemented_body, has_named_params, is_simple_non_opaque_param, wrap_return,
    wrap_return_with_mutex, wrap_return_with_mutex_mapped,
};
pub use dto_coercion::{
    CoercibleShape, PYO3_DTO_COERCE_HELPER, coercible_payload, data_enum_needs_dto_coercion,
    pyo3_wire_schema_const_name,
};
pub(crate) use enums::{
    collect_all_variant_constructors, collect_pyo3_variant_constructors, collect_variant_accessors,
    variant_constructor_is_reachable, variant_field_init,
};
pub use enums::{
    enum_has_data_variants, enum_has_sanitized_fields, gen_enum, gen_pyo3_data_enum, gen_pyo3_data_enum_with_coercion,
    gen_pyo3_data_enum_with_mapper,
};
pub use functions::{
    collect_explicit_core_imports, collect_trait_imports, gen_function, gen_function_with_mutex,
    has_unresolved_trait_methods,
};
pub use methods::{
    gen_constructor, gen_constructor_with_renames, gen_impl_block, gen_impl_block_with_renames, gen_method,
    gen_opaque_constructor, gen_opaque_impl_block, gen_static_method, is_trait_method_name,
};
pub use structs::{
    can_generate_default_impl, gen_delegating_default_impl, gen_delegating_deserialize_impl, gen_opaque_struct,
    gen_opaque_struct_prefixed, gen_struct, gen_struct_default_impl, gen_struct_with_per_field_attrs,
    gen_struct_with_rename, struct_deserialize_delegation_field_sound, struct_wants_deserialize_delegation,
    type_needs_mutex, type_needs_tokio_mutex,
};
