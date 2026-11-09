//! Flag a declared `since` that names a release newer than the crate's own version.
//!
//! `since` lives in two places on every versioned IR item: [`VersionAnnotation::since`] (from
//! `#[alef(since = "...")]`) and [`DeprecationInfo::since`] (from `#[deprecated(since =
//! "...")]`). A value in either field that is newer than [`ApiSurface::version`] claims the item
//! was introduced (or deprecated) in a release that has not shipped yet -- always wrong,
//! regardless of what the item is.

use super::{ValidationCode, ValidationDiagnostic};
use crate::core::ir::{ApiSurface, VersionAnnotation};

/// Parse a version string leniently for comparison purposes.
///
/// Tries strict semver first. Falls back to padding a bare `MAJOR.MINOR` (digits and exactly
/// one dot, no pre-release/build suffix) with `.0`, because `#[alef(since = "...")]` authors
/// commonly write two-component version labels while `semver::Version` requires all three.
/// Anything else -- a stray `v` prefix, a dash without valid semver pre-release grammar,
/// non-numeric text -- is reported as unparseable rather than guessed at. ~keep
fn parse_lenient_version(raw: &str) -> Option<semver::Version> {
    let trimmed = raw.trim();
    if let Ok(version) = semver::Version::parse(trimmed) {
        return Some(version);
    }
    let is_major_minor =
        trimmed.matches('.').count() == 1 && trimmed.chars().all(|ch| ch.is_ascii_digit() || ch == '.');
    if is_major_minor {
        return semver::Version::parse(&format!("{trimmed}.0")).ok();
    }
    None
}

/// Flag every `since` (plain or `deprecated(since = ...)`) that exceeds the crate's own
/// version, across the seven IR item kinds that carry a [`VersionAnnotation`].
///
/// Fires unconditionally rather than being scoped to `resolved_languages`: a wrong `since` is
/// a documentation-metadata defect, not a backend-specific one -- every generated language
/// surfaces the same doc comment. `binding_excluded` items are skipped, matching every other
/// check in this module: an excluded item never reaches a generated doc, so a bogus `since` on
/// it has no observable effect. ~keep
pub(super) fn since_version_diagnostics(api: &ApiSurface) -> Vec<ValidationDiagnostic> {
    let mut diagnostics = Vec::new();
    // A crate version that fails to parse is a different defect than a bad `since` -- Cargo
    // already enforces valid semver for `[package] version` -- and guessing at an ordering
    // against an unparseable baseline would be worse than skipping the whole check. ~keep
    let Some(crate_version) = parse_lenient_version(&api.version) else {
        return diagnostics;
    };

    for function in &api.functions {
        if function.binding_excluded {
            continue;
        }
        check_version_annotation(
            api,
            &crate_version,
            &format!("function {}", function.name),
            &function.version,
            &mut diagnostics,
        );
    }
    for typ in &api.types {
        if typ.binding_excluded {
            continue;
        }
        check_version_annotation(
            api,
            &crate_version,
            &format!("type {}", typ.name),
            &typ.version,
            &mut diagnostics,
        );
        for method in &typ.methods {
            if method.binding_excluded {
                continue;
            }
            check_version_annotation(
                api,
                &crate_version,
                &format!("method {}.{}", typ.name, method.name),
                &method.version,
                &mut diagnostics,
            );
        }
        for field in &typ.fields {
            if field.binding_excluded {
                continue;
            }
            check_version_annotation(
                api,
                &crate_version,
                &format!("field {}.{}", typ.name, field.name),
                &field.version,
                &mut diagnostics,
            );
        }
    }
    for enum_def in &api.enums {
        if enum_def.binding_excluded {
            continue;
        }
        check_version_annotation(
            api,
            &crate_version,
            &format!("enum {}", enum_def.name),
            &enum_def.version,
            &mut diagnostics,
        );
        for variant in &enum_def.variants {
            if variant.binding_excluded {
                continue;
            }
            check_version_annotation(
                api,
                &crate_version,
                &format!("enum variant {}.{}", enum_def.name, variant.name),
                &variant.version,
                &mut diagnostics,
            );
            for field in &variant.fields {
                if field.binding_excluded {
                    continue;
                }
                check_version_annotation(
                    api,
                    &crate_version,
                    &format!("enum variant {}.{} field {}", enum_def.name, variant.name, field.name),
                    &field.version,
                    &mut diagnostics,
                );
            }
        }
    }
    for error_def in &api.errors {
        if error_def.binding_excluded {
            continue;
        }
        check_version_annotation(
            api,
            &crate_version,
            &format!("error {}", error_def.name),
            &error_def.version,
            &mut diagnostics,
        );
        for method in &error_def.methods {
            if method.binding_excluded {
                continue;
            }
            check_version_annotation(
                api,
                &crate_version,
                &format!("error method {}.{}", error_def.name, method.name),
                &method.version,
                &mut diagnostics,
            );
        }
        for variant in &error_def.variants {
            for field in &variant.fields {
                if field.binding_excluded {
                    continue;
                }
                check_version_annotation(
                    api,
                    &crate_version,
                    &format!("error variant {}.{} field {}", error_def.name, variant.name, field.name),
                    &field.version,
                    &mut diagnostics,
                );
            }
        }
    }
    diagnostics
}

