use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Fixture, FixtureDocsOperation};
use heck::ToLowerCamelCase;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct PresentationOperation {
    pub(crate) kind: &'static str,
    pub(crate) expression: String,
    pub(crate) item: String,
    pub(crate) fields: Vec<String>,
    pub(crate) optional: bool,
    pub(crate) display: bool,
    pub(crate) destructure_source: String,
    pub(crate) destructure_item: String,
    /// True when [`Self::expression`] evaluates to an optional/nullable value.
    ///
    /// Distinct from [`Self::optional`], which says the *iterated collection* may be absent and
    /// drives a `?? []`-style guard. This says the value a `show` operation hands to the target
    /// language's print call is itself an optional. Swift needs it because `print`/`debugPrint`
    /// take `Any`, and Swift warns on every implicit optional-to-`Any` coercion — an error under
    /// the `-warnings-as-errors` the snippet validator compiles with. ~keep
    pub(crate) shown_optional: bool,
    /// Per-entry companion to [`Self::fields`], same length and order: whether each iterated
    /// field expression evaluates to an optional. Parallel rather than a struct per field so the
    /// eighteen templates that read `operation.fields` as plain strings keep working unchanged. ~keep
    pub(crate) field_optionals: Vec<bool>,
    /// Per-entry companion to [`Self::fields`], same length and order: whether each iterated
    /// field is safe to format with Rust's `{}` rather than `{:?}`. Only Rust computes this from
    /// the allowlist ([`FieldResolver::is_declared_field_display_safe`]); every other language
    /// gets `*display` for every entry unchanged, matching the pre-existing per-operation flag,
    /// because only the Rust snippet fails to compile against a `Display`-unsafe type. Parallel
    /// rather than a struct per field for the same reason [`Self::field_optionals`] is. ~keep
    pub(crate) field_displays: Vec<bool>,
    /// The `const` name a `show` operation's TypeScript/node narrowing guard binds the crossed
    /// tagged-union value to, or empty when this operation needs no guard.
    ///
    /// See [`FieldResolver::typescript_snippet_variant_guard`] for why a `show` through a
    /// discriminated-union field cannot render as the flat `expression` every other language
    /// uses: `FormatMetadata`'s `.d.ts` is a real discriminated union
    /// (`internal_tagged_union_dts_lines`), and optional chaining does not narrow one, so
    /// `metadata.format.html.title` rendered as `metadata?.format?.html?.title` is a `TS2339` on
    /// every variant but `html`. Non-empty only for node/typescript; every other language leaves
    /// all three of these fields empty and renders `expression` exactly as before. ~keep
    pub(crate) guard_binding: String,
    /// The expression assigned to [`Self::guard_binding`] -- the ordinary accessor for the path
    /// up to (not through) the union field. Empty exactly when `guard_binding` is.
    pub(crate) guard_source: String,
    /// The discriminant check gating [`Self::expression`], e.g. `format?.format_type ===
    /// "html"`. Empty exactly when `guard_binding` is; a template treats a non-empty condition
    /// as the signal to wrap the operation's `console.log` in an `if` block.
    pub(crate) guard_condition: String,
}

/// Clamp every operation to a path the target binding can actually spell, dropping the ones with
/// no spellable form at all.
///
/// ~keep swift-bridge collapses a JSON-bridged field to a single `RustString`, so nothing can be
/// subscripted, indexed, or iterated off it. The e2e generator already refuses exactly those steps
/// — `swift/leaf_shape.rs` asks [`FieldResolver::swift_json_bridged_traversal_prefix`] and writes a
/// skip comment — while the snippet generator asked nothing and emitted `labels()["theme"]`
/// against the very field the e2e file next to it declared unspellable. Two generators, one IR, one
/// field, opposite verdicts. Routing the snippet through the same derivation is what makes them one
/// answer; clamping rather than dropping a `show` lands it on the case that derivation explicitly
/// blesses (a path ending AT the bridged leaf reads fine), so the reader still sees the field.
///
/// Inert for every other language: the Swift first-class map is empty unless the Swift snippet
/// generator built it, and an empty map classifies no field as JSON-bridged.
fn clamp_swift_json_bridged_paths(
    operations: Vec<FixtureDocsOperation>,
    resolver: &FieldResolver,
) -> Vec<FixtureDocsOperation> {
    let mut clamped: Vec<FixtureDocsOperation> = Vec::with_capacity(operations.len());
    for operation in operations {
        let kept = match operation {
            FixtureDocsOperation::Show { path, display } => Some(FixtureDocsOperation::Show {
                path: resolver.swift_json_bridged_traversal_prefix(&path).unwrap_or(path),
                display,
            }),
            // An `iterate` needs elements the `RustString` does not have, and there is no shorter
            // prefix that iterates instead, so the operation goes rather than the tail. ~keep
            FixtureDocsOperation::Iterate { ref path, .. }
                if resolver.swift_json_bridged_iteration_prefix(path).is_some() =>
            {
                None
            }
            other => Some(other),
        };
        // Two `show` paths that differed only past the bridged leaf clamp to the same prefix, and
        // the snippet would otherwise print it twice. ~keep
        if let Some(kept) = kept.filter(|kept| !clamped.contains(kept)) {
            clamped.push(kept);
        }
    }
    clamped
}

