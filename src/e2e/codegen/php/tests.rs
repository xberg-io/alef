#![allow(clippy::module_name_repetitions)]

#[cfg(test)]
mod not_empty_tests {
    use super::super::assertions::render_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{BTreeMap, HashMap, HashSet};

    fn render_not_empty(result_is_array: bool) -> String {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "not_empty".to_string(),
            ..Default::default()
        };
        let mut out = String::new();
        let getter_map = crate::e2e::field_access::PhpGetterMap::default();
        let lowering = super::super::enum_variant_access::PhpEnumLowering::from_enums(&[]);
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            result_is_array,
            &BTreeMap::new(),
            false,
            &super::super::enum_variant_access::PhpVariantAccess::new(&getter_map, &lowering),
        );
        out
    }

    /// PHPUnit's `assertNotEmpty()` routes through `empty()`, which classifies a
    /// legitimate `0`, `0.0`, `"0"` and `false` as empty and fails a correct result.
    #[test]
    fn not_empty_for_php_scalars_rejects_empty_string_but_accepts_zero() {
        let out = render_not_empty(false);
        assert!(!out.contains("assertNotEmpty"), "got: {out}");
        assert_eq!(
            out.trim(),
            "$this->assertNotSame('', $result ?? '', 'expected non-empty value');"
        );
    }

    #[test]
    fn not_empty_for_php_arrays_measures_the_element_count() {
        let out = render_not_empty(true);
        assert_eq!(
            out.trim(),
            "$this->assertGreaterThan(0, count($result ?? []), 'expected non-empty value');"
        );
    }
}

#[cfg(test)]
mod trait_bridge_tests {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn make_fixture(id: &str) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
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
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
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
        let emission = super::super::stubs::emit_test_backend(&trait_bridge, &methods, &fixture);

