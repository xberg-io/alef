use super::*;

#[test]
fn test_extract_binding_excluded_fields() {
    let source = r#"
        pub struct Config {
            pub visible: String,
            #[serde(skip)]
            pub serde_skipped_visible: String,
            #[doc(hidden)]
            pub doc_hidden: String,
            #[cfg_attr(alef, alef(skip))]
            pub alef_skipped: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub still_visible: Option<String>,
        }
    "#;

    let surface = extract_from_source(source);
    let config = &surface.types[0];

    let visible = config.fields.iter().find(|field| field.name == "visible").unwrap();
    assert!(!visible.binding_excluded);

    let serde_skipped_visible = config
        .fields
        .iter()
        .find(|field| field.name == "serde_skipped_visible")
        .unwrap();
    assert!(!serde_skipped_visible.binding_excluded);

    let doc_hidden = config.fields.iter().find(|field| field.name == "doc_hidden").unwrap();
    assert!(doc_hidden.binding_excluded);
    assert_eq!(doc_hidden.binding_exclusion_reason.as_deref(), Some("doc(hidden)"));

    let alef_skipped = config.fields.iter().find(|field| field.name == "alef_skipped").unwrap();
    assert!(alef_skipped.binding_excluded);
    assert_eq!(alef_skipped.binding_exclusion_reason.as_deref(), Some("alef(skip)"));

    let still_visible = config
        .fields
        .iter()
        .find(|field| field.name == "still_visible")
        .unwrap();
    assert!(!still_visible.binding_excluded);
}

#[test]
fn test_struct_with_non_pub_field_sets_has_private_fields() {
    // A `pub(crate)` field is filtered out of the binding surface, but its existence
    // means the core struct cannot be built with struct-literal syntax from a foreign
    // crate. `has_private_fields` records that so the conversion generator picks a
    // non-literal construction strategy.
    let source = r#"
        #[derive(Default)]
        pub struct ResultLike {
            pub content: String,
            pub(crate) internal: Option<String>,
        }
    "#;

    let surface = extract_from_source(source);
    let ty = &surface.types[0];

    assert!(
        ty.has_private_fields,
        "a struct with a pub(crate) field must set has_private_fields"
    );
    assert!(
        ty.fields.iter().all(|f| f.name != "internal"),
        "the non-pub field must not appear in the binding surface"
    );
}

#[test]
fn test_struct_all_public_fields_has_no_private_fields() {
    let source = r#"
        pub struct AllPublic {
            pub a: String,
            pub b: u32,
        }
    "#;

    let surface = extract_from_source(source);
    assert!(
        !surface.types[0].has_private_fields,
        "an all-pub struct must not set has_private_fields"
    );
}

#[test]
fn test_opaque_struct() {
    let source = r#"
        pub struct Handle {
            inner: u64,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    assert!(surface.types[0].is_opaque);
    assert!(surface.types[0].fields.is_empty());
}

#[test]
fn test_pub_type_alias_with_alef_skip_is_binding_excluded() {
    // `#[cfg_attr(alef, alef(skip))]` on a type alias should mark it as
    // binding_excluded so downstream backends skip it.
    let source = r#"
        #[cfg_attr(alef, alef(skip))]
        pub type StringBufferPool = Pool<String>;
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 1);
    let alias = &surface.types[0];
    assert_eq!(alias.name, "StringBufferPool");
    assert!(alias.binding_excluded);
    assert_eq!(alias.binding_exclusion_reason.as_deref(), Some("alef(skip)"));
}

#[test]
fn test_extract_binding_excluded_struct() {
    let source = r#"
        #[cfg_attr(alef, alef(skip))]
        pub struct InternalConfig {
            pub secret: String,
        }
        pub struct PublicConfig {
            pub name: String,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 2);

    let internal = surface.types.iter().find(|t| t.name == "InternalConfig").unwrap();
    assert!(
        internal.binding_excluded,
        "InternalConfig should have binding_excluded=true"
    );
    assert_eq!(
        internal.binding_exclusion_reason.as_deref(),
        Some("alef(skip)"),
        "exclusion reason should be alef(skip)"
    );

    let public = surface.types.iter().find(|t| t.name == "PublicConfig").unwrap();
    assert!(!public.binding_excluded, "PublicConfig should not be excluded");
    assert!(public.binding_exclusion_reason.is_none());
}

