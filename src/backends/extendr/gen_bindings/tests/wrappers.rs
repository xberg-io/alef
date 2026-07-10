use super::*;

#[test]
fn generates_extendr_module_registration() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert_eq!(files.len(), 1);
    let content = &files[0].content;
    assert!(content.contains("extendr_module!"), "must emit extendr_module! macro");
    assert!(content.contains("mod testlib"), "module name must match r_package_name");
}

#[test]
fn generates_extendr_function_attribute() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;
    assert!(
        content.contains("#[extendr]"),
        "functions must carry #[extendr] attribute"
    );
    assert!(content.contains("fn process"), "process function must be generated");
}

#[test]
fn r_package_name_drives_output_path() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert!(
        files[0].path.to_string_lossy().ends_with("lib.rs"),
        "output file must be lib.rs"
    );
}

#[test]
fn generate_public_api_uses_r_package_name() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with("testlib.R")),
        "public API file must include {{package_name}}.R, got {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("extendr-wrappers.R")),
        "public API file must include extendr-wrappers.R, got {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.ends_with("NAMESPACE")),
        "public API file must include NAMESPACE, got {paths:?}"
    );
}

#[test]
fn extendr_wrappers_emits_function_call_binding() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    assert!(
        wrappers.content.contains("process <- function()"),
        "free function must produce a wrapper: {}",
        wrappers.content
    );
    assert!(
        wrappers.content.contains(".Call(\"wrap__process\""),
        "wrapper must invoke the wrap__ symbol: {}",
        wrappers.content
    );
    assert!(
        wrappers.content.contains("Config <- new.env(parent = emptyenv())"),
        "non-trait class must be registered as an env: {}",
        wrappers.content
    );
}

#[test]
fn extendr_wrappers_emits_roxygen_doc_block_for_free_functions() {
    // line + description from the Rust doc comment and emit `@param` /
    // `@return` lines from the IR's type information.
    let backend = ExtendrBackend;
    let config = make_config();
    let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "extract_bytes".to_string(),
                rust_path: "test_lib::extract_bytes".to_string(),
                original_rust_path: String::new(),
                params: vec![
                    ParamDef {
                        name: "bytes".to_string(),
                        ty: TypeRef::Bytes,
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
                    },
                    ParamDef {
                        name: "mime_type".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::String)),
                        optional: true,
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
                    },
                    ParamDef {
                        name: "config".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::Named("ExtractionConfig".to_string()))),
                        optional: true,
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
                    },
                ],
                return_type: TypeRef::Named("ExtractionResult".to_string()),
                is_async: false,
                error_type: None,
                doc: "Extract text from raw bytes.\n\nDetect the MIME type of the input bytes\nand run the appropriate extractor.".to_string(),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;

    assert!(
        content.contains("#' Extract text from raw bytes"),
        "title line derived from Rust doc comment must be emitted:\n{content}"
    );
    assert!(
        content.contains("#' Detect the MIME type of the input bytes"),
        "description from Rust doc comment must be emitted:\n{content}"
    );
    assert!(
        content.contains("#' @param bytes Raw vector of bytes."),
        "@param for bytes must describe the type:\n{content}"
    );
    assert!(
        content.contains("#' @param mime_type Optional character string."),
        "@param for optional string must include `Optional` qualifier:\n{content}"
    );
    assert!(
        content.contains("#' @param config Optional ExtractionConfig object"),
        "@param for named optional type must reference the named type:\n{content}"
    );
    assert!(
        content.contains("extract_bytes <- function(bytes, mime_type = NULL, config = NULL)"),
        "R wrapper must allow README-style omitted optional config/mime args:\n{content}"
    );
    assert!(
        content.contains("#' @return ExtractionResult object"),
        "@return must describe the return type:\n{content}"
    );
    assert!(
        content.contains("#' @export"),
        "@export tag must be preserved:\n{content}"
    );
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("#' @param ") {
            let mut parts = rest.splitn(2, ' ');
            let _name = parts.next();
            let description = parts.next().unwrap_or("").trim();
            assert!(
                !description.is_empty(),
                "@param line must include a description, got: {line:?}\nfull content:\n{content}"
            );
        }
    }
}

#[test]
fn extendr_wrappers_default_required_config_objects_in_r() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "test_lib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,

            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "extract_bytes".to_string(),
            rust_path: "test_lib::extract_bytes".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "bytes".to_string(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Named("ExtractionConfig".to_string()),
                    ..Default::default()
                },
            ],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;

    assert!(
        content.contains("extract_bytes <- function(bytes, config = ExtractionConfig$default())"),
        "R wrapper must synthesize default objects instead of advertising NULL for required config:\n{content}"
    );
}

