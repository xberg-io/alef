//! Go e2e test-backend stub emission.

use crate::codegen::naming::go_param_name;
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::fmt::Write as FmtWrite;

/// Emit a Go test backend stub.
///
/// Go is interface-based: define a package-level struct type + methods that satisfy
/// the trait's Go interface. The Plugin super-trait `Name()` method returns the fixture id.
///
/// Check if a type maps to json.RawMessage (only TypeRef::Json).
/// Named types now use their proper Go types, so we only need json import for
/// the Json type itself.
fn uses_json_type(ty: &crate::core::ir::TypeRef) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Json => true,
        TypeRef::Optional(inner) => uses_json_type(inner),
        TypeRef::Vec(inner) => uses_json_type(inner),
        TypeRef::Map(k, v) => uses_json_type(k) || uses_json_type(v),
        _ => false,
    }
}

/// Because Go does not allow method declarations inside function bodies, the `setup_block`
/// contains package-level type and method declarations. The `arg_expr` is the struct
/// literal `testStub_<id>{}` that callers pass to `Register<Trait>`.
///
/// Call `emit_test_backend_with_context` from e2e test-file renderers that have the
/// `excluded_types` set (binding-excluded types → `json.RawMessage`) and `import_alias`
/// (qualifies named types for an external test package).
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::super::TestBackendEmission {
    emit_test_backend_with_context(
        trait_bridge,
        methods,
        fixture,
        &std::collections::HashSet::new(),
        "",
        &std::collections::HashSet::new(),
        &[],
    )
}

