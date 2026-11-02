use crate::core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};

fn extract_owner(fields: &str) -> TypeDef {
    let directory = tempfile::tempdir().expect("public extractor Arc fixture");
    let source = directory.path().join("lib.rs");
    std::fs::write(
        &source,
        format!(
            r#"
use std::sync::Arc;
pub mod model {{
    #[derive(Clone)]
    pub enum Choice {{ Left, Right }}
}}
use model::Choice;
pub struct Owner {{ {fields} }}
"#
        ),
    )
    .unwrap();
    let api = crate::extract::extractor::extract(&[source.as_path()], "arc_fixture", "0.1.0", None)
        .expect("public extraction must accept qualified and imported Arc fields");
    assert_eq!(api.types.len(), 1);
    assert_eq!(api.enums.len(), 1);
    assert_eq!(api.enums[0].rust_path, "arc_fixture::model::Choice");
    assert!(api.unsupported_public_items.is_empty());
    assert!(api.unresolved_modules.is_empty());
    api.types.into_iter().find(|typ| typ.name == "Owner").unwrap()
}

fn assert_arc_field(field: &FieldDef, optional: bool, leaf_path: Option<&str>) {
    assert_eq!(field.ty, TypeRef::Named("Choice".into()));
    assert_eq!(field.core_wrapper, CoreWrapper::Arc);
    assert_eq!(field.optional, optional);
    assert_eq!(field.type_rust_path.as_deref(), leaf_path, "field {}", field.name);
    assert!(!field.is_boxed);
    assert!(!field.binding_excluded);
}

#[test]
fn qualified_arc_matches_imported_arc_leaf_metadata() {
    let owner = extract_owner("pub imported: Arc<Choice>, pub qualified: std::sync::Arc<Choice>,");
    assert_eq!(owner.fields.len(), 2);
    assert_arc_field(&owner.fields[0], false, None);
    assert_arc_field(&owner.fields[1], false, None);
}

#[test]
fn qualified_arc_preserves_the_crate_qualified_leaf_path() {
    let owner =
        extract_owner("pub imported: Arc<crate::model::Choice>, pub qualified: std::sync::Arc<crate::model::Choice>,");
    assert_eq!(owner.fields.len(), 2);
    for field in &owner.fields {
        assert_arc_field(field, false, Some("arc_fixture::model::Choice"));
    }
}

#[test]
fn optional_qualified_arc_preserves_leaf_and_wrapper_metadata() {
    let owner = extract_owner(
        "pub imported: Option<Arc<Choice>>, pub qualified: Option<std::sync::Arc<Choice>>,
         pub imported_path: Option<Arc<crate::model::Choice>>,
         pub qualified_path: Option<std::sync::Arc<crate::model::Choice>>,",
    );
    assert_eq!(owner.fields.len(), 4);
    for field in &owner.fields[..2] {
        assert_arc_field(field, true, None);
    }
    for field in &owner.fields[2..] {
        assert_arc_field(field, true, Some("arc_fixture::model::Choice"));
    }
}

fn assert_arc_lock_field(field: &FieldDef, optional: bool, leaf_path: Option<&str>) {
    assert_eq!(field.ty, TypeRef::Named("Choice".into()));
    assert_eq!(field.core_wrapper, CoreWrapper::ArcMutex);
    assert_eq!(field.optional, optional);
    assert_eq!(field.type_rust_path.as_deref(), leaf_path, "field {}", field.name);
    assert!(!field.is_boxed);
    assert!(!field.binding_excluded);
}

#[test]
fn imported_arc_with_qualified_locks_preserves_plain_leaf_metadata() {
    let owner = extract_owner(
        "pub mutex: Arc<std::sync::Mutex<Choice>>, pub rwlock: Arc<std::sync::RwLock<Choice>>,
         pub optional_mutex: Option<Arc<std::sync::Mutex<Choice>>>,
         pub optional_rwlock: Option<Arc<std::sync::RwLock<Choice>>>,",
    );
    assert_eq!(owner.fields.len(), 4);
    for (index, field) in owner.fields.iter().enumerate() {
        assert_arc_lock_field(field, index >= 2, None);
    }
}