#[test]
fn test_extract_binding_excluded_struct_doc_hidden() {
    let source = r#"
        #[doc(hidden)]
        pub struct HiddenType {
            pub value: u32,
        }
    "#;

    let surface = extract_from_source(source);
    let hidden = &surface.types[0];
    assert!(hidden.binding_excluded);
    assert_eq!(hidden.binding_exclusion_reason.as_deref(), Some("doc(hidden)"));
}

#[test]
fn test_extract_binding_excluded_enum() {
    let source = r#"
        #[cfg_attr(alef, alef(skip))]
        pub enum InternalState {
            Active,
            Inactive,
        }
        pub enum PublicState {
            On,
            Off,
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.enums.len(), 2);

    let internal = surface.enums.iter().find(|e| e.name == "InternalState").unwrap();
    assert!(internal.binding_excluded);
    assert_eq!(internal.binding_exclusion_reason.as_deref(), Some("alef(skip)"));

    let public = surface.enums.iter().find(|e| e.name == "PublicState").unwrap();
    assert!(!public.binding_excluded);
}

#[test]
fn test_extract_binding_excluded_function() {
    let source = r#"
        #[cfg_attr(alef, alef(skip))]
        pub fn internal_helper(x: u32) -> u32 {
            x
        }
        pub fn public_api(x: u32) -> u32 {
            x
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.functions.len(), 2);

    let internal = surface.functions.iter().find(|f| f.name == "internal_helper").unwrap();
    assert!(internal.binding_excluded);
    assert_eq!(internal.binding_exclusion_reason.as_deref(), Some("alef(skip)"));

    let public = surface.functions.iter().find(|f| f.name == "public_api").unwrap();
    assert!(!public.binding_excluded);
}

#[test]
fn test_extract_binding_excluded_trait() {
    let source = r#"
        #[alef(skip)]
        pub trait InternalTrait {
            fn do_thing(&self);
        }
        pub trait PublicTrait {
            fn work(&self);
        }
    "#;

    let surface = extract_from_source(source);
    assert_eq!(surface.types.len(), 2, "both traits should be extracted");

    let internal = surface.types.iter().find(|t| t.name == "InternalTrait").unwrap();
    assert!(internal.is_trait, "InternalTrait must have is_trait=true");
    assert!(
        internal.binding_excluded,
        "InternalTrait should have binding_excluded=true"
    );
    assert_eq!(internal.binding_exclusion_reason.as_deref(), Some("alef(skip)"));

    let public = surface.types.iter().find(|t| t.name == "PublicTrait").unwrap();
    assert!(public.is_trait);
    assert!(!public.binding_excluded);
}

#[test]
fn test_extract_binding_excluded_method() {
    let source = r#"
        pub struct Foo {
            pub value: u32,
        }
        impl Foo {
            #[alef(skip)]
            pub fn bar(&self) -> u32 {
                self.value
            }
            pub fn baz(&self) -> u32 {
                self.value
            }
        }
    "#;

    let surface = extract_from_source(source);
    let foo = surface.types.iter().find(|t| t.name == "Foo").unwrap();
    assert_eq!(foo.methods.len(), 2, "both methods should be extracted");

    let bar = foo.methods.iter().find(|m| m.name == "bar").unwrap();
    assert!(bar.binding_excluded, "bar should have binding_excluded=true");
    assert_eq!(bar.binding_exclusion_reason.as_deref(), Some("alef(skip)"));

    let baz = foo.methods.iter().find(|m| m.name == "baz").unwrap();
    assert!(!baz.binding_excluded, "baz should not be excluded");
}

