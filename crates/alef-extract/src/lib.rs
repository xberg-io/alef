//! Rust source extraction for alef -- parses pub API into IR.

pub mod extractor;
mod type_resolver;

use alef_core::ir::{ApiSurface, FieldDef, TypeRef};

/// Metadata about a field on a struct type -- name and resolved type reference.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// Field name as it appears in the Rust source.
    pub name: String,
    /// Resolved type reference (e.g. `TypeRef::String`, `TypeRef::Primitive(...)`).
    pub ty: TypeRef,
    /// Whether the field is optional (`Option<T>`).
    pub optional: bool,
}

impl From<&FieldDef> for FieldInfo {
    fn from(f: &FieldDef) -> Self {
        Self {
            name: f.name.clone(),
            ty: f.ty.clone(),
            optional: f.optional,
        }
    }
}

/// Query the public fields of the struct returned by a named free function.
///
/// Returns `Some(fields)` when:
/// - `function_name` exists in `surface.functions`
/// - Its `return_type` is `TypeRef::Named(struct_name)`
/// - `struct_name` is found in `surface.types` with at least one public field
///
/// Returns `None` when the return type is a primitive, `String`, `Vec`, unit, or
/// any type not directly listed in `surface.types` (e.g. after newtype resolution).
///
/// The Rust codegen agent can use this to validate `result.field` style assertions
/// at generation time -- if the field does not appear in the returned list the
/// assertion is invalid.
pub fn return_type_fields(surface: &ApiSurface, function_name: &str) -> Option<Vec<FieldInfo>> {
    let func = surface.functions.iter().find(|f| f.name == function_name)?;
    let type_name = match &func.return_type {
        TypeRef::Named(name) => name.as_str(),
        _ => return None,
    };
    let type_def = surface.types.iter().find(|t| t.name == type_name)?;
    if type_def.fields.is_empty() {
        return None;
    }
    Some(type_def.fields.iter().map(FieldInfo::from).collect())
}

/// Validation result for a `(module_path, function_name)` pair from `alef.toml`.
#[derive(Debug)]
pub enum ExportValidation {
    /// The function is exported at the claimed path -- no action needed.
    Ok,
    /// The function name exists in the surface but not at the declared module path.
    ///
    /// The `actual_paths` list contains every `rust_path` found for this function name.
    WrongPath {
        function: String,
        declared_module: String,
        actual_paths: Vec<String>,
    },
    /// No function with this name was found in the surface at all.
    NotFound { function: String },
}

/// Validate that a `(module_path, function_name)` pair from `alef.toml` resolves
/// to a function whose `rust_path` matches `"{module_path}::{function_name}"`.
///
/// This catches two bug classes:
/// - **C1** -- the declared `module` does not match the actual export path (e.g., the
///   function is defined at `kreuzberg::rendering::render_page` but `alef.toml` uses
///   `module = "kreuzberg"`, and `render_page` is NOT re-exported at the crate root).
/// - **C2** -- multiple definitions exist; the one with the wrong `rust_path` was kept
///   by the dedup pass, causing the wrong signature to be used.
///
/// Pass `module_path = "kreuzberg"` and `function_name = "render_page"` to check
/// that `kreuzberg::render_page` exists in the surface.
pub fn validate_call_export(surface: &ApiSurface, module_path: &str, function_name: &str) -> ExportValidation {
    let expected_rust_path = format!("{module_path}::{function_name}");

    let mut all_defs: Vec<String> = surface
        .functions
        .iter()
        .filter(|f| f.name == function_name)
        .map(|f| f.rust_path.clone())
        .collect();

    // Also accept method-on-type references: e.g. `function = "chat"` where `chat`
    // is a method on a public type like `LlmClient` (trait) or `DefaultClient`. The
    // e2e codegen layer already handles method-style call sites; the validator was
    // previously too strict and rejected these legitimate entry points.
    let mut method_match_found = false;
    for type_def in &surface.types {
        for method in &type_def.methods {
            if method.name == function_name {
                method_match_found = true;
                all_defs.push(format!("{}::{}", type_def.rust_path, method.name));
            }
        }
    }

    if all_defs.is_empty() {
        return ExportValidation::NotFound {
            function: function_name.to_string(),
        };
    }

    if all_defs.iter().any(|p| p == &expected_rust_path) {
        return ExportValidation::Ok;
    }

    // Lenient policy for methods: if any method anywhere in the surface matches the
    // function name, accept it. Codegen handles method-style dispatch correctly and
    // the strict module-path check is only meaningful for free functions.
    if method_match_found {
        return ExportValidation::Ok;
    }

    ExportValidation::WrongPath {
        function: function_name.to_string(),
        declared_module: module_path.to_string(),
        actual_paths: all_defs,
    }
}

