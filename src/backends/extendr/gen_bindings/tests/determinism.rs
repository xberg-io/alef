//! Regression coverage for task #132 (see the analogous rustler test at
//! `backends::rustler::gen_bindings::tests::determinism` for the full writeup). extendr's
//! `generate_bindings` concatenates every type/enum conversion and every `#[extendr]` wrapper
//! into a single Rust source file, the same shape that let rustler's `lib.rs` leak `ApiSurface`
//! Vec ordering into emitted bytes. This test checks whether extendr shares that pattern.

use super::{ExtendrBackend, make_config, make_field};
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, ErrorDef, ErrorVariant, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef,
    TypeRef, VersionAnnotation,
};

fn payload_type(name: &str, field: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        fields: vec![make_field(field, TypeRef::Primitive(PrimitiveType::U32), false)],
        has_serde: true,
        ..Default::default()
    }
}

fn flat_data_enum(name: &str, tag: Option<&str>, variants: &[(&str, &str)]) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        variants: variants
            .iter()
            .map(|(variant_name, field_type)| EnumVariant {
                name: variant_name.to_string(),
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::Named(field_type.to_string()),
                    ..Default::default()
                }],
                is_tuple: true,
                ..Default::default()
            })
            .collect(),
        serde_tag: tag.map(str::to_string),
        has_serde: true,
        ..Default::default()
    }
}

fn free_function(name: &str, param_type: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        params: vec![ParamDef {
            name: "input".to_string(),
            ty: TypeRef::Named(param_type.to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        ..Default::default()
    }
}

fn error_def(name: &str, variants: &[&str]) -> ErrorDef {
    ErrorDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: variants
            .iter()
            .map(|variant| ErrorVariant {
                name: (*variant).to_string(),
                message_template: Some(format!("{variant} failed")),
                ..Default::default()
            })
            .collect(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: VersionAnnotation::default(),
    }
}

/// `functions` and `errors` are populated too, not just `types`/`enums`: a prior fix here sorted
/// only the type/enum emission loops, and a test that reversed only `types`/`enums` passed without
/// ever exercising the still-broken `api.functions`/`api.errors` order. See task #132 follow-up. ~keep
fn determinism_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            payload_type("PdfMetadata", "pages"),
            payload_type("DocxMetadata", "words"),
            payload_type("NodeMetadata", "depth"),
            payload_type("LeafMetadata", "value"),
        ],
        functions: vec![
            free_function("summarize_pdf", "PdfMetadata"),
            free_function("summarize_docx", "DocxMetadata"),
            free_function("summarize_node", "NodeMetadata"),
        ],
        enums: vec![
            flat_data_enum(
                "FormatMetadata",
                Some("format_type"),
                &[("Pdf", "PdfMetadata"), ("Docx", "DocxMetadata")],
            ),
            flat_data_enum(
                "NodeKind",
                None,
                &[("Branch", "NodeMetadata"), ("Leaf", "LeafMetadata")],
            ),
        ],
        errors: vec![
            error_def("ParseError", &["Malformed", "Truncated"]),
            error_def("IoError", &["NotFound", "Denied"]),
        ],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn generated_files_sorted(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<(String, String)> {
    let backend = ExtendrBackend;
    let bindings: Vec<GeneratedFile> = backend.generate_bindings(api, config).expect("generate_bindings");
    let public_api: Vec<GeneratedFile> = backend.generate_public_api(api, config).expect("generate_public_api");

    let mut files: Vec<(String, String)> = bindings
        .into_iter()
        .chain(public_api)
        .map(|file| (file.path.to_string_lossy().into_owned(), file.content))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn extendr_generation_is_byte_identical_across_repeated_runs() {
    let config = make_config();
    let api = determinism_api();

    let run1 = generated_files_sorted(&api, &config);
    let run2 = generated_files_sorted(&api, &config);

    assert_eq!(
        run1, run2,
        "generating twice from the same IR in one process must be byte-identical"
    );
}

#[test]
fn extendr_generation_is_invariant_to_ir_collection_order() {
    let config = make_config();
    let forward = determinism_api();

    let mut reversed = forward.clone();
    reversed.types.reverse();
    reversed.enums.reverse();
    reversed.functions.reverse();
    reversed.errors.reverse();

    let forward_files = generated_files_sorted(&forward, &config);
    let reversed_files = generated_files_sorted(&reversed, &config);

    assert_eq!(
        forward_files, reversed_files,
        "reversing api.types/api.enums/api.functions/api.errors must not change any generated \
         file's content; a diff here means extendr codegen leaks Vec ordering into emitted text"
    );
}
