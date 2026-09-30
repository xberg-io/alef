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

    // Collect all types used in method signatures to determine needed imports. Asked of
    // the mapper that spells them rather than matched against `TypeRef::Bytes`: `Vec<f64>`
    // and `Vec<i64>` render as `Float64List`/`Int64List`, which need the same import while
    // being nothing like `Bytes` in the IR. ~keep
    let needs_typed_data = methods.iter().any(|method| {
        let signature_types = method
            .params
            .iter()
            .map(|param| &param.ty)
            .chain(std::iter::once(&method.return_type));
        signature_types
            .map(|ty| map_dart_type_with_fallback(&mapper, ty))
            .any(|rendered| crate::backends::dart::type_map::needs_dart_typed_data(&rendered))
    });

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
    if needs_typed_data {
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

/// Collect module-level test stub class definitions for Dart.
///
/// Dart does not allow class definitions inside functions, so callers that emit a
/// `test_backend` argument (a trait-bridge stub, e.g. for `register_validator`) must
/// hoist the stub class — and its `_create<Stub>Wrapper()` factory function, which the
/// call site awaits — to module scope, above `main()`/`void main()`. Both the full e2e
/// test-file emitter (`test_file.rs`) and the single-fixture doc-snippet emitter
/// (`snippet.rs`) render a call expression that references the factory function, so both
/// must call this to actually define it; skipping it here left doc snippets referencing
/// an undefined `_createTestStub...Wrapper` symbol (never declared) while the full e2e
/// suite already declared it via its own pass.
pub(super) fn collect_test_stub_classes(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &crate::e2e::config::E2eConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) {
    use std::fmt::Write as _;

    // HTTP fixtures do not use test_backend.
    if fixture.is_http_test() {
        return;
    }

    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    for arg_def in fixture.resolved_args(call_config) {
        if arg_def.arg_type != "test_backend" {
            continue;
        }
        if let Some(trait_name) = &arg_def.trait_name
            && let Some(trait_bridge) = config
                .trait_bridges_for(crate::core::config::Language::Dart)
                .find(|tb| tb.trait_name == *trait_name)
        {
            let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                .iter()
                .find(|t| t.name == *trait_name)
                .map(|t| t.methods.iter().collect())
                .unwrap_or_default();
            let emission = emit_test_backend(trait_bridge, &methods, fixture, enums);
            // Emit only the class definition at module-level.
            let _ = writeln!(out, "{}", emission.setup_block);
            let _ = writeln!(out);
        }
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
    if let crate::core::ir::TypeRef::Named(name) = ty
        && name.contains("Internal")
    {
        return format!("{name}Bridge");
    }
    mapper.map_type(ty).to_string()
}

/// Emit a Dart default value for a type, with special handling for enums and internal types.
pub(super) fn emit_dart_default_for_type(
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    ty: &crate::core::ir::TypeRef,
    enums: &[crate::core::ir::EnumDef],
) -> String {
    // Special case: Named(Float64List) and similar typed-list types need
    // explicit Dart construction. Return Float64List.fromList([]) for the default.
    if let TypeRef::Named(name) = ty
        && name == "Float64List"
    {
        return "Float64List.fromList([])".to_string();
    }
    // Special case: Vec<Float64List> should return an empty list of Float64Lists.
    if let TypeRef::Vec(inner) = ty {
        if let TypeRef::Named(name) = inner.as_ref()
            && name == "Float64List"
        {
            return "[]".to_string(); // Dart infers as List<Float64List>
        }
        // Vec<f32>/Vec<f64> maps to Float64List in Dart — needs typed constructor for default
        if let TypeRef::Primitive(crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64) =
            inner.as_ref()
        {
            return "Float64List.fromList([])".to_string();
        }
    }
    // When return type is Optional<Float64List>, unwrap and return Float64List.fromList([])
    if let TypeRef::Optional(inner) = ty
        && let TypeRef::Named(name) = inner.as_ref()
        && name == "Float64List"
    {
        return "Float64List.fromList([])".to_string();
    }

    // Map internal-only types to the opaque bridge carrier for default generation.
    let effective_ty = match ty {
        TypeRef::Named(name) if name.contains("Internal") => TypeRef::Named(format!("{name}Bridge")),
        _ => ty.clone(),
    };

    if let TypeRef::Named(name) = &effective_ty {
        // Check if this Named type is an enum in the IR; if so, return a real default value
        // for it rather than the struct/complex-type `UnimplementedError()` fallback below.
        if let Some(enum_def) = enums.iter().find(|e| &e.name == name) {
            if let Some(default_val) = dart_enum_default(enum_def) {
                return default_val;
            }
            // Every variant carries fields (e.g. a Rust enum with no unit variant at
            // all): there is no field-level default data to synthesize a compilable
            // factory-constructor call from. Warn instead of guessing a value the
            // Dart analyzer will reject. ~keep
            tracing::warn!(
                language = "dart",
                r#type = %name,
                "trait-bridge stub: enum has no fieldless variant to use as a default"
            );
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

/// A compilable default-value expression for `enum_def`, or `None` when every variant
/// carries fields (no fieldless value exists to synthesize one from).
///
/// Mirrors the two shapes the Dart binding backend itself emits for a Rust enum
/// (`backends::dart::gen_bindings::types::emit_enum`, and the FRB/Freezed codegen it
/// orchestrates for the real published binding):
///
/// - An all-unit enum lowers to a genuine Dart `enum`, whose members are referenced
///   directly (`SampleEnum.someVariant`) — no parentheses, since it is not a constructor
///   call.
/// - A mixed enum (any variant carries fields) lowers to a Freezed sealed class where
///   EVERY variant, unit or not, is a `factory` constructor
///   (`const factory SampleEnum.someUnitVariant() = ...;`) and so always needs the call
///   parentheses, even for a zero-argument unit variant. Using the bare member form here
///   previously produced a constructor tear-off (`SampleEnum Function({...})`) where a
///   constructed value was required.
///
/// Prefers the enum's own `#[default]` variant when it carries no fields, falling back to
/// the first fieldless variant otherwise — the value the real Rust bridge itself falls back
/// to on a failed/uninitialised callback. ~keep
fn dart_enum_default(enum_def: &crate::core::ir::EnumDef) -> Option<String> {
    use heck::ToLowerCamelCase;

    let all_unit = enum_def.variants.iter().all(|v| v.fields.is_empty());
    let variant = enum_def
        .variants
        .iter()
        .find(|v| v.is_default && v.fields.is_empty())
        .or_else(|| enum_def.variants.iter().find(|v| v.fields.is_empty()))?;
    let variant_name = crate::backends::dart::ident::dart_safe_ident(&variant.name.to_lower_camel_case());
    Some(if all_unit {
        format!("{}.{variant_name}", enum_def.name)
    } else {
        format!("{}.{variant_name}()", enum_def.name)
    })
}