/// Like [`emit_test_backend`] but with type-qualification context.
///
/// `excluded_types` — names of binding-excluded types (for example, `InternalRecord`) that should
/// be substituted with `json.RawMessage` in method signatures.  These types exist in the Rust
/// IR but are never emitted as Go structs; the trait-bridge interface serialises them to JSON.
///
/// `import_alias` — the import alias used for the binding package in the generated test file
/// (e.g. `"myproject"`).  When non-empty, `Named` types are qualified as `{alias}.{GoName}`
/// so the stub compiles from `package e2e_test` which imports the binding under that alias.
///
/// `enum_names` — set of type names that are enums in the IR (used to determine zero-values
/// for stub returns; enums map to string types in Go, so their zero-value is `""` not `nil`).
///
/// `enums` — full enumeration definitions, used to determine the first variant name for
/// default enum values in stub methods (e.g., `OcrBackendTypeTesseract`).
pub fn emit_test_backend_with_context(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    excluded_types: &std::collections::HashSet<&str>,
    import_alias: &str,
    enum_names: &std::collections::HashSet<&str>,
    enums: &[crate::core::ir::EnumDef],
) -> super::super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::sanitize_ident;

    let defaults = language_defaults("go");
    let safe_id = sanitize_ident(&fixture.id);
    let struct_name = format!("testStub_{safe_id}");

    let mut setup = String::new();

    // Package-level struct declaration.
    let _ = writeln!(setup, "type {struct_name} struct{{}}");
    setup.push('\n');

    // Super-trait methods: filter by trait_source matching the configured super_trait.
    // Driven from IR — no method names are hardcoded. The `name` method returns the
    // fixture id; all other super-trait methods use the standard per-method logic.
    if let Some(super_trait) = trait_bridge.super_trait.as_deref() {
        let super_methods: Vec<_> = methods
            .iter()
            .filter(|m| m.trait_source.as_deref() == Some(super_trait))
            .collect();
        for method in &super_methods {
            let go_method = method_to_camel(&method.name);
            if method.name == "name" {
                let _ = writeln!(
                    setup,
                    "func ({struct_name}) {go_method}() string {{ return \"{safe_id}\" }}"
                );
            } else {
                emit_go_stub_method_body(
                    &mut setup,
                    &struct_name,
                    &go_method,
                    method,
                    &*defaults,
                    excluded_types,
                    import_alias,
                    enum_names,
                    fixture,
                    enums,
                );
            }
        }
        if !super_methods.is_empty() {
            setup.push('\n');
        }
    }

    // Emit method stubs for all required methods.
    // Go interfaces require ALL abstract methods to be implemented, even if they have
    // default implementations in the Rust trait.
    // Skip: (1) super-trait methods already emitted above, (2) methods using excluded types
    // (which are not exported in the binding), and (3) name() when hardcoded by super_trait.
    for method in methods.iter() {
        // Skip super-trait methods already emitted above.
        if trait_bridge
            .super_trait
            .as_deref()
            .is_some_and(|st| method.trait_source.as_deref() == Some(st))
        {
            continue;
        }
        // Skip methods whose return type or parameters are excluded types
        // in ways that exclude them from the binding interface.
        // For return types: skip if directly excluded OR Optional<excluded>.
        // Don't skip Result<excluded> because binding generation converts those.
        // Skip methods whose return type is excluded in a way that excludes them
        // from the binding interface (directly excluded or Optional<excluded>).
        // Don't skip Result<excluded> because binding generation converts those.
        // Parameters with excluded types are OK - binding generation converts those.
        if should_skip_method_with_type(&method.return_type, excluded_types, method.error_type.is_some()) {
            continue;
        }
        let go_method = method_to_camel(&method.name);
        // A synthesized `Plugin` method (see `resolve_test_backend_emission`) carries no
        // `trait_source`, so it reaches this loop rather than the super-trait one above; give
        // its `Name()` the same fixture-id body the super-trait branch emits instead of the
        // generic empty-string default, matching the Java stub generator's equivalent
        // fallback (`java::stubs::emit_test_backend_with_context`). ~keep
        if method.name == "name" {
            let _ = writeln!(
                setup,
                "func ({struct_name}) {go_method}() string {{ return \"{safe_id}\" }}"
            );
            continue;
        }
        emit_go_stub_method_body(
            &mut setup,
            &struct_name,
            &go_method,
            method,
            &*defaults,
            excluded_types,
            import_alias,
            enum_names,
            fixture,
            enums,
        );
    }

    // Determine if encoding/json is needed by checking if any method uses json.RawMessage.
    // This includes both TypeRef::Json variants and excluded Named types (substituted to json.RawMessage).
    let uses_json_with_context = |ty: &crate::core::ir::TypeRef| -> bool {
        uses_json_type(ty) || {
            use crate::core::ir::TypeRef;
            matches!(ty, TypeRef::Named(n) if excluded_types.contains(n.as_str()))
        }
    };
    let needs_json = methods
        .iter()
        .any(|m| uses_json_with_context(&m.return_type) || m.params.iter().any(|p| uses_json_with_context(&p.ty)));

    let mut type_imports = Vec::new();
    if needs_json {
        type_imports.push("encoding/json".to_string());
    }

    super::super::TestBackendEmission {
        setup_block: setup,
        arg_expr: format!("{struct_name}{{}}"),
        type_imports,
        teardown_block: String::new(),
    }
}