#[test]
fn arc_locks_preserve_the_crate_qualified_leaf_path() {
    let owner = extract_owner(
        "pub mutex: Arc<std::sync::Mutex<crate::model::Choice>>,
         pub rwlock: Arc<std::sync::RwLock<crate::model::Choice>>,
         pub qualified_mutex: std::sync::Arc<std::sync::Mutex<crate::model::Choice>>,
         pub qualified_rwlock: std::sync::Arc<std::sync::RwLock<crate::model::Choice>>,
         pub optional_mutex: Option<Arc<std::sync::Mutex<crate::model::Choice>>>,
         pub optional_rwlock: Option<Arc<std::sync::RwLock<crate::model::Choice>>>,
         pub optional_qualified_mutex: Option<std::sync::Arc<std::sync::Mutex<crate::model::Choice>>>,
         pub optional_qualified_rwlock: Option<std::sync::Arc<std::sync::RwLock<crate::model::Choice>>>,",
    );
    assert_eq!(owner.fields.len(), 8);
    for (index, field) in owner.fields.iter().enumerate() {
        assert_arc_lock_field(field, index >= 4, Some("arc_fixture::model::Choice"));
    }
}

fn extract_stacked_owners() -> crate::core::ir::ApiSurface {
    let directory = tempfile::tempdir().expect("public extractor Arc stack fixture");
    let source = directory.path().join("lib.rs");
    std::fs::write(
        &source,
        r#"
use std::sync::Arc;
pub mod model {
    #[derive(Clone)]
    pub enum Choice { Left, Right }
}
use model::Choice;
#[derive(Clone)]
pub struct SingleOwner { pub choice: Arc<Choice> }
#[derive(Clone)]
pub struct NestedQualifiedOwner { pub choice: std::sync::Arc<std::sync::Arc<crate::model::Choice>> }
#[derive(Clone)]
pub struct NestedImportedOwner { pub choice: std::sync::Arc<Arc<crate::model::Choice>> }
"#,
    )
    .unwrap();
    let api = crate::extract::extractor::extract(&[source.as_path()], "arc_fixture", "0.1.0", None)
        .expect("public extraction must retain the Arc stack eligibility boundary");
    assert_eq!(api.types.len(), 3);
    assert_eq!(api.enums.len(), 1);
    assert_eq!(api.enums[0].rust_path, "arc_fixture::model::Choice");
    assert!(api.unsupported_public_items.is_empty());
    assert!(api.unresolved_modules.is_empty());
    api
}

#[test]
fn unrepresentable_arc_stacks_remain_ineligible_for_owner_conversions() {
    use crate::codegen::conversions::{convertible_types, core_to_binding_convertible_types};

    let api = extract_stacked_owners();
    let directions = [
        ("binding to core", convertible_types(&api)),
        ("core to binding", core_to_binding_convertible_types(&api, &[])),
    ];
    for (direction, eligible) in directions {
        assert!(
            eligible.contains("SingleOwner"),
            "single Arc positive control: {direction}"
        );
        for name in ["NestedQualifiedOwner", "NestedImportedOwner"] {
            assert!(
                !eligible.contains(name),
                "an unrepresentable double Arc must remain ineligible: {name}, {direction}"
            );
        }
        assert_eq!(eligible.len(), 1, "only the representable owner converts: {direction}");
    }
    for owner in &api.types {
        assert!(owner.is_clone);
        assert!(!owner.is_opaque);
        assert_eq!(owner.fields.len(), 1);
        let expected_path = (owner.name != "SingleOwner").then_some("std::sync::Arc");
        assert_arc_field(&owner.fields[0], false, expected_path);
    }
}