#[cfg(test)]
mod tests {
    use alef_core::ir::{ApiSurface, FieldDef, MethodDef, TypeDef, TypeRef};

    use super::{ExportValidation, return_type_fields, validate_call_export};

    fn make_method(name: &str, return_type: TypeRef) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }
    }

    fn make_fn(name: &str, rust_path: &str, return_type: TypeRef) -> alef_core::ir::FunctionDef {
        alef_core::ir::FunctionDef {
            name: name.to_string(),
            rust_path: rust_path.to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }
    }

    fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("my_crate::{name}"),
            original_rust_path: String::new(),
            fields,
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            doc: String::new(),
            cfg: None,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
        }
    }

    fn make_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef_core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    fn empty_surface() -> ApiSurface {
        ApiSurface {
            crate_name: "my_crate".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn validate_ok_when_exported_at_declared_module() {
        let mut surface = empty_surface();
        surface
            .functions
            .push(make_fn("render_page", "my_crate::render_page", TypeRef::String));
        assert!(matches!(
            validate_call_export(&surface, "my_crate", "render_page"),
            ExportValidation::Ok
        ));
    }

    #[test]
    fn validate_wrong_path_when_not_re_exported_at_root() {
        let mut surface = empty_surface();
        surface.functions.push(make_fn(
            "render_page",
            "my_crate::rendering::render_page",
            TypeRef::String,
        ));

        match validate_call_export(&surface, "my_crate", "render_page") {
            ExportValidation::WrongPath {
                function,
                declared_module,
                actual_paths,
            } => {
                assert_eq!(function, "render_page");
                assert_eq!(declared_module, "my_crate");
                assert!(actual_paths.contains(&"my_crate::rendering::render_page".to_string()));
            }
            other => panic!("expected WrongPath, got {other:?}"),
        }
    }

    #[test]
    fn validate_not_found_when_function_absent() {
        let surface = empty_surface();
        match validate_call_export(&surface, "my_crate", "missing_fn") {
            ExportValidation::NotFound { function } => assert_eq!(function, "missing_fn"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn validate_wrong_path_when_both_definitions_are_deep() {
        let mut surface = empty_surface();
        surface.functions.push(make_fn(
            "clean_extracted_text",
            "my_crate::utils::quality::clean_extracted_text",
            TypeRef::String,
        ));
        surface.functions.push(make_fn(
            "clean_extracted_text",
            "my_crate::text::quality::clean_extracted_text",
            TypeRef::String,
        ));
        match validate_call_export(&surface, "my_crate", "clean_extracted_text") {
            ExportValidation::WrongPath { actual_paths, .. } => assert_eq!(actual_paths.len(), 2),
            other => panic!("expected WrongPath, got {other:?}"),
        }
    }

    #[test]
    fn validate_ok_when_one_definition_is_at_root() {
        let mut surface = empty_surface();
        surface.functions.push(make_fn(
            "normalize_whitespace",
            "my_crate::normalize_whitespace",
            TypeRef::String,
        ));
        assert!(matches!(
            validate_call_export(&surface, "my_crate", "normalize_whitespace"),
            ExportValidation::Ok
        ));
    }

    #[test]
    fn return_type_fields_returns_struct_fields() {
        let mut surface = empty_surface();
        surface.functions.push(make_fn(
            "extract_doc",
            "my_crate::extract_doc",
            TypeRef::Named("ExtractionResult".into()),
        ));
        surface.types.push(make_type(
            "ExtractionResult",
            vec![
                make_field("content", TypeRef::String),
                make_field("mime_type", TypeRef::String),
            ],
        ));
        let fields = return_type_fields(&surface, "extract_doc").expect("should return fields");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "content");
        assert_eq!(fields[1].name, "mime_type");
    }

    #[test]
    fn return_type_fields_none_for_primitive_return() {
        let mut surface = empty_surface();
        surface.functions.push(make_fn(
            "count_words",
            "my_crate::count_words",
            TypeRef::Primitive(alef_core::ir::PrimitiveType::U64),
        ));
        assert!(return_type_fields(&surface, "count_words").is_none());
    }

    #[test]
    fn return_type_fields_none_for_string_return() {
        let mut surface = empty_surface();
        surface
            .functions
            .push(make_fn("get_name", "my_crate::get_name", TypeRef::String));
        assert!(return_type_fields(&surface, "get_name").is_none());
    }

    #[test]
    fn return_type_fields_none_for_unknown_function() {
        let surface = empty_surface();
        assert!(return_type_fields(&surface, "nonexistent").is_none());
    }

    #[test]
    fn return_type_fields_none_for_opaque_type() {
        let mut surface = empty_surface();
        surface.functions.push(make_fn(
            "make_handle",
            "my_crate::make_handle",
            TypeRef::Named("Handle".into()),
        ));
        surface.types.push(make_type("Handle", vec![]));
        assert!(return_type_fields(&surface, "make_handle").is_none());
    }

    fn extract_from_tmp(lib_rs_source: &str) -> ApiSurface {
        let dir = tempfile::tempdir().expect("tempdir");
        let lib_rs = dir.path().join("lib.rs");
        std::fs::write(&lib_rs, lib_rs_source).expect("write lib.rs");
        crate::extractor::extract(&[lib_rs.as_path()], "kreuzberg", "0.0.0", None).expect("extract failed")
    }

    #[test]
    fn e2e_c1_deep_path_caught_by_validate() {
        let surface = extract_from_tmp(
            r#"
            pub mod rendering {
                pub fn render_page(page: u32) -> String { todo!() }
            }
        "#,
        );
        match validate_call_export(&surface, "kreuzberg", "render_page") {
            ExportValidation::WrongPath { actual_paths, .. } => {
                assert!(actual_paths.iter().any(|p| p.contains("rendering")));
            }
            other => panic!("expected WrongPath, got {other:?}"),
        }
    }

    #[test]
    fn e2e_c1_glob_reexport_resolves_ok() {
        let surface = extract_from_tmp(
            r#"
            pub use rendering::*;
            pub mod rendering {
                pub fn render_page(page: u32) -> String { todo!() }
            }
        "#,
        );
        assert!(matches!(
            validate_call_export(&surface, "kreuzberg", "render_page"),
            ExportValidation::Ok
        ));
    }

    #[test]
    fn validate_ok_when_method_matches() {
        let mut surface = empty_surface();
        let mut client = make_type("DefaultClient", vec![]);
        client.methods.push(make_method("chat", TypeRef::String));
        surface.types.push(client);

        assert!(matches!(
            validate_call_export(&surface, "my_crate", "chat"),
            ExportValidation::Ok
        ));
    }

    #[test]
    fn validate_not_found_when_neither_function_nor_method_matches() {
        let mut surface = empty_surface();
        let mut client = make_type("DefaultClient", vec![]);
        client.methods.push(make_method("complete", TypeRef::String));
        surface.types.push(client);

        match validate_call_export(&surface, "my_crate", "chat") {
            ExportValidation::NotFound { function } => assert_eq!(function, "chat"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn e2e_c1_named_reexport_resolves_ok() {
        let surface = extract_from_tmp(
            r#"
            pub use chunking::core::chunk_text;
            pub mod chunking {
                pub mod core {
                    pub async fn chunk_text(text: String) -> Vec<String> { todo!() }
                }
            }
        "#,
        );
        assert!(matches!(
            validate_call_export(&surface, "kreuzberg", "chunk_text"),
            ExportValidation::Ok
        ));
    }
}
