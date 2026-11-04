//! `alef component plan` and the fingerprint/plan plumbing shared by `build`, `package` and `lock`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::codegen::component::{ResolvedComponent, resolve_component_targets};
use crate::component::artifact::{
    ComponentArtifactRecord, ComponentLock, ComponentLockEntry, sha256_bytes, sha256_file,
};
use crate::component::fingerprint::{
    self, FingerprintInput, HostFamily, ProducerInputs, SystemProbe, ToolProbe, build_env, compute, native_toolchain,
    parse_extras, process_vars, rustc_version,
};
use crate::component::plan::{ComponentPlan, PlanAction, PlanCandidate, decide, reused_entries};
use crate::core::config::{ComponentConfig, ComponentProducerLanguage, ResolvedCrateConfig};

use super::{extract_contracts, feature_hash, profile_selected, select_profiles, select_targets};

/// Computes input fingerprints, running `rustc -vV` once, reading the environment once and
/// resolving each producer workspace once.
pub(super) struct Fingerprinter {
    extras: Vec<(String, String)>,
    env_names: Vec<String>,
    vars: Vec<(String, String)>,
    cwd: PathBuf,
    rustc: Option<String>,
    toolchains: HashMap<String, Vec<(String, String)>>,
    host: HostFamily,
    probe: Box<dyn ToolProbe>,
    producers: HashMap<PathBuf, ProducerInputs>,
}

impl Fingerprinter {
    /// # Errors
    ///
    /// Fails on a malformed or repeated `--extra`, an empty or `=`-containing `--env` name, or when
    /// the working directory is unreadable.
    pub(super) fn new(extra: &[String], env: &[String]) -> Result<Self> {
        for name in env {
            ensure!(
                !name.is_empty() && !name.contains('='),
                "--env `{name}` must be a variable name"
            );
        }
        Ok(Self {
            extras: parse_extras(extra)?,
            env_names: env.to_vec(),
            vars: process_vars(),
            cwd: std::env::current_dir().context("cannot determine the working directory")?,
            rustc: None,
            toolchains: HashMap::new(),
            host: HostFamily::current(),
            probe: Box::new(SystemProbe),
            producers: HashMap::new(),
        })
    }

    /// The native toolchain records for `target`, probed once per target.
    fn toolchain_for(&mut self, target: &str) -> Vec<(String, String)> {
        self.toolchains
            .entry(target.to_string())
            .or_insert_with(|| native_toolchain(target, self.host, &self.vars, self.probe.as_ref()))
            .clone()
    }

    /// The OS family this process runs on (injectable in tests).
    pub(super) fn host(&self) -> HostFamily {
        self.host
    }

    #[cfg(test)]
    fn with_probe(mut self, host: HostFamily, probe: Box<dyn ToolProbe>) -> Self {
        self.host = host;
        self.probe = probe;
        self
    }

    /// The fingerprint of `profile` built for `target`, or `None` for a Go producer, which is not
    /// fingerprinted and is therefore always rebuilt.
    ///
    /// # Errors
    ///
    /// Fails when `rustc` or `cargo metadata` fails, or when a file cannot be read.
    pub(super) fn input_hash(
        &mut self,
        config: &ResolvedCrateConfig,
        profile: &ComponentConfig,
        resolved: &ResolvedComponent,
        target: &str,
        debug: bool,
    ) -> Result<Option<String>> {
        if profile.producer_language != ComponentProducerLanguage::Rust {
            return Ok(None);
        }
        let rustc = match &self.rustc {
            Some(rustc) => rustc.clone(),
            None => {
                let rustc = rustc_version()?;
                self.rustc = Some(rustc.clone());
                rustc
            }
        };
        let toolchain = self.toolchain_for(target);
        let contracts = contract_hashes(resolved)?;
        let env = build_env(self.vars.iter().cloned(), target, &self.env_names);
        let extras = self.extras.clone();
        let producer = self.producer(config, profile)?;
        Ok(Some(compute(
            &FingerprintInput {
                crate_name: &config.name,
                component: &profile.name,
                target,
                release: !debug,
                features: &profile.features,
                default_features: profile.default_features,
                contracts: &contracts,
                distribution: config.component_distribution.as_ref(),
                alef_version: env!("CARGO_PKG_VERSION"),
                rustc: &rustc,
                env: &env,
                toolchain: &toolchain,
                extras: &extras,
            },
            producer,
        )))
    }