        // setup_block must not reference any sample_core-domain trait or method names.
        assert!(
            !emission.setup_block.contains("OcrBackend"),
            "setup_block must not hardcode domain trait names, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("process_image"),
            "setup_block must not hardcode domain method names, got:\n{}",
            emission.setup_block
        );
        // Must emit the method name verbatim from MethodDef (PHP snake_case).
        assert!(
            emission.setup_block.contains("do_thing"),
            "setup_block must contain the PHP snake_case method name 'do_thing', got:\n{}",
            emission.setup_block
        );
        // Must emit Plugin name method when super_trait is set.
        assert!(
            emission.setup_block.contains("name"),
            "setup_block must emit 'name' for super_trait, got:\n{}",
            emission.setup_block
        );
        // arg_expr is the anonymous class variable.
        assert_eq!(
            emission.arg_expr, "$stub",
            "arg_expr must be '$stub', got: {}",
            emission.arg_expr
        );
    }

    /// Named return types must emit `'{}'` (JSON-safe empty-object string), not
    /// a constructor call that would reference an undefined class.
    #[test]
    fn test_backend_named_return_emits_json_string() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes), ("mime_type", TypeRef::String)],
            TypeRef::Named("InternalRecord".to_string()),
            false,
        );

        let fixture = make_fixture("register_document_extractor_trait_bridge");
        let methods = vec![&extract_bytes];
        let emission = super::super::stubs::emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("'{}'"),
            "Named return type must emit '{{}}' not a constructor call, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("new InternalRecord"),
            "setup_block must not reference undefined type InternalRecord, got:\n{}",
            emission.setup_block
        );
    }

    /// Backend name is extracted from fixture.input, not fixture.id.
    #[test]
    fn test_backend_name_from_input() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes)],
            TypeRef::Named("InternalRecord".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = super::super::stubs::emit_test_backend(&trait_bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("test-extractor"),
            "setup_block must use input-derived name 'test-extractor', got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission
                .setup_block
                .contains("register_document_extractor_trait_bridge"),
            "setup_block must not use fixture id as name, got:\n{}",
            emission.setup_block
        );
    }

    /// Snapshot: verify exact setup_block shape for a DocumentExtractor-like bridge.
    #[test]
    fn test_backend_snapshot() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        let extract_bytes = make_method(
            "extract_bytes",
            vec![
                ("content", TypeRef::Bytes),
                ("mime_type", TypeRef::String),
                ("config", TypeRef::Named("ParseConfig".to_string())),
            ],
            TypeRef::Named("InternalRecord".to_string()),
            false,
        );

        let mut fixture = make_fixture("register_document_extractor_trait_bridge");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-extractor" }
        });

        let methods = vec![&extract_bytes];
        let emission = super::super::stubs::emit_test_backend(&trait_bridge, &methods, &fixture);

        let expected_setup = concat!(
            "$stub = new class implements DocumentExtractor {\n",
            "    public function name(): string { return 'test-extractor'; }\n",
            "    public function extract_bytes($content, $mime_type, $config): mixed { return '{}'; }\n",
            "};\n",
        );
        assert_eq!(emission.setup_block, expected_setup, "setup_block snapshot mismatch");
        assert_eq!(emission.arg_expr, "$stub");
    }

    /// Verify that test stubs include both direct trait methods and super-trait methods.
    #[test]
    fn test_backend_includes_super_trait_methods() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("crate::plugin::Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        // Simulate a direct trait method.
        let extract_bytes = make_method(
            "extract_bytes",
            vec![("content", TypeRef::Bytes), ("mime_type", TypeRef::String)],
            TypeRef::Named("ProcessingResult".to_string()),
            false,
        );

        // Simulate super-trait methods (Plugin trait).
        let name_method = make_method("name", vec![], TypeRef::String, false);
        let priority_method = make_method(
            "priority",
            vec![],
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
            false,
        );

        let mut fixture = make_fixture("test_super_trait_methods");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "my-extractor" }
        });

        // Pass both direct trait methods and super-trait methods.
        let methods = vec![&extract_bytes, &name_method, &priority_method];
        let emission = super::super::stubs::emit_test_backend(&trait_bridge, &methods, &fixture);

        // Verify the setup_block includes all required methods.
        assert!(
            emission.setup_block.contains("public function name()"),
            "setup_block must include name() from super-trait (Plugin)"
        );
        assert!(
            emission.setup_block.contains("public function priority()"),
            "setup_block must include priority() from super-trait (Plugin)"
        );
        assert!(
            emission.setup_block.contains("public function extract_bytes("),
            "setup_block must include extract_bytes() from direct trait (DocumentExtractor)"
        );
        assert!(
            emission.setup_block.contains("my-extractor"),
            "setup_block must use input-derived name"
        );
    }

    /// Verify that name() is not duplicated when super_trait is set and name is in methods list.
    /// This test prevents the PHP fatal "Cannot redeclare ...::name()" bug.
    #[test]
    fn test_backend_no_duplicate_name_with_super_trait() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_ocr_backend".to_string()),
            ..TraitBridgeConfig::default()
        };

        // Direct method.
        let process_image = make_method(
            "process_image",
            vec![("image", TypeRef::Bytes)],
            TypeRef::Named("OcrResult".to_string()),
            false,
        );

        // Super-trait methods: name() and priority().
        let name_method = make_method("name", vec![], TypeRef::String, false);
        let priority_method = make_method(
            "priority",
            vec![],
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
            false,
        );

        let mut fixture = make_fixture("test_no_duplicate_name");
        fixture.input = serde_json::json!({
            "ocr_backend": { "type": "test", "name": "test-ocr" }
        });

        let methods = vec![&process_image, &name_method, &priority_method];
        let emission = super::super::stubs::emit_test_backend(&trait_bridge, &methods, &fixture);

        // Count occurrences of "function name()" to ensure exactly one.
        let name_count = emission.setup_block.matches("function name()").count();
        assert_eq!(
            name_count, 1,
            "name() must appear exactly once in setup_block, found {} occurrences:\n{}",
            name_count, emission.setup_block
        );

        // Verify all methods are present (name deduplicated, not missing).
        assert!(
            emission
                .setup_block
                .contains("public function name(): string { return 'test-ocr'; }"),
            "name() must be hardcoded with backend name, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("public function priority()"),
            "priority() must be present, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("public function process_image("),
            "process_image() must be present, got:\n{}",
            emission.setup_block
        );
    }

    /// Verify that test stubs emit methods with default implementations.
    /// PHP interfaces require ALL abstract methods to be implemented, even if the
    /// Rust trait has default implementations.
    #[test]
    fn test_backend_includes_default_impl_methods() {
        let trait_bridge = TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_document_extractor".to_string()),
            ..TraitBridgeConfig::default()
        };

        // Direct trait method (no default impl).
        let extract_bytes = make_method(
            "extract_bytes",
            vec![
                ("content", TypeRef::Bytes),
                ("mime_type", TypeRef::String),
                ("config", TypeRef::Named("ParseConfig".to_string())),
            ],
            TypeRef::Named("ParseResult".to_string()),
            false,
        );

        // Method with default impl in Rust trait.
        let mut as_sync_extractor = make_method("as_sync_extractor", vec![], TypeRef::String, false);
        as_sync_extractor.has_default_impl = true;

        // Super-trait method with default impl.
        let mut priority = make_method(
            "priority",
            vec![],
            TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
            false,
        );
        priority.has_default_impl = true;

        let mut fixture = make_fixture("test_default_impl_methods");
        fixture.input = serde_json::json!({
            "extractor": { "type": "test", "name": "test-default-impl" }
        });

        let methods = vec![&extract_bytes, &as_sync_extractor, &priority];
        let emission = super::super::stubs::emit_test_backend(&trait_bridge, &methods, &fixture);

        // PHP requires implementations of ALL abstract methods, including those with
        // default implementations in the Rust trait.
        assert!(
            emission.setup_block.contains("public function extract_bytes("),
            "extract_bytes() must be emitted, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("public function as_sync_extractor()"),
            "as_sync_extractor() with default impl must be emitted for PHP interface, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("public function priority()"),
            "priority() with default impl must be emitted for PHP interface, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("test-default-impl"),
            "Backend name must be correct, got:\n{}",
            emission.setup_block
        );
    }
}

