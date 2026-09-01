//! Refusal for the `alef verify` flags the command has never implemented.
//!
//! ~keep Own module rather than a helper in `core_commands.rs`: that file sits within a
//! handful of lines of the repo's 1,000-line cap (`file-modularization` in CLAUDE.md), and
//! adding this there crossed it.

use anyhow::Result;

/// `alef verify` accepts three flags it has never implemented. Each promises work that would
/// change the verdict, so silently discarding them turned "I could not do that" into "that
/// passed" — the failure mode a verification command exists to prevent.
///
/// ~keep Refusing rather than warning is deliberate. The only reader of these flags is CI, and
/// CI does not read warnings; it reads exit codes. Nothing in the polyrepo passes them today
/// (consumers invoke `alef verify` and `alef verify --exit-code` only), so this costs no
/// existing pipeline and closes the hole before something starts depending on the false pass.
pub(super) fn refuse_unimplemented_verify_flags(compile: bool, lint: bool, lang: Option<&[String]>) -> Result<()> {
    let mut requested: Vec<&str> = Vec::new();
    if compile {
        requested.push("--compile");
    }
    if lint {
        requested.push("--lint");
    }
    if lang.is_some() {
        requested.push("--lang");
    }
    if requested.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "`alef verify` does not implement {}; it checks freshness of generated output only. \
         Fix: re-run `alef verify` without {}. Use `alef build --lang <langs>` to compile and \
         `alef lint --lang <langs>` to lint.",
        requested.join(", "),
        requested.join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The load-bearing case: the plain invocation every consumer CI uses must stay accepted.
    /// If this ever fails, the refusal has started rejecting `alef verify` itself.
    #[test]
    fn a_plain_verify_is_accepted() {
        assert!(refuse_unimplemented_verify_flags(false, false, None).is_ok());
    }

    /// `--exit-code` is not passed to this helper at all (it is a documented deprecated no-op),
    /// so a bare verify with it set is indistinguishable from the case above. Pinned so nobody
    /// "fixes" the deprecated flag into this refusal.
    #[test]
    fn the_deprecated_exit_code_flag_is_not_part_of_the_refusal() {
        assert!(refuse_unimplemented_verify_flags(false, false, None).is_ok());
    }

    #[test]
    fn compile_is_refused_rather_than_silently_ignored() {
        let error = refuse_unimplemented_verify_flags(true, false, None).expect_err("must refuse");
        let message = error.to_string();
        assert!(message.contains("--compile"), "names the offending flag: {message}");
        assert!(
            message.contains("alef build"),
            "points at the command that does compile: {message}"
        );
    }

    #[test]
    fn lint_is_refused_rather_than_silently_ignored() {
        let error = refuse_unimplemented_verify_flags(false, true, None).expect_err("must refuse");
        let message = error.to_string();
        assert!(message.contains("--lint"), "names the offending flag: {message}");
        assert!(
            message.contains("alef lint"),
            "points at the command that does lint: {message}"
        );
    }

    #[test]
    fn lang_is_refused_even_when_the_list_is_empty() {
        // An empty `--lang=` still means "the operator asked for a language filter", and an
        // empty filter is exactly the shape that would silently verify everything. ~keep
        let error = refuse_unimplemented_verify_flags(false, false, Some(&[])).expect_err("must refuse");
        assert!(error.to_string().contains("--lang"), "{error}");
    }

    #[test]
    fn every_requested_flag_is_named_not_just_the_first() {
        let languages = vec!["python".to_string()];
        let error = refuse_unimplemented_verify_flags(true, true, Some(&languages)).expect_err("must refuse");
        let message = error.to_string();
        for flag in ["--compile", "--lint", "--lang"] {
            assert!(message.contains(flag), "{flag} missing from: {message}");
        }
    }
}