/// Returns the Go zero-value expression for a stub method return statement.
///
/// Uses go_zero_value from the type_map to ensure consistency with actual
/// Go binding signatures. Named types check enum_names to determine if they're
/// enums (zero-value is first variant) or structs (zero-value `nil`). Primitives produce
/// their standard zero values (0, false, ""), and Vec produces a nil slice.
///
/// Use `go_stub_default_with_context` with the same excluded/import-alias substitution as
/// `stub_go_type_with_context` so the emitted zero-value matches the rendered return
/// type. Excluded non-enum types become `json.RawMessage(nil)`; enums (excluded or not) name
/// their first variant via `go_enum_variant_default_expression`, which is a package-level
/// constant for a string-backed enum but a composite literal for a struct- or interface-backed
/// one; struct types qualified via `import_alias` use `alias.Type{}` (Go's struct zero-value);
/// and primitives/maps/slices/optionals fall back to `go_zero_value`.
fn go_stub_default_with_context(
    ty: &crate::core::ir::TypeRef,
    enum_names: &std::collections::HashSet<&str>,
    excluded_types: &std::collections::HashSet<&str>,
    import_alias: &str,
    enums: &[crate::core::ir::EnumDef],
) -> String {
    use crate::backends::go::type_map::go_zero_value;
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::Named(name) if excluded_types.contains(name.as_str()) && enum_names.contains(name.as_str()) => {
            match enums.iter().find(|e| e.name == *name) {
                Some(enum_def) if !enum_def.variants.is_empty() => {
                    go_enum_variant_default_expression(name, enum_def, import_alias)
                }
                // Enum with no variants (shouldn't happen) or not found: fall back to nil.
                _ => "nil".to_string(),
            }
        }
        TypeRef::Named(name) if excluded_types.contains(name.as_str()) => "nil".to_string(),
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => match enums.iter().find(|e| e.name == *name) {
            Some(enum_def) if !enum_def.variants.is_empty() => {
                go_enum_variant_default_expression(name, enum_def, import_alias)
            }
            // Enum with no variants (shouldn't happen) or not found: fall back to an empty string.
            _ => "\"\"".to_string(),
        },
        TypeRef::Named(name) if !import_alias.is_empty() => {
            let go_name = crate::codegen::naming::go_type_name(name);
            format!("{import_alias}.{go_name}{{}}")
        }
        TypeRef::Named(name) => {
            let go_name = crate::codegen::naming::go_type_name(name);
            format!("{go_name}{{}}")
        }
        _ => go_zero_value(ty),
    }
}

/// Build a Go expression naming `enum_def`'s first variant, for use as a stub method's default
/// return value.
///
/// A `UnitString`/`NewtypeTupleString` representation names each variant as a package-level
/// constant (`alias.EnumTypeFirstVariant`), so the bare identifier is itself the expression. Every
/// other representation (`DataInterface`, and the three struct-shaped tagged-union forms) has no
/// such constant: `alias.EnumTypeFirstVariant` there names either nothing or, for `DataInterface`,
/// the concrete variant *type* — which the Go compiler rejects as "not an expression" exactly like
/// the trait-bridge fixture surfaced. Those representations must be *constructed*: a
/// `DataInterface` needs the concrete variant struct's composite literal
/// (`go_data_enum_variant_struct` — the interface itself has none), and the struct-shaped forms
/// compile fine as the enum's own zero-value composite literal regardless of which variant is
/// "first". ~keep
fn go_enum_variant_default_expression(name: &str, enum_def: &crate::core::ir::EnumDef, import_alias: &str) -> String {
    use crate::backends::go::{GoEnumRepresentation, go_data_enum_variant_struct, go_enum_representation};

    let qualify = |type_name: String| {
        if import_alias.is_empty() {
            type_name
        } else {
            format!("{import_alias}.{type_name}")
        }
    };
    // Callers already checked `!enum_def.variants.is_empty()`.
    let first_variant = &enum_def.variants[0];
    let representation = go_enum_representation(enum_def);
    if representation.has_named_constants() {
        let go_name = crate::codegen::naming::go_type_name(name);
        let variant_name = crate::codegen::naming::go_type_name(&first_variant.name);
        return qualify(format!("{go_name}{variant_name}"));
    }
    match representation {
        GoEnumRepresentation::DataInterface => {
            let struct_name = go_data_enum_variant_struct(enum_def, first_variant);
            qualify(format!("{struct_name}{{}}"))
        }
        GoEnumRepresentation::AdjacentTaggedStruct
        | GoEnumRepresentation::TupleTaggedStruct
        | GoEnumRepresentation::ExternallyTaggedStruct
        | GoEnumRepresentation::RawMessage => {
            let go_name = crate::codegen::naming::go_type_name(name);
            qualify(format!("{go_name}{{}}"))
        }
        GoEnumRepresentation::UnitString | GoEnumRepresentation::NewtypeTupleString => unreachable!(
            "has_named_constants() already returned true for UnitString/NewtypeTupleString and returned above"
        ),
    }
}

