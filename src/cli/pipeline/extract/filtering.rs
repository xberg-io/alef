use crate::core::config::{ExcludeConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeDef, TypeRef, UnsupportedPublicItem};
use ahash::{AHashMap, AHashSet};

pub(super) fn is_type_excluded(name: &str, rust_path: &str, exclude_list: &[String]) -> bool {
    exclude_list
        .iter()
        .any(|entry| type_identity_matches(entry, name, rust_path))
}

/// Reason recorded on `binding_excluded` fields matched via `[crates.exclude].fields`,
/// mirroring the reason strings `alef(skip)`/`doc(hidden)` record for attribute-based exclusion.
const EXCLUDE_FIELDS_REASON: &str = "exclude.fields config";

/// Apply `[crates.exclude].fields` (`"TypeName.field_name"` entries) by marking matching
/// fields `binding_excluded`, using the exact same IR flag that `#[cfg_attr(alef, alef(skip))]`
/// sets on struct fields — every backend already filters on `binding_excluded`, so this makes
/// a globally-excluded field disappear from all bindings for free, without touching any
/// backend/language generator.
///
/// Matches both struct fields (`api.types`) and named enum variant fields (`api.enums`),
/// since attribute-based `alef(skip)` already supports exclusion on both (see
/// `extract_field` used by both `extract_field` call sites for struct and named-variant
/// fields). Malformed entries (not exactly one `.` splitting a non-empty type and field
/// name) are logged and skipped rather than panicking.
pub(super) fn apply_exclude_fields(api: &mut ApiSurface, fields: &[String]) {
    apply_exclude_fields_with_warnings(api, fields, true);
}

pub(super) fn apply_exclude_fields_silent(api: &mut ApiSurface, fields: &[String]) {
    apply_exclude_fields_with_warnings(api, fields, false);
}

fn apply_exclude_fields_with_warnings(api: &mut ApiSurface, fields: &[String], warn_unmatched: bool) {
    for entry in fields {
        let Some((type_name, field_name)) = entry.rsplit_once('.') else {
            tracing::warn!(entry = %entry, "exclude.fields entry must be \"TypeName.field_name\"; skipping");
            continue;
        };
        if type_name.is_empty() || field_name.is_empty() {
            tracing::warn!(entry = %entry, "exclude.fields entry must be \"TypeName.field_name\"; skipping");
            continue;
        }

        let mut matched = false;
        for typ in &mut api.types {
            if !type_identity_matches(type_name, &typ.name, &typ.rust_path) {
                continue;
            }
            for field in &mut typ.fields {
                if field.name == field_name {
                    field.binding_excluded = true;
                    field.binding_exclusion_reason = Some(EXCLUDE_FIELDS_REASON.to_string());
                    matched = true;
                }
            }
        }
        for enm in &mut api.enums {
            if !type_identity_matches(type_name, &enm.name, &enm.rust_path) {
                continue;
            }
            for variant in &mut enm.variants {
                for field in &mut variant.fields {
                    if field.name == field_name {
                        field.binding_excluded = true;
                        field.binding_exclusion_reason = Some(EXCLUDE_FIELDS_REASON.to_string());
                        matched = true;
                    }
                }
            }
        }

        if !matched && warn_unmatched {
            tracing::warn!(entry = %entry, "exclude.fields entry did not match any known type field");
        }
    }
}