#[test]
fn test_extract_skip_attribute_on_impl_block_drops_all_methods() {
    // Regression: an `impl` block carrying `#[cfg_attr(alef, alef(skip))]` must
    // contribute zero methods to the binding surface. The motivating case is a
    // fluent builder whose method names collide with struct fields:
    //
    //     pub struct JsonSchemaFormat {
    //         pub strict: Option<bool>,
    //         pub description: Option<String>,
    //         ...
    //     }
    //
    //     #[cfg_attr(alef, alef(skip))]
    //     impl JsonSchemaFormat {
    //         pub fn strict(mut self, on: bool) -> Self { ... }
    //         pub fn description(mut self, d: impl Into<String>) -> Self { ... }
    //     }
    //
    // The C FFI backend emits a field accessor for each public field AND a method
    // wrapper for each impl method. When the names collide, two
    // `#[no_mangle] extern "C" fn` definitions with the same symbol are emitted,
    // breaking compilation with E0428. Honoring `alef(skip)` on the impl block at
    // the IR-extraction layer means *no* backend sees the builder methods.
    let source = r#"
        pub struct JsonSchemaFormat {
            pub name: String,
            pub description: Option<String>,
            pub strict: Option<bool>,
        }

        #[cfg_attr(alef, alef(skip))]
        impl JsonSchemaFormat {
            pub fn strict(mut self, on: bool) -> Self { self.strict = Some(on); self }
            pub fn description(mut self, d: String) -> Self { self.description = Some(d); self }
        }
    "#;

    let surface = extract_from_source(source);
    let format = surface
        .types
        .iter()
        .find(|t| t.name == "JsonSchemaFormat")
        .expect("JsonSchemaFormat extracted");
    assert!(
        format.methods.is_empty(),
        "no methods should be lifted from a skipped impl block, got: {:?}",
        format.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
    // Fields must remain — only the impl block is skipped.
    let field_names: Vec<&str> = format.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"strict"));
    assert!(field_names.contains(&"description"));
}

