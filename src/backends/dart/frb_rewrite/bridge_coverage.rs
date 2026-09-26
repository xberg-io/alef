//! Verify that flutter_rust_bridge's generated Dart bridge exposes every free function
//! declared in alef's generated FRB facade (`packages/dart/rust/src/lib.rs`).
//!
//! `flutter_rust_bridge_codegen` is invoked via [`crate::core::backend::PostBuildStep::RunCommand`],
//! and that step's runner treats a missing tool (or the `ALEF_SKIP_COMMANDS` escape hatch) as a
//! non-fatal skip -- deliberately, so a host without `flutter_rust_bridge_codegen` installed can
//! still regenerate the Rust-side facade and fall back to whatever bridge Dart source is already
//! committed. The [`PostProcessFile`](crate::core::backend::PostBuildStep::PostProcessFile) steps
//! that follow it in the post-build sequence (sealed-variant rewriting, extension injection, ...)
//! run unconditionally, patching whatever is on disk regardless of whether frb actually produced
//! it this run.
//!
//! That combination is silently unsound whenever the facade changed since the bridge was last
//! regenerated: alef's own patches land on a stale bridge, producing a file that looks freshly
//! post-processed but is missing every function the facade gained in the meantime (alef #135).
//! [`missing_bridge_functions`] is the invariant that [`crate::cli::pipeline::commands::build::frb_bridge_coverage`]
//! checks immediately after the `RunCommand` step and before any `PostProcessFile` rewrite, so a
//! stale bridge fails the build loudly instead of being silently patched into a half-correct state.
//!
//! A facade function can also be legitimately absent from the bridge with frb having done nothing
//! wrong at all: flutter_rust_bridge's codegen macro expansion runs against the crate's own
//! `default` Cargo features (see the `cfg_gates` module doc), so a `#[cfg(feature = "X")]`-gated
//! function whose feature is declared in that manifest but not in its `default` list is correctly
//! invisible to frb and must not be reported missing. [`missing_bridge_functions`] takes the
//! enabled-feature set callers resolve from that same manifest (via
//! [`crate::codegen::cfg::read_default_enabled_cargo_features`]) and skips any facade function
//! whose gate that set does not satisfy -- **provided** the manifest's `[features]` table declares
//! the gated feature at all.
//!
//! A gate naming a feature the manifest never declares is a different situation, not the same
//! one: "declared but off" is a choice the manifest owner made on purpose, while "never declared"
//! means the manifest has nothing to turn on for that name at all. Alef's own generation forwards
//! every cfg-gated feature it emits into the facade's sibling manifest (see
//! `crate::codegen::cfg::collect_cfg_features`), so an undeclared feature almost always means that
//! forwarding write never landed -- most commonly because the ownership guard refused it (alef
//! #464). Treating the two cases identically let this check silently agree with a broken
//! manifest instead of catching the break: [`missing_bridge_functions`] and
//! [`active_free_function_names`] keep an undeclared-gate function a coverage candidate, via
//! `declared_features` (resolved from the same manifest via
//! [`crate::codegen::cfg::read_declared_cargo_features`]), and [`undeclared_gate_features`] lets a
//! caller attribute a resulting failure to the actual cause. ~keep

use super::cfg_gates::{cfg_gated_free_functions, free_pub_fn_name};
use super::text_transformations::{contains_function_at_token_boundary, frb_dart_function_name};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Names of every top-level (column 0) `pub fn` / `pub async fn` free function declared in
/// `lib_rs_source`, in declaration order.
///
/// This is the full set flutter_rust_bridge should bridge from the facade -- unlike
/// [`super::cfg_gates::cfg_gated_free_functions`], which only returns the `#[cfg(...)]`-gated
/// subset.
pub fn free_function_names(lib_rs_source: &str) -> Vec<String> {
    lib_rs_source.lines().filter_map(free_pub_fn_name).collect()
}

/// Strip a `#[cfg(...)]` attribute's `#[cfg(` / `)]` delimiters, returning the bare predicate
/// text [`crate::core::ir::cfg_feature_satisfied`] expects (e.g. `feature = "premium-tier"`).
///
/// `gate` is always produced by [`super::cfg_gates::cfg_gated_free_functions`], which guarantees
/// the `#[cfg(` prefix and `)]` suffix are present verbatim.
fn cfg_predicate(gate: &str) -> &str {
    gate.strip_prefix("#[cfg(")
        .and_then(|rest| rest.strip_suffix(")]"))
        .unwrap_or(gate)
}