#[test]
fn extendr_wrappers_emits_placeholder_title_when_doc_is_empty() {
    // is omitted, @param/@return lines are still emitted.
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    assert!(
        content.contains("#' process"),
        "fallback title (function name) must be emitted when doc is empty:\n{content}"
    );
    assert!(
        content.contains("#' @return Character string."),
        "@return must be emitted even without a doc comment:\n{content}"
    );
}

#[test]
fn namespace_exports_functions_and_classes() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_surface();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let namespace = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
        .expect("NAMESPACE must be generated");
    assert!(
        namespace.content.contains("export(process)"),
        "free function must be exported: {}",
        namespace.content
    );
    assert!(
        namespace.content.contains("export(Config)"),
        "class env must be exported: {}",
        namespace.content
    );
    assert!(
        namespace.content.contains("S3method(\"$\", Config)"),
        "S3 dispatch operator must be registered: {}",
        namespace.content
    );
    assert!(
        namespace.content.contains("useDynLib(testlib, .registration = TRUE)"),
        "NAMESPACE must contain bare useDynLib directive: {}",
        namespace.content
    );
    assert!(
        !namespace.content.contains("#' @useDynLib"),
        "NAMESPACE must not contain roxygen2 useDynLib form: {}",
        namespace.content
    );
}

fn make_instance_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        sanitized: false,
        receiver: Some(ReceiverKind::Ref),
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

fn make_api_with_instance_method() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "HeaderMetadata".to_string(),
            rust_path: "test_lib::HeaderMetadata".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("level", TypeRef::Primitive(PrimitiveType::U32), false)],
            methods: vec![make_instance_method("is_valid")],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
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
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn extendr_wrappers_emits_s3_generic_and_method_for_instance_methods() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_with_instance_method();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    assert!(
        content.contains("is_valid <- function(x, ...) UseMethod(\"is_valid\")"),
        "S3 generic must be emitted for instance methods:\n{content}"
    );
    assert!(
        content.contains("is_valid.HeaderMetadata <- function(x, ...) x$is_valid(...)"),
        "S3 class method must forward to the env-class binding:\n{content}"
    );
}

#[test]
fn extendr_wrappers_skips_s3_wrappers_for_static_methods() {
    let backend = ExtendrBackend;
    let config = make_config();
    let mut api = make_api_with_instance_method();
    let static_method = MethodDef {
        is_static: true,
        ..make_instance_method("default")
    };
    api.types[0].methods.push(static_method);
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    assert!(
        !content.contains("default <- function(x, ...) UseMethod"),
        "must not emit S3 generic for static methods:\n{content}"
    );
    assert!(
        !content.contains("default.HeaderMetadata <-"),
        "must not emit S3 class method for static methods:\n{content}"
    );
}

#[test]
fn extendr_wrappers_emits_one_generic_per_unique_method_name() {
    let backend = ExtendrBackend;
    let config = make_config();
    let mut api = make_api_with_instance_method();
    let second_type = TypeDef {
        name: "LinkMetadata".to_string(),
        rust_path: "test_lib::LinkMetadata".to_string(),
        methods: vec![make_instance_method("is_valid")],
        ..api.types[0].clone()
    };
    api.types.push(second_type);
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    let generic_count = content.matches("is_valid <- function(x, ...) UseMethod").count();
    assert_eq!(
        generic_count, 1,
        "exactly one S3 generic per unique method name, got {generic_count}:\n{content}"
    );
    assert!(
        content.contains("is_valid.HeaderMetadata <- function(x, ...) x$is_valid(...)"),
        "S3 method for HeaderMetadata must be emitted:\n{content}"
    );
    assert!(
        content.contains("is_valid.LinkMetadata <- function(x, ...) x$is_valid(...)"),
        "S3 method for LinkMetadata must be emitted:\n{content}"
    );
}

#[test]
fn namespace_exports_s3_generics_and_methods_for_instance_methods() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = make_api_with_instance_method();
    let files = backend.generate_public_api(&api, &config).unwrap();
    let namespace = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
        .expect("NAMESPACE must be generated");
    let content = &namespace.content;
    assert!(
        content.contains("export(is_valid)"),
        "S3 generic must be exported by name: {content}"
    );
    assert!(
        content.contains("S3method(is_valid, HeaderMetadata)"),
        "S3 class method must be registered: {content}"
    );
}