/// Extract a default value from fixture.input.backend for a stub method.
///
/// Given a method name and fixture, attempts to find the corresponding input
/// value in fixture.input.backend. Returns JSON-marshalled values for Named types,
/// and raw values for primitives. For numeric defaults, emits 1 instead of 0
/// (downstream rejects 0 for counts like dimensions).
fn extract_fixture_default(method_name: &str, fixture: &crate::e2e::fixture::Fixture) -> Option<String> {
    let backend_input = fixture.input.get("backend").and_then(|v| v.as_object())?;

    // Try snake_case first, then the lower_camel_case variant.
    let snake_name = method_name.to_snake_case();
    let val = backend_input
        .get(&snake_name)
        .or_else(|| backend_input.get(method_name))?;

    Some(match val {
        serde_json::Value::Number(n) => {
            // For numeric defaults, emit 1 instead of 0 if it's 0
            // (downstream validation rejects 0 for counts like dimensions).
            if let Some(i) = n.as_i64() {
                if i == 0 { "1".to_string() } else { i.to_string() }
            } else if let Some(u) = n.as_u64() {
                if u == 0 { "1".to_string() } else { u.to_string() }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::String(s) => format!("\"{}\"", s),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => return None, // Complex types not supported in fixture defaults
    })
}

/// Check if a type (or its top-level structure) is an excluded type in a way that would
/// exclude the entire method from the binding interface.
///
/// A method should be skipped ONLY if its return type is structurally unmarshalable or
/// not exported at all — specifically, Optional<ExcludedType>. Named excluded types
/// (including enums and other types) are always exported in the Go binding, so methods
/// returning them directly should be emitted. Methods returning Optional<ExcludedType>
/// are skipped because they would require returning nil for types that don't export.
fn should_skip_method_with_type(
    ty: &crate::core::ir::TypeRef,
    excluded_types: &std::collections::HashSet<&str>,
    _is_result_return: bool,
) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        // Optional<ExcludedType> is always skipped (would need nil, but type not exported).
        TypeRef::Optional(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(name) if excluded_types.contains(name.as_str()))
        }
        // Directly named excluded types are NOT skipped anymore. The Go binding emits them
        // (as json.RawMessage for trait-bridge purposes), so the stub must emit the method.
        // Only Optional<ExcludedType> is structurally problematic.
        _ => false,
    }
}

/// Maps a type reference to its Go representation in stub method signatures, with context.
///
/// When `excluded_types` is non-empty, any `TypeRef::Named` whose name appears in the set
/// is substituted with `json.RawMessage` (matching the actual trait-bridge interface which
/// serialises excluded/internal types to JSON), UNLESS the type is an enum (appears in `enum_names`).
/// Enums are exported as typed Go enums in the binding, so stubs must use the typed enum
/// instead of `json.RawMessage` to match the interface signature.
/// When `import_alias` is non-empty, remaining `TypeRef::Named` types are qualified as
/// `{import_alias}.{GoName}` so the stub compiles from an external test package
/// (e.g. `package e2e_test`) that imports the binding package under an alias.
pub(super) fn stub_go_type_with_context(
    ty: &crate::core::ir::TypeRef,
    excluded_types: &std::collections::HashSet<&str>,
    import_alias: &str,
    enum_names: &std::collections::HashSet<&str>,
) -> String {
    use crate::backends::go::type_map::go_type;
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
            // Check if this is an enum: if so, emit the typed enum, not json.RawMessage.
            // Enums are exported as typed Go enums (string-based) in the binding, so the
            // stub interface signature must use the typed enum to match.
            if !enum_names.is_empty() && enum_names.contains(name.as_str()) {
                let go_name = crate::codegen::naming::go_type_name(name);
                if !import_alias.is_empty() {
                    format!("{import_alias}.{go_name}")
                } else {
                    go_name
                }
            } else {
                "json.RawMessage".to_string()
            }
        }
        TypeRef::Named(name) if !import_alias.is_empty() => {
            let go_name = crate::codegen::naming::go_type_name(name);
            format!("{import_alias}.{go_name}")
        }
        TypeRef::Optional(inner) => {
            let inner_str = stub_go_type_with_context(inner, excluded_types, import_alias, enum_names);
            // Excluded types become json.RawMessage which is a slice — don't add pointer
            if inner_str == "json.RawMessage" {
                inner_str
            } else {
                format!("*{inner_str}")
            }
        }
        TypeRef::Vec(inner) => {
            let inner_str = stub_go_type_with_context(inner, excluded_types, import_alias, enum_names);
            format!("[]{inner_str}")
        }
        TypeRef::Map(k, v) => {
            let k_str = stub_go_type_with_context(k, excluded_types, import_alias, enum_names);
            let v_str = stub_go_type_with_context(v, excluded_types, import_alias, enum_names);
            format!("map[{k_str}]{v_str}")
        }
        _ => go_type(ty).into_owned(),
    }
}

