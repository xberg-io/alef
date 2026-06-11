//! Dart test-backend stub generation helpers.

use crate::core::ir::TypeRef;
use crate::e2e::fixture::Fixture;

use crate::e2e::codegen::TestBackendEmission;

/// Emit a Dart test backend stub class for a trait bridge.
///
/// Generates a concrete subclass of the trait's abstract base class. Required
/// methods are overridden with `Future.value(default)` (async) or the direct
/// default (sync). The `name` getter is emitted when a Plugin super-trait is
/// configured.
#[allow(unused_imports)]
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &Fixture,
    enums: &[crate::core::ir::EnumDef],
) -> TestBackendEmission {
    use crate::backends::dart::type_map::DartMapper;
    use crate::codegen::defaults::language_defaults;
    use crate::codegen::type_mapper::TypeMapper as _;
    use heck::{ToLowerCamelCase, ToUpperCamelCase};
    use std::fmt::Write as _;

    use super::values::escape_dart;

    let pascal_id = fixture.id.to_upper_camel_case();
    let class_name = format!("TestStub{pascal_id}");
    let trait_class = &trait_bridge.trait_name;

    // Prefer the fixture's input "name" field (e.g. "test-extractor") over the
    // fixture id, which is a snake_case internal identifier not a backend name.
    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id)
        .to_string();

    let defaults = language_defaults("dart");
    let mapper = DartMapper;

    // Collect all types used in method signatures to determine needed imports.
    let mut needs_uint8list = false;
    for method in methods {
        for param in &method.params {
            if param.ty == TypeRef::Bytes {
                needs_uint8list = true;
            }
        }
        if method.return_type == TypeRef::Bytes {
            needs_uint8list = true;
        }
    }

    let mut setup = String::new();
    let _ = writeln!(setup, "class {class_name} extends {trait_class} {{");

    // Plugin super-trait `name` getter — no @override on local class members.
    if trait_bridge.super_trait.is_some() {
        let escaped_name = escape_dart(&plugin_name);
        let _ = writeln!(setup, "  String get name => '{escaped_name}';");
    }

    // Emit all methods (both required and optional with defaults) so the factory wrapper
    // can invoke them all. Optional methods return default values.
    for method in methods {
        let method_name = method.name.to_lower_camel_case();

        // Build typed parameter list using DartMapper for concrete type names.
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let param_type = map_dart_type_with_fallback(&mapper, &p.ty);
                format!("{} {}", param_type, p.name.to_lower_camel_case())
            })
            .collect();
        let params_str = params.join(", ");

        let return_type = map_dart_type_with_fallback(&mapper, &method.return_type);
        let default_val = emit_dart_default_for_type(defaults.as_ref(), &method.return_type, enums);

        // Always emit `Future<T> ... async => default` to match the abstract trait, which
        // wraps every method in `Future<T>` because FRB bridges every Dart-side callback as
        // `DartFnFuture<T>`. Mirroring this on sync methods avoids "return type 'int' does
        // not match overridden 'Future<int>'" errors when subclassing the abstract trait.
        let _ = method.is_async;
        let _ = writeln!(
            setup,
            "  Future<{return_type}> {method_name}({params_str}) async => {default_val};"
        );
    }

    let _ = writeln!(setup, "}}");

    // Dart trait bridges require wrapping the implementation in a `create<Trait>DartImpl()` call.
    // The wrapper requires pluginName, pluginVersion, and callbacks for all trait methods.
    let create_fn = format!("create{}DartImpl", trait_bridge.trait_name);
    let plugin_name = fixture
        .input
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&fixture.id);

    let instance_name = format!("_{class_name}_instance");
    let factory_fn = format!("_create{class_name}Wrapper");

    // Emit the instance creation and factory initialization.
    // For module-level scope: declare a factory function that does the async work.
    // The actual test will call this factory function when needed.
    let _ = writeln!(setup, "final {instance_name} = {class_name}();");
    let trait_name = &trait_bridge.trait_name;
    let _ = writeln!(
        setup,
        "Future<{trait_name}DartImpl> {factory_fn}() async => await {create_fn}("
    );
    let escaped_plugin_name = escape_dart(plugin_name);
    let _ = writeln!(setup, "  pluginName: '{escaped_plugin_name}',");
    let _ = writeln!(setup, "  pluginVersion: '0.0.1',");

    // Emit method callbacks for all methods (required and optional). The factory wrapper
    // requires callbacks for all trait methods to satisfy the Rust bridge signature.
    // Skip binding_excluded methods — these are not part of the FRB-generated factory.
    // Closure parameters are emitted with explicit Dart types so they satisfy the
    // typed `BoxFn…` parameter of the FRB-generated factory; bare `(a, b) => …`
    // closures infer `dynamic` and fail Dart strong-mode type checks.
    let emitted_methods: Vec<_> = methods.iter().filter(|m| !m.binding_excluded).collect();
    for (i, method) in emitted_methods.iter().enumerate() {
        let method_name = method.name.to_lower_camel_case();
        let typed_params: Vec<String> = method
            .params
            .iter()
            .map(|p| {
                let ty = map_dart_type_with_fallback(&mapper, &p.ty);
                format!("{} {}", ty, p.name.to_lower_camel_case())
            })
            .collect();
        let typed_params_str = typed_params.join(", ");
        let param_names: Vec<String> = method.params.iter().map(|p| p.name.to_lower_camel_case()).collect();
        let arg_pass = param_names.join(", ");
        let binding = if param_names.is_empty() {
            format!("{method_name}: () => {instance_name}.{method_name}()")
        } else {
            format!("{method_name}: ({typed_params_str}) => {instance_name}.{method_name}({arg_pass})")
        };
        let comma = if i < emitted_methods.len() - 1 { "," } else { "" };
        let _ = writeln!(setup, "  {binding}{comma}");
    }
    let _ = writeln!(setup, ");");

    let mut type_imports = Vec::new();
    if needs_uint8list {
        type_imports.push("dart:typed_data".to_string());
    }

    // The arg_expr is a call to the factory function, which returns a Future.
    let factory_fn = format!("_create{class_name}Wrapper");
    let arg_expr = format!("await {factory_fn}()");

    TestBackendEmission {
        setup_block: setup,
        arg_expr,
        type_imports,
        teardown_block: String::new(),
    }
}