/// True when the value an accessor for `path` yields is optional in the target language.
///
/// An optional link anywhere in the chain makes the whole expression optional — `markdown` being
/// `Option<Markdown>` is what makes `result.markdown()?.content()` a `RustString?` even though
/// `content` itself is not optional — so every prefix is consulted, not just the full path. ~keep
fn path_yields_optional(resolver: &FieldResolver, path: &str) -> bool {
    let segments: Vec<&str> = path.split('.').collect();
    (1..=segments.len()).any(|length| resolver.is_optional(&segments[..length].join(".")))
}

/// `type_defs` feeds the same IR-derived optional-field detection every e2e assertion
/// resolver uses (see `FieldResolver::ir_field_sets`/`with_ir_fields`) so a docs snippet
/// that shows an `Option<T>` field renders the same unwrap/null-check an assertion on
/// that field would, instead of a bare (potentially non-compiling) access. ~keep
pub(crate) fn resolve(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Vec<PresentationOperation> {
    if fixture.docs.is_none() {
        return Vec::new();
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let resolver = build_resolver(e2e_config, call, language, type_defs, enums, functions);
    resolve_with(fixture, e2e_config, language, &resolver, type_defs, enums, functions)
}

/// The bare, IR-backed resolver [`resolve`] answers with. Shared with [`apply_derived_shows`] so
/// the paths written into `docs.shows` are decided by exactly the resolver that will later
/// render them, not by a second construction that could drift.
fn build_resolver(
    e2e_config: &E2eConfig,
    call: &crate::core::config::e2e::CallConfig,
    language: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> FieldResolver {
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    anchor_to_declared_result_type(
        FieldResolver::new(
            e2e_config.effective_fields(call),
            e2e_config.effective_fields_optional(call),
            e2e_config.effective_result_fields(call),
            e2e_config.effective_fields_array(call),
            e2e_config.effective_fields_method_calls(call),
        )
        .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields),
        call,
        language,
        type_defs,
        enums,
        functions,
    )
}

/// Attach the field facts of the call's OWN declared result type to `resolver`.
///
/// ~keep Everything a snippet needs to know about a field — may it be absent, is it a member of
/// the result at all — is a fact about one specific type, but `FieldResolver`'s IR sets are keyed
/// by bare name across the whole crate, because nothing had ever handed this layer the identity
/// of the type under generation. `resolve_declared_result_type` is that identity, and it is the
/// same anchor `IrEnumMap`/`IrCollectionMap` already resolve for the same reason. A call whose
/// return type does not resolve (no `functions`/`type_defs` in scope, an unresolvable name,
/// disagreeing same-named methods) yields a `None` root, which leaves every anchored answer
/// disabled and the pre-existing flat behaviour exactly intact.
fn anchor_to_declared_result_type(
    resolver: FieldResolver,
    call: &crate::core::config::e2e::CallConfig,
    language: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> FieldResolver {
    let root_type = crate::e2e::codegen::call_ir::resolve_declared_result_type(
        call,
        language,
        crate::e2e::codegen::call_ir::CallIr { functions, type_defs },
    );
    let result_fields = FieldResolver::ir_result_field_facts(type_defs, language);
    let resolver = resolver.with_ir_result_fields(result_fields, root_type.clone());
    // Only `ir_result_field_map` was ever anchored here; every per-language assertion resolver
    // ALSO anchors `ir_collection_map` and `ir_enum_map` at construction, but this shared
    // snippet/docs resolver never did — `ir_enum_map` stayed the all-default `IrEnumMap`, so
    // `FieldResolver::result_field_oracle_knows`'s `tagged_union_method_call_declares` step (which
    // reads `enum_type_at_path(&self.ir_enum_map, ..)`) could never resolve the union type at a
    // `fields_method_calls`-covered crossing, even though that same oracle already knows how to
    // answer once the map is anchored — every per-language assertion resolver anchors it via this
    // exact call, `FieldResolver::ir_enum_fields(type_defs, enums)`. Without it here, a docs
    // snippet dropped every field reached through a tagged-union crossing, and `validate_authored_operations`
    // (via `authored_shows_on_result` -> `ir_permits_result_path` -> `result_field_oracle_knows`)
    // refused the very paths the consumer's own `fields_method_calls` declared and the executable
    // e2e generators already rendered correctly. ~keep
    let resolver = resolver.with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), root_type.clone());
    // `validate_authored_operations` needs the collection map to resolve an `Iterate` operation's
    // loop-item type (`FieldResolver::collection_element_type`), so it has to be anchored here
    // too, not re-derived at the one call site that needs it. ~keep
    resolver.with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), root_type)
}

