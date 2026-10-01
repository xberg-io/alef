use super::*;
use crate::core::ir::DefaultValue;

/// Table-driven coverage for `postprocess::resolve_enum_field_defaults`: a field whose default
/// comes from `<EnumType>::default()` (or `Default::default()`, which folds identically — see
/// `defaults::expr_to_default_value`) starts out `DefaultValue::Empty` and this pass narrows it
/// to a concrete `EnumVariant` whenever the field's own enum type has a knowable default. ~keep
///
/// (a) The common case: the enum derives `Default` and one variant carries `#[default]`. The
/// field must resolve to exactly that variant.
#[test]
fn enum_field_default_resolves_to_the_marked_default_variant() {
    let source = r#"
        #[derive(Default, Clone, Copy)]
        pub enum Mode {
            Fast,
            #[default]
            Balanced,
            Slow,
        }

        pub struct Config {
            pub mode: Mode,
        }

        impl Default for Config {
            fn default() -> Self {
                Self { mode: Mode::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "Config").unwrap();
    let mode_field = config.fields.iter().find(|field| field.name == "mode").unwrap();

    assert_eq!(
        mode_field.typed_default,
        Some(DefaultValue::EnumVariant("Balanced".to_string())),
        "a field defaulting to an enum's own `default()` must resolve to the `#[default]`-marked variant"
    );
}

/// (b) The enum has a hand-written `impl Default` rather than `#[derive(Default)]`, so no variant
/// carries the `#[default]` attribute — that attribute only exists for the derive macro. The
/// variant is read out of the impl body instead (see
/// `extract::extractor::functions::impl_blocks::manual_default_unit_variant`), so the resolved
/// value is the one the Rust core actually returns.
///
/// The fixture is deliberately built so a positional guess gives the wrong answer: `Priority`
/// declares `Low` first but its `default()` returns `Medium`. Asserting `Medium` here is what
/// distinguishes reading the impl from falling back to the first declared variant.
#[test]
fn enum_field_default_reads_the_variant_a_manual_default_impl_returns() {
    let source = r#"
        #[derive(Clone, Copy)]
        pub enum Priority {
            Low,
            Medium,
            High,
        }

        impl Default for Priority {
            fn default() -> Self {
                Priority::Medium
            }
        }

        pub struct Task {
            pub priority: Priority,
        }

        impl Default for Task {
            fn default() -> Self {
                Self { priority: Priority::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let priority_enum = surface.enums.iter().find(|e| e.name == "Priority").unwrap();
    assert!(
        priority_enum.has_default,
        "a manual `impl Default for Priority` must still set `has_default=true`"
    );
    assert_eq!(
        priority_enum
            .variants
            .iter()
            .filter(|v| v.is_default)
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Medium"],
        "the manual impl's returned variant must be marked `is_default`, and only that variant"
    );

    let task = surface.types.iter().find(|typ| typ.name == "Task").unwrap();
    let priority_field = task.fields.iter().find(|field| field.name == "priority").unwrap();

    assert_eq!(
        priority_field.typed_default,
        Some(DefaultValue::EnumVariant("Medium".to_string())),
        "the field must resolve to the variant the manual impl returns, not the first declared one"
    );
}

/// (b2) A manual `impl Default` returning a *non-unit* variant must not be narrowed.
/// `DefaultValue::EnumVariant` names a bare unit-variant path with no arguments of its own, so
/// emitting one for a struct variant would name a value that cannot be constructed without its
/// fields — a fabrication that fails to compile in the generated binding. The field stays `Empty`
/// and each backend keeps its own honest guard.
#[test]
fn enum_field_default_stays_empty_when_the_default_variant_carries_data() {
    let source = r#"
        pub enum NodeContent {
            Heading { level: u8, text: String },
            Paragraph { text: String },
        }

        impl Default for NodeContent {
            fn default() -> Self {
                Self::Heading { level: 1, text: String::new() }
            }
        }

        pub struct Node {
            pub content: NodeContent,
        }

        impl Default for Node {
            fn default() -> Self {
                Self { content: NodeContent::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let node_content = surface.enums.iter().find(|e| e.name == "NodeContent").unwrap();
    assert!(
        node_content.variants.iter().all(|v| !v.is_default),
        "a struct-variant default body must not mark any variant `is_default`"
    );

    let node = surface.types.iter().find(|typ| typ.name == "Node").unwrap();
    let content_field = node.fields.iter().find(|field| field.name == "content").unwrap();

    assert_eq!(
        content_field.typed_default,
        Some(DefaultValue::Empty),
        "a data-carrying default variant must stay `Empty` rather than become a bare variant name"
    );
}

/// (c) The enum has no `Default` impl at all — `EnumDef::has_default` is false. The genuinely
/// unresolved case must stay `Empty`, preserving the existing honest fallback (every backend
/// already treats `Empty` on a `Named` field as "unknown" and guards accordingly, e.g. C#'s
/// `required`). This is the control: resolving a field must never happen when alef cannot know
/// the enum's default at all.
#[test]
fn enum_field_default_stays_empty_when_the_enum_has_no_default_impl() {
    let source = r#"
        pub enum Format {
            Plain,
            Rich,
        }

        pub struct Document {
            pub format: Format,
        }

        impl Default for Document {
            fn default() -> Self {
                Self { format: Format::default() }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let format_enum = surface.enums.iter().find(|e| e.name == "Format").unwrap();
    assert!(
        !format_enum.has_default,
        "`Format` derives no `Default` and has no manual impl in this fixture"
    );

    let document = surface.types.iter().find(|typ| typ.name == "Document").unwrap();
    let format_field = document.fields.iter().find(|field| field.name == "format").unwrap();

    assert_eq!(
        format_field.typed_default,
        Some(DefaultValue::Empty),
        "a field whose enum type has no knowable default must stay `Empty`, not a guessed variant"
    );
}

/// (h) A bare field-level `#[serde(default)]` field whose declared enum type's real default is
/// *not* its first-declared variant, alongside a manual `impl Default` that sets the field
/// directly to a variant path (`OcrStrategy::Auto`, not `OcrStrategy::default()`). The field must
/// resolve to that concrete variant — never fall back to the first-declared one, and never stay
/// `Empty` — regardless of extra attributes (`deserialize_with`) riding alongside `default` on
/// the same `#[serde(...)]` list.
#[test]
fn bare_serde_default_field_resolves_to_the_manual_impls_direct_variant_path() {
    let source = r#"
        #[derive(Clone, Copy)]
        pub enum OcrStrategy {
            Never,
            Auto,
            Always,
        }

        pub struct ExtractionConfig {
            #[serde(default, deserialize_with = "deserialize_null_default")]
            pub ocr_strategy: OcrStrategy,
        }

        impl Default for ExtractionConfig {
            fn default() -> Self {
                Self {
                    ocr_strategy: OcrStrategy::Auto,
                }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "ExtractionConfig").unwrap();
    let ocr_strategy_field = config.fields.iter().find(|field| field.name == "ocr_strategy").unwrap();

    assert_eq!(
        ocr_strategy_field.typed_default,
        Some(DefaultValue::EnumVariant("Auto".to_string())),
        "a bare `#[serde(default)]` field must resolve to the manual impl's concrete variant, \
         not stay `Empty` or default to the first-declared variant"
    );
}

/// A field-level bare `#[serde(default)]` invokes the field type's `Default` implementation
/// when the wire key is absent; it does not require the owning struct to implement `Default`.
/// Keep an unrelated field in the same non-`Default` owner to prove only the annotated field
/// becomes omission-tolerant. ~keep
#[test]
fn bare_serde_default_resolves_enum_variant_without_owner_default() {
    let source = r#"
        #[derive(Default, Clone, Copy)]
        pub enum OcrStrategy {
            Never,
            #[default]
            Auto,
            Always,
        }

        #[derive(Default)]
        pub struct OcrQualityThresholds {
            pub min_quality: f64,
        }

        pub struct ExtractionConfig {
            pub endpoint: String,
            #[serde(default)]
            pub ocr_strategy: OcrStrategy,
            #[serde(default)]
            pub quality_thresholds: OcrQualityThresholds,
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "ExtractionConfig").unwrap();
    assert!(
        !config.has_default,
        "the owner deliberately has no `Default` implementation"
    );

    let endpoint = config.fields.iter().find(|field| field.name == "endpoint").unwrap();
    assert_eq!(endpoint.typed_default, None, "an unrelated field must remain required");

    let strategy = config.fields.iter().find(|field| field.name == "ocr_strategy").unwrap();
    assert_eq!(strategy.default.as_deref(), Some("/* serde(default) */"));
    assert_eq!(
        strategy.typed_default,
        Some(DefaultValue::EnumVariant("Auto".to_string())),
        "bare serde default must resolve the field type's default even when the owner has none"
    );

    let thresholds = config
        .fields
        .iter()
        .find(|field| field.name == "quality_thresholds")
        .unwrap();
    assert_eq!(
        thresholds.typed_default, None,
        "non-enum bare defaults must preserve their pre-existing unresolved shape"
    );
}

#[test]
fn bare_serde_default_uses_qualified_type_identity_when_short_names_collide() {
    let source = r#"
        pub mod module_a {
            #[derive(Default)]
            pub struct Mode;
        }

        pub mod module_b {
            #[derive(Default, Clone, Copy)]
            pub enum Mode {
                Disabled,
                #[default]
                Enabled,
            }
        }

        pub struct Config {
            #[serde(default)]
            pub struct_mode: crate::module_a::Mode,
            #[serde(default)]
            pub enum_mode: crate::module_b::Mode,
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "Config").unwrap();
    let struct_mode = config.fields.iter().find(|field| field.name == "struct_mode").unwrap();
    let enum_mode = config.fields.iter().find(|field| field.name == "enum_mode").unwrap();

    assert_eq!(
        struct_mode.type_rust_path.as_deref(),
        Some("test_crate::module_a::Mode")
    );
    assert_eq!(
        struct_mode.typed_default, None,
        "same-named struct must not inherit the enum's default variant"
    );
    assert!(!struct_mode.has_bare_serde_enum_default());
    assert_eq!(enum_mode.type_rust_path.as_deref(), Some("test_crate::module_b::Mode"));
    assert_eq!(
        enum_mode.typed_default,
        Some(DefaultValue::EnumVariant("Enabled".to_string()))
    );
    assert!(enum_mode.has_bare_serde_enum_default());
}

/// (i) Security control for the shape task #558 was filed against: an `Option<Enum>` field with
/// no explicit default (no `#[serde(default = "...")]`, no field initializer in the owning
/// struct's `impl`/derived `Default`) must resolve to absence, never to a materialized variant.
///
/// `extract::extractor::helpers::fields::unwrap_optional` collapses `Option<DetectionPolicy>` to
/// `field.ty = TypeRef::Named("DetectionPolicy")` with `field.optional = true` before this pass
/// runs, so `resolve_enum_field_defaults` sees the same `Named` shape as the required-field case
/// in test (a) above and must not treat them alike: for the required field `Empty` really is
/// "the enum's own zero"; for this optional field `Empty` is "the field's own type's zero" —
/// `Option<DetectionPolicy>::default()`, which is `None` — and narrowing it to a variant would
/// fabricate a value the source crate never specified. A generated binding that forwards that
/// fabricated variant as a non-null value is indistinguishable from a caller's explicit choice,
/// and can silently override a stricter caller-supplied policy elsewhere in the same call.
#[test]
fn optional_enum_field_default_stays_empty_never_a_materialized_variant() {
    let source = r#"
        #[derive(Default, Clone, Copy)]
        pub enum DetectionPolicy {
            #[default]
            PreferContent,
            ContentOnly,
            TrustExtension,
        }

        #[derive(Default)]
        pub struct FileConfig {
            pub detection_policy: Option<DetectionPolicy>,
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "FileConfig").unwrap();
    let policy_field = config
        .fields
        .iter()
        .find(|field| field.name == "detection_policy")
        .unwrap();

    assert!(
        policy_field.optional,
        "Option<DetectionPolicy> must unwrap to optional=true"
    );
    assert_eq!(
        policy_field.typed_default,
        Some(DefaultValue::Empty),
        "an `Option<Enum>` field absent from an explicit default must stay `Empty` (renders as \
         None/null downstream), never be narrowed to the enum's own default variant"
    );
}

/// (j) Negative control for (i): when the same optional field genuinely does have an explicit
/// default naming a concrete variant (`Some(DetectionPolicy::ContentOnly)`, not omitted from the
/// literal), that real default must still be preserved. A fix that strips every optional-field
/// default rather than only the fabricated-from-`Empty` case would pass (i) and silently break
/// this legitimate one.
#[test]
fn optional_enum_field_with_explicit_default_still_resolves_to_the_named_variant() {
    let source = r#"
        #[derive(Clone, Copy)]
        pub enum DetectionPolicy {
            PreferContent,
            ContentOnly,
            TrustExtension,
        }

        pub struct FileConfig {
            pub detection_policy: Option<DetectionPolicy>,
        }

        impl Default for FileConfig {
            fn default() -> Self {
                Self { detection_policy: Some(DetectionPolicy::ContentOnly) }
            }
        }
    "#;

    let surface = extract_from_source(source);
    let config = surface.types.iter().find(|typ| typ.name == "FileConfig").unwrap();
    let policy_field = config
        .fields
        .iter()
        .find(|field| field.name == "detection_policy")
        .unwrap();

    assert!(
        policy_field.optional,
        "Option<DetectionPolicy> must unwrap to optional=true"
    );
    assert_eq!(
        policy_field.typed_default,
        Some(DefaultValue::EnumVariant("ContentOnly".to_string())),
        "an explicit `Some(Variant)` default must still be preserved for an optional field"
    );
}
