//! Free function and module-init code generators for the Magnus (Ruby) backend.

mod async_wrappers;
mod module_init;
mod scan_args_defaults;
mod serde_bindings;
mod sync_wrappers;

pub(super) use async_wrappers::{ASYNC_RETURN_IS_FALLIBLE, gen_async_function};
pub(super) use module_init::gen_module_init;
pub(super) use sync_wrappers::{gen_function, gen_magnus_unimplemented_body};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn make_config() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
        )
    }

    fn simple_func(name: &str, error: bool) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("test_lib::{name}"),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
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
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: if error { Some("Error".to_string()) } else { None },
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
        }
    }

    #[test]
    fn gen_function_emits_fn_name() {
        let func = simple_func("process", false);
        let mapper = crate::backends::magnus::type_map::MagnusMapper;
        let api = crate::core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::BTreeMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let code = gen_function(
            &func,
            &mapper,
            &Default::default(),
            &Default::default(),
            "test_lib",
            &api,
        );
        assert!(code.contains("fn process("), "must emit function name");
        assert!(code.contains("input: String"), "must include typed param");
    }

    #[test]
    fn gen_function_with_error_wraps_result() {
        let func = simple_func("process", true);
        let mapper = crate::backends::magnus::type_map::MagnusMapper;
        let api = crate::core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::BTreeMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let code = gen_function(
            &func,
            &mapper,
            &Default::default(),
            &Default::default(),
            "test_lib",
            &api,
        );
        assert!(code.contains("Result<"), "error function must return Result");
    }

    #[test]
    fn gen_module_init_emits_magnus_init_attr() {
        let config = make_config();
        let api = crate::core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::BTreeMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let code = gen_module_init(
            "TestLib",
            &api,
            &config,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &[],
            &Default::default(),
            &[],
        );
        assert!(code.contains("#[magnus::init]"), "must emit #[magnus::init]");
        assert!(code.contains("fn ruby_init(ruby: &Ruby)"), "must emit init fn");
        assert!(code.contains("define_module(\"TestLib\")"), "must define the module");
    }

    fn zero_arg_func(name: &str, cfg: Option<&str>) -> FunctionDef {
        FunctionDef {
            params: vec![],
            cfg: cfg.map(str::to_string),
            ..simple_func(name, false)
        }
    }

    /// Regression for a `#[cfg(feature = "X")]`-gated top-level function reaching a binding
    /// crate that does not declare `X`: the `fn` definition (see `prepend_cfg` in
    /// `gen_bindings::mod`) and its `define_module_function` registration must carry the exact
    /// same gate, or the registration names a function the gate compiled out
    /// (`E0425 cannot find value`). Before the fix, this registration was emitted ungated
    /// regardless of `func.cfg`.
    #[test]
    fn gen_module_init_gates_function_registration_to_match_its_definition() {
        let config = make_config();
        let gated = zero_arg_func("count_tokens", Some(r#"feature = "tokenizer""#));
        let api = crate::core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![gated],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::BTreeMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let code = gen_module_init(
            "TestLib",
            &api,
            &config,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &[],
            &Default::default(),
            &[],
        );
        assert_eq!(
            code.matches(
                "#[cfg(feature = \"tokenizer\")]\n    module.define_module_function(\"count_tokens\", \
                 function!(count_tokens, 0))?;"
            )
            .count(),
            1,
            "the registration must carry the identical `#[cfg(feature = \"tokenizer\")]` gate as \
             the function definition; got:\n{code}"
        );
    }

    /// An ungated function's registration must stay exactly as it was: no `#[cfg(...)]` line
    /// introduced by the fix above.
    #[test]
    fn gen_module_init_leaves_ungated_function_registration_unchanged() {
        let config = make_config();
        let ungated = zero_arg_func("completion_cost", None);
        let api = crate::core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![ungated],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::BTreeMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let code = gen_module_init(
            "TestLib",
            &api,
            &config,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &[],
            &Default::default(),
            &[],
        );
        assert!(
            code.contains("module.define_module_function(\"completion_cost\", function!(completion_cost, 0))?;"),
            "must still register the ungated function; got:\n{code}"
        );
        assert!(
            !code.contains("#[cfg("),
            "an ungated function must not gain a cfg gate; got:\n{code}"
        );
    }

    #[test]
    fn needs_variadic_arity_detects_optional_params() {
        let required = ParamDef {
            name: "x".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
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
        };
        let optional = ParamDef {
            optional: true,
            ..required.clone()
        };
        assert!(
            !scan_args_defaults::needs_variadic_arity(std::slice::from_ref(&required)),
            "required-only: no variadic"
        );
        assert!(
            scan_args_defaults::needs_variadic_arity(&[optional]),
            "optional param: needs variadic"
        );
    }
}