/// Convert snake_case method names to Go camelCase.
pub(super) fn method_to_camel(snake: &str) -> String {
    snake.to_upper_camel_case()
}

/// Emit a single Go stub method receiver function into `out`.
///
/// Used by both the main method loop and the super-trait method section of
/// `emit_test_backend` so both paths share the same formatting logic.
/// `go_method` is the already-PascalCased method name (caller's responsibility).
///
/// `excluded_types` — names of binding-excluded types substituted with `json.RawMessage`.
/// `import_alias` — binding package import alias; qualifies Named types for external packages.
/// `enum_names` — set of type names that are enums (map to string types, zero-value is first variant).
/// `enums` — full enum definitions, used to determine first variant names for default values.
#[allow(clippy::too_many_arguments)]
fn emit_go_stub_method_body(
    out: &mut String,
    struct_name: &str,
    go_method: &str,
    method: &crate::core::ir::MethodDef,
    defaults: &dyn crate::codegen::defaults::LanguageDefaults,
    excluded_types: &std::collections::HashSet<&str>,
    import_alias: &str,
    enum_names: &std::collections::HashSet<&str>,
    fixture: &crate::e2e::fixture::Fixture,
    enums: &[crate::core::ir::EnumDef],
) {
    use crate::core::ir::TypeRef;

    // Build parameter list: `name GoType` pairs, substituting opaque Named types
    // with json.RawMessage (matches the generated Go interface signatures).
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let go_param = go_param_name(&p.name);
            let type_str = stub_go_type_with_context(&p.ty, excluded_types, import_alias, enum_names);
            format!("{go_param} {type_str}")
        })
        .collect();
    let param_str = params.join(", ");

    let ret_ty = stub_go_type_with_context(&method.return_type, excluded_types, import_alias, enum_names);

    // Build return type.
    let return_type_str = if method.error_type.is_some() {
        match &method.return_type {
            TypeRef::Unit => "error".to_string(),
            _ => format!("({ret_ty}, error)"),
        }
    } else {
        ret_ty.clone()
    };

    // Build return expression.
    let return_expr = if method.error_type.is_some() {
        match &method.return_type {
            TypeRef::Unit => "return nil".to_string(),
            _ => {
                let default_val = extract_fixture_default(&method.name, fixture).unwrap_or_else(|| {
                    go_stub_default_with_context(&method.return_type, enum_names, excluded_types, import_alias, enums)
                });
                format!("return {default_val}, nil")
            }
        }
    } else if matches!(method.return_type, TypeRef::Unit) {
        String::new()
    } else {
        let default_val = extract_fixture_default(&method.name, fixture).unwrap_or_else(|| {
            go_stub_default_with_context(&method.return_type, enum_names, excluded_types, import_alias, enums)
        });
        format!("return {default_val}")
    };

    // Drop the `defaults` parameter — the stub uses go_stub_default directly.
    let _ = defaults; // suppress unused-variable warning

    let _ = writeln!(
        out,
        "func ({struct_name}) {go_method}({param_str}) {return_type_str} {{ {return_expr} }}"
    );
}