/// `resolver` re-anchored to the element type of the collection at `path`, for resolving an
/// `Iterate` operation's per-item `fields` -- which name members of that element type, never of
/// `resolver`'s own result-type anchor. Falls back to `resolver` unchanged when the element type
/// does not resolve (an unresolvable collection path), matching the existing permissive default
/// [`iterate_field_is_renderable`] uses for the same lookup.
fn resolver_anchored_at_element(
    resolver: &FieldResolver,
    path: &str,
    language: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> FieldResolver {
    let Some(element_type) = resolver.collection_element_type(path) else {
        return resolver.clone();
    };
    let result_fields = FieldResolver::ir_result_field_facts(type_defs, language);
    let resolver = resolver
        .clone()
        .with_ir_result_fields(result_fields, Some(element_type.clone()));
    let resolver = resolver.with_ir_enum_map(
        FieldResolver::ir_enum_fields(type_defs, enums),
        Some(element_type.clone()),
    );
    resolver.with_ir_collection_map(FieldResolver::ir_collection_fields(type_defs), Some(element_type))
}

/// Write the field paths [`resolve`] would derive from `fixture`'s own assertions into the
/// fixture's `docs.shows`, so that [`Fixture::has_docs_presentation`] reports them.
///
/// ~keep That predicate is the single question the *call emitter* asks about the *snippet's*
/// intent: `rust/test_file/test_function.rs` uses it to decide both whether the call binds a
/// named `result` at all (rather than `let _ =`) and whether a `Result`-returning call is
/// unwrapped before that binding. Before this, it only ever saw hand-authored
/// `shows`/`presentation` blocks, so the operations #199 derives from assertions were invisible
/// to it: 283 generated Rust snippets in one consumer repo bound `let _ = convert(...)` and then
/// printed `result.content` (`E0425`), and any that had bound it would have field-accessed a
/// `Result` (`E0609`). Materializing the derivation into the fixture — rather than teaching the
/// call emitter to re-derive it — is what keeps the two generators reading one fact.
///
/// A fixture that already hand-authors `shows` or `presentation.operations` is left alone: its
/// operations are the authored ones, and `has_docs_presentation` already reports them. Must be
/// called before any caller clears `assertions`, which is where the derivation reads from.
pub(crate) fn apply_derived_shows(
    fixture: &mut Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) {
    if fixture.docs.is_none() || fixture.has_docs_presentation() {
        return;
    }
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let resolver = build_resolver(e2e_config, call, language, type_defs, enums, functions);
    let paths: Vec<String> = default_operations_from_assertions(fixture, call, language, &resolver)
        .into_iter()
        .filter_map(|operation| match operation {
            FixtureDocsOperation::Show { path, .. } => Some(path),
            FixtureDocsOperation::Iterate { .. } => None,
        })
        .collect();
    if paths.is_empty() {
        return;
    }
    if let Some(docs) = fixture.docs.as_mut() {
        docs.shows = paths;
    }
}

/// [`resolve`], but against a caller-supplied [`FieldResolver`].
///
/// Two backends cannot use the bare [`FieldResolver::new`] resolver that [`resolve`]
/// builds, because their accessor syntax is decided by a per-type classification the
/// bare resolver does not carry:
///
/// - Swift dispatches property (`result.text`) vs. swift-bridge method (`result.text()`)
///   syntax on a [`SwiftFirstClassMap`]; an empty map classifies every type as opaque,
///   so every accessor would gain a spurious `()`.
/// - PHP dispatches property (`$result->text`) vs. getter (`$result->getText()`) syntax
///   on a [`PhpGetterMap`]; an empty map emits property syntax for the non-scalar fields
///   that ext-php-rs only exposes through a getter.
///
/// [`SwiftFirstClassMap`]: crate::e2e::field_access::SwiftFirstClassMap
/// [`PhpGetterMap`]: crate::e2e::field_access::PhpGetterMap
pub(crate) fn resolve_with(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    language: &str,
    resolver: &FieldResolver,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    functions: &[crate::core::ir::FunctionDef],
) -> Vec<PresentationOperation> {
    let Some(docs) = fixture.docs.as_ref() else {
        return Vec::new();
    };
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    // The two callers that supply their own resolver (php's getter map, swift's first-class map)
    // build it from config alone, so the anchoring has to be applied here rather than at each
    // construction site -- one place decides what a snippet knows about its result type. ~keep
    let resolver = &anchor_to_declared_result_type(resolver.clone(), call, language, type_defs, enums, functions);
    let result_var = call.effective_result_var();
    let result_root = root_variable(language, result_var);
    let operations = docs
        .shows
        .iter()
        .cloned()
        // `docs.shows` is the shorthand form and carries no formatting choice, so it keeps
        // the debug-formatted default; only `presentation.operations` can opt in. ~keep
        .map(|path| FixtureDocsOperation::Show { path, display: false })
        .chain(
            docs.presentation
                .iter()
                .flat_map(|presentation| presentation.operations.iter().cloned()),
        )
        .collect::<Vec<_>>();
    // A hand-authored `shows`/`presentation.operations` entry never went through
    // `default_operations_from_assertions`'s IR check at all -- `shows_on_result` was the ONLY
    // gate on a field path reaching this snippet, and it only ever saw derived paths. Applying it
    // here too is what makes a fixture author's own typo (spelling a renamed field the way it used
    // to be spelled) get caught the same way a derivation mistake already was, instead of reaching
    // five language backends' compilers as the first check. ~keep
    let operations = validate_authored_operations(operations, fixture, call, language, resolver);
    // A fixture-driven docs entry (the common shape: authored once as `assertions`, never
    // hand-annotated with `shows`/`presentation`) has no explicit field list here, but its
    // `assertions` already name the exact result fields it checks -- the same field paths the
    // e2e assertion resolver renders `assert_eq!`/`assertEquals`/etc. against. Reading that
    // existing data instead of leaving the snippet at a bare `print(result)` is what turns
    // "call the function" into "here is how you use what it returns", without a second,
    // independently-derived notion of which fields exist on the result. ~keep
    let operations = if operations.is_empty() {
        default_operations_from_assertions(fixture, call, language, resolver)
    } else {
        operations
    };
    let operations = clamp_swift_json_bridged_paths(operations, resolver);
    // Rust is the only backend a display-unsafe type actually fails to compile against: Go's
    // `%v`, Zig's `{any}`, Swift's `print`, Ruby's `puts`, and PHP/R's equivalents all accept
    // any value, so only the Rust snippet needs the downgrade. See
    // `downgrade_display_unsafe_operations`. ~keep
    let operations = if language == "rust" {
        downgrade_display_unsafe_operations(operations, resolver, &fixture.id)
    } else {
        operations
    };
    // Only now are the paths this snippet will render known, and the accessor renderers read
    // optionality out of a path set rather than by asking a question -- so the anchored answer
    // for exactly these paths has to be materialised into that set before anything renders. An
    // `Iterate` operation's per-item `fields` are deliberately left out: they are rooted at the
    // loop variable, not at the result, so the result type is the wrong anchor for them. ~keep
    let resolver = &resolver
        .clone()
        .with_anchored_optional_paths(operations.iter().map(|operation| match operation {
            FixtureDocsOperation::Show { path, .. } | FixtureDocsOperation::Iterate { path, .. } => path.as_str(),
        }));
    operations
        .iter()
        .map(|operation| match operation {
            FixtureDocsOperation::Show { path, display } => {
                let guard = resolver.typescript_snippet_variant_guard(path, language, &result_root);
                let expression = guard
                    .as_ref()
                    .map(|(_, _, _, expression)| expression.clone())
                    .unwrap_or_else(|| resolver.accessor(path, language, &result_root));
                let (guard_binding, guard_source, guard_condition) = guard
                    .map(|(binding, source, condition, _)| (binding, source, condition))
                    .unwrap_or_default();
                PresentationOperation {
                    kind: "show",
                    expression,
                    item: String::new(),
                    fields: Vec::new(),
                    optional: false,
                    display: *display,
                    destructure_source: String::new(),
                    destructure_item: String::new(),
                    shown_optional: path_yields_optional(resolver, path),
                    field_optionals: Vec::new(),
                    field_displays: Vec::new(),
                    guard_binding,
                    guard_source,
                    guard_condition,
                }
            }
            FixtureDocsOperation::Iterate {
                path,
                item,
                fields,
                display,
                optional,
            } => {
                let (destructure_source, destructure_item, expression) =
                    typescript_first_item(path, language, resolver, &result_root);
                let item_root = root_variable(language, item);
                let field_displays = iterate_field_displays(fields, *display, path, language, resolver, &fixture.id);
                // `fields` is rooted at the loop variable, not the call's result, so it must be
                // resolved against `path`'s element type -- never against `resolver`, which stays
                // anchored to the result. Reusing `resolver` here rendered `item.results?.[0]?.content`
                // for a per-item `content` field: the result-anchored root type (`ExtractionResult`)
                // does declare `content` at `results[].content`, so the still-result-anchored resolver
                // dutifully reproduced that whole path underneath the already-peeled loop variable, in
                // every backend that shares this one presentation layer. ~keep
                let item_resolver = resolver_anchored_at_element(resolver, path, language, type_defs, enums);
                PresentationOperation {
                    kind: "iterate",
                    expression,
                    item: item.clone(),
                    fields: fields
                        .iter()
                        .map(|field| item_resolver.accessor(field, language, &item_root))
                        .collect(),
                    // A fixture's own `optional` flag is authored by hand and can
                    // drift from the field-optionality data already known to the
                    // resolver (`fields_optional` in the e2e config). When the
                    // resolver knows the iterated path is optional but the fixture
                    // wasn't updated to say so, trusting only `*optional` emits a
                    // bare `for (const x of first?.optionalField)` with no `?? []`
                    // guard -- a TS18048 in strict mode. OR the two signals so a
                    // stale fixture flag can't regress a snippet that alef already
                    // has the type information to render safely.
                    optional: *optional || resolver.is_optional(path),
                    display: *display,
                    destructure_source,
                    destructure_item,
                    shown_optional: false,
                    field_optionals: fields
                        .iter()
                        .map(|field| path_yields_optional(&item_resolver, field))
                        .collect(),
                    field_displays,
                    guard_binding: String::new(),
                    guard_source: String::new(),
                    guard_condition: String::new(),
                }
            }
        })
        .collect()
}

/// Refuse a Rust `display: true` whose resolved path targets a type alef cannot vouch for as
/// implementing `Display`, falling back to the debug formatter instead of letting
/// `rust/snippet_body.rs.jinja` emit `println!("{}", ...)` against it.
///
/// `extract` discards every `impl Display for X` before it reaches the IR (`Display` is one of
/// `STD_TRAITS` in `extract::extractor::functions::impl_blocks`), so `display: true` on a fixture
/// is a hand-authored claim alef has no way to check against the real Rust type — a struct or
/// enum with no derived/hand-written `Display` compiles fine with `{:?}` and not at all with
/// `{}`. This turns that compile failure into a `tracing::warn!` naming the fixture and path,
/// and keeps the snippet compiling by rendering the same debug output every fixture without the
/// flag already gets.
///
/// Only `Show` and a `fields`-less `Iterate` (which prints the raw item, not a per-item field)
/// are checked here: an `Iterate`'s per-item `fields` are rooted at the loop variable, not the
/// anchored result type [`resolve_with`] built `resolver` against, so [`FieldResolver::is_display_unsafe`]
/// — which walks from the anchored result root — has no answer for them. [`iterate_field_displays`]
/// is the separate, per-field answer for that case, computed where the loop item's own element
/// type is in scope (`resolve_with`'s final `.map()`), not here. ~keep
fn downgrade_display_unsafe_operations(
    operations: Vec<FixtureDocsOperation>,
    resolver: &FieldResolver,
    fixture_id: &str,
) -> Vec<FixtureDocsOperation> {
    operations
        .into_iter()
        .map(|operation| match operation {
            FixtureDocsOperation::Show { path, display: true } if resolver.is_display_unsafe(&path) => {
                warn_display_unsafe(fixture_id, &path);
                FixtureDocsOperation::Show { path, display: false }
            }
            FixtureDocsOperation::Iterate {
                path,
                item,
                fields,
                display: true,
                optional,
            } if fields.is_empty() && resolver.is_display_unsafe(&path) => {
                warn_display_unsafe(fixture_id, &path);
                FixtureDocsOperation::Iterate {
                    path,
                    item,
                    fields,
                    display: false,
                    optional,
                }
            }
            other => other,
        })
        .collect()
}

fn warn_display_unsafe(fixture_id: &str, path: &str) {
    tracing::warn!(
        target: "alef::e2e::presentation",
        fixture = fixture_id,
        path,
        "fixture `{fixture_id}` sets `display: true` on `{path}`, but its resolved type is a \
         struct/enum alef cannot confirm implements `Display` (extract does not record `Display` \
         impls). Falling back to the debug formatter so the generated Rust snippet still \
         compiles -- if `{path}`'s type genuinely implements `Display`, this warning cannot be \
         resolved from the fixture alone."
    );
}

/// Per-field companion to [`downgrade_display_unsafe_operations`]: whether each of an `Iterate`
/// operation's per-item `fields` may render with Rust's `{}` rather than `{:?}`.
///
/// Only Rust computes this from the allowlist; a `display: true` fixture rendered for any other
/// language keeps every entry `true` — the pre-existing per-operation behaviour — because only
/// the Rust snippet fails to compile against a `Display`-unsafe type (see
/// [`downgrade_display_unsafe_operations`]'s own gate on `language == "rust"`). When the
/// operation itself did not request `display: true`, every entry is `false` without consulting
/// the allowlist at all, matching the template's pre-existing behaviour of formatting every
/// field with `{:?}` in that case.
fn iterate_field_displays(
    fields: &[String],
    display: bool,
    collection_path: &str,
    language: &str,
    resolver: &FieldResolver,
    fixture_id: &str,
) -> Vec<bool> {
    if !display {
        return fields.iter().map(|_| false).collect();
    }
    if language != "rust" {
        return fields.iter().map(|_| true).collect();
    }
    let element_type = resolver.collection_element_type(collection_path);
    fields
        .iter()
        .map(|field| {
            let safe = element_type
                .as_deref()
                .is_some_and(|element_type| resolver.is_declared_field_display_safe(element_type, field));
            if !safe {
                warn_iterate_field_display_unsafe(fixture_id, collection_path, field);
            }
            safe
        })
        .collect()
}

fn warn_iterate_field_display_unsafe(fixture_id: &str, collection_path: &str, field: &str) {
    tracing::warn!(
        target: "alef::e2e::presentation",
        fixture = fixture_id,
        collection_path,
        field,
        "fixture `{fixture_id}` sets `display: true` while iterating `{collection_path}`, but \
         per-item field `{field}` is not a `String`/`char`/numeric/`bool` primitive alef can \
         positively confirm implements `Display`. Falling back to the debug formatter for this \
         field so the generated Rust snippet still compiles -- if `{field}`'s type genuinely \
         implements `Display`, this warning cannot be resolved from the fixture alone."
    );
}

/// Default field-access operations for a docs-tagged fixture whose `docs.shows` and
/// `docs.presentation.operations` are both empty.
///
/// Every generated assertion already anchors on `Assertion::field`, so the field paths a
/// fixture cares about are known even when nobody hand-authored a `shows` list for the docs
/// snippet. This derives one `show` per distinct field, in first-appearance order, from
/// exactly that same data -- deliberately not re-deriving field names from the IR or the
/// input shape, which would let this and the assertion resolver disagree about what fields a
/// result has. Assertions with no `field` (method-result checks, `error` assertions) name
/// nothing to show and are skipped; a void call has no result to access at all. ~keep
///
/// A derived path is only shown when the assertion renderer would itself have rendered a member
/// access on the result for it. `Assertion::field` is not a promise that the name is a member of
/// the return type — three whole classes of assertion name something else, and 0.67.2 emitted a
/// non-compiling accessor for every one of them:
///
/// * an **error-path fixture**. Every backend's error block renders the must-fail check and
///   returns without visiting another assertion (that is the entire premise of
///   [`error_path_assertions`]), so `error.status_code` is a claim about the raised error, never
///   about a result — and on the success path there is no result to show anyway.
/// * a **non-struct result**. When `result_is_simple`/`result_is_bytes` holds for this language,
///   the field is a pseudo-field naming the buffer or scalar itself, exactly as
///   `java/assertions.rs`'s byte-buffer arm documents; the snippet falls back to showing the
///   whole result.
/// * a **name the availability oracle does not recognize**. Both halves are needed:
///   [`FieldResolver::is_valid_for_result`] rejects what the oracle positively excludes, and
///   [`FieldResolver::result_field_oracle_knows`] additionally rejects what it has simply never
///   heard of — an assertion *grouping* like `rate_limit.` or a streaming pseudo-field is not a
///   member path, and defaulting an unrecognized name to "valid" is right for an authored
///   assertion but wrong for an inferred accessor. See that method for the asymmetry.
///
/// Rejection falls back to no operation, i.e. the pre-#199 whole-result display — never to a
/// guess. ~keep
///
/// [`error_path_assertions`]: crate::e2e::codegen::error_path_assertions
fn default_operations_from_assertions(
    fixture: &Fixture,
    call: &crate::core::config::e2e::CallConfig,
    language: &str,
    resolver: &FieldResolver,
) -> Vec<FixtureDocsOperation> {
    if call.returns_void
        || call.effective_result_is_simple(language)
        || call.effective_result_is_bytes(language)
        || fixture.assertions.iter().any(|a| a.assertion_type == "error")
    {
        return Vec::new();
    }
    // ~keep A streaming fixture's `chunks`/`stream_content` assertions name a locally collected
    // list, not a member of the result, so no accessor may be derived for them. A NON-streaming
    // fixture whose result type genuinely declares a field of one of those names is the opposite
    // case: rejecting the name by spelling alone dropped `result.chunks` from 52 snippets in one
    // consumer's suite while its 16 e2e files kept asserting on the very same field.
    // `resolve_is_streaming` is the call-scoped question every assertion renderer already gates
    // its streaming branch on, so both generators answer it once and cannot disagree.
    let fixture_is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    let mut seen_fields = Vec::new();
    fixture
        .assertions
        .iter()
        .filter_map(|assertion| assertion.field.as_deref())
        .filter(|field| shows_on_result(field, resolver, fixture_is_streaming, language))
        .filter(|field| {
            let is_new = !seen_fields.contains(field);
            if is_new {
                seen_fields.push(*field);
            }
            is_new
        })
        .map(|field| FixtureDocsOperation::Show {
            path: field.to_string(),
            display: false,
        })
        .collect()
}

/// Whether a derived field path names a member of the call's result, per the oracles the
/// assertion renderers already consult. See [`default_operations_from_assertions`].
///
/// A refusal is silent unless the consumer's own `alef.toml` declared the path. That case is
/// config drift — a field path claimed by hand against a result type that does not declare it in
/// this target — and it is fixable only in the consuming repo, so it is reported rather than
/// swallowed. A warning, never a failure: the same path can be perfectly reachable in another
/// target (a Dart freezed union exposes no accessor a PyO3 class does), and refusing to build one
/// target's docs over a per-target shape difference would be worse than the missing line. ~keep
///
/// The streaming pseudo-field rejection is conditional on the fixture being a STREAMING fixture.
/// A non-streaming result type may genuinely declare a field spelled `chunks`; rejecting that name
/// by spelling alone dropped `result.chunks` from 52 snippets in one consumer's suite while its
/// e2e files kept asserting on the very same field. ~keep
fn shows_on_result(field: &str, resolver: &FieldResolver, fixture_is_streaming: bool, language: &str) -> bool {
    if !path_is_renderable_at_all(field, fixture_is_streaming) {
        return false;
    }
    if !resolver.is_valid_for_result(field) {
        return false;
    }
    ir_permits_result_path(field, resolver, language)
}

/// The authored-path counterpart of [`shows_on_result`]: the same IR refutation, without
/// [`FieldResolver::is_valid_for_result`]'s membership test against the `[e2e].result_fields`
/// config list.
///
/// `result_fields` is an incomplete allow-list by construction -- a real struct segment the config
/// omits is still a real field, which is why the resolver already declines to reject one. Applying
/// it to a hand-authored `docs.shows` would drop exactly the deliberately-documented virtual and
/// namespaced paths the author wrote it for, which is what
/// `a_hand_authored_shows_entry_is_never_filtered` protects. The IR is the authority that can
/// refute an authored path, and it answers in three states: only a positive `Some(false)` -- "this
/// result type exists and has no such member" -- drops the entry. `None` ("no IR wired up for this
/// call") leaves it alone, the same "no answer, don't reject" rule
/// [`iterate_field_is_renderable`] applies to an unresolvable element type. ~keep
fn authored_shows_on_result(field: &str, resolver: &FieldResolver, fixture_is_streaming: bool, language: &str) -> bool {
    if !path_is_renderable_at_all(field, fixture_is_streaming) {
        return false;
    }
    ir_permits_result_path(field, resolver, language)
}

/// Neither an empty path nor a streaming-only virtual field can be shown on a result value,
/// whoever wrote the path.
fn path_is_renderable_at_all(field: &str, fixture_is_streaming: bool) -> bool {
    !field.is_empty()
        && !(fixture_is_streaming && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(field))
}

/// Whether the IR declines to refute `field` as a member of the call's result type. `false` only
/// when the oracle positively knows the result type and positively lacks the member.
fn ir_permits_result_path(field: &str, resolver: &FieldResolver, language: &str) -> bool {
    if !resolver.has_ir_result_evidence() || resolver.result_field_oracle_knows(field) != Some(false) {
        return true;
    }
    if let Some(config_key) = resolver.declaring_config_key(field) {
        tracing::warn!(
            target: "alef::e2e::presentation",
            field,
            language,
            config_key,
            "`{field}` is declared in `[e2e].{config_key}` but the `{language}` binding's result \
             type has no such member, so the documentation snippet omits it. Correct the path \
             or drop it from `{config_key}`."
        );
    }
    false
}

/// Drop a hand-authored `docs.shows`/`docs.presentation.operations` entry the IR cannot vouch
/// for, and trim an `Iterate`'s per-item `fields` list the same way.
///
/// [`default_operations_from_assertions`] already runs every path it derives through
/// [`shows_on_result`]; an EXPLICIT operation skipped that check entirely before this function
/// existed, because `resolve_with` only reached the derivation path when `operations` came back
/// empty. A fixture author who spelled a renamed field the way it used to be spelled, or wrote a
/// path through a tagged-union field `accessor()` cannot walk any further into, therefore got
/// exactly the accessor spelled -- uncaught until the per-language snippet validator compiled it
/// and failed identically in every backend sharing this one resolved path.
///
/// `Iterate`'s per-item `fields` are checked against the collection path's OWN element type
/// (`FieldResolver::collection_element_type`), never the call's result type: they are rooted at
/// the loop variable, the same reason `default_operations_from_assertions` declines to derive
/// `Iterate` operations at all. A field the element type can't be resolved for is left alone --
/// "no answer, don't reject" -- rather than dropped for want of an anchor `resolve_with` never
/// needed before.
fn validate_authored_operations(
    operations: Vec<FixtureDocsOperation>,
    fixture: &Fixture,
    call: &crate::core::config::e2e::CallConfig,
    language: &str,
    resolver: &FieldResolver,
) -> Vec<FixtureDocsOperation> {
    let fixture_is_streaming =
        crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, call.streaming_enabled());
    operations
        .into_iter()
        .filter_map(|operation| match operation {
            FixtureDocsOperation::Show { path, display } => {
                let renderable = authored_shows_on_result(&path, resolver, fixture_is_streaming, language);
                renderable.then_some(FixtureDocsOperation::Show { path, display })
            }
            FixtureDocsOperation::Iterate {
                path,
                item,
                fields,
                display,
                optional,
            } => {
                if !authored_shows_on_result(&path, resolver, fixture_is_streaming, language) {
                    return None;
                }
                let element_type = resolver.collection_element_type(&path);
                let fields = fields
                    .into_iter()
                    .filter(|field| iterate_field_is_renderable(element_type.as_deref(), field, resolver, &path))
                    .collect();
                Some(FixtureDocsOperation::Iterate {
                    path,
                    item,
                    fields,
                    display,
                    optional,
                })
            }
        })
        .collect()
}

