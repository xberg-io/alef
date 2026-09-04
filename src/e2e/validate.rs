//! JSON Schema and semantic validation for e2e fixture files.

use crate::e2e::codegen::assertion_recipes;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, group_fixtures};
use anyhow::{Context, Result};
use std::fmt;
use std::path::Path;

mod url_preservation;

static FIXTURE_SCHEMA: &str = include_str!("schema/fixture.schema.json");

/// Severity level for validation diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Hard error — fixture is broken and will not produce correct tests.
    Error,
    /// Warning — fixture may not behave as intended.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

/// A validation error with its source file and message.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Relative path of the fixture file that failed validation.
    pub file: String,
    /// Human-readable error message.
    pub message: String,
    /// Severity level.
    pub severity: Severity,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.file, self.message)
    }
}

/// Validate all JSON fixture files in a directory against the fixture schema.
///
/// Returns a list of validation errors. An empty list means all fixtures are valid.
pub fn validate_fixtures(fixtures_dir: &Path) -> Result<Vec<ValidationError>> {
    let mut schema_value: serde_json::Value =
        serde_json::from_str(FIXTURE_SCHEMA).context("failed to parse embedded fixture schema")?;
    let document_validator = jsonschema::validator_for(&schema_value).context("failed to compile fixture schema")?;
    schema_value
        .as_object_mut()
        .context("embedded fixture schema root must be an object")?
        .insert("$ref".to_string(), serde_json::json!("#/$defs/fixture"));
    schema_value
        .as_object_mut()
        .context("embedded fixture schema root must be an object")?
        .remove("oneOf");
    let fixture_validator =
        jsonschema::validator_for(&schema_value).context("failed to compile fixture element schema")?;

    let mut errors = Vec::new();
    validate_recursive(
        fixtures_dir,
        fixtures_dir,
        &document_validator,
        &fixture_validator,
        &mut errors,
    )?;
    Ok(errors)
}

/// Perform semantic validation on loaded fixtures against e2e configuration.
///
/// Checks for:
/// 1. Fixtures skipped for all languages (empty `skip.languages`)
/// 2. Unknown call references not in `[e2e.calls.*]`
/// 3. Categories where all fixtures are skipped (produces 0 test functions)
/// 4. Missing required input fields for the resolved call config
/// 5. (D1) Argument arity and type mismatches in call configs
/// 6. (D2) Field path assertions against simple return types
/// 7. Domain-shaped assertions without required assertion recipes
pub fn validate_fixtures_semantic(
    fixtures: &[Fixture],
    e2e_config: &E2eConfig,
    languages: &[String],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_unsupported_in_languages(e2e_config, languages, &mut errors);

    // Per-fixture checks
    for fixture in fixtures {
        // Check 1: skip-all detection
        // Fixtures in excluded categories are intentionally excluded at the
        // category level; empty skip.languages with no reason is the correct
        // shape there. Do not warn for them.
        if !e2e_config.exclude_categories.contains(&fixture.resolved_category())
            && let Some(skip) = &fixture.skip
            && skip.languages.is_empty()
        {
            let reason = skip.reason.as_deref().unwrap_or("no reason given");
            errors.push(ValidationError {
                file: fixture.source.clone(),
                message: format!(
                    "fixture '{}' is skipped for all languages (skip.languages is empty). Reason: {}",
                    fixture.id, reason
                ),
                severity: Severity::Warning,
            });
        }

        // Check 2: unknown call reference
        if let Some(call_name) = &fixture.call
            && !e2e_config.calls.contains_key(call_name)
        {
            errors.push(ValidationError {
                file: fixture.source.clone(),
                message: format!(
                    "fixture '{}' references unknown call '{}', will fall back to default [e2e.call]",
                    fixture.id, call_name
                ),
                severity: Severity::Error,
            });
        }

        // Check 4: missing required input fields
        // Resolve call using select_when auto-routing (not just explicit fixture.call)
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        for language in languages {
            if let Some(missing) =
                assertion_recipes::missing_recipe_for_language(fixture, call_config, language, e2e_config)
            {
                errors.push(ValidationError {
                    file: fixture.source.clone(),
                    message: format!(
                        "fixture '{}' assertion '{}' requires assertion recipe '{}' for language '{}'",
                        fixture.id, missing.field, missing.recipe, language
                    ),
                    severity: Severity::Error,
                });
            }
        }
        for arg in fixture.resolved_args(call_config) {
            if arg.optional {
                continue;
            }
            // When the arg's field is exactly the top-level "input" path (no dot),
            // the whole fixture.input object IS the JSON value for that arg — no
            // sub-key lookup applies. Only dotted paths like "input.foo" require a
            // specific key to exist inside fixture.input.
            if !arg.field.starts_with("input.") {
                continue;
            }
            let input_field = arg.field.strip_prefix("input.").expect("starts_with checked above");
            if !fixture.input.is_null()
                && let Some(obj) = fixture.input.as_object()
                && !obj.contains_key(input_field)
            {
                // Skip check for error-type assertions (they may intentionally omit fields)
                let is_error_test = fixture.assertions.iter().any(|a| a.assertion_type == "error");
                if !is_error_test {
                    errors.push(ValidationError {
                        file: fixture.source.clone(),
                        message: format!(
                            "fixture '{}' is missing required input field '{}' for call '{}'",
                            fixture.id,
                            input_field,
                            fixture.call.as_deref().unwrap_or("<default>")
                        ),
                        severity: Severity::Warning,
                    });
                }
            }
        }

        // Check 5: `preserve_input_urls` disagrees with the call's mock_url arguments.
        url_preservation::check_preserve_input_urls(fixture, call_config, &mut errors);
    }

    // Check 3: empty categories (all fixtures skipped for all languages)
    if !languages.is_empty() {
        let groups = group_fixtures(fixtures);
        for group in &groups {
            // Categories explicitly excluded from cross-language codegen are
            // expected to produce 0 test functions; do not warn.
            if e2e_config.exclude_categories.contains(&group.category) {
                continue;
            }
            let has_any_non_skipped = group.fixtures.iter().any(|f| {
                match &f.skip {
                    None => true, // no skip → will generate
                    Some(skip) => {
                        // At least one language is NOT skipped
                        languages.iter().any(|lang| !skip.should_skip(lang))
                    }
                }
            });

            if !has_any_non_skipped {
                // Collect all skip reasons from fixtures to see if they're uniform
                let all_have_skip = group.fixtures.iter().all(|f| f.skip.is_some());
                let skip_reasons: Vec<&Option<String>> = if all_have_skip {
                    group
                        .fixtures
                        .iter()
                        .map(|f| &f.skip.as_ref().unwrap().reason)
                        .collect()
                } else {
                    vec![]
                };

                // Check if all fixtures have the same skip reason
                let same_reason = if !skip_reasons.is_empty() {
                    skip_reasons.iter().all(|r| r == skip_reasons.first().unwrap())
                } else {
                    false
                };

                if all_have_skip && same_reason && skip_reasons.first().unwrap().is_some() {
                    // All fixtures skip with the same reason — demote to INFO
                    // Use tracing::info if available; otherwise push as INFO level
                    // For now, we skip adding this to errors so it doesn't appear as a warning
                } else {
                    // Mixed or no reason — report as Error
                    errors.push(ValidationError {
                        file: format!("{}/ (category)", group.category),
                        message: format!(
                            "category '{}' produces 0 test functions — all {} fixture(s) are skipped for all languages",
                            group.category,
                            group.fixtures.len()
                        ),
                        severity: Severity::Error,
                    });
                }
            }
        }
    }

    errors
}

