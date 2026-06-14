use super::*;

/// Regression test: when a Rust function has `Option<T>` parameters (e.g., `mime_type: Option<&str>`),
/// the PHP wrapper must emit nullable type hints (`?string`) with defaults (`= null`), not non-nullable.
/// Previously, when `TypeRef::Optional` was already prepended by `php_type()`, the code would incorrectly
/// add another `?` prefix, creating invalid double-nullable types or failing to detect existing nullability.
#[test]
fn test_php_option_param_emits_nullable_with_default() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "do_thing".to_string(),
            rust_path: "test_lib::do_thing".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "required_str".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "optional_str".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Do a thing with strings".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).expect("generate ok");

    let facade_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".php"))
        .expect("facade file exists");

    let content = &facade_file.content;

    // Required parameter should be `string $required_str` (not nullable, no default).
    assert!(
        content.contains("string $required_str"),
        "required parameter must be non-nullable; got:\n{content}"
    );

    // Optional parameter should be `?string $optional_str = null` (nullable with default).
    // Must NOT be `??string` (double-nullable) or `string $optional_str` (missing null default).
    assert!(
        content.contains("?string $optional_str = null"),
        "optional parameter must be ?string with = null default; got:\n{content}"
    );

    // Verify no double-nullable nonsense.
    assert!(
        !content.contains("??string"),
        "must not have double-nullable ??string; got:\n{content}"
    );
}

