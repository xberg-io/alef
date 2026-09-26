//! Origin-substitution token detection for `mock_responses` route-array fixtures.
//!
//! Split out of `fixture.rs` (already over the file-modularization cap) so this predicate has
//! its own home rather than growing the parent further -- see CLAUDE.md's
//! `file-modularization` rule.
//!
//! The generated standalone mock-server binary (`codegen/rust/mock_server/route_loading.rs`,
//! a sibling module this one does not touch) rewrites four placeholders in response bodies and
//! header values at serve time (alef issue #433):
//!
//! | token                        | value                                  |
//! |-------------------------------|-----------------------------------------|
//! | `{{mock_origin}}`             | `http://127.0.0.1:<port>`               |
//! | `{{mock_alt_origin}}`         | `http://<alt_host>:<port>`              |
//! | `{{mock_sub_origin}}`         | `http://sub.<alt_host>:<port>`          |
//! | `{{mock_foreign_origin}}`     | `http://127.0.0.2:<port>`               |
//!
//! Every one of these resolves to a host-absolute URL. A fixture that serves one of them needs
//! a dedicated per-fixture listener at the origin root: the shared, `/fixtures/<id>`-namespaced
//! route table has no entry for the path a client resolves against that URL, so the request
//! 404s for a reason that has nothing to do with what the fixture is testing. This is the same
//! failure shape the sibling `is_host_root_path` / inline-host-link checks in `fixture.rs`
//! already guard against for relative redirect targets and `href="/"` links -- this module
//! extends that guard to full origin URLs. ~keep

use serde_json::Value;

/// The four origin-substitution placeholders a generated mock-server response may contain.
/// See the module doc for what each one resolves to. ~keep
pub const ORIGIN_SUBSTITUTION_TOKENS: [&str; 4] = [
    "{{mock_origin}}",
    "{{mock_alt_origin}}",
    "{{mock_sub_origin}}",
    "{{mock_foreign_origin}}",
];

/// `ORIGIN_SUBSTITUTION_TOKENS` joined for display in validation/error messages.
pub fn known_tokens_display() -> String {
    ORIGIN_SUBSTITUTION_TOKENS.join(", ")
}

/// True if `value` contains one of [`ORIGIN_SUBSTITUTION_TOKENS`] verbatim.
pub fn contains_origin_token(value: &str) -> bool {
    ORIGIN_SUBSTITUTION_TOKENS.iter().any(|token| value.contains(token))
}

/// True if a single `mock_responses` route-array entry's `body_inline` or any header value
/// carries an origin-substitution token, meaning the fixture it belongs to needs a dedicated
/// per-fixture listener rather than the shared `/fixtures/<id>`-namespaced one.
pub fn entry_has_origin_token(entry: &Value) -> bool {
    let body_has_token = entry
        .get("body_inline")
        .and_then(Value::as_str)
        .is_some_and(contains_origin_token);
    let header_has_token = entry
        .get("headers")
        .and_then(Value::as_object)
        .is_some_and(|headers| headers.values().any(|v| v.as_str().is_some_and(contains_origin_token)));
    body_has_token || header_has_token
}

/// Scan `value` for a `{{mock_...}}`-shaped placeholder that is not one of
/// [`ORIGIN_SUBSTITUTION_TOKENS`], returning the first one found verbatim.
///
/// Catches a typo'd or made-up origin token before it reaches a client under test as literal,
/// unresolved text instead of the URL the fixture author intended.
pub fn find_unknown_origin_token(value: &str) -> Option<String> {
    const PREFIX: &str = "{{mock_";
    const SUFFIX: &str = "}}";

    let mut rest = value;
    while let Some(start) = rest.find(PREFIX) {
        let candidate_and_beyond = &rest[start..];
        let Some(end) = candidate_and_beyond.find(SUFFIX) else {
            // An opened-but-unterminated placeholder is malformed on its own.
            return Some(candidate_and_beyond.to_string());
        };
        let candidate = &candidate_and_beyond[..end + SUFFIX.len()];
        if !ORIGIN_SUBSTITUTION_TOKENS.contains(&candidate) {
            return Some(candidate.to_string());
        }
        rest = &candidate_and_beyond[end + SUFFIX.len()..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_origin_token_true_for_each_known_token() {
        for token in ORIGIN_SUBSTITUTION_TOKENS {
            let body = format!("<a href=\"{token}/page2\">link</a>");
            assert!(
                contains_origin_token(&body),
                "expected {token} to be detected in: {body}"
            );
        }
    }

    #[test]
    fn contains_origin_token_false_for_plain_text() {
        assert!(!contains_origin_token("<html><body>no tokens here</body></html>"));
    }

    #[test]
    fn entry_has_origin_token_true_for_body_inline_token() {
        let entry = serde_json::json!({
            "path": "/",
            "status_code": 200,
            "body_inline": "<a href=\"{{mock_alt_origin}}/page2\">cross-host</a>"
        });
        assert!(entry_has_origin_token(&entry));
    }

    #[test]
    fn entry_has_origin_token_true_for_header_token() {
        let entry = serde_json::json!({
            "path": "/",
            "status_code": 302,
            "headers": {"Location": "{{mock_sub_origin}}/final"},
            "body_inline": ""
        });
        assert!(entry_has_origin_token(&entry));
    }

    #[test]
    fn entry_has_origin_token_false_when_absent() {
        let entry = serde_json::json!({"path": "/data.json", "status_code": 200, "body_inline": "{}"});
        assert!(!entry_has_origin_token(&entry));
    }

    #[test]
    fn find_unknown_origin_token_none_for_known_tokens() {
        let body = "{{mock_origin}} and {{mock_alt_origin}} and {{mock_sub_origin}} and {{mock_foreign_origin}}";
        assert_eq!(find_unknown_origin_token(body), None);
    }

    #[test]
    fn find_unknown_origin_token_detects_typo() {
        let body = "see {{mock_alt_orgin}} for details";
        assert_eq!(find_unknown_origin_token(body), Some("{{mock_alt_orgin}}".to_string()));
    }

    #[test]
    fn find_unknown_origin_token_detects_unterminated_placeholder() {
        let body = "see {{mock_alt_origin for details";
        assert_eq!(
            find_unknown_origin_token(body),
            Some("{{mock_alt_origin for details".to_string())
        );
    }

    #[test]
    fn find_unknown_origin_token_ignores_unrelated_double_brace_text() {
        assert_eq!(find_unknown_origin_token("digest is {{digest}}"), None);
    }
}
