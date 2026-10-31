//! Reads `.github/workflows/ci.yml` for the tests that pin a `ALEF_REQUIRE_*` oracle to the
//! install step that actually puts its tool on `PATH`.

/// The CI workflow as committed, so the shape tests read exactly what GitHub runs.
pub(crate) const CI_WORKFLOW: &str = include_str!("../../.github/workflows/ci.yml");

const SETUP_MISE: &str = "xberg-io/actions/setup-mise@v1";

/// Whether `job` has an unconditional `setup-mise` step whose `tools` list names `tool`.
///
/// Unconditional matters: a step behind an `if:` installs the tool on some matrix legs only,
/// while the `ALEF_REQUIRE_*` variable it backs fails every leg that lacks the tool. Matching is
/// per whitespace-separated entry, so `go` is not satisfied by `github:foo/go-tool`. ~keep
pub(crate) fn job_installs_with_mise(workflow: &str, job: &str, tool: &str) -> bool {
    let parsed: serde_json::Value = serde_saphyr::from_str(workflow).expect("CI workflow YAML");
    let Some(steps) = parsed["jobs"][job]["steps"].as_array() else {
        return false;
    };
    steps.iter().any(|step| {
        step["uses"].as_str() == Some(SETUP_MISE)
            && step.get("if").is_none()
            && step["with"]["tools"]
                .as_str()
                .is_some_and(|tools| tools.split_whitespace().any(|entry| entry == tool))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = "jobs:
  test:
    steps:
      - uses: xberg-io/actions/setup-mise@v1
        with:
          tools: go github:wasm-bindgen/wasm-bindgen
      - uses: xberg-io/actions/setup-mise@v1
        if: runner.os != 'Windows'
        with:
          tools: ruby
";

    #[test]
    fn finds_a_tool_in_an_unconditional_setup_mise_step() {
        assert!(job_installs_with_mise(WORKFLOW, "test", "go"));
        assert!(job_installs_with_mise(
            WORKFLOW,
            "test",
            "github:wasm-bindgen/wasm-bindgen"
        ));
    }

    #[test]
    fn rejects_a_tool_installed_only_on_some_legs() {
        assert!(!job_installs_with_mise(WORKFLOW, "test", "ruby"));
    }

    #[test]
    fn rejects_a_missing_tool_a_partial_entry_and_a_missing_job() {
        assert!(!job_installs_with_mise(WORKFLOW, "test", "node"));
        assert!(!job_installs_with_mise(WORKFLOW, "test", "wasm-bindgen"));
        assert!(!job_installs_with_mise(WORKFLOW, "build", "go"));
    }
}