fn check_version_annotation(
    api: &ApiSurface,
    crate_version: &semver::Version,
    item_path: &str,
    annotation: &VersionAnnotation,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    if let Some(since) = annotation.since.as_deref() {
        check_since_value(api, crate_version, item_path, "since", since, diagnostics);
    }
    if let Some(since) = annotation.deprecated.as_ref().and_then(|info| info.since.as_deref()) {
        check_since_value(api, crate_version, item_path, "deprecated(since)", since, diagnostics);
    }
}

fn check_since_value(
    api: &ApiSurface,
    crate_version: &semver::Version,
    item_path: &str,
    label: &str,
    since: &str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let Some(since_version) = parse_lenient_version(since) else {
        diagnostics.push(ValidationDiagnostic::warning(
            ValidationCode::SinceVersionUnparseable,
            api.crate_name.clone(),
            None,
            Some(item_path.to_string()),
            format!(
                "`{label} = \"{since}\"` is not a parseable version; alef could not check it against crate version `{}`",
                api.version
            ),
            "record `since`/`deprecated(since = ...)` as MAJOR.MINOR.PATCH (or MAJOR.MINOR) so alef can validate it \
             against the crate's own version",
        ));
        return;
    };
    // `Version`'s derived `Ord`/`PartialOrd` (the `>` operator) totally orders build metadata
    // too, so two versions differing only in `+build` are NOT equal under it -- useful for
    // sorting, wrong for this check. `cmp_precedence` implements the SemVer-spec rule that
    // build metadata never affects precedence, which is what "is this since actually newer"
    // means here. ~keep
    if since_version.cmp_precedence(crate_version) == std::cmp::Ordering::Greater {
        diagnostics.push(ValidationDiagnostic::warning(
            ValidationCode::SinceNewerThanCrateVersion,
            api.crate_name.clone(),
            None,
            Some(item_path.to_string()),
            format!(
                "`{label} = \"{since}\"` is newer than the crate's own version `{}`",
                api.version
            ),
            "correct the declared version to a release that has actually shipped",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{DeprecationInfo, FunctionDef, ParamDef, TypeRef};
    use crate::core::validation::ValidationSeverity;

    fn function_with_version(name: &str, version: VersionAnnotation) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("sample_lib::{name}"),
            original_rust_path: String::new(),
            params: Vec::<ParamDef>::new(),
            return_type: TypeRef::Unit,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version,
        }
    }

    fn api_with_function(crate_version: &str, item_version: VersionAnnotation) -> ApiSurface {
        ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: crate_version.to_string(),
            functions: vec![function_with_version("do_thing", item_version)],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn since_newer_than_crate_version_fires() {
        let api = api_with_function(
            "1.0.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("2.0.0".to_string()),
                deprecated: None,
            },
        );

        let diagnostics = since_version_diagnostics(&api);

        assert_eq!(diagnostics.len(), 1, "exactly one since-exceeds-crate diagnostic");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.severity, ValidationSeverity::Warning);
        assert_eq!(diagnostic.code, ValidationCode::SinceNewerThanCrateVersion);
        assert_eq!(diagnostic.item_path.as_deref(), Some("function do_thing"));
        assert_eq!(
            diagnostic.reason,
            "`since = \"2.0.0\"` is newer than the crate's own version `1.0.0`"
        );
        assert_eq!(
            diagnostic.suggested_fix,
            "correct the declared version to a release that has actually shipped"
        );
    }

    #[test]
    fn since_equal_to_crate_version_does_not_fire() {
        let api = api_with_function(
            "1.0.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("1.0.0".to_string()),
                deprecated: None,
            },
        );

        assert!(
            since_version_diagnostics(&api).is_empty(),
            "since equal to the crate version must not be flagged"
        );
    }

    #[test]
    fn since_older_than_crate_version_does_not_fire() {
        let api = api_with_function(
            "1.5.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("1.0.0".to_string()),
                deprecated: None,
            },
        );

        assert!(
            since_version_diagnostics(&api).is_empty(),
            "since older than the crate version must not be flagged"
        );
    }

    #[test]
    fn deprecated_since_newer_than_crate_version_fires() {
        let api = api_with_function(
            "1.0.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: None,
                deprecated: Some(DeprecationInfo {
                    since: Some("3.0.0".to_string()),
                    note: None,
                }),
            },
        );

        let diagnostics = since_version_diagnostics(&api);

        assert_eq!(
            diagnostics.len(),
            1,
            "exactly one deprecated-since-exceeds-crate diagnostic"
        );
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.severity, ValidationSeverity::Warning);
        assert_eq!(diagnostic.code, ValidationCode::SinceNewerThanCrateVersion);
        assert_eq!(diagnostic.item_path.as_deref(), Some("function do_thing"));
        assert_eq!(
            diagnostic.reason,
            "`deprecated(since) = \"3.0.0\"` is newer than the crate's own version `1.0.0`"
        );
    }

    #[test]
    fn unparseable_since_is_reported_not_silently_skipped() {
        let api = api_with_function(
            "1.0.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("not-a-version".to_string()),
                deprecated: None,
            },
        );

        let diagnostics = since_version_diagnostics(&api);

        assert_eq!(
            diagnostics.len(),
            1,
            "an unparseable since must still surface a diagnostic"
        );
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.severity, ValidationSeverity::Warning);
        assert_eq!(diagnostic.code, ValidationCode::SinceVersionUnparseable);
        assert_eq!(diagnostic.item_path.as_deref(), Some("function do_thing"));
        assert_eq!(
            diagnostic.reason,
            "`since = \"not-a-version\"` is not a parseable version; alef could not check it against crate version \
             `1.0.0`"
        );
    }

    #[test]
    fn major_minor_since_is_parsed_leniently() {
        // A two-component `since` ("1.2", no patch) must compare correctly against a full
        // crate version, not be treated as unparseable. ~keep
        let api = api_with_function(
            "1.2.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("1.3".to_string()),
                deprecated: None,
            },
        );

        let diagnostics = since_version_diagnostics(&api);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, ValidationCode::SinceNewerThanCrateVersion);
    }

    #[test]
    fn prerelease_and_build_metadata_since_compare_correctly() {
        // `1.2.0-rc.1` and `1.2.0+build` around a `1.2.0` crate version: the pre-release is
        // strictly less than the release it precedes, and build metadata does not affect
        // ordering at all, per semver. Neither must fire.
        let api = ApiSurface {
            crate_name: "sample-lib".to_string(),
            version: "1.2.0".to_string(),
            functions: vec![
                function_with_version(
                    "before_release",
                    VersionAnnotation {
                        constructor_history: Vec::new(),
                        legacy_constructor_fields: Vec::new(),
                        declared_constructor_order: false,
                        since: Some("1.2.0-rc.1".to_string()),
                        deprecated: None,
                    },
                ),
                function_with_version(
                    "same_release_with_build",
                    VersionAnnotation {
                        constructor_history: Vec::new(),
                        legacy_constructor_fields: Vec::new(),
                        declared_constructor_order: false,
                        since: Some("1.2.0+build".to_string()),
                        deprecated: None,
                    },
                ),
            ],
            ..ApiSurface::default()
        };

        assert!(since_version_diagnostics(&api).is_empty());
    }

    #[test]
    fn binding_excluded_function_is_skipped() {
        let mut api = api_with_function(
            "1.0.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("9.0.0".to_string()),
                deprecated: None,
            },
        );
        api.functions[0].binding_excluded = true;

        assert!(
            since_version_diagnostics(&api).is_empty(),
            "a binding_excluded item never reaches a generated doc, so its since is not checked"
        );
    }

    #[test]
    fn unparseable_crate_version_skips_the_whole_check() {
        let api = api_with_function(
            "not-a-crate-version",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("9.0.0".to_string()),
                deprecated: None,
            },
        );

        assert!(
            since_version_diagnostics(&api).is_empty(),
            "no baseline to compare against, so this defect is out of scope for this check"
        );
    }

    /// Confirms this check is actually wired into the public entry point every caller (and
    /// `cli/pipeline/generate/validation.rs`'s warning print loop) goes through -- not just
    /// reachable in isolation via `since_version_diagnostics`. `validate_api_surface` is the
    /// same function `ValidatedApiSurface::new` and the generation pipeline call. ~keep
    #[test]
    fn reaches_the_public_validate_api_surface_entry_point_as_a_warning() {
        let api = api_with_function(
            "1.0.0",
            VersionAnnotation {
                constructor_history: Vec::new(),
                legacy_constructor_fields: Vec::new(),
                declared_constructor_order: false,
                since: Some("2.0.0".to_string()),
                deprecated: None,
            },
        );

        let report = crate::core::validation::validate_api_surface(&api);

        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == ValidationCode::SinceNewerThanCrateVersion)
            .expect("since-exceeds-crate diagnostic must reach the public validation report");
        assert_eq!(diagnostic.severity, ValidationSeverity::Warning);
        assert!(
            !report.has_errors(),
            "a wrong `since` must never be Error severity -- it must not abort a consumer's build"
        );
    }
}
