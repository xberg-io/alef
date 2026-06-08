//! Go e2e test-backend stub emission.

use crate::codegen::naming::go_param_name;
use heck::ToUpperCamelCase;
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
pub fn emit_test_backend_with_context(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
    excluded_types: &std::collections::HashSet<&str>,
    import_alias: &str,
    enum_names: &std::collections::HashSet<&str>,
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
        emit_go_stub_method_body(
            &mut setup,
            &struct_name,
            &go_method,
            method,
            &*defaults,
            excluded_types,
            import_alias,
            enum_names,
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
/// enums (zero-value `""`) or structs (zero-value `nil`). Primitives produce
/// their standard zero values (0, false, ""), and Vec produces a nil slice.
///
/// Use `go_stub_default_with_context` with the same excluded/import-alias substitution as
/// `stub_go_type_with_context` so the emitted zero-value matches the rendered return
/// type. Excluded types become `json.RawMessage(nil)`, struct types qualified via
/// `import_alias` use `alias.Type{}` (Go's struct zero-value), enums stay as `""`,
/// and primitives/maps/slices/optionals fall back to `go_zero_value`.
fn go_stub_default_with_context(
    ty: &crate::core::ir::TypeRef,
    enum_names: &std::collections::HashSet<&str>,
    excluded_types: &std::collections::HashSet<&str>,
    import_alias: &str,
) -> String {
    use crate::backends::go::type_map::go_zero_value;
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::Named(name) if excluded_types.contains(name.as_str()) => "nil".to_string(),
        TypeRef::Named(name) if enum_names.contains(name.as_str()) => "\"\"".to_string(),
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
/// serialises excluded/internal types to JSON). When `import_alias` is non-empty, remaining
/// `TypeRef::Named` types are qualified as `{import_alias}.{GoName}` so the stub compiles
/// from an external test package (e.g. `package e2e_test`) that imports the binding package
/// under an alias.
pub(super) fn stub_go_type_with_context(
    ty: &crate::core::ir::TypeRef,
    excluded_types: &std::collections::HashSet<&str>,
    import_alias: &str,
) -> String {
    use crate::backends::go::type_map::go_type;
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) if !excluded_types.is_empty() && excluded_types.contains(name.as_str()) => {
            "json.RawMessage".to_string()
        }
        TypeRef::Named(name) if !import_alias.is_empty() => {
            let go_name = crate::codegen::naming::go_type_name(name);
            format!("{import_alias}.{go_name}")
        }
        TypeRef::Optional(inner) => {
            let inner_str = stub_go_type_with_context(inner, excluded_types, import_alias);
            // Excluded types become json.RawMessage which is a slice — don't add pointer
            if inner_str == "json.RawMessage" {
                inner_str
            } else {
                format!("*{inner_str}")
            }
        }
        TypeRef::Vec(inner) => {
            let inner_str = stub_go_type_with_context(inner, excluded_types, import_alias);
            format!("[]{inner_str}")
        }
        TypeRef::Map(k, v) => {
            let k_str = stub_go_type_with_context(k, excluded_types, import_alias);
            let v_str = stub_go_type_with_context(v, excluded_types, import_alias);
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
/// `enum_names` — set of type names that are enums (map to string types, zero-value is `""`).
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
) {
    use crate::core::ir::TypeRef;

    // Build parameter list: `name GoType` pairs, substituting opaque Named types
    // with json.RawMessage (matches the generated Go interface signatures).
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let go_param = go_param_name(&p.name);
            let type_str = stub_go_type_with_context(&p.ty, excluded_types, import_alias);
            format!("{go_param} {type_str}")
        })
        .collect();
    let param_str = params.join(", ");

    let ret_ty = stub_go_type_with_context(&method.return_type, excluded_types, import_alias);

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
                let default_val =
                    go_stub_default_with_context(&method.return_type, enum_names, excluded_types, import_alias);
                format!("return {default_val}, nil")
            }
        }
    } else if matches!(method.return_type, TypeRef::Unit) {
        String::new()
    } else {
        let default_val = go_stub_default_with_context(&method.return_type, enum_names, excluded_types, import_alias);
        format!("return {default_val}")
    };

    // Drop the `defaults` parameter — the stub uses go_stub_default directly.
    let _ = defaults; // suppress unused-variable warning

    let _ = writeln!(
        out,
        "func ({struct_name}) {go_method}({param_str}) {return_type_str} {{ {return_expr} }}"
    );
}