/// Map a Dart type, with an explicit bridge carrier for internal-only types.
/// Internal named types use a generated `<TypeName>Bridge` carrier so tests preserve
/// the Rust trait contract instead of substituting a public DTO.
pub(super) fn map_dart_type_with_fallback(
    mapper: &crate::backends::dart::type_map::DartMapper,
    ty: &crate::core::ir::TypeRef,
) -> String {
    use crate::codegen::type_mapper::TypeMapper as _;
    if let crate::core::ir::TypeRef::Named(name) = ty {
        if name.contains("Internal") {
            return format!("{name}Bridge");
        }
    }
    mapper.map_type(ty).to_string()
}

/// Emit a Dart default value for a type, with special handling for enums and internal types.
pub(super) fn emit_dart_default_for_type(
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    ty: &crate::core::ir::TypeRef,
    enums: &[crate::core::ir::EnumDef],
) -> String {
    // Map internal-only types to the opaque bridge carrier for default generation.
    let effective_ty = match ty {
        TypeRef::Named(name) if name.contains("Internal") => TypeRef::Named(format!("{name}Bridge")),
        _ => ty.clone(),
    };

    if let TypeRef::Named(name) = &effective_ty {
        // Check if this Named type is an enum in the IR; if so, return the first variant
        if let Some(enum_def) = enums.iter().find(|e| &e.name == name) {
            if let Some(first_variant) = enum_def.variants.first() {
                let variant_name = first_variant.name.to_lowercase();
                return format!("{name}.{variant_name}");
            }
        }
        // For non-enum Named types, throw UnimplementedError (struct/complex type stubs
        // are registration-only and methods are never invoked).
        return "throw UnimplementedError()".to_string();
    }
    // Integer primitives default to `1` (not `0`). Floats stay at `0.0`;
    // booleans stay at `false`. Mirrors the Python e2e generator policy.
    if let TypeRef::Primitive(p) = &effective_ty {
        use crate::core::ir::PrimitiveType;
        match p {
            PrimitiveType::Bool | PrimitiveType::F32 | PrimitiveType::F64 => {}
            _ => return "1".to_string(),
        }
    }
    defaults.emit_default(&effective_ty).to_string()
}