    fn producer(&mut self, config: &ResolvedCrateConfig, profile: &ComponentConfig) -> Result<&ProducerInputs> {
        let manifest = super::build::producer_manifest_path(config, profile);
        if !self.producers.contains_key(&manifest) {
            let producer = fingerprint::producer_inputs(&manifest, &self.cwd)?;
            self.producers.insert(manifest.clone(), producer);
        }
        Ok(&self.producers[&manifest])
    }
}

// The lock can be assembled on a different host from the build runner. Hash only
// the dependency closure and workspace manifest inputs common to both. ~keep
fn portable_producer_hash(producer: &ProducerInputs) -> String {
    let bytes = serde_json::to_vec(&(
        &producer.packages,
        &producer.manifest_tables,
        &producer.checkout_config_files,
    ))
    .expect("producer input strings are serializable");
    sha256_bytes(&bytes)
}

fn plan_source_hash(
    producer: &ProducerInputs,
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    resolved: &ResolvedComponent,
) -> Result<String> {
    let mut contracts = contract_hashes(resolved)?;
    contracts.sort();
    let bytes = serde_json::to_vec(&(
        portable_producer_hash(producer),
        feature_hash(&profile.features, profile.default_features),
        contracts,
        &profile.provides,
        &config.component_distribution,
    ))
    .context("failed to encode portable component inputs")?;
    Ok(sha256_bytes(&bytes))
}

/// `(contract name, hash)` for every contract the component provides.
fn contract_hashes(resolved: &ResolvedComponent) -> Result<Vec<(String, String)>> {
    resolved
        .provides
        .iter()
        .map(|provided| Ok((provided.contract.name.clone(), provided.contract.hash_hex()?)))
        .collect()
}

/// The identity hash a lock records for a component: the first provided contract by name.
fn primary_contract_hash(resolved: &ResolvedComponent) -> Result<String> {
    let mut contracts = contract_hashes(resolved)?;
    contracts.sort();
    contracts
        .into_iter()
        .next()
        .map(|(_, hash)| hash)
        .context("component provides no contract")
}

pub(super) struct PlanArgs {
    pub previous_lock: Option<PathBuf>,
    pub component: Vec<String>,
    pub target: Vec<String>,
    pub extra: Vec<String>,
    pub env: Vec<String>,
    pub debug: bool,
    pub output: Option<PathBuf>,
}

pub(super) fn read_lock(path: &Path) -> Result<ComponentLock> {
    let bytes = std::fs::read(path).with_context(|| format!("failed to read component lock {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse component lock {}", path.display()))
}