/// Names of the four `Plugin` super-trait methods every Go trait-bridge interface
/// requires, paired with whether the real interface declares them fallible (`error`).
/// Go interfaces have no default-method mechanism (unlike Java's `default` methods), so
/// a stub must implement all four regardless of which the Rust trait leaves at their
/// default body.
///
/// Mirrors `gen_plugin_trampolines` (`backends::go::trait_bridge::dispatch`), which
/// unconditionally emits trampolines for exactly these four names — the actual
/// interface contract, not a re-derivation of it. ~keep
const SUPER_TRAIT_REQUIRED_METHODS: [(&str, bool); 4] = [
    ("name", false),
    ("version", false),
    ("initialize", true),
    ("shutdown", true),
];

/// Resolve the full stub emission for a Go `test_backend` fixture argument.
///
/// Looks up the trait's own IR methods, merges in whatever `Plugin` super-trait methods
/// the IR happens to expose under a matching `rust_path`, and synthesizes any of the
/// four fixed `Plugin` methods still missing afterward.
///
/// The `rust_path` lookup below finds nothing when `Plugin` is declared in a private
/// module and re-exported via `pub use` — its `rust_path` need not equal the configured
/// `super_trait` value — silently leaving every Go trait-bridge stub without any of the
/// four methods the real interface requires and failing to compile. Java hit and fixed
/// the identical failure (`e2e::codegen::java::args`,
/// `backends::java::gen_bindings::trait_bridge_naming::SUPER_TRAIT_REQUIRED_METHODS`);
/// this synthesizes whichever required method the lookup did not already supply, instead
/// of re-deriving the same convention a second way. ~keep
pub(super) fn resolve_test_backend_emission(
    fixture: &crate::e2e::fixture::Fixture,
    trait_name: &str,
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    config: &crate::core::config::ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    import_alias: &str,
) -> super::super::TestBackendEmission {
    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
        .iter()
        .find(|t| t.name == *trait_name)
        .map(|t| t.methods.iter().collect())
        .unwrap_or_default();

    if let Some(super_trait) = &trait_bridge.super_trait
        && let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait)
    {
        for method in &super_type.methods {
            if !methods.iter().any(|m| m.name == method.name) {
                methods.push(method);
            }
        }
    }

    let synthetic_super_trait_methods: Vec<crate::core::ir::MethodDef> = if trait_bridge.super_trait.is_some() {
        SUPER_TRAIT_REQUIRED_METHODS
            .iter()
            .copied()
            .filter(|(name, _)| !methods.iter().any(|m| m.name == *name))
            .map(|(name, fallible)| crate::core::ir::MethodDef {
                name: name.to_string(),
                // Initialize/Shutdown are `() error`; Name/Version are `() string`.
                return_type: if fallible {
                    crate::core::ir::TypeRef::Unit
                } else {
                    crate::core::ir::TypeRef::String
                },
                error_type: fallible.then(|| "Error".to_string()),
                ..Default::default()
            })
            .collect()
    } else {
        Vec::new()
    };
    methods.extend(synthetic_super_trait_methods.iter());

    let excluded_named = crate::e2e::codegen::recipe::trait_bridge_excluded_type_names(
        config,
        crate::core::config::Language::Go,
        type_defs,
        &methods,
    );
    let enum_names: std::collections::HashSet<&str> = enums.iter().map(|e| e.name.as_str()).collect();
    emit_test_backend_with_context(
        trait_bridge,
        &methods,
        fixture,
        &excluded_named,
        import_alias,
        &enum_names,
        enums,
    )
}

#[cfg(test)]
mod trait_bridge_tests;

#[cfg(test)]
#[path = "test_backend/enum_default_tests.rs"]
mod enum_default_tests;