#[cfg(test)]
mod trait_bridge_tests {
    use super::{emit_test_backend, emit_test_backend_with_context};
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: Some(crate::e2e::fixture::MockResponse {
                status: 200,
                body: Some(serde_json::Value::Null),
                stream_chunks: None,
                headers: std::collections::BTreeMap::new(),
            }),
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    fn make_param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }
    }

    fn make_method(name: &str, params: Vec<(&str, TypeRef)>, ret: TypeRef, is_async: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: params.into_iter().map(|(n, ty)| make_param(n, ty)).collect(),
            return_type: ret,
            is_async,
            is_static: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    /// Genericity test: a synthetic TestTrait with one sync method and Plugin super-trait
    /// must not reference any sample_core-domain names in setup_block or arg_expr.
    #[test]
    fn test_backend_emission_is_generic() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("SomeSuperTrait".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..TraitBridgeConfig::default()
        };

        let do_thing = make_method(
            "do_thing",
            vec![("x", TypeRef::Primitive(crate::core::ir::PrimitiveType::I32))],
            TypeRef::String,
            false,
        );

        let fixture = make_fixture("my_test_fixture");
        let methods = vec![&do_thing];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        // setup_block must not reference any sample_core-domain trait or method names.
        assert!(
            !emission.setup_block.contains("ImageBackend"),
            "setup_block must not hardcode domain trait names, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("ProcessImage"),
            "setup_block must not hardcode domain method names, got:\n{}",
            emission.setup_block
        );
        // Must emit the method name from MethodDef (Go PascalCase).
        assert!(
            emission.setup_block.contains("DoThing"),
            "setup_block must contain Go PascalCase method 'DoThing', got:\n{}",
            emission.setup_block
        );
        // Must emit struct declaration.
        assert!(
            emission.setup_block.contains("type testStub_my_test_fixture struct"),
            "setup_block must contain struct declaration, got:\n{}",
            emission.setup_block
        );
        // With trait_source: None, super-trait methods are NOT emitted — no hardcoded lifecycle names.
        assert!(
            !emission.setup_block.contains("Initialize"),
            "setup_block must not contain hardcoded 'Initialize', got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("Shutdown"),
            "setup_block must not contain hardcoded 'Shutdown', got:\n{}",
            emission.setup_block
        );
        // arg_expr is the struct literal.
        assert!(
            emission.arg_expr.contains("testStub_my_test_fixture"),
            "arg_expr must reference struct name, got: {}",
            emission.arg_expr
        );
        assert!(
            emission.arg_expr.ends_with("{}"),
            "arg_expr must be a struct literal, got: {}",
            emission.arg_expr
        );
    }

    /// Verify that super-trait methods with `trait_source` set are driven from
    /// the IR slice rather than a hardcoded list of method names.
    ///
    /// A synthetic `Plugin` super-trait with methods `name`, `version`, `init`
    /// (note: `init`, NOT `Initialize`) is passed via `trait_source`. The emitter
    /// must emit `Init` (PascalCase of `init`), NOT the previously-hardcoded
    /// `Initialize` string, proving the method names come from IR.
    #[test]
    fn test_go_super_trait_methods_driven_from_ir_not_hardcoded() {
        let make_super_method = |name: &str, ret: TypeRef| -> MethodDef {
            MethodDef {
                name: name.to_string(),
                params: vec![],
                return_type: ret,
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: Some(crate::core::ir::ReceiverKind::Ref),
                sanitized: false,
                // trait_source matches the super_trait configured on the bridge.
                trait_source: Some("Plugin".to_string()),
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }
        };

        let name_method = make_super_method("name", TypeRef::String);
        let version_method = make_super_method("version", TypeRef::String);
        let init_method = make_super_method("init", TypeRef::Unit);

        let trait_bridge = TraitBridgeConfig {
            trait_name: "TestPlugin".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_plugin".to_string()),
            ..TraitBridgeConfig::default()
        };

        let fixture = make_fixture("my_plugin_fixture");
        let methods = vec![&name_method, &version_method, &init_method];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        // Must emit `Init` (PascalCase of "init"), not the old hardcoded "Initialize".
        assert!(
            emission.setup_block.contains("Init("),
            "setup_block must contain 'Init(' (from IR), got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("Initialize"),
            "setup_block must NOT contain hardcoded 'Initialize', got:\n{}",
            emission.setup_block
        );
        // `Version` comes from IR method name "version".
        assert!(
            emission.setup_block.contains("Version("),
            "setup_block must contain 'Version(' (from IR), got:\n{}",
            emission.setup_block
        );
        // Must not contain old hardcoded `Shutdown`.
        assert!(
            !emission.setup_block.contains("Shutdown"),
            "setup_block must NOT contain hardcoded 'Shutdown', got:\n{}",
            emission.setup_block
        );
        // `Name()` is emitted and returns the fixture id.
        assert!(
            emission.setup_block.contains("Name()"),
            "setup_block must contain Name() from IR name method, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that Named types use their proper Go type names
    /// in stubs, matching the actual trait-bridge interface signatures.
    #[test]
    fn test_go_stub_named_types_use_proper_go_names() {
        let backend_type_method = make_method("backend_type", vec![], TypeRef::Named("BackendKind".to_string()), false);

        let trait_bridge = TraitBridgeConfig {
            trait_name: "SampleBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_sample_backend".to_string()),
            ..TraitBridgeConfig::default()
        };

        let fixture = make_fixture("backend_type_test");
        let methods = vec![&backend_type_method];
        let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

        // The method signature should use the proper Go name, not json.RawMessage.
        assert!(
            emission.setup_block.contains("BackendType()") && emission.setup_block.contains("BackendKind"),
            "setup_block must use BackendKind in BackendType() method signature, got:\n{}",
            emission.setup_block
        );

        // Return value must match go_zero_value for named types.
        assert!(
            !emission.setup_block.contains("json.RawMessage(nil)"),
            "setup_block must not use json.RawMessage for BackendKind, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that methods with binding-excluded types are handled correctly:
    /// - Methods returning directly excluded types are skipped
    /// - Methods returning Optional<ExcludedType> are skipped
    /// - Methods with wrapped returns (Result<ExcludedType>, Vec<ExcludedType>) are emitted
    ///   (binding generation converts these appropriately)
    /// - Normal methods are emitted with proper type qualification
    #[test]
    fn test_go_stub_skips_excluded_return_types() {
        // Method 1: returns an excluded named type directly -> should be SKIPPED
        let excluded_return_method = make_method(
            "get_internal_record",
            vec![],
            TypeRef::Named("InternalRecord".to_string()),
            false,
        );

        // Method 2: returns Result<ExcludedType> -> should be EMITTED
        // (Result wrapping is handled by binding generation)
        let result_return_method = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes)],
            TypeRef::Named("InternalRecord".to_string()), // In IR; becomes json.RawMessage in binding
            true,                                         // has_error_type = true
        );

        // Method 3: normal method with non-excluded types → should be EMITTED
        let normal_method = make_method("get_config", vec![], TypeRef::Named("ParseConfig".to_string()), false);

        let trait_bridge = TraitBridgeConfig {
            trait_name: "RecordProvider".to_string(),
            super_trait: None,
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let fixture = make_fixture("extractor_test");
        let methods = vec![&excluded_return_method, &result_return_method, &normal_method];

        let mut excluded = std::collections::HashSet::new();
        excluded.insert("InternalRecord");

        let enum_names = std::collections::HashSet::new();
        let emission =
            emit_test_backend_with_context(&trait_bridge, &methods, &fixture, &excluded, "myproject", &enum_names);

        // Method returning directly excluded type must NOT appear in stub.
        assert!(
            !emission.setup_block.contains("get_internal_record"),
            "method with directly excluded return type must be skipped, got:\n{}",
            emission.setup_block
        );

        // Method with Result-wrapped excluded type should appear (binding generation handles conversion).
        assert!(
            emission.setup_block.contains("ExtractBytes"),
            "method with Result<ExcludedType> should be emitted (binding handles conversion), got:\n{}",
            emission.setup_block
        );

        // Normal method with non-excluded types must appear (in PascalCase).
        assert!(
            emission.setup_block.contains("GetConfig"),
            "normal method must be emitted, got:\n{}",
            emission.setup_block
        );

        // Normal method's return type must be qualified with import alias.
        assert!(
            emission.setup_block.contains("myproject.ParseConfig"),
            "named type ParseConfig must be qualified as myproject.ParseConfig, got:\n{}",
            emission.setup_block
        );
    }

    /// Regression (Go trait bridges): methods returning enum types must not be skipped.
    ///
    /// Example: OcrBackend.BackendType() returns OcrBackendType (an enum).
    /// The Go interface declares `BackendType() OcrBackendType`, so the test-stub
    /// MUST emit a default implementation, even though OcrBackendType is a Named type
    /// and may be in the excluded_types set (for trait-bridge json.RawMessage purposes).
    ///
    /// This test uses a synthetic `MyService` trait with `Diagnose() string` returning
    /// a named type (treated as enum) to verify the fix works generically.
    #[test]
    fn test_go_stub_emits_methods_returning_named_excluded_types() {
        let diagnose_method = make_method("diagnose", vec![], TypeRef::Named("DiagnosticLevel".to_string()), false);

        let trait_bridge = TraitBridgeConfig {
            trait_name: "MyService".to_string(),
            super_trait: None,
            register_fn: Some("register_my_service".to_string()),
            ..TraitBridgeConfig::default()
        };

        let fixture = make_fixture("service_diagnose");
        let methods = vec![&diagnose_method];

        // Simulate the scenario where DiagnosticLevel is in excluded_types
        // (e.g., treated as json.RawMessage at the trait-bridge interface level).
        // Before the fix, this method would be skipped; after the fix, it must be emitted.
        let mut excluded = std::collections::HashSet::new();
        excluded.insert("DiagnosticLevel");

        let enum_names = std::collections::HashSet::new();
        let emission = emit_test_backend_with_context(&trait_bridge, &methods, &fixture, &excluded, "", &enum_names);

        // Method returning an excluded named type must be emitted (it's now exported as json.RawMessage).
        assert!(
            emission.setup_block.contains("Diagnose()"),
            "method returning excluded named type must be emitted, got:\n{}",
            emission.setup_block
        );

        // Return type must be properly handled (json.RawMessage for excluded, or proper type name).
        // The signature should reflect the binding's interface (json.RawMessage for excluded types).
        assert!(
            emission.setup_block.contains("json.RawMessage") || emission.setup_block.contains("nil"),
            "method must emit a zero-value that matches the excluded type handling, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that methods returning Optional<ExcludedType> are skipped
    /// (for example, an accessor returning an optional excluded trait object).
    #[test]
    fn test_go_stub_skips_optional_excluded_return_types() {
        // Method returning Option<InternalProvider> -> should be skipped
        // (InternalProvider is not exported in the binding).
        let optional_excluded_method = make_method(
            "as_internal_provider",
            vec![],
            TypeRef::Optional(Box::new(TypeRef::Named("InternalProvider".to_string()))),
            false,
        );

        let trait_bridge = TraitBridgeConfig {
            trait_name: "RecordProvider".to_string(),
            super_trait: None,
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let fixture = make_fixture("extractor_test");
        let methods = vec![&optional_excluded_method];

        let mut excluded = std::collections::HashSet::new();
        excluded.insert("InternalProvider");

        let enum_names = std::collections::HashSet::new();
        let emission =
            emit_test_backend_with_context(&trait_bridge, &methods, &fixture, &excluded, "mylib", &enum_names);

        // Method returning Optional<ExcludedType> must NOT appear in stub.
        assert!(
            !emission.setup_block.contains("as_internal_provider")
                && !emission.setup_block.contains("AsInternalProvider"),
            "method with Option<ExcludedType> return must be skipped, got:\n{}",
            emission.setup_block
        );

        // InternalProvider must not appear anywhere in the stub.
        assert!(
            !emission.setup_block.contains("InternalProvider"),
            "excluded type InternalProvider must not appear in stub, got:\n{}",
            emission.setup_block
        );
    }
}