/// Run `alef component plan`.
pub(super) fn run(crates: &[&ResolvedCrateConfig], config_path: &Path, args: PlanArgs) -> Result<()> {
    let previous = args.previous_lock.as_deref().map(read_lock).transpose()?;
    let previous_entries = previous.as_ref().map_or(&[][..], |lock| lock.artifacts.as_slice());
    let mut fingerprinter = Fingerprinter::new(&args.extra, &args.env)?;
    let mut rows = Vec::new();
    for config in crates {
        let version = config
            .resolved_version()
            .context("cannot plan a component without a crate version")?;
        let contracts = extract_contracts(config, config_path)?;
        for profile in select_profiles(config, &args.component)? {
            let resolved = contracts
                .get(profile.name.as_str())
                .with_context(|| format!("component `{}` has no resolved contract", profile.name))?;
            let feature_hash = feature_hash(&profile.features, profile.default_features);
            let contract_hash = primary_contract_hash(resolved)?;
            for target in select_targets(config, profile, &args.target)? {
                let input_hash = fingerprinter.input_hash(config, profile, resolved, &target, args.debug)?;
                ensure!(
                    input_hash.is_some(),
                    "component planning requires an input fingerprint; Go producers are unsupported"
                );
                let mut row = decide(
                    &PlanCandidate {
                        crate_name: config.name.clone(),
                        component: profile.name.clone(),
                        target,
                        feature_hash: feature_hash.clone(),
                        contract_hash: contract_hash.clone(),
                        input_hash,
                    },
                    previous_entries,
                )?;
                row.host_family = Some(fingerprinter.host());
                row.version = Some(version.clone());
                let producer = fingerprinter.producer(config, profile)?;
                row.source_hash = Some(plan_source_hash(producer, config, profile, resolved)?);
                rows.push(row);
            }
        }
    }
    ensure!(!rows.is_empty(), "no component and target selected to plan");
    let plan = ComponentPlan::new(args.debug, rows);
    let mut json = serde_json::to_string_pretty(&plan).context("failed to serialize the component plan")?;
    json.push('\n');
    match &args.output {
        Some(path) => {
            if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::write(path, json).with_context(|| format!("failed to write component plan {}", path.display()))?;
            tracing::info!("Wrote component plan {}", path.display());
        }
        None => crate::bin_cli::output::payload(json.trim_end()),
    }
    Ok(())
}

/// Load the plan given to `build` or `package` and check it was made for the same profile.
pub(super) fn load_selection(path: Option<&Path>, debug: bool) -> Result<Option<ComponentPlan>> {
    let Some(path) = path else { return Ok(None) };
    let plan = ComponentPlan::read(path)?;
    if plan.debug != debug {
        bail!(
            "plan {} was made {} `--debug`; pass the same flag here",
            path.display(),
            if plan.debug { "with" } else { "without" }
        );
    }
    Ok(Some(plan))
}

/// Check current inputs for a selected plan row, then skip reuse or return the build fingerprint.
///
/// # Errors
///
/// Fails when the plan has no row for the triple, or the inputs changed since the plan was made.
pub(super) fn gate(
    plan: &ComponentPlan,
    fingerprinter: &mut Fingerprinter,
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    resolved: &ResolvedComponent,
    target: &str,
    debug: bool,
) -> Result<Option<Option<String>>> {
    let row = plan
        .row(&config.name, &profile.name, target)
        .with_context(|| format!("{}/{} for {target} is not in the plan", config.name, profile.name))?;
    ensure!(
        row.input_hash.is_some(),
        "component planning requires an input fingerprint"
    );
    check_plan_host(row.host_family, fingerprinter.host(), target)?;
    let current = fingerprinter.input_hash(config, profile, resolved, target, debug)?;
    ensure!(
        current == row.input_hash,
        "inputs of `{}` for `{target}` changed since the plan was made",
        profile.name
    );
    verify_source_row(row, config, profile, resolved, fingerprinter)?;
    if row.action == PlanAction::Reuse {
        tracing::info!("Skipping `{}` for `{target}`: the plan reuses it", profile.name);
        return Ok(None);
    }
    Ok(Some(row.input_hash.clone()))
}

/// Fail with a clear message when a plan for an Apple or MSVC `target` was made on a different OS
/// family than the one building it, instead of a misleading "inputs changed" from the gate.
pub(super) fn check_plan_host(planned: Option<HostFamily>, current: HostFamily, target: &str) -> Result<()> {
    if let Some(planned) = planned
        && planned != current
        && HostFamily::needs_matching_host(target)
    {
        bail!(
            "plan for {target} was made on a {} host but this build runs on {}; plan Apple and MSVC targets on a matching runner",
            planned.name(),
            current.name()
        );
    }
    Ok(())
}

