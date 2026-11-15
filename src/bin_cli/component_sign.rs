//! `alef component sign`: runs before configuration loads, so the signing host needs no checkout.

use anyhow::Result;

use crate::component::plan::ComponentPlan;
use crate::component::sign::{SignOptions, sign_directory};

use super::super::args::{ComponentAction, SignArgs};

/// Run `component sign` and return `None`; hand any other action back unchanged.
pub(super) fn intercept(action: ComponentAction) -> Result<Option<ComponentAction>> {
    match action {
        ComponentAction::Sign(args) => run(args).map(|()| None),
        other => Ok(Some(other)),
    }
}

fn run(args: SignArgs) -> Result<()> {
    let source = super::signing_key_source(args.key_file, args.key_env)
        .ok_or_else(|| anyhow::anyhow!("pass --key-file or --key-env"))?;
    // Read once, before any archive is touched, so a bad key fails first; the loaded
    // `SigningKey` zeroizes itself on drop.
    let signing_key = crate::component::artifact::load_signing_key_from_source(&source)?;
    let plan = args.plan.as_deref().map(ComponentPlan::read).transpose()?;
    let records = sign_directory(
        &args.input,
        &args.output,
        &SignOptions {
            signing_key: &signing_key,
            key_id: &args.key_id,
            expected_public_key: args.public_key.as_deref(),
            plan: plan.as_ref(),
            resign: args.resign,
        },
    )?;
    for record in &records {
        tracing::info!("Signed component {}", args.output.join(&record.archive).display());
    }
    tracing::info!("Component sign complete: {} artifact(s)", records.len());
    Ok(())
}
