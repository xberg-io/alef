//! Crate-filter dispatch helper.
//!
//! [`select_crates`] resolves the `--crate` filter list against the resolved
//! crate vector loaded from `alef.toml`.  When no filter is given, every crate
//! is returned in declaration order.

use crate::core::config::ResolvedCrateConfig;
use anyhow::{Result, bail};

/// Resolve the `--crate` filter against the resolved crate vector.
///
/// - If `filter` is empty: returns every crate in declaration order.
/// - If `filter` is non-empty: returns crates matching by `name`, in the order
///   specified by `filter`.
/// - Errors on the first unknown name with a clear message listing known crates.
pub fn select_crates<'a>(
    resolved: &'a [ResolvedCrateConfig],
    filter: &[String],
) -> Result<Vec<&'a ResolvedCrateConfig>> {
    if filter.is_empty() {
        return Ok(resolved.iter().collect());
    }

    let mut result = Vec::with_capacity(filter.len());
    for name in filter {
        match resolved.iter().find(|c| &c.name == name) {
            Some(c) => result.push(c),
            None => {
                let known: Vec<&str> = resolved.iter().map(|c| c.name.as_str()).collect();
                bail!("crate `{name}` not found in workspace; known: {}", known.join(", "));
            }
        }
    }
    Ok(result)
}

/// Return `true` when multiple crates are in `crates_to_process`, so callers
/// can decide whether to prefix output lines with the crate name.
pub fn is_multi_crate(crates_to_process: &[&ResolvedCrateConfig]) -> bool {
    crates_to_process.len() > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(name: &str) -> ResolvedCrateConfig {
        let toml = format!(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "{name}"
sources = ["src/lib.rs"]
"#
        );
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(&toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn empty_filter_returns_all_crates() {
        let a = make_config("alpha");
        let b = make_config("beta");
        let resolved = vec![a, b];
        let result = select_crates(&resolved, &[]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "alpha");
        assert_eq!(result[1].name, "beta");
    }

    #[test]
    fn filter_returns_matching_crates_in_filter_order() {
        let a = make_config("alpha");
        let b = make_config("beta");
        let c = make_config("gamma");
        let resolved = vec![a, b, c];
        let filter = vec!["gamma".to_string(), "alpha".to_string()];
        let result = select_crates(&resolved, &filter).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "gamma");
        assert_eq!(result[1].name, "alpha");
    }

    #[test]
    fn unknown_name_produces_clear_error() {
        let a = make_config("alpha");
        let b = make_config("beta");
        let resolved = vec![a, b];
        let filter = vec!["unknown".to_string()];
        let err = select_crates(&resolved, &filter).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("crate `unknown` not found"), "msg: {msg}");
        assert!(msg.contains("alpha"), "msg: {msg}");
        assert!(msg.contains("beta"), "msg: {msg}");
    }

    #[test]
    fn mixed_valid_invalid_errors_on_first_unknown() {
        let a = make_config("alpha");
        let b = make_config("beta");
        let resolved = vec![a, b];
        let filter = vec!["alpha".to_string(), "missing".to_string()];
        let err = select_crates(&resolved, &filter).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("crate `missing` not found"), "msg: {msg}");
    }

    #[test]
    fn is_multi_crate_returns_false_for_single() {
        let a = make_config("alpha");
        let resolved = [a];
        let refs: Vec<&ResolvedCrateConfig> = resolved.iter().collect();
        assert!(!is_multi_crate(&refs));
    }

    #[test]
    fn is_multi_crate_returns_true_for_two() {
        let a = make_config("alpha");
        let b = make_config("beta");
        let resolved = [a, b];
        let refs: Vec<&ResolvedCrateConfig> = resolved.iter().collect();
        assert!(is_multi_crate(&refs));
    }
}
