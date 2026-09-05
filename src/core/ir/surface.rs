use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::items::{EnumDef, ErrorDef, FunctionDef, TypeDef};
use super::metadata::ErrorTaxonomy;
use super::service::{HandlerContractDef, ServiceDef};

/// Returns `true` when the `#[cfg(...)]` condition string is satisfied by the
/// given set of enabled feature names.
///
/// Supports the cfg shapes produced by the extractor for feature gates:
/// `feature = "x"`, `any(...)`, `all(...)`, and `not(...)`, composed
/// arbitrarily. A `None` cfg is always satisfied, and the synthetic `full`
/// feature satisfies every gate (mirroring a conventional umbrella feature in
/// source crates).
///
/// Non-feature leaves (e.g. `target_arch = "wasm32"`, `target_os = "..."`)
/// cannot be resolved at generation time — the same binding may compile the
/// item on one target and a stub on another. Such leaves are treated as
/// *indeterminate*, and any expression whose truth value depends on an
/// indeterminate leaf resolves to "satisfied" so the item is **kept** (the
/// conservative choice: never drop an item purely because of a target
/// predicate). Only expressions that are decided entirely by feature gates can
/// cause an item to be dropped.
///
/// The IR encodes cfgs via `proc_macro2::TokenStream::to_string()`, which
/// inserts whitespace between tokens (e.g. `any (feature = "a" , ...)`); the
/// evaluator normalises that before parsing.
#[must_use]
pub fn cfg_feature_satisfied(cfg: Option<&str>, enabled_features: &HashSet<&str>) -> bool {
    let Some(cfg_str) = cfg else {
        return true;
    };

    // Indeterminate (target predicate) => keep the item. ~keep
    let full_satisfies_features = enabled_features.contains("full") && !cfg_contains_test_predicate(cfg_str);
    cfg_expr_satisfied(cfg_str, enabled_features, full_satisfies_features).unwrap_or(true)
}

fn cfg_contains_test_predicate(cfg_str: &str) -> bool {
    cfg_str
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .any(|token| token == "test")
}

/// Three-valued evaluation of a normalised cfg predicate.
///
/// Returns `Some(true)`/`Some(false)` when the predicate is fully decided by
/// feature gates, and `None` when its value depends on an unresolved
/// non-feature leaf (e.g. `target_arch`). `all`/`any`/`not` propagate the
/// indeterminate result with standard Kleene logic so, for example,
/// `all(feature = "x", target_arch = "wasm32")` is `Some(false)` when `x` is
/// off (short-circuit) but `None` when `x` is on.
fn cfg_expr_satisfied(cfg_str: &str, enabled_features: &HashSet<&str>, full_satisfies_features: bool) -> Option<bool> {
    let normalized = cfg_str.trim().replace(" (", "(");
    let cfg_str = normalized.as_str();

    if let Some(rest) = cfg_str.strip_prefix("feature = \"")
        && let Some(feature_name) = rest.strip_suffix('"')
    {
        return Some(full_satisfies_features || enabled_features.contains(feature_name));
    }

    if cfg_str == "test" {
        return Some(false);
    }

    if let Some(inner) = cfg_str.strip_prefix("any(").and_then(|s| s.strip_suffix(')')) {
        let mut saw_indeterminate = false;
        for cond in split_cfg_operands(inner) {
            match cfg_expr_satisfied(&cond, enabled_features, full_satisfies_features) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => saw_indeterminate = true,
            }
        }
        return if saw_indeterminate { None } else { Some(false) };
    }

    if let Some(inner) = cfg_str.strip_prefix("all(").and_then(|s| s.strip_suffix(')')) {
        let mut saw_indeterminate = false;
        for cond in split_cfg_operands(inner) {
            match cfg_expr_satisfied(&cond, enabled_features, full_satisfies_features) {
                Some(false) => return Some(false),
                Some(true) => {}
                None => saw_indeterminate = true,
            }
        }
        return if saw_indeterminate { None } else { Some(true) };
    }

    if let Some(inner) = cfg_str.strip_prefix("not(").and_then(|s| s.strip_suffix(')')) {
        return cfg_expr_satisfied(inner.trim(), enabled_features, full_satisfies_features).map(|value| !value);
    }

    // Unrecognised leaf (e.g. `target_arch = "..."`): indeterminate. ~keep
    None
}

/// Split the comma-separated operand list inside an `any(...)`/`all(...)`,
/// respecting nested parentheses so compound operands stay intact.
fn split_cfg_operands(inner: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    result.push(trimmed);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
}