/// Every `feature = "..."` name a (possibly `any`/`all`/`not`-composed) cfg predicate
/// references, via the same recursive walk [`crate::codegen::cfg::collect_cfg_features`] uses to
/// decide which features a binding crate's own `[features]` table must forward. `predicate` is
/// the bare text [`cfg_predicate`] returns (no `#[cfg(` / `)]` wrapper).
fn cfg_predicate_feature_names(predicate: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    crate::codegen::cfg::collect_cfg_feature_names(predicate, &mut names);
    names
}

/// Names of every top-level free function in `lib_rs_source` that flutter_rust_bridge would
/// actually see and bridge, given `enabled_features`.
///
/// A function with no `#[cfg(...)]` gate is always included. A gated function whose gate
/// `enabled_features` satisfies (evaluated with the same [`crate::core::ir::cfg_feature_satisfied`]
/// every other cfg filter in this repo uses) is also included. When `enabled_features` is `None`
/// -- the manifest that would supply it could not be read or parsed -- every function, gated or
/// not, is included: this is the pre-cfg-awareness behavior, so an unreadable manifest degrades to
/// the old blanket check rather than silently exempting every gated function from coverage.
///
/// A gate `enabled_features` does NOT satisfy is only excluded (treated as a deliberate,
/// legitimate "off by default" choice frb agrees with) when `declared_features` shows the
/// manifest actually declares every feature the gate names. A gate naming a feature the manifest
/// never declares at all cannot be that kind of choice -- there is nothing in the manifest to
/// turn on -- so it is kept as a coverage candidate instead, the same way an unresolved
/// `target_arch` leaf is kept indeterminate rather than dropped. This is what lets a real
/// consumer-repo failure surface: `collect_cfg_features` wants to add a forwarding `[features]`
/// entry for a newly cfg-gated facade function, the ownership guard refuses that write because
/// the manifest predates alef's marker convention, and the function's gate is left referencing a
/// feature name the manifest never got. Silently excluding it here (the old behavior) let the
/// coverage check agree with a broken manifest instead of catching the break. ~keep
fn active_free_function_names(
    lib_rs_source: &str,
    enabled_features: Option<&HashSet<&str>>,
    declared_features: Option<&HashSet<&str>>,
) -> Vec<String> {
    let gates: HashMap<String, String> = cfg_gated_free_functions(lib_rs_source).into_iter().collect();
    free_function_names(lib_rs_source)
        .into_iter()
        .filter(|name| {
            let Some(gate) = gates.get(name) else {
                return true;
            };
            let Some(features) = enabled_features else {
                return true;
            };
            if crate::core::ir::cfg_feature_satisfied(Some(cfg_predicate(gate)), features) {
                return true;
            }
            match declared_features {
                Some(declared) => cfg_predicate_feature_names(cfg_predicate(gate))
                    .iter()
                    .any(|feature_name| !declared.contains(feature_name.as_str())),
                // No declared-feature information to check against (an unreadable/unparseable
                // manifest, which `enabled_features` being `Some` here should never pair with in
                // practice -- both are derived from the same read). Keep the function a
                // candidate rather than silently trusting an unverifiable "off by default". ~keep
                None => true,
            }
        })
        .collect()
}