/// The profiles `build`/`package` act on: the explicitly requested ones, or with a plan and no
/// `--component`, exactly the components the plan has rows for in this crate.
pub(super) fn plan_profiles<'a>(
    config: &'a ResolvedCrateConfig,
    requested: &[String],
    plan: Option<&ComponentPlan>,
) -> Result<Vec<&'a ComponentConfig>> {
    let Some(plan) = plan.filter(|_| requested.is_empty()) else {
        return select_profiles(config, requested);
    };
    let mut names = Vec::new();
    for row in plan.rows.iter().filter(|row| row.crate_name == config.name) {
        if !names.contains(&row.component) {
            names.push(row.component.clone());
        }
    }
    if names.is_empty() {
        return Ok(Vec::new());
    }
    select_profiles(config, &names)
}

/// The targets `build`/`package` act on for `profile`: the explicitly requested ones, or with a
/// plan and no `--target`, exactly the targets the plan has rows for.
pub(super) fn plan_targets(
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    requested: &[String],
    plan: Option<&ComponentPlan>,
) -> Result<Vec<String>> {
    let Some(plan) = plan.filter(|_| requested.is_empty()) else {
        return select_targets(config, profile, requested);
    };
    let targets = plan
        .rows
        .iter()
        .filter(|row| row.crate_name == config.name && row.component == profile.name)
        .map(|row| row.target.clone())
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Ok(targets);
    }
    select_targets(config, profile, &targets)
}

const SIDECAR_SUFFIX: &str = ".input-hash";

/// `<library path>.input-hash`, where `build --plan` records the fingerprint a library was built at.
pub(super) fn sidecar_path(library: &Path) -> PathBuf {
    let mut name = library.as_os_str().to_os_string();
    name.push(SIDECAR_SUFFIX);
    PathBuf::from(name)
}