fn validate_unsupported_in_languages(e2e_config: &E2eConfig, languages: &[String], errors: &mut Vec<ValidationError>) {
    if languages.is_empty() {
        return;
    }

    for (call_name, call_config) in &e2e_config.calls {
        for language in call_config.unsupported_in.keys() {
            if !languages.iter().any(|configured| configured == language) {
                errors.push(ValidationError {
                    file: "alef.toml".to_string(),
                    message: format!(
                        "call '{call_name}' marks unsupported language '{language}', but that language is not in the \
                         resolved e2e language set"
                    ),
                    severity: Severity::Error,
                });
            }
        }
    }
}

fn validate_recursive(
    base: &Path,
    dir: &Path,
    document_validator: &jsonschema::Validator,
    fixture_validator: &jsonschema::Validator,
    errors: &mut Vec<ValidationError>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("failed to read directory: {}", dir.display()))?;

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            validate_recursive(base, &path, document_validator, fixture_validator, errors)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip schema files and files starting with _
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }

            let relative = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(ValidationError {
                        file: relative,
                        message: format!("failed to read file: {e}"),
                        severity: Severity::Error,
                    });
                    continue;
                }
            };

            let value: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(ValidationError {
                        file: relative,
                        message: format!("invalid JSON: {e}"),
                        severity: Severity::Error,
                    });
                    continue;
                }
            };

            validate_json_value(&relative, &value, document_validator, fixture_validator, errors);
        }
    }
    Ok(())
}

/// The `alef.toml` file every field-classification diagnostic points at. These entries live in
/// config, not in a fixture, so the `file` slot names the config rather than a fixture path. ~keep
const CONFIG_FILE_LABEL: &str = "alef.toml";

/// Which field-classification table an entry came from, and what it claims about the field.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldClassification {
    /// `fields_optional` — the accessor renderers unwrap this path's leaf as an `Option`.
    Optional,
    /// `fields_array` — the accessor renderers index this path's leaf at `[0]`.
    Array,
}

impl FieldClassification {
    fn table(self) -> &'static str {
        match self {
            Self::Optional => "fields_optional",
            Self::Array => "fields_array",
        }
    }

    /// What the emitted accessor does when the entry is honoured, quoted back to the operator so
    /// the diagnostic explains the compile error the entry would otherwise have caused. ~keep
    fn emitted_effect(self) -> &'static str {
        match self {
            Self::Optional => "an Option unwrap (`.as_ref().unwrap()` / `?.` / `!!` / `.?`)",
            Self::Array => "an element index (`[0]`)",
        }
    }

    /// True when the IR's own shape for a field agrees with what this table claims about it.
    fn agrees_with(self, field: &crate::core::ir::FieldDef) -> bool {
        use crate::core::ir::TypeRef;
        match self {
            // `FieldDef::ty` has already had one `Option<..>` layer peeled off into
            // `FieldDef::optional` (see `extract::extractor::helpers::fields::extract_field`), so
            // both spellings have to be accepted -- hand-built `TypeDef`s keep the wrapper. ~keep
            Self::Optional => field.optional || matches!(field.ty, TypeRef::Optional(_)),
            Self::Array => is_indexable(&field.ty),
        }
    }

    /// Whether this classification is consistent with the ELEMENT a subscripted entry
    /// (`open_graph[title]`, `choices[0]`) reaches on `field`.
    ///
    /// `Optional` clears any subscriptable container: a map key or a list index is a lookup that
    /// can miss, which is precisely what every host binding models as an optional. `Array` has to
    /// look one level deeper — `foo[0]` being itself indexable means `foo` is a nested
    /// collection. ~keep
    fn agrees_with_element_of(self, field: &crate::core::ir::FieldDef) -> bool {
        match element_shape(&field.ty) {
            ElementShape::NotSubscriptable => false,
            ElementShape::Opaque => true,
            ElementShape::Known(inner) => match self {
                Self::Optional => true,
                Self::Array => is_indexable(inner),
            },
        }
    }
}