/// Complete API surface extracted from a Rust crate's public interface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSurface {
    pub crate_name: String,
    pub version: String,
    pub types: Vec<TypeDef>,
    pub functions: Vec<FunctionDef>,
    pub enums: Vec<EnumDef>,
    pub errors: Vec<ErrorDef>,
    #[serde(default)]
    pub excluded_type_paths: std::collections::BTreeMap<String, String>,
    #[serde(default, serialize_with = "super::ordered_serde::serialize_set")]
    pub excluded_trait_names: std::collections::HashSet<String>,
    #[serde(default)]
    pub services: Vec<ServiceDef>,
    #[serde(default)]
    pub handler_contracts: Vec<HandlerContractDef>,
    #[serde(default)]
    pub unsupported_public_items: Vec<UnsupportedPublicItem>,
}

impl ApiSurface {
    pub(crate) const FFI_ERROR_CODE_NONE: u32 = 0;
    pub(crate) const FFI_ERROR_CODE_CONVERSION: u32 = 1;
    pub(crate) const FFI_ERROR_CODE_UNKNOWN: u32 = 2;
    pub(crate) const FFI_ERROR_CODE_PANIC: u32 = 3;
    pub(crate) const FFI_ERROR_CODE_INVALID_HANDLE: u32 = 4;
    pub(crate) const FFI_ERROR_CODE_DOMAIN_MIN: u32 = 100;
    pub(crate) const FFI_ERROR_CODE_DOMAIN_MAX: u32 = i32::MAX as u32;

    /// Returns checked-in taxonomy metadata for explicitly numbered error variants. ~keep
    pub fn error_taxonomy(&self) -> Vec<ErrorTaxonomy> {
        self.errors
            .iter()
            .flat_map(|error| {
                error
                    .variants
                    .iter()
                    .filter_map(|variant| variant.taxonomy(&error.rust_path))
            })
            .collect()
    }

    pub(crate) fn validate_error_taxonomy(&self) -> anyhow::Result<()> {
        let mut allocated: std::collections::HashMap<u32, (String, String)> = std::collections::HashMap::new();
        let mut member_names = std::collections::HashMap::new();
        for entry in self.error_taxonomy() {
            if !(Self::FFI_ERROR_CODE_DOMAIN_MIN..=Self::FFI_ERROR_CODE_DOMAIN_MAX).contains(&entry.code) {
                anyhow::bail!(
                    "FFI error code {} for {}::{} is outside the domain range {}..={}",
                    entry.code,
                    entry.error_type,
                    entry.variant,
                    Self::FFI_ERROR_CODE_DOMAIN_MIN,
                    Self::FFI_ERROR_CODE_DOMAIN_MAX
                );
            }
            if let Some((error_type, variant)) =
                allocated.insert(entry.code, (entry.error_type.clone(), entry.variant.clone()))
            {
                anyhow::bail!(
                    "FFI error code {} for {}::{} duplicates {}::{}",
                    entry.code,
                    entry.error_type,
                    entry.variant,
                    error_type,
                    variant
                );
            }
            let member_name = crate::codegen::naming::ffi_error_code_variant_name(&entry.error_type, &entry.variant);
            if let Some((error_type, variant)) =
                member_names.insert(member_name.clone(), (entry.error_type.clone(), entry.variant.clone()))
            {
                anyhow::bail!(
                    "FFI error enum member {member_name} for {}::{} collides with {}::{}",
                    entry.error_type,
                    entry.variant,
                    error_type,
                    variant
                );
            }
        }
        Ok(())
    }

    /// Returns a clone of this surface with same-named cfg-variant functions collapsed to one.
    ///
    /// Single-surface backends (Java, C#, Go, Kotlin, Swift, Dart, PHP, Ruby, Elixir, R/extendr)
    /// emit one host method per function. When the extractor preserves a real impl and a
    /// stub fallback under disjoint `cfg` gates, those two entries would otherwise become two
    /// host methods with identical signatures — a duplicate-method compile error. This collapses
    /// each such group into a single entry whose `cfg` is the OR of all members'. Rust-cfg-gated
    /// backends (FFI, napi, pyo3, wasm) must NOT call this: they emit both `#[cfg]`-guarded items
    /// and rely on `rustc` selecting one per feature set.
    #[must_use]
    pub fn with_deduped_functions(&self) -> Self {
        let mut deduped = self.clone();
        deduped.functions = crate::codegen::fn_dedup::dedup_same_name_functions(&self.functions);
        deduped
    }

