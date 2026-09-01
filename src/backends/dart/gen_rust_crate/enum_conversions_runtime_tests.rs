use super::{enum_conversions::emit_from_impl_for_enum, opaque::emit_enum_from_json_fn};
use crate::core::ir::{EnumDef, EnumVariant};

fn unit_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        ..Default::default()
    }
}

#[test]
fn generated_json_decoder_returns_error_for_excluded_variant_at_runtime() {
    let en = EnumDef {
        name: "WorkflowStep".to_string(),
        rust_path: "mylib::WorkflowStep".to_string(),
        has_serde: true,
        variants: vec![unit_variant("Ready")],
        excluded_variants: vec![unit_variant("Internal")],
        ..Default::default()
    };
    let mut generated = String::new();
    emit_from_impl_for_enum(&mut generated, &en, "mylib", None);
    emit_enum_from_json_fn(&mut generated, &en, "mylib");
    let generated = generated.replace("#[frb]\n", "");
    let source = format!(
        "mod mylib {{\n    #[derive(serde::Deserialize)]\n    pub enum WorkflowStep {{ Ready, Internal }}\n}}\n\n#[derive(Debug, PartialEq)]\nenum WorkflowStep {{ Ready }}\n\n{generated}\nfn main() {{\n    let result = create_workflow_step_from_json(\"\\\"Internal\\\"\".to_string());\n    assert_eq!(result, Err(\"WorkflowStep contains a variant unavailable in the Dart binding\".to_string()));\n}}\n"
    );
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(temp.path().join("src")).expect("create src");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"dart-decoder-runtime\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .expect("write manifest");
    std::fs::write(temp.path().join("src/main.rs"), source).expect("write generated bridge source");
    let output = std::process::Command::new("cargo")
        .args(["run", "--quiet"])
        .env("CARGO_TARGET_DIR", temp.path().join("target"))
        // ~keep `RUSTFLAGS` is REMOVED, not passed through, and that is the whole point of this
        // line. This test spawns a child cargo, and a child cargo inherits the parent's
        // environment -- so on CI, where `xberg-io/actions/setup-rust` writes
        // `RUSTFLAGS=-D warnings` into `$GITHUB_ENV` (`scripts/configure-flags.sh:42,85`), two
        // warnings in the synthetic fixture below became hard errors and this test failed on all
        // three runners while passing on every developer machine. The two are fixture artifacts,
        // not defects in what alef emits: the hand-written `enum WorkflowStep { Ready }` is
        // private while the generated fn is `pub` (`private_interfaces`), and the generated
        // `_ => unreachable!(..)` catch-all is genuinely unreachable when the fixture declares no
        // cfg-gated variant -- that arm exists precisely because a wrapper crate cannot forward a
        // foreign cfg, per `emit_cfg_gated_arm`'s rule, so it is correct in the real case this
        // fixture does not model. What this test asserts is that the generated bridge COMPILES
        // AND RETURNS `Err` WITHOUT PANICKING. Inheriting the flag silently widened that to "and
        // is warning-clean under whatever the host happens to set", making the outcome a property
        // of the environment rather than of the code under test.
        .env_remove("RUSTFLAGS")
        .current_dir(temp.path())
        .output()
        .expect("run generated bridge crate");

    assert!(
        output.status.success(),
        "generated bridge crate must compile and return Err without panicking:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
