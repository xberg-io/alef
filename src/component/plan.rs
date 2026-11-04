//! Release planning: decide, per component and target, whether to build or reuse a locked artifact.
//!
//! A plan row is `reuse` only when the previous lock holds an entry with the same input
//! fingerprint (`input_hash`) and the same identity apart from `version`: crate, component,
//! target, feature hash and contract hash. Everything else is `build`. A row with no fingerprint
//! is always `build` in the decision model; the CLI only plans producers it can fingerprint.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::artifact::{ComponentDeliveryMode, ComponentLock, ComponentLockEntry};

/// Version of the plan file format.
pub const PLAN_SCHEMA_VERSION: u32 = 1;

/// What to do with one component and target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAction {
    /// Compile, package and lock it afresh.
    Build,
    /// Carry the previous lock entry over unchanged.
    Reuse,
}

/// One planned component and target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRow {
    #[serde(rename = "crate")]
    pub crate_name: String,
    pub component: String,
    pub target: String,
    /// The build input fingerprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_hash: Option<String>,
    pub action: PlanAction,
    /// Configured version of the release. A fresh artifact must carry this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Portable source fingerprint checked on the lock assembly host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    /// The OS family of the machine that made the plan; absent in plans from older alef versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_family: Option<crate::component::fingerprint::HostFamily>,
    /// The lock entry to carry over, present exactly when `action` is `reuse`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<ComponentLockEntry>,
}

/// The planner's output: `alef component plan --output`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentPlan {
    pub schema_version: u32,
    /// Whether the fingerprints are for debug builds (`--debug`); `build` and `package` must agree.
    #[serde(default)]
    pub debug: bool,
    pub rows: Vec<PlanRow>,
}

/// A component and target about to be planned, with the identity parts a reused entry must match.
#[derive(Debug, Clone)]
pub struct PlanCandidate {
    pub crate_name: String,
    pub component: String,
    pub target: String,
    pub feature_hash: String,
    pub contract_hash: String,
    pub input_hash: Option<String>,
}

impl PlanCandidate {
    fn matches(&self, entry: &ComponentLockEntry) -> bool {
        let identity = &entry.identity;
        identity.crate_name == self.crate_name
            && identity.component == self.component
            && identity.target == self.target
            && identity.feature_hash == self.feature_hash
            && identity.contract_hash == self.contract_hash
            && entry.mode == ComponentDeliveryMode::Download
            && self.input_hash.is_some()
            && entry.input_hash == self.input_hash
    }
}

/// Decide `build` or `reuse` for one candidate against the previous lock's entries.
///
/// # Errors
///
/// Fails when more than one previous entry matches, since the lock then holds two artifacts for one
/// component and target and the plan cannot say which to carry over.
pub fn decide(candidate: &PlanCandidate, previous: &[ComponentLockEntry]) -> Result<PlanRow> {
    let matching = previous
        .iter()
        .filter(|entry| candidate.matches(entry))
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        let described = matching
            .iter()
            .map(|entry| format!("{} ({})", entry.identity.version, entry.url))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "the previous lock has {} entries that match {}/{} for {}: {described}",
            matching.len(),
            candidate.crate_name,
            candidate.component,
            candidate.target
        );
    }
    let reusable = matching.first().copied();
    Ok(PlanRow {
        crate_name: candidate.crate_name.clone(),
        component: candidate.component.clone(),
        target: candidate.target.clone(),
        input_hash: candidate.input_hash.clone(),
        action: if reusable.is_some() {
            PlanAction::Reuse
        } else {
            PlanAction::Build
        },
        version: None,
        source_hash: None,
        host_family: None,
        previous: reusable.cloned(),
    })
}

impl ComponentPlan {
    /// A plan over `rows`, sorted by crate, component and target.
    #[must_use]
    pub fn new(debug: bool, mut rows: Vec<PlanRow>) -> Self {
        rows.sort_by(|left, right| {
            (&left.crate_name, &left.component, &left.target).cmp(&(&right.crate_name, &right.component, &right.target))
        });
        Self {
            schema_version: PLAN_SCHEMA_VERSION,
            debug,
            rows,
        }
    }

