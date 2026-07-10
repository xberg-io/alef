use minijinja::context;

pub(super) fn gen_streaming_adapter_facade_method(
    adapter: &crate::core::config::AdapterConfig,
    _mapper: &crate::backends::php::type_map::PhpMapper,
    _opaque_types: &ahash::AHashSet<String>,
    _core_import: &str,
) -> String {
    use heck::ToLowerCamelCase;

    let method_name = adapter.name.to_lower_camel_case();
    let adapter_name = &adapter.name;
    let owner_type = adapter.owner_type.as_deref().unwrap_or_else(|| {
        panic!(
            "php adapter `{adapter_name}`: streaming adapter requires `owner_type` in `[[adapters]]` config (the Rust handle type that owns the streaming method)"
        )
    });

    let mut params: Vec<String> = vec![format!("engine: &{owner_type}")];

    for p in &adapter.params {
        let param_type = p.ty.rsplit("::").next().unwrap_or(&p.ty);
        let ref_indicator = if matches!(param_type, "String" | "Vec<String>") {
            ""
        } else {
            "&"
        };
        let nullable = if p.optional { "Option<" } else { "" };
        let close_nullable = if p.optional { ">" } else { "" };
        params.push(format!(
            "{}: {}{}{}{}",
            p.name, ref_indicator, nullable, param_type, close_nullable
        ));
    }

    let return_type = "std::result::Result<Vec<String>, ext_php_rs::exception::PhpException>";

    let rust_method_name = &adapter.name;
    let call_args = adapter
        .params
        .iter()
        .map(|p| format!("&{}", p.name))
        .collect::<Vec<_>>()
        .join(", ");

    crate::backends::php::template_env::render(
        "php_streaming_adapter_method.jinja",
        context! {
            method_name => method_name,
            params => &params.join(", "),
            return_type => return_type,
            rust_method_name => rust_method_name,
            call_args => &call_args,
        },
    )
}

/// Check if an opaque type has a no-arg `pub fn new() -> Self` (not Result).
pub(super) fn has_no_arg_new_returning_self(typ: &crate::core::ir::TypeDef) -> bool {
    typ.methods
        .iter()
        .any(|m| m.name == "new" && m.receiver.is_none() && m.params.is_empty() && m.error_type.is_none())
}

pub(crate) fn php_variant_wrapper_constructor_method(
    typ: &crate::core::ir::TypeDef,
    mapper: &crate::backends::php::type_map::PhpMapper,
    core_import: &str,
    opaque_types: &ahash::AHashSet<String>,
) -> Option<String> {
    use crate::codegen::type_mapper::TypeMapper as _;
    let ctor = typ.methods.iter().find(|m| m.name == "new" && m.receiver.is_none())?;
    let map_fn = |t: &crate::core::ir::TypeRef| mapper.map_type(t);
    let sig_params = crate::codegen::shared::function_params(&ctor.params, &map_fn);
    let call_args =
        crate::backends::php::gen_bindings::helpers::gen_php_call_args(&ctor.params, opaque_types, &mapper.enum_names);
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);
    let body = if call_args.is_empty() {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new()) }}")
    } else {
        format!("Self {{ inner: std::sync::Arc::new({core_path}::new({call_args})) }}")
    };
    let fn_sig = if sig_params.is_empty() {
        "pub fn new() -> Self".to_string()
    } else {
        format!("pub fn new({sig_params}) -> Self")
    };
    Some(format!("#[php(constructor)]\n{fn_sig} {{\n    {body}\n}}"))
}

/// Generate config.m4 for PIE (PHP Installer for Extensions) to enable building Rust-based PHP extensions.
///
/// PHPize expects config.m4 to describe the build configuration. For Rust extensions built
/// with ext-php-rs, we generate a minimal config.m4 that informs phpize of the extension name
/// and directs the build to use cargo. This allows PIE to fall back from pre-packaged binaries
/// to source compilation without errors.
pub(super) fn generate_config_m4(extension_name: &str, package_name: &str) -> String {
    let cargo_crate_name = package_name;
    let lib_name = extension_name.replace('_', "-");

    format!(
        r#"dnl Configuration for Rust-based PHP extension via ext-php-rs.
dnl Allows phpize to recognize this extension during source compilation (PIE fallback).

PHP_ARG_ENABLE([{}],
  [whether to enable the {} extension],
  [AS_HELP_STRING([--enable-{}],
    [Enable {} extension support])],
  [yes])

if test "$PHP_{}_ENABLED" = "yes"; then
  dnl Register the extension directory so phpize creates modules/ and sets up build rules.
  PHP_NEW_EXTENSION({}, [], $ext_shared)

  dnl Invoke cargo build to compile the Rust FFI library and copy it to modules/.
  AC_CONFIG_COMMANDS([cargo-build], [
    if test -f "crates/{}-php/Cargo.toml"; then
      (cd crates/{}-php && cargo build --release) || exit 1

      dnl Detect output filename based on platform
      if test -f "crates/{}-php/target/release/lib{}_php.dylib"; then
        cargo_lib="crates/{}-php/target/release/lib{}_php.dylib"
      elif test -f "crates/{}-php/target/release/lib{}_php.so"; then
        cargo_lib="crates/{}-php/target/release/lib{}_php.so"
      else
        echo "ERROR: cargo build succeeded but .so/.dylib not found in crates/{}-php/target/release" >&2
        exit 1
      fi

      mkdir -p modules
      cp "$cargo_lib" "modules/{}.so" || exit 1
    else
      echo "ERROR: crates/{}-php/Cargo.toml not found" >&2
      exit 1
    fi
  ], [])
fi
"#,
        extension_name,
        extension_name,
        extension_name,
        extension_name,
        extension_name.to_uppercase(),
        extension_name,
        cargo_crate_name,
        cargo_crate_name,
        cargo_crate_name,
        lib_name,
        cargo_crate_name,
        lib_name,
        cargo_crate_name,
        lib_name,
        cargo_crate_name,
        cargo_crate_name,
        extension_name,
        cargo_crate_name,
        extension_name,
    )
}