/// Whether an `Iterate` operation's per-item `field` is a real member of `element_type` — or, when
/// `element_type` could not be resolved, the pre-existing permissive default.
fn iterate_field_is_renderable(
    element_type: Option<&str>,
    field: &str,
    resolver: &FieldResolver,
    collection_path: &str,
) -> bool {
    let Some(element_type) = element_type else {
        return true;
    };
    match resolver.is_declared_field_of_type(element_type, field) {
        Some(false) => {
            tracing::warn!(
                target: "alef::e2e::presentation",
                collection_path,
                element_type,
                field,
                "fixture iterates `{collection_path}` and shows per-item field `{field}`, but \
                 `{element_type}` has no such member. Dropping the field rather than emitting a \
                 non-compiling accessor -- correct the field name in the fixture's `docs` block."
            );
            false
        }
        _ => true,
    }
}

/// The root variable an accessor chain is anchored on, spelled the way the target
/// language spells a variable reference.
///
/// PHP is the only backend whose variables carry a sigil, and the sigil has to be part
/// of the root handed to `FieldResolver::accessor` rather than prepended in the
/// template: `render_php` wraps a trailing `.length` segment as `count(<chain>)`, so a
/// template-side `$` would land outside the call (`$count(...)`) instead of on the
/// variable. Matches `php::assertions`, which passes `format!("${result_var}")`. ~keep
fn root_variable(language: &str, name: &str) -> String {
    if language == "php" {
        format!("${name}")
    } else {
        name.to_string()
    }
}