/// The single rule for "does this configured entry name this type?", shared by
/// `[crates.include].types`, `[crates.exclude].types` and `[crates.exclude].fields`.
///
/// A bare entry matches the short name; a `::` entry matches the exact `rust_path`, or — for a
/// two-segment `crate::Type` entry — any path in that crate ending in `::Type`. The three lists
/// used to answer this question three different ways, so `exclude.types = ["c::Foo"]` was a silent
/// no-op for `c::inner::Foo` while `exclude.fields = ["c::Foo.bar"]` matched it. ~keep
fn type_identity_matches(configured: &str, name: &str, rust_path: &str) -> bool {
    if configured.contains("::") {
        let configured = configured.replace('-', "_");
        let rust_path = rust_path.replace('-', "_");
        if rust_path == configured {
            return true;
        }
        let mut segments = configured.split("::");
        let Some(crate_name) = segments.next() else {
            return false;
        };
        let Some(type_name) = segments.next() else {
            return false;
        };
        segments.next().is_none()
            && rust_path.starts_with(&format!("{crate_name}::"))
            && rust_path.ends_with(&format!("::{type_name}"))
    } else {
        name == configured
    }
}

/// The owning item name an `include` entry is compared against for an unsupported-item
/// diagnostic: the last path segment for a function, the type before the `.` for a method.
///
/// The include *retention* pass and the include *resolution* pass must agree on this, or an
/// entry that retains a diagnostic is reported as matching nothing. ~keep
fn owner_name(item: &UnsupportedPublicItem) -> &str {
    let short_name = item.item_path.rsplit("::").next().unwrap_or(item.item_path.as_str());
    short_name.split('.').next().unwrap_or(short_name)
}

