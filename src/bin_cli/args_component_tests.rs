//! Parsing coverage for `alef component` subcommand arguments.

use super::*;

fn component_action(args: &[&str]) -> Result<ComponentAction, clap::Error> {
    let mut argv = vec!["alef", "component"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).map(|cli| match cli.command {
        Commands::Component { action } => action,
        _ => panic!("expected component command"),
    })
}

#[test]
fn component_package_accepts_a_signing_key_from_env_or_stdin() {
    let action = component_action(&["package", "--signing-key-env", "RELEASE_KEY", "--key-id", "k1"]).unwrap();
    let ComponentAction::Package {
        signing_key,
        signing_key_env,
        ..
    } = action
    else {
        panic!("expected package");
    };
    assert_eq!(signing_key, None);
    assert_eq!(signing_key_env.as_deref(), Some("RELEASE_KEY"));

    let action = component_action(&["package", "--signing-key", "-", "--key-id", "k1"]).unwrap();
    let ComponentAction::Package { signing_key, .. } = action else {
        panic!("expected package");
    };
    assert_eq!(signing_key, Some(PathBuf::from("-")));
}

#[test]
fn component_package_signing_key_sources_are_mutually_exclusive() {
    assert!(
        component_action(&[
            "package",
            "--signing-key",
            "key.pem",
            "--signing-key-env",
            "RELEASE_KEY",
            "--key-id",
            "k1",
        ])
        .is_err()
    );
    assert!(component_action(&["package", "--signing-key-env", "RELEASE_KEY", "--unsigned"]).is_err());
    assert!(component_action(&["package", "--signing-key-env", "RELEASE_KEY"]).is_err());
}

#[test]
fn component_lock_accepts_unsigned_with_a_url_template_override() {
    let action = component_action(&["lock", "--unsigned", "--url-template", "file:///tmp/dist/{artifact}"]).unwrap();
    let ComponentAction::Lock {
        unsigned, url_template, ..
    } = action
    else {
        panic!("expected lock");
    };
    assert!(unsigned);
    assert_eq!(url_template.as_deref(), Some("file:///tmp/dist/{artifact}"));
}

#[test]
fn component_build_and_package_accept_a_shared_target_dir() {
    let ComponentAction::Build { target_dir, .. } = component_action(&["build", "--target-dir", "target/c"]).unwrap()
    else {
        panic!("expected build");
    };
    assert_eq!(target_dir, Some(PathBuf::from("target/c")));
    let ComponentAction::Package { target_dir, .. } =
        component_action(&["package", "--unsigned", "--target-dir", "target/c"]).unwrap()
    else {
        panic!("expected package");
    };
    assert_eq!(target_dir, Some(PathBuf::from("target/c")));
}

#[test]
fn component_contract_hash_parses_with_a_crate_filter() {
    assert!(matches!(
        component_action(&["contract-hash"]).unwrap(),
        ComponentAction::ContractHash
    ));
    let cli = Cli::try_parse_from(["alef", "component", "contract-hash", "--crate", "demo"]).unwrap();
    assert_eq!(cli.crate_filter, vec!["demo".to_string()]);
}