/// Regression test for Block B7: required &str parameters must not be marked nullable.
/// When a function has both required and optional string parameters, the required ones
/// should remain non-nullable (string $param) even if they're followed by optional ones.
/// This test ensures that nullable inference doesn't propagate from optional params
/// to required ones, which would cause null to pass through the PHP wrapper and panic
/// in the Rust core where the parameter is actually required.
#[test]
fn test_php_required_str_param_not_nullable_with_optional_tail() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![FunctionDef {
            name: "process_document".to_string(),
            rust_path: "test_lib::process_document".to_string(),
            original_rust_path: String::new(),
            params: vec![
                // Required &str parameter (maps to TypeRef::String with optional=false)
                ParamDef {
                    name: "content_type".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    is_ref: true, // Rust signature: &str
                    ..ParamDef::default()
                },
                // Optional &str parameter (maps to TypeRef::Optional(String) with is_ref=true)
                ParamDef {
                    name: "hint".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
            doc: "Process a document with optional hint".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();
    let files = backend.generate_public_api(&api, &config).expect("generate ok");

    let facade_file = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with(".php"))
        .expect("facade file exists");

    let content = &facade_file.content;

    // Required parameter MUST be non-nullable, not "?string $content_type".
    // The Rust core function signature is processDocument(content_type: &str, ...)
    // so null is never valid for this parameter.
    assert!(
        content.contains("string $content_type") && !content.contains("?string $content_type"),
        "required &str parameter must be non-nullable string; got:\n{content}"
    );

    // Optional parameter MUST be nullable with default.
    // The Rust core function accepts Option<&str>, so PHP can pass null.
    assert!(
        content.contains("?string $hint = null"),
        "optional parameter must be ?string with = null default; got:\n{content}"
    );

    // Sanity: no double-nullable.
    assert!(
        !content.contains("??string"),
        "must not have double-nullable ??string; got:\n{content}"
    );
}

/// Every generated PHP source file must have a blank line immediately after the
/// `<?php` opening tag. PSR-12's `blank_line_after_opening_tag` rule (enforced by
/// php-cs-fixer) inserts one post-write, which would mutate the alef-hash-tracked
/// file and break `alef verify`. Emitting it natively keeps the formatter a no-op.
#[test]
fn test_php_source_files_have_blank_line_after_opening_tag() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "Config".to_string(),
                rust_path: "test_lib::Config".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), true)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Config".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
            TypeDef {
                name: "Handle".to_string(),
                rust_path: "test_lib::Handle".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![MethodDef {
                    name: "close".to_string(),
                    params: vec![],
                    return_type: TypeRef::Unit,
                    is_async: false,
                    is_static: false,
                    error_type: None,
                    doc: "Close the handle".to_string(),
                    receiver: Some(ReceiverKind::Owned),
                    sanitized: false,
                    returns_ref: false,
                    returns_cow: false,
                    return_newtype_wrapper: None,
                    has_default_impl: false,
                    trait_source: None,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    version: Default::default(),
                }],
                is_opaque: true,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: "Opaque handle".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let config = make_config();

    // Collect all generated PHP source files (facade + opaque class files + type stubs).
    let mut php_files: Vec<alef::core::backend::GeneratedFile> = Vec::new();
    php_files.extend(backend.generate_public_api(&api, &config).expect("public api ok"));
    php_files.extend(backend.generate_type_stubs(&api, &config).expect("type stubs ok"));
    php_files.retain(|f| f.path.extension().and_then(|e| e.to_str()) == Some("php"));
    assert!(!php_files.is_empty(), "expected at least one generated .php file");

    for file in &php_files {
        let name = file.path.to_string_lossy().to_string();
        assert!(
            file.content.starts_with("<?php\n\n"),
            "{name} must have a blank line after `<?php` (PSR-12 blank_line_after_opening_tag). got:\n{}",
            &file.content[..file.content.len().min(120)],
        );
    }

    // Strongest check: run php-cs-fixer with the scaffold's @PSR12 ruleset and assert it
    // produces zero changes. Skips when php or php-cs-fixer are unavailable.
    use std::process::Command;
    let tools_available = Command::new("php").arg("--version").output().is_ok()
        && Command::new("php-cs-fixer").arg("--version").output().is_ok();
    if !tools_available {
        eprintln!("skipping php-cs-fixer no-op check: php or php-cs-fixer not installed");
        return;
    }

    let dir = tempfile::tempdir().unwrap();

    // The scaffold's php-cs-fixer config formats `src/` but explicitly excludes `stubs/`
    // (`->notPath('stubs')`) because the stub files carry ext-php-rs scaffolding the formatter
    // would otherwise rewrite. So the formatter no-op contract applies to the userland `src/`
    // files (facade + opaque DTO classes) — those are what `alef verify` and the formatter must
    // agree on. Stub files only need the blank-line-after-`<?php` guarantee asserted above.
    for file in php_files.iter().filter(|f| !f.path.to_string_lossy().contains("stubs")) {
        let php_path = dir.path().join("subject.php");
        std::fs::write(&php_path, &file.content).unwrap();
        let output = Command::new("php-cs-fixer")
            .arg("fix")
            .arg("--using-cache=no")
            .arg("--rules=@PSR12")
            .arg(&php_path)
            .output()
            .expect("run php-cs-fixer");
        let after = std::fs::read_to_string(&php_path).unwrap();
        assert_eq!(
            after,
            file.content,
            "php-cs-fixer rewrote {}; stderr:\n{}",
            file.path.display(),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn facade_emits_nullable_marker_for_non_tail_optional_param() {
    // Regression: when an `Option<T>` param is followed by a non-nullable required
    // param, PHP 8.1 ordering forces the optional param into a non-tail position.
    // The facade must still emit `?T $name` (nullable, no default) so callers can
    // pass `null`. Before the fix, the emitter dropped the `?` entirely, producing
    // `string $mime_type` for the canonical `extract_file(path, mime_type, config)`
    // signature, which made every test passing `null` for `mime_type` fail with a
    // PHP TypeError.
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract_file".to_string(),
            rust_path: "test_lib::extract_file".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "path".to_string(),
                    ty: TypeRef::Path,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "mime_type".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("ExtractionConfig".to_string()),
                    is_ref: true,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("Error".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = backend.generate_public_api(&api, &make_config()).unwrap();
    let facade = files.first().expect("facade file generated");
    assert!(
        facade.content.contains("?string $mime_type"),
        "facade must keep the nullable marker on non-tail Option<T> params; got:\n{}",
        facade.content
    );
    // Reject a non-nullable `string $mime_type` (must be `?string`). Use leading-space
    // anchors so the `?` form isn't a substring match for the non-`?` form.
    assert!(
        !facade.content.contains(" string $mime_type"),
        "facade must not emit a non-nullable `string $mime_type`; got:\n{}",
        facade.content
    );
}

#[test]
fn module_entry_uses_explicit_extension_name_not_cargo_pkg_name() {
    // Regression test for PHP module registration bug where the module name
    // did not match the extension name, causing `php -m` to fail and PIE
    // install to error with "already loaded". The root cause was #[php_module]
    // macro expansion using env!("CARGO_PKG_NAME") which could differ from
    // the publishable extension_name (e.g., crate "ts-pack-core-php" vs.
    // extension "tree_sitter_language_pack").
    // Solution: generate ModuleBuilder::new(extension_name, version) explicitly.
    let backend = PhpBackend;
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.2.3".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_config_with_extension("tree_sitter_language_pack");
    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib_rs = files
        .iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs generated");

    // Verify the module entry function explicitly passes extension_name to ModuleBuilder::new()
    assert!(
        lib_rs.content.contains("ModuleBuilder::new(") && lib_rs.content.contains("tree_sitter_language_pack"),
        "module entry must use explicit extension name in ModuleBuilder::new(); got:\n{}",
        lib_rs.content
    );

    // Verify it does NOT use env!("CARGO_PKG_NAME") fallback
    assert!(
        !lib_rs.content.contains("CARGO_PKG_NAME"),
        "module entry must not rely on CARGO_PKG_NAME macro; got:\n{}",
        lib_rs.content
    );

    // Verify the get_module function is properly formed with manual ModuleBuilder
    assert!(
        lib_rs.content.contains("extern \"C\" fn get_module()"),
        "module entry must export get_module extern function; got:\n{}",
        lib_rs.content
    );
    assert!(
        lib_rs.content.contains("StaticModuleEntry"),
        "module entry must use StaticModuleEntry for thread-safe singleton; got:\n{}",
        lib_rs.content
    );
}