#[test]
fn extendr_wrappers_emits_roxygen_class_block_with_field_lines_for_struct() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ServerConfig".to_string(),
            rust_path: "test_lib::ServerConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                FieldDef {
                    doc: "TCP port the server binds to.".to_string(),
                    ..make_field("port", TypeRef::Primitive(PrimitiveType::U32), false)
                },
                FieldDef {
                    doc: "Maximum number of in-flight requests.\n\nApplies to all listener sockets.".to_string(),
                    ..make_field("max_connections", TypeRef::Primitive(PrimitiveType::U32), false)
                },
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: "Server configuration.\n\nHolds tunable parameters for the network listener.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,

            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    assert!(
        content.contains("#' Server configuration"),
        "class title from struct doc must be emitted:\n{content}"
    );
    assert!(
        content.contains("#' Holds tunable parameters for the network listener."),
        "class description must be emitted:\n{content}"
    );
    assert!(
        content.contains("#' @field port TCP port the server binds to."),
        "@field with single-line doc must be emitted:\n{content}"
    );
    assert!(
        content.contains("#' @field max_connections Maximum number of in-flight requests."),
        "@field must collapse multi-paragraph doc to the first paragraph:\n{content}"
    );
    assert!(
        content.contains("ServerConfig <- new.env(parent = emptyenv())"),
        "class env definition must still be emitted:\n{content}"
    );
}

#[test]
fn extendr_wrappers_emits_param_doc_from_arguments_section_for_function() {
    // type-based description on the `#' @param` line, and the `# Returns`
    // section must drive the `#' @return` line.
    let backend = ExtendrBackend;
    let config = make_config();
    let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "render".to_string(),
                rust_path: "test_lib::render".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "template".to_string(),
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
                error_type: None,
                doc: "Render a template to a string.\n\n# Arguments\n\n* `template` - Mustache template source.\n\n# Returns\n\nThe fully interpolated output.".to_string(),
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
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    assert!(
        content.contains("#' @param template Mustache template source."),
        "@param must use description from `# Arguments` bullet:\n{content}"
    );
    assert!(
        content.contains("#' @return The fully interpolated output."),
        "@return must use prose from `# Returns` section:\n{content}"
    );
    assert!(
        !content.contains("#' # Arguments"),
        "raw `# Arguments` heading must not appear in roxygen output:\n{content}"
    );
    assert!(
        !content.contains("#' # Returns"),
        "raw `# Returns` heading must not appear in roxygen output:\n{content}"
    );
}