    /// Returns a clone of this surface with every type, enum, and function whose
    /// `#[cfg(feature = "...")]` gate is not satisfied by `enabled_features` removed.
    ///
    /// Single-surface host facades (Swift, and any other backend that emits one
    /// host symbol per item with no Rust-cfg gating of its own) must only emit
    /// references to items that the Rust bridge crate actually compiles for the
    /// configured feature set. The Rust bridge crate filters its `visible_*` sets
    /// with the same cfg check, so applying this to the host-facing `ApiSurface`
    /// keeps the two layers consistent: the high-level facade never references a
    /// type or function that the bridge layer dropped under the active features.
    ///
    /// Only feature gates are evaluated (see [`cfg_feature_satisfied`]); other cfg
    /// shapes such as `target_os` are left in place for the compiler to resolve.
    #[must_use]
    pub fn with_cfg_filtered(&self, enabled_features: &HashSet<&str>) -> Self {
        let mut filtered = self.clone();
        filtered
            .types
            .retain(|t| cfg_feature_satisfied(t.cfg.as_deref(), enabled_features));
        filtered
            .enums
            .retain(|e| cfg_feature_satisfied(e.cfg.as_deref(), enabled_features));
        filtered
            .functions
            .retain(|f| cfg_feature_satisfied(f.cfg.as_deref(), enabled_features));
        // `ServiceDef` carries its own `cfg` (see `cbindgen_feature_defines`, which reads it for
        // the FFI header's `#if` guards); host-facing service-API generators must drop a
        // cfg-gated service the same way they drop a cfg-gated type/enum/function, or they emit
        // glue that calls FFI service entrypoints the active feature set never compiles. ~keep
        filtered
            .services
            .retain(|s| cfg_feature_satisfied(s.cfg.as_deref(), enabled_features));
        filtered
    }

    /// Like [`with_cfg_filtered`](Self::with_cfg_filtered), but also drops
    /// cfg-gated *members* — struct fields, methods, enum variants, and
    /// enum-variant fields — whose gate is not satisfied by `enabled_features`.
    ///
    /// The docs generator renders the full public surface of a type (its fields
    /// and variants), so a shallow top-level filter is not enough: a
    /// type that survives can still carry a `#[cfg(feature = "...")]`-gated field
    /// (e.g. `ExtractionConfig.tree_sitter`) that is compiled out of a binding
    /// whose feature set excludes that feature. This produces documentation that
    /// diverges from the binding's real generated surface. Deep filtering keeps
    /// the reference docs consistent with what each binding actually exposes.
    #[must_use]
    pub fn with_cfg_filtered_deep(&self, enabled_features: &HashSet<&str>) -> Self {
        let mut filtered = self.with_cfg_filtered(enabled_features);

        for typ in &mut filtered.types {
            typ.fields
                .retain(|field| cfg_feature_satisfied(field.cfg.as_deref(), enabled_features));
            typ.methods.retain(|method| method.cfg_satisfied(enabled_features));
        }

        for enum_def in &mut filtered.enums {
            enum_def
                .variants
                .retain(|variant| cfg_feature_satisfied(variant.cfg.as_deref(), enabled_features));
            for variant in &mut enum_def.variants {
                variant
                    .fields
                    .retain(|field| cfg_feature_satisfied(field.cfg.as_deref(), enabled_features));
            }
            enum_def.methods.retain(|method| method.cfg_satisfied(enabled_features));
        }

        // Error introspection methods (`status_code`, `is_transient`, …) reach the same
        // single-surface emitters as type methods, so an unsatisfied gate must drop them here
        // too — otherwise the host facade calls an FFI export the linked library never
        // compiled. ~keep
        for error in &mut filtered.errors {
            error.methods.retain(|method| method.cfg_satisfied(enabled_features));
        }

        filtered
    }
}