#[test]
fn test_extract_skip_attribute_on_bare_alef_impl_block_drops_all_methods() {
    // Same as above but using bare `#[alef(skip)]` instead of `#[cfg_attr(alef, ...)]`.
    let source = r#"
        pub struct Builder {
            pub value: u32,
        }

        #[alef(skip)]
        impl Builder {
            pub fn value(mut self, v: u32) -> Self { self.value = v; self }
        }
    "#;

    let surface = extract_from_source(source);
    let builder = surface
        .types
        .iter()
        .find(|t| t.name == "Builder")
        .expect("Builder extracted");
    assert!(
        builder.methods.is_empty(),
        "no methods should be lifted from a skipped impl block, got: {:?}",
        builder.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
}

#[test]
fn test_disambiguation_pass_runs_on_full_extract() {
    // Two structs named `Event` in sibling modules. Without disambiguation, both
    // would survive with the same `name`, and downstream codegen would emit two
    // conflicting binding definitions. With disambiguation, the second is renamed
    // by prepending its PascalCase parent module segment.
    let dir = tempfile::tempdir().expect("tempdir");
    let lib_rs = dir.path().join("lib.rs");
    std::fs::write(
        &lib_rs,
        r#"
        pub mod stream {
            pub struct Event { pub data: String }
        }
        pub mod testing {
            pub struct Event { pub data: String }
        }
        "#,
    )
    .expect("write lib.rs");

    let surface = super::extract(&[lib_rs.as_path()], "my_crate", "0.0.0", None).expect("extract failed");

    let names: Vec<&str> = surface.types.iter().map(|t| t.name.as_str()).collect();
    // First-seen by sorted rust_path: `my_crate::stream::Event` < `my_crate::testing::Event`.
    assert!(
        names.contains(&"Event"),
        "stream::Event kept its original name: {names:?}"
    );
    assert!(
        names.contains(&"TestingEvent"),
        "testing::Event renamed with PascalCase parent: {names:?}"
    );
}

#[test]
fn test_error_enum_methods_whitelist() {
    // Simulates a downstream error enum with whitelisted introspection methods
    // plus a noisy Display::fmt that must be excluded.
    let source = r#"
        #[derive(Debug, thiserror::Error)]
        pub enum SampleLlmError {
            #[error("authentication failed")]
            AuthenticationFailed,
            #[error("rate limited: retry after {retry_after_secs}s")]
            RateLimited { retry_after_secs: u64 },
            #[error("provider unavailable")]
            ProviderUnavailable,
            #[error("invalid request: {message}")]
            InvalidRequest { message: String },
        }

        impl SampleLlmError {
            pub fn status_code(&self) -> u16 {
                match self {
                    Self::AuthenticationFailed => 401,
                    Self::RateLimited { .. } => 429,
                    Self::ProviderUnavailable => 503,
                    Self::InvalidRequest { .. } => 400,
                }
            }

            pub fn is_transient(&self) -> bool {
                matches!(self, Self::RateLimited { .. } | Self::ProviderUnavailable)
            }

            pub fn error_type(&self) -> &'static str {
                match self {
                    Self::AuthenticationFailed => "authentication_failed",
                    Self::RateLimited { .. } => "rate_limited",
                    Self::ProviderUnavailable => "provider_unavailable",
                    Self::InvalidRequest { .. } => "invalid_request",
                }
            }

            // This helper must NOT appear in the IR — it is not on the whitelist.
            pub fn to_status_message(&self) -> String {
                format!("{} ({})", self.error_type(), self.status_code())
            }
        }
    "#;

    let surface = extract_from_source(source);

    assert_eq!(surface.errors.len(), 1);
    let err = &surface.errors[0];
    assert_eq!(err.name, "SampleLlmError");
    assert_eq!(err.variants.len(), 4);

    // Exactly 3 whitelisted methods must be extracted, noisy helper excluded.
    assert_eq!(
        err.methods.len(),
        3,
        "expected 3 whitelisted methods, got {}: {:?}",
        err.methods.len(),
        err.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    let method_names: std::collections::HashSet<&str> = err.methods.iter().map(|m| m.name.as_str()).collect();

    assert!(method_names.contains("status_code"), "status_code must be extracted");
    assert!(method_names.contains("is_transient"), "is_transient must be extracted");
    assert!(method_names.contains("error_type"), "error_type must be extracted");
    assert!(
        !method_names.contains("to_status_message"),
        "to_status_message is not whitelisted and must be excluded"
    );

    // Verify return types are correctly resolved.
    let status_code = err.methods.iter().find(|m| m.name == "status_code").unwrap();
    assert_eq!(
        status_code.return_type,
        crate::core::ir::TypeRef::Primitive(PrimitiveType::U16),
        "status_code must return u16"
    );

    let is_transient = err.methods.iter().find(|m| m.name == "is_transient").unwrap();
    assert_eq!(
        is_transient.return_type,
        crate::core::ir::TypeRef::Primitive(PrimitiveType::Bool),
        "is_transient must return bool"
    );
}

/// Generic public functions cannot be safely represented as a concrete binding
/// surface unless explicit monomorphization metadata exists.

#[test]
fn test_extract_excludes_dyn_trait_object_fields() {
    // Fields whose type contains `dyn Trait` must be auto-excluded from bindings,
    // regardless of whether `#[serde(skip)]` is also present.
    // Trait objects cannot be marshaled through serde or constructed from non-Rust code.
    let source = r#"
        pub struct Config {
            pub normal: String,
            pub arc_dyn: std::sync::Arc<dyn MyTrait>,
            pub option_box_dyn: Option<Box<dyn MyTrait>>,
            pub vec_arc_dyn: Vec<std::sync::Arc<dyn MyTrait>>,
            #[serde(skip)]
            pub serde_skip_plain: String,
        }
    "#;

    let surface = extract_from_source(source);
    let config = &surface.types[0];

    let normal = config.fields.iter().find(|f| f.name == "normal").unwrap();
    assert!(!normal.binding_excluded, "plain String must not be excluded");

    let arc_dyn = config.fields.iter().find(|f| f.name == "arc_dyn").unwrap();
    assert!(arc_dyn.binding_excluded, "Arc<dyn Trait> must be excluded");
    assert_eq!(
        arc_dyn.binding_exclusion_reason.as_deref(),
        Some("dyn-trait-object"),
        "exclusion reason must be dyn-trait-object"
    );

    let option_box_dyn = config.fields.iter().find(|f| f.name == "option_box_dyn").unwrap();
    assert!(
        option_box_dyn.binding_excluded,
        "Option<Box<dyn Trait>> must be excluded"
    );
    assert_eq!(
        option_box_dyn.binding_exclusion_reason.as_deref(),
        Some("dyn-trait-object"),
        "exclusion reason must be dyn-trait-object"
    );

    let vec_arc_dyn = config.fields.iter().find(|f| f.name == "vec_arc_dyn").unwrap();
    assert!(vec_arc_dyn.binding_excluded, "Vec<Arc<dyn Trait>> must be excluded");
    assert_eq!(
        vec_arc_dyn.binding_exclusion_reason.as_deref(),
        Some("dyn-trait-object"),
        "exclusion reason must be dyn-trait-object"
    );

    // `#[serde(skip)]` alone on a plain type must NOT trigger binding exclusion
    // (consumers use serde(skip) on types that bindings can still access directly).
    let serde_skip_plain = config.fields.iter().find(|f| f.name == "serde_skip_plain").unwrap();
    assert!(
        !serde_skip_plain.binding_excluded,
        "serde(skip) on plain String must not exclude from bindings"
    );
}

// --- Generic public functions ---