/// True for the IR shapes an `[0]` index is legal against.
///
/// `Json` and `Map` are in deliberately: a `serde_json::Value` field can carry an array, and
/// `FieldResolver::inject_array_indexing` upgrades a numeric map key on a registered array field
/// to an index. Neither is a mis-declaration the operator could act on. ~keep
fn is_indexable(ty: &crate::core::ir::TypeRef) -> bool {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Vec(_) | TypeRef::Bytes | TypeRef::Json | TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) => is_indexable(inner),
        _ => false,
    }
}

/// What one `[...]` subscript against `ty` lands on.
enum ElementShape<'a> {
    /// The subscript is legal and reaches this type.
    Known(&'a crate::core::ir::TypeRef),
    /// The subscript is legal but the element type is not expressible as a `TypeRef` here
    /// (`Vec<u8>` indexes to a byte). Subscripting is not a mis-declaration, so this clears the
    /// entry the same way `IrAbsent` does rather than manufacturing a diagnostic. ~keep
    Opaque,
    /// `ty` cannot be subscripted at all — the entry's path is wrong, and that IS actionable.
    NotSubscriptable,
}

/// Resolve one `[...]` subscript against an IR type.
fn element_shape(ty: &crate::core::ir::TypeRef) -> ElementShape<'_> {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Vec(inner) | TypeRef::Map(_, inner) => ElementShape::Known(inner),
        // A `serde_json::Value` subscripts to another `Value`, whatever it carries. ~keep
        TypeRef::Json => ElementShape::Known(ty),
        TypeRef::Bytes => ElementShape::Opaque,
        TypeRef::Optional(inner) => element_shape(inner),
        _ => ElementShape::NotSubscriptable,
    }
}

/// What the core IR knows about the leaf field name a classification entry names.
///
/// Mirrors `e2e::codegen::c::assertions::TargetParams` and `e2e::codegen::c::ResultTypeName`:
/// "the IR was never consulted" and "the IR was consulted and had nothing" are different facts,
/// and collapsing them into a bare `Option` is what would let an unverifiable entry be reported
/// as a wrong one. The halves of one rule must agree on what an absent IR licenses. ~keep
enum IrFieldShape<'a> {
    /// The IR declares at least one field with this name — these are every occurrence of it,
    /// across every type. Enough to rule on the entry.
    Known(Vec<&'a crate::core::ir::FieldDef>),
    /// No IR was supplied at all, so nothing was consulted and nothing can be concluded. A
    /// legitimate, common state (unit tests and several snippet entry points generate from empty
    /// IR slices), so it produces no diagnostic rather than refusing. Refusing here would fail
    /// every IR-less caller — a far larger blast radius than the defect being fixed. ~keep
    IrAbsent,
    /// The IR was there to consult and no type declares a field of this name. That is not proof
    /// the entry is wrong: virtual namespace prefixes, streaming pseudo-fields and synthetic
    /// assertion paths all legitimately name things the IR has never heard of, exactly as
    /// `FieldResolver::is_valid_for_result` documents. Unverifiable, so it warns rather than
    /// failing. ~keep
    Unresolvable,
}

/// Every occurrence of `leaf` across the IR's type definitions, or why there is none.
fn ir_field_shape<'a>(leaf: &str, type_defs: &'a [crate::core::ir::TypeDef]) -> IrFieldShape<'a> {
    if type_defs.is_empty() {
        return IrFieldShape::IrAbsent;
    }
    let occurrences: Vec<_> = type_defs
        .iter()
        .flat_map(|type_def| type_def.fields.iter())
        .filter(|field| field.name == leaf)
        .collect();
    if occurrences.is_empty() {
        IrFieldShape::Unresolvable
    } else {
        IrFieldShape::Known(occurrences)
    }
}

/// The field name a classification entry's accessor lands on, plus whether the entry reaches it
/// through a `[0]` / `[]` / `["key"]` subscript.
///
/// The renderers check the FULL prefix path at every segment against `fields_optional` /
/// `fields_array` (see `field_access::optional_renderers::push_key_field_name`), so an entry
/// `metadata.article.tags` is a claim about `tags` — not about `metadata` or `article`. ~keep
///
/// The subscript flag is not cosmetic: `open_graph[title]` says nothing about `open_graph`, it
/// classifies the ELEMENT that subscript reaches. Ruling on the container with the bare-field
/// predicate is what rejected every legal `HashMap<String, String>` key lookup as "contradicts
/// the core IR" — the map is exactly the right home for an optional `[title]`, and exactly the
/// wrong home for an optional bare field. ~keep
fn classification_target(entry: &str) -> (&str, bool) {
    let last = entry.rsplit('.').next().unwrap_or(entry);
    match last.split_once('[') {
        Some((name, _)) => (name.trim(), true),
        None => (last.trim(), false),
    }
}