/// A public item that was discovered but not extracted into binding IR.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnsupportedPublicItem {
    pub item_kind: String,
    pub item_path: String,
    pub reason: String,
    pub suggested_fix: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, FunctionDef, TypeDef};

    fn features(names: &[&'static str]) -> HashSet<&'static str> {
        names.iter().copied().collect()
    }

    fn ty(name: &str, cfg: Option<&str>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..TypeDef::default()
        }
    }

    fn en(name: &str, cfg: Option<&str>) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..EnumDef::default()
        }
    }

    fn func(name: &str, cfg: Option<&str>) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..FunctionDef::default()
        }
    }

    #[test]
    fn cfg_feature_satisfied_handles_none_and_full() {
        let enabled = features(&["pdf"]);
        assert!(cfg_feature_satisfied(None, &enabled));
        assert!(cfg_feature_satisfied(Some("feature = \"pdf\""), &enabled));
        assert!(!cfg_feature_satisfied(Some("feature = \"presets\""), &enabled));
        assert!(cfg_feature_satisfied(
            Some("feature = \"presets\""),
            &features(&["full"])
        ));
    }

    #[test]
    fn cfg_feature_satisfied_handles_any_gate() {
        let enabled = features(&["ocr"]);
        assert!(cfg_feature_satisfied(
            Some("any(feature = \"ocr\", feature = \"paddle-ocr\")"),
            &enabled
        ));
        assert!(!cfg_feature_satisfied(
            Some("any(feature = \"presets\", feature = \"heuristics\")"),
            &enabled
        ));
    }

    #[test]
    fn cfg_feature_satisfied_handles_all_and_not_gates() {
        let enabled = features(&["layout-types"]);
        assert!(cfg_feature_satisfied(
            Some("all(feature = \"layout-types\", not(feature = \"wasm-target\"))"),
            &enabled
        ));
        assert!(!cfg_feature_satisfied(
            Some("not(feature = \"layout-types\")"),
            &enabled
        ));
        assert!(cfg_feature_satisfied(Some("not(feature = \"wasm-target\")"), &enabled));
        assert!(!cfg_feature_satisfied(
            Some("all(feature = \"layout-types\", feature = \"wasm-target\")"),
            &enabled
        ));
    }

    #[test]
    fn cfg_feature_satisfied_handles_whitespace_and_nested_operands() {
        let enabled = features(&["wasm-target"]);
        assert!(cfg_feature_satisfied(
            Some("any (feature = \"wasm-target\" , all(feature = \"a\", feature = \"b\"))"),
            &enabled
        ));
    }

    #[test]
    fn cfg_feature_satisfied_leaves_non_feature_gates_satisfied() {
        assert!(cfg_feature_satisfied(
            Some("target_os = \"windows\""),
            &features(&["pdf"])
        ));
    }

    #[test]
    fn cfg_feature_satisfied_keeps_items_gated_on_target_predicates() {
        // A feature gate kept on any non-wasm native binding: the feature leaf is enabled and ~keep
        // the target-arch leaf is indeterminate at generation time, so it must not cause a drop. ~keep
        let enabled = features(&["native"]);
        assert!(cfg_feature_satisfied(
            Some("all (feature = \"native\" , not (target_arch = \"wasm32\"))"),
            &enabled
        ));
        assert!(!cfg_feature_satisfied(
            Some("all (feature = \"native\" , not (target_arch = \"wasm32\"))"),
            &features(&["pdf"])
        ));
        assert!(cfg_feature_satisfied(
            Some("not (target_arch = \"wasm32\")"),
            &features(&["pdf"])
        ));
    }

    #[test]
    fn with_cfg_filtered_drops_unsatisfied_items_only() {
        let mut surface = ApiSurface::default();
        surface.types.push(ty("PdfMetadata", Some("feature = \"pdf\"")));
        surface.types.push(ty("Preset", Some("feature = \"presets\"")));
        surface.types.push(ty("AlwaysOn", None));
        surface.enums.push(en("PresetCategory", Some("feature = \"presets\"")));
        surface.enums.push(en("MimeKind", None));
        surface
            .functions
            .push(func("analyze_document", Some("feature = \"heuristics\"")));
        surface.functions.push(func("extract", None));

        let filtered = surface.with_cfg_filtered(&features(&["pdf"]));

        let type_names: Vec<&str> = filtered.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(type_names, vec!["PdfMetadata", "AlwaysOn"]);
        let enum_names: Vec<&str> = filtered.enums.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(enum_names, vec!["MimeKind"]);
        let fn_names: Vec<&str> = filtered.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(fn_names, vec!["extract"]);
    }

    #[test]
    fn with_cfg_filtered_deep_drops_gated_fields_and_variants() {
        use crate::core::ir::{EnumVariant, FieldDef};

        let mut config_ty = ty("ExtractionConfig", None);
        config_ty.fields = vec![
            FieldDef {
                name: "use_cache".to_string(),
                cfg: None,
                ..FieldDef::default()
            },
            FieldDef {
                name: "tree_sitter".to_string(),
                cfg: Some("feature = \"tree-sitter\"".to_string()),
                ..FieldDef::default()
            },
        ];

        let mut fmt_enum = en("OutputFormat", None);
        fmt_enum.variants = vec![
            EnumVariant {
                name: "Markdown".to_string(),
                cfg: None,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "CodeAst".to_string(),
                cfg: Some("feature = \"tree-sitter\"".to_string()),
                ..EnumVariant::default()
            },
        ];

        let mut surface = ApiSurface::default();
        surface.types.push(config_ty);
        surface
            .types
            .push(ty("TreeSitterConfig", Some("feature = \"tree-sitter\"")));
        surface.enums.push(fmt_enum);

        let filtered = surface.with_cfg_filtered_deep(&features(&["pdf"]));

        let type_names: Vec<&str> = filtered.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(type_names, vec!["ExtractionConfig"]);
        let field_names: Vec<&str> = filtered.types[0].fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(field_names, vec!["use_cache"]);
        let variant_names: Vec<&str> = filtered.enums[0].variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(variant_names, vec!["Markdown"]);
    }

    #[test]
    fn with_cfg_filtered_deep_drops_gated_methods_on_types_enums_and_errors() {
        use crate::core::ir::{ErrorDef, MethodDef};

        fn method(name: &str, cfg: Option<&str>) -> MethodDef {
            MethodDef {
                name: name.to_string(),
                cfg: cfg.map(str::to_string),
                ..MethodDef::default()
            }
        }

        let mut client = ty("Client", None);
        client.methods = vec![method("ping", None), method("stream", Some("feature = \"streaming\""))];

        let mut format = en("Format", None);
        format.methods = vec![method("all", None), method("from_mime", Some("feature = \"mime\""))];

        let error = ErrorDef {
            name: "ClientError".to_string(),
            rust_path: "mylib::ClientError".to_string(),
            original_rust_path: String::new(),
            variants: vec![],
            doc: String::new(),
            methods: vec![
                method("status_code", None),
                method("retry_after", Some("feature = \"retry\"")),
            ],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let mut surface = ApiSurface::default();
        surface.types.push(client);
        surface.enums.push(format);
        surface.errors.push(error);

        let filtered = surface.with_cfg_filtered_deep(&features(&["mime"]));

        let type_methods: Vec<&str> = filtered.types[0].methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(type_methods, vec!["ping"], "unsatisfied type method must be dropped");
        let enum_methods: Vec<&str> = filtered.enums[0].methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            enum_methods,
            vec!["all", "from_mime"],
            "a satisfied gate keeps the method"
        );
        let error_methods: Vec<&str> = filtered.errors[0].methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(error_methods, vec!["status_code"], "error methods are filtered too");
    }

    #[test]
    fn with_cfg_filtered_deep_keeps_gated_members_under_full() {
        use crate::core::ir::FieldDef;

        let mut config_ty = ty("ExtractionConfig", None);
        config_ty.fields = vec![FieldDef {
            name: "tree_sitter".to_string(),
            cfg: Some("feature = \"tree-sitter\"".to_string()),
            ..FieldDef::default()
        }];

        let mut surface = ApiSurface::default();
        surface.types.push(config_ty);

        let filtered = surface.with_cfg_filtered_deep(&features(&["full"]));
        assert_eq!(filtered.types[0].fields.len(), 1);
    }

    #[test]
    fn with_cfg_filtered_deep_drops_test_only_members_under_full() {
        use crate::core::ir::EnumVariant;

        let mut strategy = en("Strategy", None);
        strategy.variants = vec![
            EnumVariant {
                name: "Stable".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "HarnessOnly".to_string(),
                cfg: Some("any(test, feature = \"testkit\")".to_string()),
                ..EnumVariant::default()
            },
        ];
        let mut surface = ApiSurface::default();
        surface.enums.push(strategy);

        let filtered = surface.with_cfg_filtered_deep(&features(&["full"]));
        let variant_names: Vec<&str> = filtered.enums[0]
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect();
        assert_eq!(variant_names, vec!["Stable"]);
    }

    #[test]
    fn with_cfg_filtered_keeps_everything_under_full() {
        let mut surface = ApiSurface::default();
        surface.types.push(ty("Preset", Some("feature = \"presets\"")));
        surface
            .functions
            .push(func("analyze_document", Some("feature = \"heuristics\"")));

        let filtered = surface.with_cfg_filtered(&features(&["full"]));
        assert_eq!(filtered.types.len(), 1);
        assert_eq!(filtered.functions.len(), 1);
    }
}