fn typescript_first_item(
    path: &str,
    language: &str,
    resolver: &FieldResolver,
    result_var: &str,
) -> (String, String, String) {
    if matches!(language, "node" | "wasm")
        && let Some((source, tail)) = path.split_once("[0].")
    {
        let source = resolver.accessor(source, language, result_var);
        // `tail` names a field on the destructured `first` item. Both node (napi-rs) and wasm
        // expose struct fields camelCased (napi's default `#[napi(object)]` derive; wasm's
        // `to_node_name` — see `gen_getter` in backends/wasm/gen_bindings/types.rs), never the
        // fixture's snake_case IR/wire name, so splicing `tail` in verbatim referenced a member
        // neither binding declares whenever the path's tail segment wasn't already camelCase by
        // coincidence (e.g. a fixture path of `results[0].extracted_keywords` produced
        // `first?.extracted_keywords` against a binding that only exports `.extractedKeywords`).
        // Only the field-name casing is fixed here, per segment; the unconditional `?.`/`?? []`
        // guarding this function already applies to `source` is left exactly as-is rather than
        // rerouted through `resolver`'s own (narrower) optionality detection, which is a
        // separate, unverified behavior change. ~keep
        let tail_camel = tail
            .split('.')
            .map(|segment| segment.to_lower_camel_case())
            .collect::<Vec<_>>()
            .join(".");
        return (
            format!("{source} ?? []"),
            "first".into(),
            format!("first?.{tail_camel}"),
        );
    }
    (
        String::new(),
        String::new(),
        resolver.accessor(path, language, result_var),
    )
}

#[cfg(test)]
#[path = "presentation/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "presentation/derived_show_resolution_tests.rs"]
mod derived_show_resolution_tests;

#[cfg(test)]
#[path = "presentation/anchored_result_facts_tests.rs"]
mod anchored_result_facts_tests;

#[cfg(test)]
#[path = "presentation/deep_result_path_tests.rs"]
mod deep_result_path_tests;

#[cfg(test)]
#[path = "presentation/wasm_optional_leaf_field_tests.rs"]
mod wasm_optional_leaf_field_tests;

#[cfg(test)]
#[path = "presentation/node_wasm_iterate_tail_casing_tests.rs"]
mod node_wasm_iterate_tail_casing_tests;

#[cfg(test)]
#[path = "presentation/authored_operation_validation_tests.rs"]
mod authored_operation_validation_tests;

#[cfg(test)]
#[path = "presentation/iterate_field_display_safety_tests.rs"]
mod iterate_field_display_safety_tests;

#[cfg(test)]
#[path = "presentation/iterate_element_anchor_tests.rs"]
mod iterate_element_anchor_tests;

#[cfg(test)]
#[path = "presentation/tagged_union_crossing_tests.rs"]
mod tagged_union_crossing_tests;