/// The feature names among `function_name`'s facade cfg gate (if any) that `declared_features`
/// does not declare at all.
///
/// Always empty for an ungated function, or for a gated function whose every referenced feature
/// IS declared (whether on or off by default) -- both are outside the shape
/// [`active_free_function_names`]'s doc comment describes. Exists so a caller that already has a
/// [`missing_bridge_functions`] result can attribute each name accurately: a name this returns
/// non-empty for is missing because its manifest entry never landed (most often a refused write),
/// not because `flutter_rust_bridge_codegen` failed to run.
pub fn undeclared_gate_features(
    lib_rs_source: &str,
    function_name: &str,
    declared_features: &HashSet<&str>,
) -> BTreeSet<String> {
    cfg_gated_free_functions(lib_rs_source)
        .into_iter()
        .find(|(name, _)| name == function_name)
        .map(|(_, gate)| {
            cfg_predicate_feature_names(cfg_predicate(&gate))
                .into_iter()
                .filter(|feature_name| !declared_features.contains(feature_name.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

/// Names (as declared in `lib_rs_source`, snake_case) of facade free functions that have no
/// matching function in `bridge_dart_source`.
///
/// `exclude_functions` are removed from consideration: those are deliberately stripped from the
/// bridge post-frb by [`super::text_transformations::filter_excluded_functions`] and are expected
/// to be permanently absent, independent of whether the bridge is otherwise fresh.
///
/// `enabled_features` -- see [`active_free_function_names`] -- excludes a `#[cfg(...)]`-gated
/// function from consideration only when its gate is not satisfied AND `declared_features` shows
/// the manifest declares every feature the gate names; a gate naming an undeclared feature stays
/// a candidate (see that function's doc for why).
///
/// Matching uses flutter_rust_bridge's snake_case -> lowerCamelCase convention and a
/// token-boundary lookup ([`contains_function_at_token_boundary`]) rather than a literal
/// leading-space check, so a function whose name `dartfmt` wrapped onto its own line (a long
/// return type pushes it past the line's whitespace) is still found (alef #191). This is the
/// same boundary-safe lookup [`super::text_transformations::filter_excluded_functions`] uses to
/// find a function to strip.
pub fn missing_bridge_functions(
    lib_rs_source: &str,
    bridge_dart_source: &str,
    exclude_functions: &[String],
    enabled_features: Option<&HashSet<&str>>,
    declared_features: Option<&HashSet<&str>>,
) -> Vec<String> {
    let excluded: HashSet<&str> = exclude_functions.iter().map(String::as_str).collect();
    active_free_function_names(lib_rs_source, enabled_features, declared_features)
        .into_iter()
        .filter(|name| !excluded.contains(name.as_str()))
        .filter(|name| {
            let camel = frb_dart_function_name(name);
            !contains_function_at_token_boundary(bridge_dart_source, &camel)
        })
        .collect()
}

/// `(struct_name, field_name)` for every named (non-tuple, non-unit) `pub` field of every
/// top-level `pub struct Name { ... }` block in `lib_rs_source`, in declaration order.
///
/// Depth-tracked by brace count rather than indentation, so it does not depend on `lib_rs_source`
/// already being rustfmt-canonical. A struct's own fields are the only lines collected while its
/// body is open; nested item kinds (an enum's variant-payload braces, for instance) never open a
/// `current_struct` scope in the first place, since only a `pub struct ... {` header line starts
/// one.
fn struct_field_pairs(lib_rs_source: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut current_struct: Option<String> = None;
    let mut depth: i32 = 0;
    for line in lib_rs_source.lines() {
        if current_struct.is_none() {
            if let Some(name) = pub_struct_name(line) {
                current_struct = Some(name);
                depth = brace_delta(line);
            }
            continue;
        }
        depth += brace_delta(line);
        if depth <= 0 {
            current_struct = None;
            continue;
        }
        if let Some(field) = pub_field_name(line.trim()) {
            pairs.push((current_struct.clone().unwrap_or_default(), field));
        }
    }
    pairs
}

/// Net change in brace depth `line` contributes (opens minus closes).
fn brace_delta(line: &str) -> i32 {
    line.matches('{').count() as i32 - line.matches('}').count() as i32
}

/// The struct name from a `pub struct Name { ... }` (or `pub struct Name<T> { ... }`) header
/// line, or `None` for a tuple struct (`pub struct Name(...)`), a unit struct (`pub struct
/// Name;`), or any non-struct-header line. Only a named-field struct has Dart-side properties for
/// [`missing_bridge_struct_fields`] to check.
fn pub_struct_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("pub struct ")?;
    if !trimmed.ends_with('{') {
        return None;
    }
    let name_end = rest.find(|c: char| c == '{' || c == '<' || c.is_whitespace())?;
    let name = &rest[..name_end];
    (!name.is_empty()).then(|| name.to_string())
}

/// The field name from a `pub field_name: Type,` struct-body line (already trimmed), or `None`
/// for a doc comment, attribute, or any other line a struct body can contain.
fn pub_field_name(trimmed_line: &str) -> Option<String> {
    let rest = trimmed_line.strip_prefix("pub ")?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim();
    let is_identifier = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    is_identifier.then(|| name.to_string())
}

/// Whether `line` opens a new top-level Dart type declaration -- the boundary
/// [`dart_class_body`] stops a class's body at, so one class's field list is never contaminated
/// by the next declaration's source text.
fn is_top_level_type_header(line: &str) -> bool {
    if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    [
        "class ",
        "sealed class ",
        "abstract class ",
        "base class ",
        "final class ",
        "mixin ",
        "enum ",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

/// Whether `line` is the (column-0) header of a Dart type declaration named `class_name` --
/// `class Name {`, `sealed class Name with ...`, `abstract class Name implements ...`, and so on,
/// which is exactly how flutter_rust_bridge's generated `lib.dart` opens every data class.
fn is_class_header_for(line: &str, class_name: &str) -> bool {
    if !is_top_level_type_header(line) {
        return false;
    }
    let Some(after_class_kw) = line.split_once("class ").map(|(_, rest)| rest) else {
        return false;
    };
    let name_end = after_class_kw
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(after_class_kw.len());
    &after_class_kw[..name_end] == class_name
}

/// The Dart source text of the `class_name` declaration in `dart_bridge_source`, from its header
/// line up to (not including) the next top-level type header, or `None` when no such class is
/// declared at all.
///
/// A missing class is deliberately not itself reported by [`missing_bridge_struct_fields`]: a
/// facade struct with no matching Dart class could be an excluded type, an unused mirror, or a
/// type this specific check has not learned to find -- a different and more fundamental gap than
/// "the class exists but is missing a field", which is the narrower, well-evidenced claim this
/// module makes. See its doc comment.
fn dart_class_body(dart_bridge_source: &str, class_name: &str) -> Option<String> {
    let lines: Vec<&str> = dart_bridge_source.lines().collect();
    let start = lines.iter().position(|line| is_class_header_for(line, class_name))?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| is_top_level_type_header(line))
        .map_or(lines.len(), |offset| start + 1 + offset);
    Some(lines[start..end].join("\n"))
}

/// Whether `name` occurs in `haystack` as a whole identifier -- neither the character
/// immediately before nor immediately after any match is itself an identifier character. Unlike
/// [`contains_function_at_token_boundary`] (which only needs a left boundary, because its fixed
/// `(` suffix already anchors the right side), a Dart field name can be followed by any of
/// several punctuation marks (`;`, `,`, `.`, `==`), so both sides need checking here.
fn contains_identifier_at_token_boundary(haystack: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let is_identifier_char = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut search_start = 0;
    while let Some(relative_offset) = haystack[search_start..].find(name) {
        let absolute_offset = search_start + relative_offset;
        let preceding_ok = !matches!(haystack[..absolute_offset].chars().next_back(), Some(c) if is_identifier_char(c));
        let following_ok =
            !matches!(haystack[absolute_offset + name.len()..].chars().next(), Some(c) if is_identifier_char(c));
        if preceding_ok && following_ok {
            return true;
        }
        search_start = absolute_offset + 1;
    }
    false
}

/// `TypeName.field_name` for every named field of a facade struct (outside `exclude_types`) whose
/// Dart class exists in `bridge_dart_source` but does not declare a matching field.
///
/// This is [`missing_bridge_functions`]'s counterpart for struct fields rather than free
/// functions -- the same alef #135 shape (a `RunCommand` skip leaves post-processing to patch a
/// stale bridge) applies identically to a struct that gained or renamed a field, not only to a
/// newly added function. `alef #437`'s reported bug -- a public field changed and the Dart bridge
/// class silently kept its old shape -- is exactly what a skipped/no-op `flutter_rust_bridge_codegen`
/// run produces: the facade (`lib.rs`) is fresh, but the Dart class frb already emitted for an
/// earlier version of the struct is not revisited.
///
/// A struct absent from the bridge entirely is not reported here -- see [`dart_class_body`]'s doc.
pub fn missing_bridge_struct_fields(
    lib_rs_source: &str,
    bridge_dart_source: &str,
    exclude_types: &[String],
) -> Vec<String> {
    let excluded: HashSet<&str> = exclude_types.iter().map(String::as_str).collect();
    struct_field_pairs(lib_rs_source)
        .into_iter()
        .filter(|(struct_name, _)| !excluded.contains(struct_name.as_str()))
        .filter_map(|(struct_name, field_name)| {
            let body = dart_class_body(bridge_dart_source, &struct_name)?;
            let camel = frb_dart_function_name(&field_name);
            (!contains_identifier_at_token_boundary(&body, &camel)).then(|| format!("{struct_name}.{field_name}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_function_names_lists_every_top_level_pub_fn() {
        let lib_rs = "\
use std::sync::Arc;

pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

pub async fn record_price(id: String, price_cents: i64) -> Result<(), String> {
    Ok(())
}

fn private_helper() {}
";
        assert_eq!(
            free_function_names(lib_rs),
            vec!["count_widgets".to_string(), "record_price".to_string()],
        );
    }

    #[test]
    fn missing_bridge_functions_reports_a_facade_function_absent_from_the_bridge() {
        let lib_rs = "\
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

pub fn record_price(id: String, price_cents: i64) -> Result<(), String> {
    Ok(())
}
";
        // The bridge only has `countWidgets` -- as if frb ran once before `record_price` was
        // added to the facade and never ran again afterward.
        let bridge_dart = "Future<int> countWidgets({required String collection}) => RustLib.instance.api.crateCountWidgets(collection: collection);\n";

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], None, None);
        assert_eq!(
            missing,
            vec!["record_price".to_string()],
            "record_price is declared in the facade but has no matching function in the bridge"
        );
    }

    #[test]
    fn missing_bridge_functions_is_empty_when_every_facade_function_is_bridged() {
        let lib_rs = "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n";
        let bridge_dart = "Future<int> countWidgets({required String collection}) => RustLib.instance.api.crateCountWidgets(collection: collection);\n";

        assert!(missing_bridge_functions(lib_rs, bridge_dart, &[], None, None).is_empty());
    }

    /// `dartfmt` wraps a long return type onto its own line, pushing the function name onto the
    /// line below preceded by a newline (and indentation), not a literal space -- the alef #191
    /// shape: a facade function that IS present and correctly bridged is reported missing purely
    /// because of how the bridge happened to be line-wrapped.
    #[test]
    fn missing_bridge_functions_finds_a_function_whose_name_is_wrapped_onto_its_own_line() {
        let lib_rs = "\
pub fn create_chunk_classification_definition_from_json(
    json: String,
) -> Result<ChunkClassificationDefinition, String> {
    todo!()
}
";
        // dartfmt wrapped the long return type onto its own line, so the function name starts a
        // fresh line with no preceding space -- only a preceding newline.
        let bridge_dart = "\
Future<ChunkClassificationDefinition>
createChunkClassificationDefinitionFromJson({required String json}) =>
    RustLib.instance.api.crateCreateChunkClassificationDefinitionFromJson(json: json);
";

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], None, None);
        assert_eq!(
            missing,
            Vec::<String>::new(),
            "the function is present and correctly bridged -- only line-wrapped -- and must not \
             be reported missing: {missing:?}"
        );
    }

    #[test]
    fn missing_bridge_functions_ignores_configured_exclusions() {
        let lib_rs = "\
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

pub fn internal_only(id: String) -> Result<(), String> {
    Ok(())
}
";
        // `internal_only` is deliberately stripped from the bridge post-frb (it never appears),
        // and that is expected -- excluding it must not show up as a coverage gap.
        let bridge_dart = "Future<int> countWidgets({required String collection}) => RustLib.instance.api.crateCountWidgets(collection: collection);\n";

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &["internal_only".to_string()], None, None);
        assert!(
            missing.is_empty(),
            "excluded function must not be reported missing: {missing:?}"
        );
    }

    /// The critical regression: a facade function gated behind a feature that IS declared in the
    /// manifest but not in the enabled (default) set must never be reported missing -- frb's own
    /// codegen never saw it either, so the bridge is not stale with respect to it. This is the
    /// shape of a real `alef all` failure this fix closes: several `#[cfg(feature = "...")]`
    /// functions a dart rust crate's manifest never enabled by default were reported as a stale
    /// bridge, even though frb correctly never saw them.
    #[test]
    fn missing_bridge_functions_ignores_a_facade_function_behind_a_declared_but_inactive_cfg_gate() {
        let lib_rs = "\
#[cfg(feature = \"premium-tier\")]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        // The bridge has nothing for this function at all -- exactly what a correct frb run
        // produces when the gating feature is off.
        let bridge_dart = "";
        let enabled: HashSet<&str> = HashSet::new();
        // The manifest DOES declare `premium-tier` (just not in `default`) -- a deliberate,
        // legitimate "off by default" choice.
        let declared: HashSet<&str> = ["premium-tier"].into_iter().collect();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled), Some(&declared));
        assert!(
            missing.is_empty(),
            "a facade function behind a declared-but-inactive cfg gate must not be reported \
             missing: {missing:?}"
        );
    }

    /// The other half of the same regression, and the shape alef #464 pins: a facade
    /// function gated behind a feature the manifest does not declare AT ALL (not even as an
    /// off-by-default entry) must still be reported missing. There is nothing to "turn on" for an
    /// undeclared feature, so this cannot be the same deliberate choice the declared-but-inactive
    /// case above is -- most often it means a forwarding `[features]` entry alef's own generation
    /// wanted to write to this manifest never landed (e.g. the ownership guard refused the
    /// write), and that gap must surface as a build failure rather than being silently agreed
    /// with. ~keep
    #[test]
    fn missing_bridge_functions_still_reports_a_function_behind_an_undeclared_cfg_gate() {
        let lib_rs = "\
#[cfg(feature = \"widgets\")]
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}
";
        let bridge_dart = "";
        let enabled: HashSet<&str> = HashSet::new();
        // The manifest's `[features]` table does not mention `widgets` at all.
        let declared: HashSet<&str> = HashSet::new();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled), Some(&declared));
        assert_eq!(
            missing,
            vec!["count_widgets".to_string()],
            "a facade function behind an undeclared cfg gate must still be reported missing: \
             {missing:?}"
        );
    }

    /// Negative control for both cases above: a facade function that is genuinely absent from the
    /// bridge -- either because it carries no cfg gate at all, or because its gate IS active --
    /// must still be reported missing. Without this control, a fix that simply stopped reporting
    /// every cfg-gated function (active or not) would pass the positive tests above while quietly
    /// breaking the coverage check's entire purpose.
    #[test]
    fn missing_bridge_functions_still_reports_a_genuinely_missing_function_under_an_active_gate() {
        let lib_rs = "\
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}

#[cfg(feature = \"premium-tier\")]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        // Neither function is bridged: `count_widgets` is ungated (always expected), and
        // `create_premium_backend_options_from_json`'s gate IS in the enabled set, so it is also
        // expected -- both must be reported.
        let bridge_dart = "";
        let enabled: HashSet<&str> = ["premium-tier"].into_iter().collect();
        let declared: HashSet<&str> = ["premium-tier"].into_iter().collect();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled), Some(&declared));
        assert_eq!(
            missing,
            vec![
                "count_widgets".to_string(),
                "create_premium_backend_options_from_json".to_string(),
            ],
            "an ungated function and a function under an active gate must both still be reported \
             missing: {missing:?}"
        );
    }

    /// The real facade shape both positive tests above skip: `#[cfg(...)]` immediately followed
    /// by `#[frb]` before `pub fn` (see `frb_rewrite::cfg_gates::cfg_gated_free_functions`'s doc
    /// for why this specific shape matters -- it is what
    /// `backends::dart::templates::rust_from_json_bridge_fn.rs.jinja` always emits). Before that
    /// scanner learned to skip an intervening attribute, this gate was never recorded, so the
    /// function below was treated as ungated -- unconditionally expected in the bridge -- and
    /// reported missing even though its feature is off and frb correctly never emitted it.
    ///
    /// The gate's feature is passed as DECLARED here: an inactive gate is only legitimately
    /// excluded from coverage when the manifest actually declares the feature it names. An
    /// undeclared one is covered by
    /// `missing_bridge_functions_still_reports_a_function_behind_an_undeclared_cfg_gate`. ~keep
    #[test]
    fn missing_bridge_functions_ignores_an_inactive_gate_behind_an_intervening_frb_attribute() {
        let lib_rs = "\
#[cfg(feature = \"premium-tier\")]
#[frb]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        let bridge_dart = "";
        let enabled: HashSet<&str> = HashSet::new();
        let declared: HashSet<&str> = ["premium-tier"].into_iter().collect();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled), Some(&declared));
        assert!(
            missing.is_empty(),
            "a gate followed by an intervening #[frb] attribute must still be recognized and, \
             being inactive but declared, must not be reported missing: {missing:?}"
        );
    }

    /// Negative control for the test above: with the SAME `#[cfg(...)]` / `#[frb]` / `pub fn`
    /// shape, a function whose gate IS in the enabled set but is genuinely absent from the
    /// bridge must still be reported. Without this control, a fix that made the scanner treat
    /// every `#[frb]`-preceded gate as inactive (rather than correctly attaching and then
    /// evaluating it) would pass the positive test above while silencing this exact shape's
    /// coverage check entirely. ~keep
    #[test]
    fn missing_bridge_functions_still_reports_a_genuinely_missing_function_behind_an_intervening_frb_attribute() {
        let lib_rs = "\
#[cfg(feature = \"premium-tier\")]
#[frb]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        let bridge_dart = "";
        let enabled: HashSet<&str> = ["premium-tier"].into_iter().collect();
        let declared: HashSet<&str> = ["premium-tier"].into_iter().collect();

        let missing = missing_bridge_functions(lib_rs, bridge_dart, &[], Some(&enabled), Some(&declared));
        assert_eq!(
            missing,
            vec!["create_premium_backend_options_from_json".to_string()],
            "a function under an active gate behind an intervening #[frb] attribute must still \
             be reported missing when absent from the bridge: {missing:?}"
        );
    }

    #[test]
    fn undeclared_gate_features_is_empty_for_an_ungated_function() {
        let lib_rs = "pub fn count_widgets(collection: String) -> Result<i64, String> {\n    Ok(0)\n}\n";
        let declared: HashSet<&str> = HashSet::new();

        assert!(undeclared_gate_features(lib_rs, "count_widgets", &declared).is_empty());
    }

    #[test]
    fn undeclared_gate_features_is_empty_when_the_gate_is_declared() {
        let lib_rs = "\
#[cfg(feature = \"premium-tier\")]
pub fn create_premium_backend_options_from_json(json: String) -> Result<String, String> {
    Ok(json)
}
";
        let declared: HashSet<&str> = ["premium-tier"].into_iter().collect();

        assert!(undeclared_gate_features(lib_rs, "create_premium_backend_options_from_json", &declared).is_empty());
    }

    #[test]
    fn undeclared_gate_features_names_a_feature_the_manifest_never_declared() {
        let lib_rs = "\
#[cfg(feature = \"widgets\")]
pub fn count_widgets(collection: String) -> Result<i64, String> {
    Ok(0)
}
";
        let declared: HashSet<&str> = HashSet::new();

        let undeclared = undeclared_gate_features(lib_rs, "count_widgets", &declared);
        assert_eq!(
            undeclared,
            BTreeSet::from(["widgets".to_string()]),
            "the gate's feature name must be reported as undeclared: {undeclared:?}"
        );
    }

    const PAGE_RESULT_LIB_RS: &str = "\
#[frb(mirror(CrawlPageResult))]
pub struct CrawlPageResult {
    /// The page URL.
    pub url: String,
    /// Redirect hops taken to reach `final_url` from `url`.
    pub redirect_count: i64,
}
";

    const PAGE_RESULT_DART_UP_TO_DATE: &str = "\
class CrawlPageResult {
  final String url;

  /// Redirect hops taken to reach `final_url` from `url`.
  final PlatformInt64 redirectCount;

  const CrawlPageResult({required this.url, required this.redirectCount});
}