pub(super) fn apply_filters(mut api: ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<ApiSurface> {
    let exclude = &config.exclude;
    let include = &config.include;

    warn_unmatched_exclude_entries(&api, exclude);

    let mut expanded_include: Option<AHashSet<String>> = None;
    if !include.types.is_empty() {
        let seeds = resolve_include_types(&api, config)?;
        let expanded = expand_include_list(&api, &seeds, &include.functions);
        api.types.retain(|t| expanded.contains(&t.name));
        api.enums.retain(|e| expanded.contains(&e.name));
        expanded_include = Some(expanded);
    }
    if !include.functions.is_empty() {
        check_include_functions(&api, &include.functions)?;
        api.functions.retain(|f| include.functions.contains(&f.name));
    }
    if expanded_include.is_some() || !include.functions.is_empty() {
        api.unsupported_public_items.retain(|item| {
            let owner = owner_name(item);
            let included_type = expanded_include
                .as_ref()
                .is_some_and(|expanded| expanded.contains(owner));
            let included_function = item.item_kind == "function" && include.functions.iter().any(|name| name == owner);
            included_type || included_function
        });
    }

    for typ in &api.types {
        if is_type_excluded(&typ.name, &typ.rust_path, &exclude.types) {
            api.excluded_type_paths
                .insert(typ.name.clone(), typ.rust_path.replace('-', "_"));
        }
    }
    for enm in &api.enums {
        if is_type_excluded(&enm.name, &enm.rust_path, &exclude.types) {
            api.excluded_type_paths
                .insert(enm.name.clone(), enm.rust_path.replace('-', "_"));
        }
    }

    api.types
        .retain(|t| !is_type_excluded(&t.name, &t.rust_path, &exclude.types));
    api.functions.retain(|f| !exclude.functions.contains(&f.name));
    api.enums
        .retain(|e| !is_type_excluded(&e.name, &e.rust_path, &exclude.types));
    api.errors
        .retain(|e| !is_type_excluded(&e.name, &e.rust_path, &exclude.types));

    // attribute-based skip check (`#[alef::skip]`, `#[doc(hidden)]`) is necessarily
    api.unsupported_public_items.retain(|item| {
        let short_name = item.item_path.rsplit("::").next().unwrap_or(item.item_path.as_str());
        let by_type_name = is_type_excluded(short_name, &item.item_path, &exclude.types);
        let by_fn_name = item.item_kind == "function" && exclude.functions.contains(&short_name.to_string());
        let by_method_name = item.item_kind == "method" && exclude.methods.contains(&short_name.to_string());
        let by_parent_excluded = if item.item_kind == "method" {
            if let Some((owner_short, _)) = short_name.split_once('.') {
                let owner_full = item
                    .item_path
                    .rsplit_once('.')
                    .map(|(p, _)| p)
                    .unwrap_or(item.item_path.as_str());
                is_type_excluded(owner_short, owner_full, &exclude.types)
            } else {
                false
            }
        } else {
            false
        };
        !(by_type_name || by_fn_name || by_method_name || by_parent_excluded)
    });

    if !exclude.methods.is_empty() {
        for typ in &mut api.types {
            typ.methods.retain(|m| {
                let key = format!("{}.{}", typ.name, m.name);
                !exclude.methods.contains(&key)
            });
        }
        for service in &mut api.services {
            service.configurators.retain(|m| {
                let key = format!("{}.{}", service.name, m.name);
                !exclude.methods.contains(&key)
            });
        }
    }

    if !exclude.fields.is_empty() {
        apply_exclude_fields(&mut api, &exclude.fields);
    }

    Ok(api)
}

/// Resolve every `[crates.include].types` entry against the extracted surface and return the
/// short names to seed the include expansion with.
///
/// `include` is an allowlist, so an entry that resolves to nothing does not merely fail to add a
/// type — it shrinks the binding, and an entry list that resolves to nothing at all empties the
/// binding completely while `alef build` still exits 0. A typo, or the qualified `crate::Type`
/// spelling that `exclude.types` accepts, used to drop every type and enum in silence. Failing
/// here mirrors `external_types::resolve_root_names`, which already refuses an unresolvable root
/// on the strictly analogous `[[crates.source_crates]].roots` allowlist. ~keep
fn resolve_include_types(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<String>> {
    let mut seeds = Vec::new();
    let mut unmatched = Vec::new();

    for entry in &config.include.types {
        let mut matched = false;
        for typ in &api.types {
            if type_identity_matches(entry, &typ.name, &typ.rust_path) {
                seeds.push(typ.name.clone());
                matched = true;
            }
        }
        for enm in &api.enums {
            if type_identity_matches(entry, &enm.name, &enm.rust_path) {
                seeds.push(enm.name.clone());
                matched = true;
            }
        }
        // Declared opaque types are pushed into the surface by `inject_declared_opaque_types`
        // after filtering runs, so naming one here is legitimate even though it is absent now.
        // It needs no seed: injection is unconditional on the include list. ~keep
        if !matched && config.opaque_types.contains_key(entry) {
            matched = true;
        }
        // Error enums live in `api.errors`, which the include list never filters — they are
        // always kept. Naming one is therefore legitimate and needs no seed, exactly like a
        // declared opaque type. `warn_unmatched_exclude_entries` already consults `api.errors`
        // for the exclude side; omitting it here made `include` and `exclude` disagree about
        // whether an error enum is a type, and hard-failed a valid config naming the crate's
        // own public error enum. ~keep
        if !matched
            && api
                .errors
                .iter()
                .any(|err| type_identity_matches(entry, &err.name, &err.rust_path))
        {
            matched = true;
        }
        // A type may exist only as the owner of an `unsupported_public_items` diagnostic, which
        // the include list is also used to filter; naming one is a match, not a typo. ~keep
        if !matched
            && api
                .unsupported_public_items
                .iter()
                .any(|item| owner_name(item) == entry)
        {
            matched = true;
        }
        if !matched {
            unmatched.push(format!("`{entry}`"));
        }
    }

    if !unmatched.is_empty() {
        anyhow::bail!(
            "[crates.include].types matched no type or enum in crate `{}`: {}\n  \
             Fix: use the type's short name or its full `crate::path::Type`. The crate exposes {} types and \
             {} enums.",
            api.crate_name,
            unmatched.join(", "),
            api.types.len(),
            api.enums.len(),
        );
    }

    // Every entry matched, but none of them seeded a type or enum — they all resolved to
    // something `include` does not filter (an error enum, a declared opaque type, or an
    // `unsupported_public_items` owner). `expand_include_list` would then produce an empty
    // set and the retains below it would drop the entire surface, which is the silent
    // binding-emptying this whole function exists to prevent. Fail loudly instead. ~keep
    if seeds.is_empty() && !(api.types.is_empty() && api.enums.is_empty()) {
        anyhow::bail!(
            "[crates.include].types in crate `{}` matched only items `include` does not filter (error enums, \
             declared opaque types, unsupported-item owners), so every binding would be emptied.\n  \
             Fix: name one of the crate's {} types or {} enums, or drop `include.types` to keep the whole \
             surface.",
            api.crate_name,
            api.types.len(),
            api.enums.len(),
        );
    }

    Ok(seeds)
}

/// Reject `[crates.include].functions` entries that name no public function, for the same reason
/// [`resolve_include_types`] rejects unmatched type entries. ~keep
fn check_include_functions(api: &ApiSurface, include_functions: &[String]) -> anyhow::Result<()> {
    let unmatched: Vec<String> = include_functions
        .iter()
        .filter(|entry| {
            // A generic function reaches the surface only as an `unsupported_public_items`
            // diagnostic, which the include list also filters; naming one is a match. ~keep
            !api.functions.iter().any(|func| func.name == **entry)
                && !api
                    .unsupported_public_items
                    .iter()
                    .any(|item| item.item_kind == "function" && owner_name(item) == entry.as_str())
        })
        .map(|entry| format!("`{entry}`"))
        .collect();

    if !unmatched.is_empty() {
        anyhow::bail!(
            "[crates.include].functions matched no public function in crate `{}`: {}\n  \
             Fix: entries are bare function names. The crate exposes {} public functions.",
            api.crate_name,
            unmatched.join(", "),
            api.functions.len(),
        );
    }

    Ok(())
}

/// Warn once per `[crates.exclude]` entry that matches nothing at all, and separately trace (at
/// debug) an entry that redundantly excludes a public item alef already knows it can never
/// extract. The two are split because only the first can hide a real mistake -- see
/// [`unmatched_exclude_entries`] and [`redundant_generic_exclude_entries`] for why. ~keep
fn warn_unmatched_exclude_entries(api: &ApiSurface, exclude: &ExcludeConfig) {
    for (list, entry) in redundant_generic_exclude_entries(api, exclude) {
        tracing::debug!(
            entry = %entry,
            "exclude.{list} entry redundantly excludes a public item alef never extracts (generic without \
             explicit monomorphization metadata)"
        );
    }
    for (list, entry) in unmatched_exclude_entries(api, exclude) {
        tracing::warn!(
            entry = %entry,
            "exclude.{list} entry matched no extracted or diagnosed item; expected if it names a \
             private item, an item already excluded via attribute, or one dropped by an unmet cfg \
             -- otherwise check for a typo"
        );
    }
}

/// Whether a `types`-list entry already names something in the extracted (non-diagnostic)
/// surface -- a real, currently-effective exclusion, shared by [`unmatched_exclude_entries`] and
/// [`redundant_generic_exclude_entries`] so the two only disagree on how to classify a miss here,
/// not on what counts as a hit. ~keep
fn type_entry_extracted(api: &ApiSurface, entry: &str) -> bool {
    api.types
        .iter()
        .any(|typ| type_identity_matches(entry, &typ.name, &typ.rust_path))
        || api
            .enums
            .iter()
            .any(|enm| type_identity_matches(entry, &enm.name, &enm.rust_path))
        || api
            .errors
            .iter()
            .any(|err| type_identity_matches(entry, &err.name, &err.rust_path))
}

/// The `functions`-list counterpart to [`type_entry_extracted`].
fn function_entry_extracted(api: &ApiSurface, entry: &str) -> bool {
    api.functions.iter().any(|func| func.name == entry)
}

/// The `methods`-list counterpart to [`type_entry_extracted`].
fn method_entry_extracted(api: &ApiSurface, entry: &str) -> bool {
    api.types
        .iter()
        .any(|typ| typ.methods.iter().any(|m| format!("{}.{}", typ.name, m.name) == entry))
        || api.services.iter().any(|service| {
            service
                .configurators
                .iter()
                .any(|m| format!("{}.{}", service.name, m.name) == entry)
        })
}

/// A short-name/path/is-generic view of every `unsupported_public_items` entry, shared by
/// [`unmatched_exclude_entries`] and [`redundant_generic_exclude_entries`] so the two agree on
/// what counts as a proven, non-typo name.
///
/// The `bool` records whether `reason` names alef's inability to extract generics specifically.
/// [`unmatched_exclude_entries`] ignores it: ANY recorded diagnostic, regardless of reason, proves
/// the entry names a real item and must not warn as if it were a typo. Only
/// [`redundant_generic_exclude_entries`] reason-gates on it, because only the generic case is
/// specific and correct enough to explain at DEBUG -- a future non-generic diagnostic reason must
/// still count as a match (no warning) without being mis-described as "generic". ~keep
fn unsupported_short_names(api: &ApiSurface) -> Vec<(&str, &str, &str, bool)> {
    api.unsupported_public_items
        .iter()
        .map(|item| {
            let short = item.item_path.rsplit("::").next().unwrap_or(item.item_path.as_str());
            (
                item.item_kind.as_str(),
                short,
                item.item_path.as_str(),
                item.reason.contains("generic"),
            )
        })
        .collect()
}

/// Every `[crates.exclude]` entry that matches nothing, as `(list name, entry)`.
///
/// An exclusion is only ever observable by what it removes, so a typo'd entry excludes nothing
/// and reports nothing. `exclude.fields` already warns (see
/// [`apply_exclude_fields_with_warnings`]); this extends the same diagnostic to the three lists
/// that had none. Warn rather than fail: an entry may legitimately name a private item, an item
/// already excluded via attribute, or a cfg-gated item absent under the currently enabled
/// features -- none of those leave any record here, so they are indistinguishable from a typo at
/// this layer (see [`redundant_generic_exclude_entries`] for the one case that IS distinguishable).
/// Kept separate from the `tracing` call so the matching is assertable without installing a
/// subscriber. ~keep
pub(super) fn unmatched_exclude_entries(api: &ApiSurface, exclude: &ExcludeConfig) -> Vec<(&'static str, String)> {
    let unsupported = unsupported_short_names(api);
    let mut unmatched = Vec::new();

    for entry in &exclude.types {
        let matched = type_entry_extracted(api, entry)
            || unsupported
                .iter()
                .any(|(_, short, path, _)| type_identity_matches(entry, short, path));
        if !matched {
            unmatched.push(("types", entry.clone()));
        }
    }

    for entry in &exclude.functions {
        let matched = function_entry_extracted(api, entry)
            || unsupported
                .iter()
                .any(|(kind, short, _, _)| *kind == "function" && short == entry);
        if !matched {
            unmatched.push(("functions", entry.clone()));
        }
    }

    for entry in &exclude.methods {
        let matched = method_entry_extracted(api, entry)
            || unsupported
                .iter()
                .any(|(kind, short, _, _)| *kind == "method" && short == entry);
        if !matched {
            unmatched.push(("methods", entry.clone()));
        }
    }

    unmatched
}

/// `[crates.exclude]` entries that redundantly name a public item alef recorded as unrepresentable
/// (generic without explicit monomorphization metadata), as `(list name, entry)`.
///
/// alef never extracts a generic item, so excluding one can never remove anything -- but unlike a
/// truly unmatched entry, this case is provable: the item's existence and the reason it was never
/// extracted both come from `unsupported_public_items`, not from silence. A private item, an item
/// already excluded via `#[alef(skip)]`/`#[doc(hidden)]`, an item dropped by an unmet `cfg`, and a
/// genuine typo all produce identical silence in the IR and stay indistinguishable in
/// [`unmatched_exclude_entries`] -- downgrading that case would risk hiding a real typo. This case
/// carries positive proof instead, so it alone can safely drop to a quieter diagnostic. ~keep
pub(super) fn redundant_generic_exclude_entries(
    api: &ApiSurface,
    exclude: &ExcludeConfig,
) -> Vec<(&'static str, String)> {
    let unsupported = unsupported_short_names(api);
    let mut redundant = Vec::new();

    for entry in &exclude.types {
        if !type_entry_extracted(api, entry)
            && unsupported
                .iter()
                .any(|(_, short, path, is_generic)| *is_generic && type_identity_matches(entry, short, path))
        {
            redundant.push(("types", entry.clone()));
        }
    }

    for entry in &exclude.functions {
        if !function_entry_extracted(api, entry)
            && unsupported
                .iter()
                .any(|(kind, short, _, is_generic)| *is_generic && *kind == "function" && short == entry)
        {
            redundant.push(("functions", entry.clone()));
        }
    }

    for entry in &exclude.methods {
        if !method_entry_extracted(api, entry)
            && unsupported
                .iter()
                .any(|(kind, short, _, is_generic)| *is_generic && *kind == "method" && short == entry)
        {
            redundant.push(("methods", entry.clone()));
        }
    }

    redundant
}

/// Expand the include list by transitively discovering all types referenced by fields,
/// method parameters, and return types of the included types, plus the signatures
/// (return type and params) of `include_functions`.
pub(super) fn expand_include_list(
    api: &ApiSurface,
    include_types: &[String],
    include_functions: &[String],
) -> AHashSet<String> {
    let mut needed: AHashSet<String> = include_types.iter().cloned().collect();
    let mut changed = true;

    let all_types: AHashMap<String, &TypeDef> = api.types.iter().map(|t| (t.name.clone(), t)).collect();
    let all_enums: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    let include_function_set: AHashSet<&str> = include_functions.iter().map(String::as_str).collect();
    if !include_function_set.is_empty() {
        for func in &api.functions {
            if !include_function_set.contains(func.name.as_str()) {
                continue;
            }
            collect_named_types(&func.return_type, &mut needed, &all_types, &all_enums, &mut changed);
            for param in &func.params {
                collect_named_types(&param.ty, &mut needed, &all_types, &all_enums, &mut changed);
            }
        }
    }

    while changed {
        changed = false;
        let current: Vec<String> = needed.iter().cloned().collect();
        for type_name in &current {
            if let Some(typ) = all_types.get(type_name) {
                for field in &typ.fields {
                    if field.binding_excluded {
                        continue;
                    }
                    collect_named_types(&field.ty, &mut needed, &all_types, &all_enums, &mut changed);
                }
                for method in &typ.methods {
                    collect_named_types(&method.return_type, &mut needed, &all_types, &all_enums, &mut changed);
                    for param in &method.params {
                        collect_named_types(&param.ty, &mut needed, &all_types, &all_enums, &mut changed);
                    }
                }
            }
        }
    }
    needed
}

/// Recursively collect all named type references from a TypeRef into the needed set.
fn collect_named_types(
    ty: &TypeRef,
    needed: &mut AHashSet<String>,
    all_types: &AHashMap<String, &TypeDef>,
    all_enums: &AHashSet<String>,
    changed: &mut bool,
) {
    match ty {
        TypeRef::Named(name)
            if (all_types.contains_key(name) || all_enums.contains(name)) && needed.insert(name.clone()) =>
        {
            *changed = true;
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => {
            collect_named_types(inner, needed, all_types, all_enums, changed);
        }
        TypeRef::Map(k, v) => {
            collect_named_types(k, needed, all_types, all_enums, changed);
            collect_named_types(v, needed, all_types, all_enums, changed);
        }
        _ => {}
    }
}