#[cfg(test)]
mod composer_json_tests {
    use super::super::project::{render_composer_json, render_install_sh};
    use crate::e2e::config::DependencyMode;

    #[test]
    fn registry_composer_json_omits_ext_platform_req() {
        let content = render_composer_json(
            "sample_crate/e2e-php",
            "SampleLlm\\\\E2e\\\\",
            "demo_client",
            "Demo\\Client",
            "../../packages/php/src/",
            "1.4.0-rc.32",
            DependencyMode::Registry,
        );
        // Must NOT declare the ext-<name> platform require. The extension is
        // installed via PIE (in install.sh) before `composer install` runs, so
        // Composer doesn't manage it. Declaring ext-<name> in composer.json causes
        // Composer's platform resolver to fail when the extension hasn't been
        // loaded into the current PHP process yet.
        assert!(
            !content.contains(r#""ext-demo_client":"#),
            "registry composer.json must NOT require ext-demo_client (PIE installs it), got:\n{content}"
        );
        // Must declare the php platform require.
        assert!(
            content.contains(r#""php": ">=8.2""#),
            "registry composer.json must require php >=8.2, got:\n{content}"
        );
        // Must NOT contain a direct package require (composer can't resolve it
        // before PIE has installed the .so).
        assert!(
            !content.contains("sample_crate/demo-client"),
            "registry composer.json must not contain a direct package require, got:\n{content}"
        );
        // Must NOT carry minimum-stability / prefer-stable (not load-bearing with
        // only php platform req).
        assert!(
            !content.contains("minimum-stability"),
            "registry composer.json must not contain minimum-stability, got:\n{content}"
        );
        assert!(
            !content.contains("prefer-stable"),
            "registry composer.json must not contain prefer-stable, got:\n{content}"
        );
        // Must keep require-dev (phpunit + guzzle).
        assert!(
            content.contains("phpunit/phpunit"),
            "registry composer.json must keep phpunit in require-dev, got:\n{content}"
        );
        assert!(
            content.contains("guzzlehttp/guzzle"),
            "registry composer.json must keep guzzle in require-dev, got:\n{content}"
        );
    }

    #[test]
    fn registry_composer_json_declares_userland_autoload() {
        let content = render_composer_json(
            "sample_crate/e2e-php",
            "SampleLlm\\\\E2e\\\\",
            "demo_client",
            "Demo\\Client",
            "../../packages/php/src/",
            "1.4.0-rc.32",
            DependencyMode::Registry,
        );
        // The userland PHP classes are layered over the native ext-php-rs
        // extension and are NOT registered by it, so PHPUnit needs a PSR-4
        // autoload mapping to the local package source. Without it every test
        // fails with `Class not found` even after PIE installs the extension.
        assert!(
            content.contains(r#""autoload""#),
            "registry composer.json must declare an autoload section, got:\n{content}"
        );
        assert!(
            content.contains(r#""Demo\\Client\\": "../../packages/php/src/""#),
            "registry composer.json must map the userland namespace to the package src, got:\n{content}"
        );
    }

    #[test]
    fn local_composer_json_declares_userland_autoload() {
        let content = render_composer_json(
            "sample_crate/e2e-php",
            "SampleLlm\\\\E2e\\\\",
            "demo_client",
            "Demo\\Client",
            "../../packages/php/src/",
            "1.4.0-rc.32",
            DependencyMode::Local,
        );
        assert!(
            content.contains(r#""Demo\\Client\\": "../../packages/php/src/""#),
            "local composer.json must map the userland namespace to the package src, got:\n{content}"
        );
    }

    /// A single-segment camelCase namespace (`[crates.php] namespace = "WidgetToolkit"`,
    /// declared in PHP as `namespace WidgetToolkit;`) must stay one PSR-4 segment.
    /// Word-splitting it into `Widget\Toolkit\` produces a prefix that matches no
    /// declared namespace, so Composer autoloads none of the userland classes and
    /// every test dies with `Class "WidgetToolkit\..." not found`.
    #[test]
    fn composer_json_does_not_word_split_camel_case_namespace() {
        for dep_mode in [DependencyMode::Local, DependencyMode::Registry] {
            let content = render_composer_json(
                "acme/e2e-php",
                "WidgetToolkit\\\\E2e\\\\",
                "widget_toolkit",
                "WidgetToolkit",
                "../../crates/widget-toolkit-php/src/",
                "1.0.0",
                dep_mode,
            );
            assert!(
                content.contains(r#""WidgetToolkit\\": "../../crates/widget-toolkit-php/src/""#),
                "{dep_mode:?} composer.json must use the configured namespace verbatim, got:\n{content}"
            );
            assert!(
                !content.contains(r#""Widget\\Toolkit\\""#),
                "{dep_mode:?} composer.json must not split camelCase humps into PSR-4 segments, got:\n{content}"
            );
        }
    }

    /// A namespace that legitimately contains separators (`Acme\Widget`) keeps them,
    /// JSON-escaped, rather than being re-derived from the composer package name.
    #[test]
    fn composer_json_preserves_multi_segment_namespace() {
        let content = render_composer_json(
            "acme/e2e-php",
            "Acme\\\\Widget\\\\E2e\\\\",
            "widget",
            "Acme\\Widget",
            "../../packages/php/src/",
            "1.0.0",
            DependencyMode::Local,
        );
        assert!(
            content.contains(r#""Acme\\Widget\\": "../../packages/php/src/""#),
            "composer.json must preserve explicit namespace separators, got:\n{content}"
        );
    }

    #[test]
    fn registry_install_sh_contains_pie_install() {
        let content = render_install_sh("sample_crate/demo-client", "demo_client", "1.4.0-rc.32");
        // The script uses $PIE as the resolved pie binary path.
        assert!(
            content.contains("\"$PIE\" install"),
            "install.sh must invoke pie via $PIE install, got:\n{content}"
        );
        assert!(
            content.contains("sample_crate/demo-client"),
            "install.sh must reference the package name, got:\n{content}"
        );
        assert!(
            content.starts_with("#!/usr/bin/env bash"),
            "install.sh must start with bash shebang, got:\n{content}"
        );
        // Version is baked in so callers can run `bash install.sh` with no args.
        assert!(
            content.contains("PINNED_VERSION='1.4.0-rc.32'") && content.contains(r#"VERSION="${1:-$PINNED_VERSION}""#),
            "install.sh must contain version default, got:\n{content}"
        );
    }

    #[test]
    fn registry_install_sh_strips_version_constraints() {
        // Test constraint operators are stripped from version strings.
        let tests = vec![
            (">=3.5.1", "3.5.1"),
            ("^1.2.3", "1.2.3"),
            ("~2.0.0", "2.0.0"),
            (">1.0", "1.0"),
            ("<2.0", "2.0"),
            ("1.4.0-rc.32", "1.4.0-rc.32"), // Already clean
        ];
        for (input, expected) in tests {
            let content = render_install_sh("test/pkg", "ext", input);
            assert!(
                content.contains(&format!("PINNED_VERSION='{expected}'")),
                "install.sh must strip constraint from '{}' to '{}', got:\n{}",
                input,
                expected,
                content
            );
        }
    }

    #[test]
    fn registry_install_sh_downloads_pie_phar() {
        let content = render_install_sh("test/pkg", "ext", "1.0.0");
        // Ensure the script downloads PIE as a PHAR, not via composer require.
        assert!(
            content.contains("https://github.com/php/pie/releases/download/$PIE_VERSION/pie.phar"),
            "install.sh must download PIE PHAR from GitHub, got:\n{content}"
        );
        assert!(
            !content.contains("composer global require php/pie"),
            "install.sh must not use composer to install PIE, got:\n{content}"
        );
    }

    /// A `releases/latest/download/` URL executes whatever upstream published at run time,
    /// unverified and unreproducible. The pin and the digest are one pair: assert BOTH, and
    /// assert the floating form is gone, so reintroducing either half of the exposure fails.
    #[test]
    fn registry_install_sh_pins_and_checksums_the_pie_phar() {
        let content = render_install_sh("test/pkg", "ext", "1.0.0");
        let version = crate::core::template_versions::github_release::PIE_VERSION;
        let sha256 = crate::core::template_versions::github_release::PIE_PHAR_SHA256;

        assert!(
            !content.contains("releases/latest/download"),
            "install.sh must not fetch a floating `latest` release, got:\n{content}"
        );
        assert!(
            content.contains(&format!("PIE_VERSION={version}")),
            "install.sh must pin PIE to {version}, got:\n{content}"
        );
        assert!(
            content.contains(&format!("PIE_PHAR_SHA256={sha256}")),
            "install.sh must carry the pinned PHAR digest, got:\n{content}"
        );
        assert_eq!(sha256.len(), 64, "a SHA-256 digest is 64 hex characters");
        assert!(
            sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "the digest must be lowercase hex, or the `!=` comparison in the script never matches"
        );
        assert!(
            content.contains("$pie_actual_sha256\" != \"$PIE_PHAR_SHA256"),
            "install.sh must compare the download's digest against the pin, got:\n{content}"
        );
    }

    /// The digest check must be a hard gate, not a best-effort one: a script that skips
    /// verification when no hashing tool is present verifies nothing on exactly the machines
    /// where verification matters most.
    #[test]
    fn registry_install_sh_refuses_to_run_an_unverified_phar() {
        let content = render_install_sh("test/pkg", "ext", "1.0.0");
        for expected in [
            "sha256sum",
            "shasum -a 256",
            "refusing to run an unverified PIE PHAR",
            "checksum mismatch",
        ] {
            assert!(
                content.contains(expected),
                "install.sh must contain {expected:?}, got:\n{content}"
            );
        }
        // The PHAR is installed only after the digest comparison, never before it.
        let verify = content.find("checksum mismatch").expect("mismatch guard present");
        let install = content
            .find("install -m 0755 \"$pie_tmp\"")
            .expect("phar installed with `install`");
        assert!(
            verify < install,
            "the digest must be checked before the PHAR is made executable, got:\n{content}"
        );
        assert!(
            !content.contains("chmod +x \"$pie_dir/pie\""),
            "the old download-then-chmod path executed an unverified PHAR, got:\n{content}"
        );
    }

    #[test]
    fn registry_install_sh_enables_extension_in_php_ini() {
        let content = render_install_sh("test/pkg", "my_ext", "1.0.0");
        // After PIE install, script must locate php.ini and enable the extension.
        assert!(
            content.contains("php --ini"),
            "install.sh must locate php.ini using 'php --ini', got:\n{content}"
        );
        assert!(
            content.contains("Loaded Configuration File:"),
            "install.sh must grep for 'Loaded Configuration File:' from php --ini, got:\n{content}"
        );
        // Script must append the extension line (idempotently).
        assert!(
            content.contains("EXTENSION_NAME='my_ext'") && content.contains("extension=$EXTENSION_NAME"),
            "install.sh must append 'extension=my_ext' to php.ini, got:\n{content}"
        );
        assert!(
            content.contains("grep -Fqx \"extension=$EXTENSION_NAME\""),
            "install.sh must guard against duplicate extension entries, got:\n{content}"
        );
        assert!(
            content.contains(">> \"$PHP_INI\""),
            "install.sh must append to php.ini, got:\n{content}"
        );
    }

    #[test]
    fn registry_install_sh_keeps_config_values_out_of_shell_syntax() {
        let malicious = "literal'$(touch /tmp/alef-php-install); #";
        let content = render_install_sh(malicious, malicious, malicious);
        let escaped = crate::core::config::shell::quote_word(malicious);
        for key in ["PKG_NAME", "EXTENSION_NAME", "PINNED_VERSION"] {
            assert!(content.contains(&format!("{key}={escaped}")), "got: {content}");
        }
        let status = crate::core::bash_command()
            .args(["-n", "-c", &content])
            .status()
            .expect("bash should parse generated script");
        assert!(status.success());
    }

    /// Regression for alef task #477: `render_install_sh`'s `GeneratedFile` uses
    /// `generated_header: false`, so ownership tracking depends entirely on the rendered
    /// content itself carrying a marker `hash::content_has_alef_marker` recognizes. A prior
    /// version hand-spelled `"# alef-generated installer..."`, which that guard does not
    /// match, permanently stranding `install.sh` as unowned/unadoptable. Asserts through the
    /// real guard function rather than a hand-copied literal, so this fails if the two ever
    /// disagree again.
    #[test]
    fn registry_install_sh_marker_is_recognised_by_the_real_ownership_guard() {
        let content = render_install_sh("test/pkg", "my_ext", "1.0.0");
        assert!(
            crate::core::hash::content_has_alef_marker(&content),
            "install.sh must carry a marker the real alef ownership guard recognises, got:\n{content}"
        );
    }

    /// Negative control for the test above: content with no ownership marker at all must
    /// still be reported unowned, proving the guard is not vacuously true.
    #[test]
    fn content_with_no_marker_is_not_recognised_by_the_ownership_guard() {
        let content = "#!/usr/bin/env bash\n# just a plain hand-written script\nset -euo pipefail\necho hi\n";
        assert!(
            !crate::core::hash::content_has_alef_marker(content),
            "content with no alef marker must not be recognised as alef-owned, got:\n{content}"
        );
    }
}

#[cfg(test)]
mod autoload_namespace_wiring_tests {
    use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
    use crate::e2e::codegen::E2eCodegen;
    use crate::e2e::codegen::php::PhpCodegen;
    use crate::e2e::config::E2eConfig;

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    fn composer_json_for(toml_text: &str) -> String {
        let config = resolve_config(toml_text);
        let files = PhpCodegen
            .generate(&[], &E2eConfig::default(), &config, &[], &[], &[], &[])
            .expect("php e2e generation must succeed");
        files
            .iter()
            .find(|f| f.path.file_name().is_some_and(|n| n == "composer.json"))
            .map(|f| f.content.clone())
            .expect("php e2e generation must emit composer.json")
    }

    /// The PSR-4 prefix must come from `[crates.php] namespace`, not from the composer
    /// package name derived from the hyphenated crate name. Deriving it from
    /// `<vendor>/widget-toolkit` word-split the name into `Widget\Toolkit\`, which
    /// matches no declared namespace, so Composer autoloaded nothing and every PHPUnit
    /// e2e test failed with `Class "WidgetToolkit\WidgetToolkit" not found`.
    #[test]
    fn psr4_prefix_uses_configured_namespace_not_hyphenated_package_name() {
        let content = composer_json_for(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "widget-toolkit"
sources = []
[crates.php]
namespace = "WidgetToolkit"
"#,
        );
        assert!(
            content.contains(r#""WidgetToolkit\\": "../../crates/widget-toolkit-php/src/""#),
            "composer.json must map the configured namespace verbatim, got:\n{content}"
        );
        assert!(
            !content.contains("Widget\\\\Toolkit"),
            "composer.json must not word-split the configured namespace, got:\n{content}"
        );
    }

    /// With no `[crates.php] namespace`, the fallback still derives from the extension
    /// name (`widget_toolkit` → `Widget\Toolkit`), which is the documented default —
    /// this pins that the fix did not change unconfigured behaviour.
    #[test]
    fn psr4_prefix_falls_back_to_extension_derived_namespace() {
        let content = composer_json_for(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "widget-toolkit"
sources = []
"#,
        );
        assert!(
            content.contains(r#""Widget\\Toolkit\\": "../../crates/widget-toolkit-php/src/""#),
            "composer.json must fall back to the extension-derived namespace, got:\n{content}"
        );
    }
}

#[cfg(test)]
mod default_pkg_path_tests {
    use super::super::default_php_pkg_path;
    use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};

    fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml_text).expect("valid config");
        cfg.resolve().expect("resolve").remove(0)
    }

    /// Unconfigured `[crates.output] php` resolves through `OutputTemplate::resolve`'s
    /// default to the co-located `crates/<pkg>-php` crate root -- the same root the
    /// scaffolder writes `crates/<pkg>-php/Cargo.toml` into.
    #[test]
    fn defaults_to_the_co_located_binding_crate_when_output_unconfigured() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
"#,
        );
        assert_eq!(default_php_pkg_path(&config), "../../crates/my-lib-php");
    }

    /// `[crates.output] php = "crates/<pkg>-php/src/"` (the 0.51 co-located layout) must
    /// resolve to the crate root, not the `src/` subdirectory — `php_autoload_section`
    /// re-appends its own `/src/` suffix, so keeping it here would double up into a
    /// nonexistent `.../src/src/`.
    #[test]
    fn strips_trailing_src_for_co_located_output() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "crates/my-lib-php/src/"
"#,
        );
        assert_eq!(default_php_pkg_path(&config), "../../crates/my-lib-php");
    }

    /// A co-located output path without a trailing `src` segment is used as-is.
    #[test]
    fn keeps_co_located_output_without_trailing_src() {
        let config = resolve_config(
            r#"
[workspace]
languages = ["php"]
[[crates]]
name = "my-lib"
sources = []
[crates.output]
php = "crates/my-lib-php"
"#,
        );
        assert_eq!(default_php_pkg_path(&config), "../../crates/my-lib-php");
    }
}