/// The Rust spelling of an IR field's type, for quoting back in a diagnostic.
fn describe_field_type(field: &crate::core::ir::FieldDef) -> String {
    let inner = describe_type_ref(&field.ty);
    if field.optional {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

fn describe_type_ref(ty: &crate::core::ir::TypeRef) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Primitive(primitive) => format!("{primitive:?}").to_lowercase(),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", describe_type_ref(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", describe_type_ref(inner)),
        TypeRef::Map(key, value) => format!("HashMap<{}, {}>", describe_type_ref(key), describe_type_ref(value)),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "Duration".to_string(),
    }
}

/// Check every `fields_optional` / `fields_array` entry in `e2e_config` against the core IR.
///
/// A wrong entry used to reach the operator as a compiler diagnostic pointed at GENERATED code:
/// `metadata.article.tags` declared optional against a plain `Vec<String>` emits
/// `.as_ref().unwrap().len()` and fails with "type annotations needed", naming a file the
/// operator never wrote and no config line at all. This turns that into one line naming the
/// table, the entry, and the type the IR actually declares.
///
/// The check is on the entry's LEAF field name, and an occurrence of that name that agrees with
/// the entry anywhere in the IR clears it — the same predicate
/// [`FieldResolver::ir_field_sets`] already uses for reachability, for the same reason: a bare
/// field name cannot be pinned to one result type from here, so agrees-on-any-type has to win or
/// a name shared by two structs would produce a false failure. That trade makes this check
/// under-report (an entry wrong on the type it is actually used against still passes when a
/// same-named field elsewhere agrees) and never over-report. ~keep
///
/// [`FieldResolver::ir_field_sets`]: crate::e2e::field_access::FieldResolver::ir_field_sets
pub fn validate_field_classifications(
    e2e_config: &E2eConfig,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    // The same authority `FieldResolver::result_field_oracle_knows`'s
    // `tagged_union_method_call_declares` step anchors via `with_ir_enum_map` before rendering a
    // `fields_method_calls`-covered accessor for the executable e2e generators. Consulted here so
    // this check and those generators read one fact about which leaf names are real tagged-union
    // crossing fields, instead of this check re-deriving that answer from `type_defs` alone -- see
    // `is_declared_union_crossing`. ~keep
    let enum_map = crate::e2e::field_access::FieldResolver::ir_enum_fields(type_defs, enums);
    let method_calls = declared_method_call_crossings(e2e_config);
    let mut tables: Vec<(&std::collections::HashSet<String>, FieldClassification, String)> = vec![
        (
            &e2e_config.fields_optional,
            FieldClassification::Optional,
            "[e2e]".into(),
        ),
        (&e2e_config.fields_array, FieldClassification::Array, "[e2e]".into()),
        (
            &e2e_config.call.fields_optional,
            FieldClassification::Optional,
            "[e2e.call]".into(),
        ),
        (
            &e2e_config.call.fields_array,
            FieldClassification::Array,
            "[e2e.call]".into(),
        ),
    ];
    for (name, call) in &e2e_config.calls {
        let key = format!("[e2e.calls.{name}]");
        tables.push((&call.fields_optional, FieldClassification::Optional, key.clone()));
        tables.push((&call.fields_array, FieldClassification::Array, key));
    }
    for (entries, classification, config_key) in tables {
        check_classification_table(
            entries,
            classification,
            &config_key,
            type_defs,
            &enum_map,
            &method_calls,
            &mut errors,
        );
    }
    errors
}

/// Every `fields_method_calls` entry declared anywhere in `e2e_config` (`[e2e]`, `[e2e.call]`,
/// and every `[e2e.calls.*]`), pooled into one flat set.
///
/// ~keep [`check_classification_table`] is a leaf-name check with no per-call scoping of its
/// own -- it already pools `fields_optional`/`fields_array` the same way across every call --
/// so a crossing declared under one call's override still vouches for the same leaf validated
/// against a different table's entry. Under-scoping this the OTHER way (requiring an exact
/// per-call match) would reintroduce the false "unverified" warning for a consumer whose
/// `fields_method_calls` lives on `[e2e.call]` while the `fields_optional` entry it covers is
/// declared globally on `[e2e]`, or vice versa.
fn declared_method_call_crossings(e2e_config: &E2eConfig) -> std::collections::HashSet<&str> {
    let mut method_calls: std::collections::HashSet<&str> =
        e2e_config.fields_method_calls.iter().map(String::as_str).collect();
    method_calls.extend(e2e_config.call.fields_method_calls.iter().map(String::as_str));
    for call in e2e_config.calls.values() {
        method_calls.extend(call.fields_method_calls.iter().map(String::as_str));
    }
    method_calls
}

