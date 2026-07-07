use std::path::PathBuf;

use anyhow::{Result, bail};

use super::args::{Cli, Commands};

pub(crate) struct DispatchContext {
    pub(crate) config_path: PathBuf,
    pub(crate) crate_filter: Vec<String>,
    /// Verbosity level propagated from the global `-v`/`--verbose` flag.
    /// `0` = default output; `>0` = verbose (e.g. per-file hash detail in verify).
    pub(crate) verbose: u8,
}

pub(crate) fn run(cli: Cli) -> Result<()> {
    let context = DispatchContext {
        config_path: cli.config,
        crate_filter: cli.crate_filter,
        verbose: cli.verbose,
    };

    let mut command = cli.command;
    for handler in [
        super::core_commands::handle,
        super::all_commands::handle,
        super::aux_commands::handle,
        super::publish_commands::handle,
        super::release_commands::handle,
    ] {
        match handler(command, &context)? {
            Some(next) => command = next,
            None => return Ok(()),
        }
    }

    bail!("unhandled command: {}", command_name(&command))
}

fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Extract { .. } => "extract",
        Commands::Generate { .. } => "generate",
        Commands::Stubs { .. } => "stubs",
        Commands::Scaffold { .. } => "scaffold",
        Commands::Readme { .. } => "readme",
        Commands::Docs { .. } => "docs",
        Commands::SyncVersions { .. } => "sync-versions",
        Commands::Fmt { .. } => "fmt",
        Commands::Lint { .. } => "lint",
        Commands::Test { .. } => "test",
        Commands::Setup { .. } => "setup",
        Commands::Clean { .. } => "clean",
        Commands::Update { .. } => "update",
        Commands::Verify { .. } => "verify",
        Commands::Diff { .. } => "diff",
        Commands::Build { .. } => "build",
        Commands::All { .. } => "all",
        Commands::Init { .. } => "init",
        Commands::Schema { .. } => "schema",
        Commands::Migrate { .. } => "migrate",
        Commands::E2e { .. } => "e2e",
        Commands::TestApps { .. } => "test-apps",
        Commands::Publish { .. } => "publish",
        Commands::Cache { .. } => "cache",
        Commands::Validate { .. } => "validate",
        Commands::ReleaseMetadata { .. } => "release-metadata",
        Commands::CheckRegistry { .. } => "check-registry",
        Commands::GoTag { .. } => "go-tag",
        Commands::Snippets { .. } => "snippets",
    }
}