    /// Read and validate a plan file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or parsed, has an unknown schema version, or holds a
    /// `reuse` row without a `previous` entry (or the reverse) or without an `input_hash`.
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("failed to read component plan {}", path.display()))?;
        let plan: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse component plan {}", path.display()))?;
        ensure!(
            plan.schema_version == PLAN_SCHEMA_VERSION,
            "unsupported component plan schema version {} (expected {PLAN_SCHEMA_VERSION})",
            plan.schema_version
        );
        for row in &plan.rows {
            ensure!(
                (row.action == PlanAction::Reuse) == row.previous.is_some(),
                "plan row {}/{} for {} must carry `previous` exactly when its action is `reuse`",
                row.crate_name,
                row.component,
                row.target
            );
            ensure!(
                row.action == PlanAction::Build || row.input_hash.is_some(),
                "plan row {}/{} for {} reuses an artifact but has no `input_hash`",
                row.crate_name,
                row.component,
                row.target
            );
        }
        Ok(plan)
    }

    /// The row for a triple, if the plan has one.
    #[must_use]
    pub fn row(&self, crate_name: &str, component: &str, target: &str) -> Option<&PlanRow> {
        self.rows
            .iter()
            .find(|row| row.crate_name == crate_name && row.component == component && row.target == target)
    }

    /// Whether `build` and `package` should handle a selected triple: its row when the plan says
    /// `build`, `None` when it says `reuse`.
    ///
    /// # Errors
    ///
    /// Fails when the plan has no row for the triple.
    pub fn build_row(&self, crate_name: &str, component: &str, target: &str) -> Result<Option<&PlanRow>> {
        let row = self
            .row(crate_name, component, target)
            .with_context(|| format!("{crate_name}/{component} for {target} is not in the plan"))?;
        Ok((row.action == PlanAction::Build).then_some(row))
    }
}

/// The previous-lock entries a plan reuses for `crate_name`, copied verbatim.
///
/// Each `reuse` row must name an entry present, unchanged, in `previous`. The entry's signing key
/// must still be trusted: with `trusted` set (a signed lock) the entry's `key_id` must be in it and
/// `previous` must list the same public key under that id; with `trusted` unset (a dev lock) the
/// entry must be unsigned.
///
/// # Errors
///
/// Fails on a row naming an entry that `previous` does not hold, or an untrusted signing key.
pub fn reused_entries(
    plan: &ComponentPlan,
    previous: &ComponentLock,
    crate_name: &str,
    trusted: Option<&BTreeMap<String, String>>,
) -> Result<Vec<ComponentLockEntry>> {
    let mut reused = Vec::new();
    for row in plan
        .rows
        .iter()
        .filter(|row| row.action == PlanAction::Reuse && row.crate_name == crate_name)
    {
        let wanted = row.previous.as_ref().context("a reuse row has no previous entry")?;
        let identity = &wanted.identity;
        ensure!(
            identity.crate_name == row.crate_name
                && identity.component == row.component
                && identity.target == row.target
                && wanted.mode == ComponentDeliveryMode::Download,
            "reuse entry identity does not match plan row {}/{} for {}",
            row.crate_name,
            row.component,
            row.target
        );
        ensure!(
            previous.artifacts.contains(wanted),
            "plan reuses {}/{} for {} but the previous lock has no identical entry",
            row.crate_name,
            row.component,
            row.target
        );
        ensure!(
            wanted.input_hash.is_some() && wanted.input_hash == row.input_hash,
            "plan reuses {}/{} for {} but the previous entry was built from different inputs",
            row.crate_name,
            row.component,
            row.target
        );
        match trusted {
            Some(trusted) => {
                let key = trusted.get(&wanted.key_id);
                ensure!(
                    key.is_some() && previous.public_keys.get(&wanted.key_id) == key,
                    "cannot reuse {}/{} for {}: signing key `{}` is not among the trusted public keys",
                    row.crate_name,
                    row.component,
                    row.target,
                    wanted.key_id
                );
            }
            None => ensure!(
                wanted.key_id.is_empty(),
                "cannot reuse signed entry {}/{} for {} in an unsigned lock",
                row.crate_name,
                row.component,
                row.target
            ),
        }
        reused.push(wanted.clone());
    }
    let distinct = reused
        .iter()
        .map(|entry| (&entry.identity.component, &entry.identity.target))
        .collect::<BTreeSet<_>>();
    if distinct.len() != reused.len() {
        bail!("plan reuses the same artifact twice for crate `{crate_name}`");
    }
    Ok(reused)
}

#[cfg(test)]
#[path = "plan/tests.rs"]
mod tests;