/// Whether `entry` is a declared, IR-confirmed tagged-union crossing: the consumer's own
/// `fields_method_calls` names `entry` verbatim, AND `leaf` is a real crossing field name --
/// the single field an `EnumVariant` carries, per
/// [`crate::e2e::field_access::FieldResolver::ir_enum_fields`]'s `variant_payload_types` (built
/// by `ir_enum::build_variant_payload_types`).
///
/// ~keep Both halves are required. The config declaration alone is not enough: trusting it
/// unconditionally would let a misspelled `fields_method_calls` entry that names an ordinary,
/// non-union field silence a genuine "unverified" warning for it -- the exact over-correction
/// `check_classification_table`'s own doc comment warns never happens today ("never
/// over-report"). Requiring the IR to independently confirm the leaf is a real crossing field
/// keeps this additive: it can only clear an entry the IR itself backs, never any entry a
/// config typo merely claims.
///
/// A crossing field's own [`crate::core::ir::FieldDef::optional`] is never `true` -- a Rust
/// tuple-variant field is not itself `Option<T>` -- even though every accessor into it returns
/// `Option<..>` (the enum might not be that variant). `FieldClassification::agrees_with`'s
/// literal `field.optional` check is the wrong question for a crossing field, which is why this
/// clears the entry before that check runs rather than teaching it a second notion of
/// optionality.
///
/// A tuple variant's single field has no source-level name -- `extract_enum_variant`
/// (`extract/extractor/helpers/enum_variants.rs`) synthesizes the placeholder `_0` for it, since
/// Rust never gives it one. Config entries and generated accessors instead spell the crossing
/// after the VARIANT (`metadata.format.excel` for `Excel(ExcelMetadata)`), matching the same
/// `snake_case(variant name)` the generators themselves emit. So `leaf` is checked against BOTH
/// the payload's field name (a real name, for a named-field variant) and the snake-cased variant
/// name it belongs to (the only name a tuple variant's payload has from the outside) -- neither
/// check alone covers both variant shapes, and accepting an unrelated leaf that merely happens to
/// equal some other variant's name would defeat the point, so the variant name checked is always
/// the one that OWNS the matched payload entry, never a same-enum sibling. ~keep
fn is_declared_union_crossing(
    entry: &str,
    leaf: &str,
    enum_map: &crate::e2e::field_access::IrEnumMap,
    method_calls: &std::collections::HashSet<&str>,
) -> bool {
    use heck::ToSnakeCase;

    method_calls.contains(entry)
        && enum_map.variant_payload_types.values().any(|variants| {
            variants
                .iter()
                .any(|(variant_name, (field_name, _))| field_name == leaf || variant_name.to_snake_case() == leaf)
        })
}

fn check_classification_table(
    entries: &std::collections::HashSet<String>,
    classification: FieldClassification,
    config_key: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enum_map: &crate::e2e::field_access::IrEnumMap,
    method_calls: &std::collections::HashSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    let table = classification.table();
    // Sorted so the diagnostics one run emits are stable across `HashSet` iteration order. ~keep
    let mut sorted: Vec<&String> = entries.iter().collect();
    sorted.sort_unstable();
    for entry in sorted {
        let (leaf, subscripted) = classification_target(entry);
        if leaf.is_empty() {
            continue;
        }
        if is_declared_union_crossing(entry, leaf, enum_map, method_calls) {
            continue;
        }
        match ir_field_shape(leaf, type_defs) {
            IrFieldShape::IrAbsent => {}
            IrFieldShape::Unresolvable => errors.push(ValidationError {
                file: CONFIG_FILE_LABEL.to_string(),
                message: format!(
                    "{config_key}.{table} entry `{entry}` is unverified: no type in the core IR declares a \
                     field named `{leaf}`, so alef cannot confirm the classification (expected for virtual \
                     namespace prefixes and synthetic/streaming paths, a typo otherwise)"
                ),
                severity: Severity::Warning,
            }),
            IrFieldShape::Known(occurrences) => {
                let agrees = occurrences.iter().any(|field| {
                    if subscripted {
                        classification.agrees_with_element_of(field)
                    } else {
                        classification.agrees_with(field)
                    }
                });
                if agrees {
                    continue;
                }
                let mut declared: Vec<String> = occurrences.iter().map(|field| describe_field_type(field)).collect();
                declared.sort_unstable();
                declared.dedup();
                let effect = if subscripted {
                    format!(
                        "subscripts it and emits {} against the element",
                        classification.emitted_effect()
                    )
                } else {
                    format!("emits {} against it", classification.emitted_effect())
                };
                errors.push(ValidationError {
                    file: CONFIG_FILE_LABEL.to_string(),
                    message: format!(
                        "{config_key}.{table} entry `{entry}` contradicts the core IR: `{leaf}` is declared as \
                         {} there, and honouring this entry {effect} — remove the entry or fix the path",
                        declared.join(" / "),
                    ),
                    severity: Severity::Error,
                });
            }
        }
    }
}

fn validate_json_value(
    relative: &str,
    value: &serde_json::Value,
    document_validator: &jsonschema::Validator,
    fixture_validator: &jsonschema::Validator,
    errors: &mut Vec<ValidationError>,
) {
    if let Some(fixtures) = value.as_array() {
        for (index, fixture) in fixtures.iter().enumerate() {
            validate_fixture_value(&format!("{relative}[{index}]"), fixture, fixture_validator, errors);
        }
        return;
    }
    validate_fixture_value(relative, value, document_validator, errors);
}