/// Remove a sidecar left by an earlier build, so a failed or unplanned rebuild cannot leave a
/// fingerprint next to a library it does not describe.
pub(super) fn clear_sidecar(library: &Path) -> Result<()> {
    let path = sidecar_path(library);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

/// Record the fingerprint a library was built at.
pub(super) fn write_sidecar(library: &Path, input_hash: &str, version: &str) -> Result<()> {
    let path = sidecar_path(library);
    let library_hash = sha256_file(library)?;
    std::fs::write(&path, format!("{input_hash}\n{library_hash}\n{version}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

/// Require that `library` was built by `build --plan` at the planned fingerprint.
///
/// # Errors
///
/// Fails when the sidecar is missing, unreadable or holds a different fingerprint. Nothing is
/// required when the plan has no fingerprint (`expected` is `None`).
pub(super) fn check_sidecar(library: &Path, expected: Option<&str>, library_hash: &str, version: &str) -> Result<()> {
    let Some(expected) = expected else { return Ok(()) };
    let path = sidecar_path(library);
    let recorded = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "{} is missing; build the library with `alef component build --plan` before packaging it",
            path.display()
        )
    })?;
    let mut lines = recorded.lines();
    let recorded_input = lines.next().unwrap_or_default();
    ensure!(
        recorded_input == expected,
        "{} was built at a different fingerprint ({}) than the plan ({expected}); rebuild it with `alef component build --plan`",
        library.display(),
        recorded_input
    );
    ensure!(
        lines.next() == Some(library_hash),
        "{} library bytes differ from the planned build; rebuild it with `alef component build --plan`",
        library.display()
    );
    ensure!(
        lines.next() == Some(version) && lines.next().is_none(),
        "{} was not built for the planned release version {version}; rebuild it with `alef component build --plan`",
        library.display()
    );
    Ok(())
}

/// Check each fresh record against the plan: it must have a plan row, the row must be `build`, and
/// the record's fingerprint must be the planned one.
pub(super) fn check_records_against_plan(
    plan: &ComponentPlan,
    crate_name: &str,
    records: &[&(PathBuf, ComponentArtifactRecord)],
) -> Result<()> {
    for (path, record) in records.iter().map(|record| (&record.0, &record.1)) {
        let identity = &record.manifest.identity;
        let row = plan
            .row(crate_name, &identity.component, &identity.target)
            .with_context(|| {
                format!(
                    "{} packages {} for {}, which is not in the plan",
                    path.display(),
                    identity.component,
                    identity.target
                )
            })?;
        ensure!(
            row.action == PlanAction::Build,
            "{} packages {} for {}, which the plan reuses",
            path.display(),
            identity.component,
            identity.target
        );
        ensure!(
            row.input_hash.is_some(),
            "planned artifact {} requires an input fingerprint",
            path.display()
        );
        ensure!(
            row.version.as_deref() == Some(identity.version.as_str()),
            "{} version {} does not match the planned version {:?}",
            path.display(),
            identity.version,
            row.version
        );
        ensure!(
            record.input_hash == row.input_hash,
            "{} was built from different inputs than the plan (record {:?}, plan {:?})",
            path.display(),
            record.input_hash,
            row.input_hash
        );
    }
    Ok(())
}

/// The artifact records under `--input` for `component lock`. When reusing, an absent `input` is an
/// empty set: a release that reuses every component builds no archives, so nothing writes the
/// directory. Without reuse a missing `input` is an error.
pub(super) fn lock_input_records(input: &Path, reusing: bool) -> Result<Vec<(PathBuf, ComponentArtifactRecord)>> {
    if reusing
        && !input
            .try_exists()
            .with_context(|| format!("cannot access component record input {}", input.display()))?
    {
        return Ok(Vec::new());
    }
    super::records_in(input)
}

/// Every `build` row of the plan for `crate_name` that `--component` / `--target` select must have a
/// fresh record: a build row without one would otherwise silently drop out of the lock.
///
/// Rows the filters do not select are skipped, because a filtered lock deliberately covers only part
/// of the plan. A `reuse` row the filters do not select is an error instead (`check_reuse_rows`):
/// the plan says to carry it over, and dropping it silently would ship a lock missing an artifact
/// the operator asked to keep. The configured-matrix check requires a record or reuse entry for
/// each selected configured pair, so any selected build row passing here also satisfies it.
pub(super) fn check_build_rows_have_records(
    plan: &ComponentPlan,
    crate_name: &str,
    records: &[&(PathBuf, ComponentArtifactRecord)],
    components: &[String],
    targets: &[String],
) -> Result<()> {
    for row in plan.rows.iter().filter(|row| {
        row.action == PlanAction::Build
            && row.crate_name == crate_name
            && (components.is_empty() || components.contains(&row.component))
            && (targets.is_empty() || targets.contains(&row.target))
    }) {
        ensure!(
            records.iter().any(|(_, record)| {
                record.manifest.identity.component == row.component && record.manifest.identity.target == row.target
            }),
            "plan builds {}/{} for {}, but the lock input has no record for it",
            row.crate_name,
            row.component,
            row.target
        );
    }
    Ok(())
}

/// Read the previous lock and the plan named by `lock --reuse --plan`.
pub(super) fn load_reuse(reuse: Option<&Path>, plan: Option<&Path>) -> Result<Option<(ComponentLock, ComponentPlan)>> {
    match (reuse, plan) {
        (Some(previous), Some(plan)) => Ok(Some((read_lock(previous)?, ComponentPlan::read(plan)?))),
        _ => Ok(None),
    }
}

/// Reject a `--url-template` together with `--reuse`: reused entries keep the URL of the release
/// that built them, so a different template would leave one lock with two URL schemes.
pub(super) fn check_reuse_options(url_template: Option<&str>, reusing: bool) -> Result<()> {
    ensure!(
        !(reusing && url_template.is_some()),
        "--url-template cannot be combined with --reuse: reused entries keep their original URL"
    );
    Ok(())
}

/// Every `reuse` row of the plan for `config` must name a component and target the crate configures
/// and that `--component` / `--target` select.
pub(super) fn check_reuse_rows(
    plan: &ComponentPlan,
    config: &ResolvedCrateConfig,
    components: &[String],
    targets: &[String],
) -> Result<()> {
    for row in plan
        .rows
        .iter()
        .filter(|row| row.action == PlanAction::Reuse && row.crate_name == config.name)
    {
        let profile = config
            .components
            .iter()
            .find(|profile| profile.name == row.component)
            .with_context(|| {
                format!(
                    "plan reuses {}/{} for {}, but the crate has no such component",
                    row.crate_name, row.component, row.target
                )
            })?;
        if !resolve_component_targets(config, profile).contains(&row.target) {
            bail!(
                "plan reuses {}/{} for {}, but the component is not configured for that target",
                row.crate_name,
                row.component,
                row.target
            );
        }
        if !profile_selected(components, profile) || !(targets.is_empty() || targets.contains(&row.target)) {
            bail!(
                "plan reuses {}/{} for {}, which --component/--target do not select",
                row.crate_name,
                row.component,
                row.target
            );
        }
    }
    Ok(())
}

/// Revalidate selected rows against the current portable inputs on the lock assembly host.
pub(super) fn verify_lock_inputs(
    plan: &ComponentPlan,
    config: &ResolvedCrateConfig,
    config_path: &Path,
    components: &[String],
    targets: &[String],
) -> Result<()> {
    let rows = plan.rows.iter().filter(|row| {
        row.crate_name == config.name
            && (components.is_empty() || components.contains(&row.component))
            && (targets.is_empty() || targets.contains(&row.target))
    });
    let mut rows = rows.peekable();
    if rows.peek().is_none() {
        return Ok(());
    }
    let contracts = extract_contracts(config, config_path)?;
    let mut fingerprinter = Fingerprinter::new(&[], &[])?;
    for row in rows {
        let profile = config
            .components
            .iter()
            .find(|profile| profile.name == row.component)
            .context("planned component is not configured")?;
        let resolved = contracts
            .get(&row.component)
            .context("planned component has no contract")?;
        verify_source_row(row, config, profile, resolved, &mut fingerprinter)?;
    }
    Ok(())
}

fn verify_source_row(
    row: &crate::component::plan::PlanRow,
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    resolved: &ResolvedComponent,
    fingerprinter: &mut Fingerprinter,
) -> Result<()> {
    ensure!(
        row.input_hash.is_some() && profile.producer_language == ComponentProducerLanguage::Rust,
        "component planning requires an input fingerprint; Go producers are unsupported"
    );
    ensure!(
        row.version.is_some() && row.version == config.resolved_version(),
        "component `{}` does not match the planned version; create a new plan",
        row.component
    );
    let producer = fingerprinter.producer(config, profile)?;
    let source_hash = plan_source_hash(producer, config, profile, resolved)?;
    ensure!(
        row.source_hash.as_deref() == Some(source_hash.as_str()),
        "inputs of `{}` for `{}` changed since the plan was made; create a new plan",
        row.component,
        row.target
    );
    if let Some(previous) = &row.previous {
        ensure!(
            previous.identity.feature_hash == feature_hash(&profile.features, profile.default_features)
                && previous.identity.contract_hash == primary_contract_hash(resolved)?,
            "previous artifact identity does not match the current component `{}`",
            row.component
        );
    }
    Ok(())
}

/// The previous-lock entries to carry over for one crate, after validating the fresh records and
/// the reuse rows against the plan. Empty when the lock is not reusing.
pub(super) fn reuse_for_crate(
    reuse: Option<&(ComponentLock, ComponentPlan)>,
    config: &ResolvedCrateConfig,
    records: &[&(PathBuf, ComponentArtifactRecord)],
    trusted: Option<&std::collections::BTreeMap<String, String>>,
    components: &[String],
    targets: &[String],
) -> Result<Vec<ComponentLockEntry>> {
    let Some((previous, plan)) = reuse else {
        return Ok(Vec::new());
    };
    check_records_against_plan(plan, &config.name, records)?;
    check_build_rows_have_records(plan, &config.name, records, components, targets)?;
    check_reuse_rows(plan, config, components, targets)?;
    reused_entries(plan, previous, &config.name, trusted)
}

#[cfg(test)]
#[path = "component_plan_tests.rs"]
mod tests;