class OtherClass {
  final int redirectCountUnrelated;
}
";

    #[test]
    fn struct_field_pairs_lists_every_named_field_of_every_top_level_struct() {
        let lib_rs = "\
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub struct Tuple(pub String);

pub struct Unit;

pub enum Shape {
    Circle { pub radius: f64 },
}
";
        assert_eq!(
            struct_field_pairs(lib_rs),
            vec![
                ("Point".to_string(), "x".to_string()),
                ("Point".to_string(), "y".to_string()),
            ],
            "only the named-field struct's own fields are collected -- tuple/unit structs and \
             enum variant payloads are excluded"
        );
    }

    /// alef #437: a public struct field added or renamed after the Dart bridge was last
    /// generated (a skipped/no-op `flutter_rust_bridge_codegen` run) must be reported missing,
    /// exactly as [`missing_bridge_functions`] already does for a missing free function.
    #[test]
    fn missing_bridge_struct_fields_reports_a_facade_field_absent_from_the_bridge_class() {
        // The bridge's `CrawlPageResult` predates `redirect_count` entirely.
        let stale_bridge = "\
class CrawlPageResult {
  final String url;

  const CrawlPageResult({required this.url});
}
";
        let missing = missing_bridge_struct_fields(PAGE_RESULT_LIB_RS, stale_bridge, &[]);
        assert_eq!(
            missing,
            vec!["CrawlPageResult.redirect_count".to_string()],
            "a facade field with no matching Dart class property must be reported missing: {missing:?}"
        );
    }

    /// THE CONTROL. Once the bridge class already declares every facade field, nothing may be
    /// reported -- otherwise every up-to-date consumer repo would fail this check forever.
    #[test]
    fn missing_bridge_struct_fields_is_empty_when_every_facade_field_is_bridged() {
        let missing = missing_bridge_struct_fields(PAGE_RESULT_LIB_RS, PAGE_RESULT_DART_UP_TO_DATE, &[]);
        assert!(
            missing.is_empty(),
            "every facade field is present on the bridge class and must not be reported: {missing:?}"
        );
    }

    /// A field name reused on a DIFFERENT Dart class (`OtherClass.redirectCountUnrelated` above,
    /// which merely contains `redirectCount` as a substring of a longer identifier) must never
    /// stand in for `CrawlPageResult`'s own field -- this pins [`dart_class_body`]'s per-class
    /// scoping and [`contains_identifier_at_token_boundary`]'s whole-identifier match together, so
    /// a regression in either one is caught here even though each has its own direct coverage too.
    #[test]
    fn missing_bridge_struct_fields_does_not_cross_class_boundaries() {
        let stale_bridge = "\
class CrawlPageResult {
  final String url;

  const CrawlPageResult({required this.url});
}