fn validate_fixture_value(
    relative: &str,
    fixture: &serde_json::Value,
    validator: &jsonschema::Validator,
    errors: &mut Vec<ValidationError>,
) {
    for error in validator.iter_errors(fixture) {
        errors.push(ValidationError {
            file: relative.to_string(),
            message: format!("{} at {}", error, error.instance_path()),
            severity: Severity::Error,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::{ArgMapping, CallConfig, CallOverride};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

    /// `FormatMetadata::Excel(ExcelMetadata)` reproduced with the exact shape
    /// `extract_enum_variant` (`extract/extractor/helpers/enum_variants.rs`) actually emits for a
    /// tuple variant: the payload field's name is the synthetic `_0`, never `excel`. Any fixture
    /// that instead names the field `excel` directly would falsify the regression this guards --
    /// the whole defect was that the IR never spells a tuple payload after its variant. ~keep
    fn format_metadata_enum() -> Vec<EnumDef> {
        vec![EnumDef {
            name: "FormatMetadata".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Excel".to_string(),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::Named("ExcelMetadata".to_string()),
                        ..FieldDef::default()
                    }],
                    ..EnumVariant::default()
                },
                EnumVariant {
                    name: "Html".to_string(),
                    fields: vec![FieldDef {
                        name: "_0".to_string(),
                        ty: TypeRef::Named("HtmlMetadata".to_string()),
                        ..FieldDef::default()
                    }],
                    ..EnumVariant::default()
                },
            ],
            ..EnumDef::default()
        }]
    }

    fn config_with_crossing(entry: &str) -> E2eConfig {
        E2eConfig {
            fields_optional: [entry.to_string()].into_iter().collect(),
            fields_method_calls: [entry.to_string()].into_iter().collect(),
            ..Default::default()
        }
    }

    /// The false positive this change fixes: `metadata.format.excel` is a real, generated tuple
    /// -variant crossing (`FormatMetadata::Excel(ExcelMetadata)`), declared correctly in BOTH
    /// `fields_optional` and `fields_method_calls`. Before this fix `is_declared_union_crossing`
    /// compared the synthetic field name `_0` against the leaf `excel` and never matched, so this
    /// produced a spurious "unverified" warning despite the entry being exactly right.
    #[test]
    fn tuple_variant_crossing_by_snake_case_variant_name_produces_no_warning() {
        let config = config_with_crossing("metadata.format.excel");
        let errors = validate_field_classifications(&config, &[], &format_metadata_enum());
        assert!(
            errors.is_empty(),
            "a declared, IR-backed tuple-variant crossing must not warn: {errors:?}"
        );
    }

    /// The negative control that proves the fix above did not just blanket-suppress the check: a
    /// leaf that names no variant of `FormatMetadata` at all (a plausible typo for `excel`) must
    /// still warn, even though it is declared in `fields_method_calls` exactly like the real
    /// entry -- the config declaration alone was never sufficient, the IR still has to back it.
    #[test]
    fn misspelled_variant_leaf_still_produces_unverified_warning() {
        let config = config_with_crossing("metadata.format.exccel");
        let errors = validate_field_classifications(&config, &[], &format_metadata_enum());
        assert_eq!(errors.len(), 1, "a leaf naming no real variant must still warn: {errors:?}");
        assert!(
            errors[0].message.contains("exccel"),
            "warning must name the unresolved leaf: {}",
            errors[0].message
        );
    }

    #[test]
    fn validates_each_fixture_in_top_level_array() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("fixtures.json"),
            r#"[
                {"id": "first_fixture", "description": "first"},
                {
                    "id": "02_second_fixture",
                    "description": "second",
                    "custom_protocol": {"expected": true}
                }
            ]"#,
        )
        .unwrap();

        let errors = validate_fixtures(directory.path()).unwrap();

        assert!(errors.is_empty(), "unexpected validation errors: {errors:?}");
    }

    #[test]
    fn array_validation_identifies_invalid_fixture_index() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("fixtures.json"),
            r#"[
                {"id": "valid_fixture", "description": "valid"},
                {"id": "invalid_fixture"}
            ]"#,
        )
        .unwrap();

        let errors = validate_fixtures(directory.path()).unwrap();

        assert!(!errors.is_empty());
        assert!(errors.iter().all(|error| error.file == "fixtures.json[1]"));
    }

    #[test]
    fn fixture_schema_accepts_current_docs_metadata() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("fixture.json"),
            r#"{
                "id": "documented_fixture",
                "description": "documented",
                "docs": {
                    "topic": "configuration",
                    "paths": {"node": "configuration/example.md"},
                    "presentation": {
                        "call": "process_file",
                        "input": {"source": "guide.txt"},
                        "args": [{"name": "source", "field": "input.source", "type": "string"}],
                        "files": [{"field": "/source", "path": "examples/guide.txt"}],
                        "operations": [
                            {"op": "show", "path": "summary"},
                            {
                                "op": "iterate",
                                "path": "items",
                                "item": "item",
                                "fields": ["text"],
                                "display": true,
                                "optional": true
                            }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();

        let errors = validate_fixtures(directory.path()).unwrap();

        assert!(errors.is_empty(), "unexpected validation errors: {errors:?}");
    }
    use crate::e2e::codegen::assertion_recipes::{EMBEDDINGS_RECIPE, KEYWORDS_RECIPE};
    use crate::e2e::fixture::{Assertion, SkipDirective};

    fn make_fixture(id: &str, source: &str, skip: Option<SkipDirective>, call: Option<&str>) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: id.to_string(),
            category: None,
            description: format!("Test {id}"),
            tags: vec![],
            skip,
            env: None,
            setup: Vec::new(),
            call: call.map(|s| s.to_string()),
            input: serde_json::json!({"path": "test.pdf"}),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: source.to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        }
    }

    fn make_e2e_config(calls: Vec<(&str, CallConfig)>) -> E2eConfig {
        E2eConfig {
            calls: calls.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_skip_all_languages_detected() {
        let fixtures = vec![make_fixture(
            "test_skipped",
            "code/test.json",
            Some(SkipDirective {
                languages: vec![],
                reason: Some("Requires feature X".to_string()),
            }),
            None,
        )];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("skipped for all languages")));
    }

    #[test]
    fn test_unknown_call_detected() {
        let fixtures = vec![make_fixture("test_bad_call", "test.json", None, Some("nonexistent"))];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("unknown call 'nonexistent'")));
    }

    #[test]
    fn test_known_call_not_flagged() {
        let fixtures = vec![make_fixture("test_good_call", "test.json", None, Some("embed"))];
        let config = make_e2e_config(vec![("embed", CallConfig::default())]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(!errors.iter().any(|e| e.message.contains("unknown call")));
    }

    // `"_default"` is not a recognized sentinel for "use [e2e.call]" — the schema says the
    // default is selected by OMITTING `call`, not by naming it. A fixture author who writes
    // `"call": "_default"` gets exactly the same unknown-call error as any other typo, but
    // this test pins the message to name the string, so the failure reads as "you wrote the
    // sentinel wrong" instead of surfacing only through a whole-pipeline gate's generic
    // "unknown call" diagnostic. ~keep
    #[test]
    fn fixture_call_named_default_sentinel_is_still_flagged_as_unknown() {
        let fixtures = vec![make_fixture(
            "test_default_sentinel",
            "test.json",
            None,
            Some("_default"),
        )];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(
            errors
                .iter()
                .any(|e| e.severity == Severity::Error && e.message.contains("unknown call '_default'")),
            "expected an unknown-call error naming '_default'; to use [e2e.call], omit `call` \
             entirely rather than spelling out the sentinel name: {errors:?}"
        );
    }

    #[test]
    fn domain_assertion_without_recipe_is_error() {
        let mut fixture = make_fixture("test_embeddings", "test.json", None, None);
        fixture.assertions = vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("embeddings_valid".to_string()),
            ..Default::default()
        }];
        let config = make_e2e_config(vec![]);

        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);

        assert!(
            errors.iter().any(|e| {
                e.severity == Severity::Error && e.message.contains("requires assertion recipe 'embeddings'")
            }),
            "expected missing embeddings recipe error, got: {errors:?}"
        );
    }

    #[test]
    fn fixture_recipe_allows_domain_assertion() {
        let mut fixture = make_fixture("test_embeddings", "test.json", None, None);
        fixture.assertion_recipes.push(EMBEDDINGS_RECIPE.to_string());
        fixture.assertions = vec![Assertion {
            assertion_type: "is_true".to_string(),
            field: Some("embeddings_valid".to_string()),
            ..Default::default()
        }];
        let config = make_e2e_config(vec![]);

        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);

        assert!(
            !errors.iter().any(|e| e.message.contains("requires assertion recipe")),
            "fixture-level recipe should allow embeddings assertion, got: {errors:?}"
        );
    }

    #[test]
    fn language_override_allows_domain_assertion_for_that_language_only() {
        let mut fixture = make_fixture("test_keywords", "test.json", None, Some("extract"));
        fixture.assertions = vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("keywords".to_string()),
            ..Default::default()
        }];
        let mut call = CallConfig::default();
        let mut python_override = CallOverride::default();
        python_override.assertion_recipes.insert(KEYWORDS_RECIPE.to_string());
        call.overrides.insert("python".to_string(), python_override);
        let config = make_e2e_config(vec![("extract", call)]);

        let errors = validate_fixtures_semantic(&[fixture], &config, &["python".to_string(), "rust".to_string()]);

        assert!(
            !errors.iter().any(|e| e.message.contains("language 'python'")),
            "python override should allow keywords assertion, got: {errors:?}"
        );
        assert!(
            errors.iter().any(|e| e.message.contains("language 'rust'")),
            "rust should still require an explicit recipe, got: {errors:?}"
        );
    }

    #[test]
    fn test_unsupported_in_unknown_language_detected() {
        let mut call = CallConfig::default();
        call.unsupported_in
            .insert("brew".to_string(), "CLI backend cannot pass complex args".to_string());
        let config = make_e2e_config(vec![("interact", call)]);

        let errors = validate_fixtures_semantic(&[], &config, &["rust".to_string()]);

        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("marks unsupported language 'brew'")),
            "unsupported_in for inactive languages should be rejected; got: {:?}",
            errors
        );
    }

    #[test]
    fn test_unsupported_in_resolved_language_is_valid() {
        let mut call = CallConfig::default();
        call.unsupported_in
            .insert("brew".to_string(), "CLI backend cannot pass complex args".to_string());
        let config = make_e2e_config(vec![("interact", call)]);

        let errors = validate_fixtures_semantic(&[], &config, &["rust".to_string(), "brew".to_string()]);

        assert!(
            !errors.iter().any(|e| e.message.contains("marks unsupported language")),
            "unsupported_in should accept active languages; got: {:?}",
            errors
        );
    }

    #[test]
    fn test_empty_category_detected() {
        let fixtures = vec![
            make_fixture(
                "test_a",
                "orphan/a.json",
                Some(SkipDirective {
                    languages: vec![],
                    reason: None, // No reason — error will be raised
                }),
                None,
            ),
            make_fixture(
                "test_b",
                "orphan/b.json",
                Some(SkipDirective {
                    languages: vec![],
                    reason: None, // No reason — error will be raised
                }),
                None,
            ),
        ];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("produces 0 test functions")));
    }

    #[test]
    fn test_missing_required_input_field() {
        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "test_missing".to_string(),
            category: None,
            description: "Test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("extract_bytes".to_string()),
            input: serde_json::json!({"data": "abc"}), // missing "mime_type"
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "test.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let call = CallConfig {
            function: "extract_bytes".to_string(),
            args: vec![
                ArgMapping {
                    name: "data".to_string(),
                    field: "input.data".to_string(),
                    arg_type: "bytes".to_string(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
                ArgMapping {
                    name: "mime_type".to_string(),
                    field: "input.mime_type".to_string(),
                    arg_type: "string".to_string(),
                    optional: false,
                    owned: false,
                    element_type: None,
                    go_type: None,
                    vec_inner_is_ref: false,
                    trait_name: None,
                },
            ],
            ..Default::default()
        };
        let config = make_e2e_config(vec![("extract_bytes", call)]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("missing required input field 'mime_type'"))
        );
    }

    #[test]
    fn test_no_errors_for_valid_fixture() {
        let fixtures = vec![make_fixture("test_valid", "contract/test.json", None, None)];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        // Only check for errors/warnings beyond the expected "missing input" ones
        // (default call config has no args, so no input field checks)
        assert!(errors.is_empty());
    }

    /// Bare `field = "input"` (no dot) must NOT emit a "missing required input
    /// field 'input'" warning — the whole fixture.input IS the arg value.
    #[test]
    fn test_bare_input_field_no_false_positive_warning() {
        use crate::core::config::e2e::ArgMapping;

        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "basic_chat".to_string(),
            category: None,
            description: "Chat completion".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("chat".to_string()),
            input: serde_json::json!({"model": "gpt-4", "messages": []}),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "smoke/basic_chat.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let call = CallConfig {
            function: "chat".to_string(),
            args: vec![ArgMapping {
                name: "request".to_string(),
                // Bare "input" — the whole fixture.input is the arg value
                field: "input".to_string(),
                arg_type: "ChatCompletionRequest".to_string(),
                optional: false,
                owned: true,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..Default::default()
        };
        let config = make_e2e_config(vec![("chat", call)]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            !errors
                .iter()
                .any(|e| e.message.contains("missing required input field 'input'")),
            "bare 'input' field should not produce a false-positive missing-field warning; got: {:?}",
            errors
        );
    }

    #[test]
    fn test_fixture_args_override_missing_field_validation() {
        use crate::core::config::e2e::ArgMapping;

        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "url_batch".to_string(),
            category: None,
            description: "URL batch".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: Some("extract_batch".to_string()),
            input: serde_json::json!({"extract_inputs": []}),
            mock_response: None,
            visitor: None,
            args: vec![ArgMapping {
                name: "inputs".to_string(),
                field: "input.extract_inputs".to_string(),
                arg_type: "json_object".to_string(),
                optional: false,
                owned: true,
                element_type: Some("ExtractInput".to_string()),
                go_type: Some("ExtractInput".to_string()),
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            assertion_recipes: vec![],
            assertions: vec![],
            source: "url/url_batch.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let call = CallConfig {
            function: "extract_batch".to_string(),
            args: vec![ArgMapping {
                name: "inputs".to_string(),
                field: "input.inputs".to_string(),
                arg_type: "json_object".to_string(),
                optional: false,
                owned: true,
                element_type: Some("ExtractInput".to_string()),
                go_type: Some("ExtractInput".to_string()),
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..Default::default()
        };
        let config = make_e2e_config(vec![("extract_batch", call)]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            !errors
                .iter()
                .any(|e| e.message.contains("missing required input field 'inputs'")),
            "fixture-level args should replace call args for missing-field validation; got: {:?}",
            errors
        );
    }

    /// A fixture in an excluded category with empty `skip.languages` must NOT
    /// emit a "skipped for all languages" warning — the exclusion is intentional
    /// at the category level.
    #[test]
    fn test_excluded_category_no_skip_all_warning() {
        use std::collections::HashSet;

        let fixture = Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "budget_enforced".to_string(),
            category: None,
            description: "Budget enforcement test".to_string(),
            tags: vec![],
            skip: Some(SkipDirective {
                languages: vec![], // empty — would normally trigger the warning
                reason: None,
            }),
            env: None,
            setup: Vec::new(),
            call: Some("chat".to_string()),
            input: serde_json::json!({"model": "gpt-4", "messages": []}),
            mock_response: None,
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
            assertions: vec![],
            // resolved_category() derives "budget" from this path
            source: "budget/budget_enforced.json".to_string(),
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
        };
        let mut config = make_e2e_config(vec![]);
        config.exclude_categories = HashSet::from(["budget".to_string()]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            !errors.iter().any(|e| e.message.contains("skipped for all languages")),
            "excluded-category fixture should not trigger skip-all warning; got: {:?}",
            errors
        );
    }
}

#[cfg(test)]
#[path = "field_classification_tests.rs"]
mod field_classification_tests;