#[test]
fn extendr_wrappers_emits_roxygen_block_for_flat_data_enum_with_variant_fields() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![EnumDef {
            name: "Payload".to_string(),
            rust_path: "test_lib::Payload".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Text".to_string(),
                    fields: vec![make_field("inner", TypeRef::String, false)],
                    doc: "UTF-8 encoded text payload.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    is_tuple: true,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
                EnumVariant {
                    name: "Binary".to_string(),
                    fields: vec![make_field("inner", TypeRef::String, false)],
                    doc: "Base64-encoded binary payload.".to_string(),
                    is_default: false,
                    serde_rename: None,
                    is_tuple: true,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                    cfg: None,
                    version: Default::default(),
                },
            ],
            methods: vec![],
            doc: "Wire payload variants.".to_string(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            has_default: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    let content = &wrappers.content;
    assert!(
        content.contains("#' Wire payload variants"),
        "enum title from Rust doc must be emitted:\n{content}"
    );
    assert!(
        content.contains("#' @field Text UTF-8 encoded text payload."),
        "@field per variant must carry the variant's doc:\n{content}"
    );
    assert!(
        content.contains("#' @field Binary Base64-encoded binary payload."),
        "every variant must produce a `@field` line:\n{content}"
    );
    assert!(
        content.contains("Payload <- new.env(parent = emptyenv())"),
        "enum class env must still be emitted:\n{content}"
    );
}

#[test]
fn extendr_module_registration_registers_complementary_cfg_functions_once() {
    let backend = ExtendrBackend;
    let config = make_config();
    let mut api = make_api_surface();

    let paired_fn = FunctionDef {
        name: "embed_texts_async".to_string(),
        rust_path: "test_lib::embed_texts_async".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: Some("feature = \"embeddings\"".to_string()),
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let stub_fn = FunctionDef {
        name: "embed_texts_async".to_string(),
        rust_path: "test_lib::embed_texts_async".to_string(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: None,
        doc: String::new(),
        cfg: Some("not(feature = \"embeddings\")".to_string()),
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    api.functions.push(paired_fn);
    api.functions.push(stub_fn);

    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    let module_block_start = content
        .find("extendr_module!")
        .expect("must emit extendr_module! macro");
    let module_block = &content[module_block_start..];

    assert!(
        module_block.contains("    fn embed_texts_async;\n"),
        "complementary cfg functions must register once as an always-present entry, got:\n{module_block}"
    );
    assert!(
        !module_block.contains("#[cfg("),
        "extendr_module! entries must not carry cfg attributes, got:\n{module_block}"
    );

    assert!(
        module_block.contains("    fn process;\n"),
        "cfg-less function must register without a #[cfg(...)] prefix:\n{module_block}"
    );
}

#[test]
fn extendr_codegen_keeps_cfg_fields_enabled_by_explicit_r_features() {
    let backend = ExtendrBackend;
    let config = make_r_config_with_features(&["url-ingestion"], false);
    let mut api = make_api_surface();

    let mut crawl_field = make_field("crawl", TypeRef::Named("CrawlConfig".to_string()), false);
    crawl_field.cfg = Some("feature = \"url-ingestion\"".to_string());

    api.types = vec![
        TypeDef {
            name: "UrlExtractionConfig".to_string(),
            rust_path: "test_lib::UrlExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![crawl_field],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        },
        TypeDef {
            name: "CrawlConfig".to_string(),
            rust_path: "test_lib::CrawlConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("max_depth", TypeRef::Primitive(PrimitiveType::Usize), false)],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        },
    ];

    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub crawl: CrawlConfig"),
        "cfg field enabled by the R feature set must remain in the binding struct:\n{content}"
    );
    assert!(
        content.contains("crawl: val.crawl.into()"),
        "enabled cfg field must participate in core conversion instead of being defaulted away:\n{content}"
    );
}

fn make_r_config_with_features(features: &[&str], default_features: bool) -> ResolvedCrateConfig {
    let features = features
        .iter()
        .map(|feature| format!("\"{feature}\""))
        .collect::<Vec<_>>()
        .join(", ");
    resolved_one(&format!(
        r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"
features = [{features}]
default_features = {default_features}
"#
    ))
}

#[test]
fn r_public_api_omits_from_json_for_unregistered_dto_roots() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "SearchRequest".to_string(),
            rust_path: "test_lib::SearchRequest".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field(
                "modes",
                TypeRef::Vec(Box::new(TypeRef::Named("SearchMode".to_string()))),
                false,
            )],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
            name: "search".to_string(),
            rust_path: "test_lib::search".to_string(),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "request".to_string(),
                ty: TypeRef::Named("SearchRequest".to_string()),
                ..Default::default()
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: None,
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
        enums: vec![EnumDef {
            name: "SearchMode".to_string(),
            rust_path: "test_lib::SearchMode".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Fast".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: true,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            }],
            methods: vec![],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            has_default: true,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
            version: Default::default(),
        }],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let binding_files = backend.generate_bindings(&api, &config).unwrap();
    let module_block = binding_files[0]
        .content
        .split("extendr_module!")
        .nth(1)
        .expect("extendr module must be generated");
    assert!(
        !module_block.contains("impl SearchRequest;"),
        "SearchRequest is not registered because Vec<Enum> fields are not extendr-native:\n{module_block}"
    );

    let files = backend.generate_public_api(&api, &config).unwrap();
    let wrappers = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
        .expect("extendr-wrappers.R must be generated");
    assert!(
        !wrappers.content.contains("SearchRequest$from_json"),
        "R wrappers must not expose from_json for classes absent from extendr_module!:\n{}",
        wrappers.content
    );
    let namespace = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
        .expect("NAMESPACE must be generated");
    assert!(
        !namespace.content.contains("export(SearchRequest)"),
        "NAMESPACE must not export unregistered classes:\n{}",
        namespace.content
    );
}

#[test]
fn extendr_json_bridged_function_with_named_return_and_optional_named_params() {
    let backend = ExtendrBackend;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "ExtractionResult".to_string(),
                rust_path: "test_lib::ExtractionResult".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("text", TypeRef::String, false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
            TypeDef {
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
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
        ],
        functions: vec![FunctionDef {
            name: "extract_with_config".to_string(),
            rust_path: "test_lib::extract_with_config".to_string(),
            original_rust_path: String::new(),
            params: vec![
                ParamDef {
                    name: "bytes".to_string(),
                    ty: TypeRef::Bytes,
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
                },
                ParamDef {
                    name: "config".to_string(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("ExtractionConfig".to_string()))),
                    optional: true,
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
                },
            ],
            return_type: TypeRef::Named("ExtractionResult".to_string()),
            is_async: false,
            error_type: None,
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

    let files = backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    // The function must emit an #[extendr] wrapper for JSON-bridged dispatch
    assert!(
        content.contains("fn extract_with_config"),
        "extendr function wrapper must be emitted:\n{content}"
    );

    assert!(
        content.contains("config: Option<String>"),
        "optional Named param must use JSON bridging when return is Named struct requiring JSON:\n{content}"
    );

    assert!(
        content.contains("config") && content.contains("serde_json"),
        "preamble must deserialize JSON for optional config param:\n{content}"
    );
}