class OtherClass {
  final int redirectCount;
}
";
        let missing = missing_bridge_struct_fields(PAGE_RESULT_LIB_RS, stale_bridge, &[]);
        assert_eq!(
            missing,
            vec!["CrawlPageResult.redirect_count".to_string()],
            "a same-named field on an unrelated class must not satisfy CrawlPageResult's own \
             coverage: {missing:?}"
        );
    }

    #[test]
    fn missing_bridge_struct_fields_ignores_configured_exclusions() {
        let stale_bridge = "\
class CrawlPageResult {
  final String url;

  const CrawlPageResult({required this.url});
}
";
        let missing = missing_bridge_struct_fields(PAGE_RESULT_LIB_RS, stale_bridge, &["CrawlPageResult".to_string()]);
        assert!(
            missing.is_empty(),
            "an excluded type's fields must not be reported missing: {missing:?}"
        );
    }

    #[test]
    fn dart_class_body_stops_at_the_next_top_level_type_header() {
        let body = dart_class_body(PAGE_RESULT_DART_UP_TO_DATE, "CrawlPageResult")
            .expect("CrawlPageResult class must be found");
        assert!(body.contains("redirectCount"));
        assert!(
            !body.contains("redirectCountUnrelated"),
            "the next class's body must not leak into CrawlPageResult's own: {body}"
        );
    }

    #[test]
    fn dart_class_body_is_none_for_an_absent_class() {
        assert!(dart_class_body(PAGE_RESULT_DART_UP_TO_DATE, "NeverEmitted").is_none());
    }
}
