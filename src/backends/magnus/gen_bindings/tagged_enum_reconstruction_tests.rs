use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::test_support::ruby_native::{artifact_directory, configure_build_target, ruby_native_target};
use crate::test_support::toolchain;
use quote::ToTokens;
use std::path::Path;
use std::process::Command;

fn fixture() -> (ApiSurface, ResolvedCrateConfig) {
    let config: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=['ruby']\n[[crates]]\nname='fixture'\nsources=['src/lib.rs']\n")
            .unwrap();
    let mut enums: Vec<EnumDef> = [
        ("PlainInternal", false, false),
        ("BoxedInternal", true, false),
        ("PlainAdjacent", false, true),
        ("BoxedAdjacent", true, true),
    ]
    .into_iter()
    .map(|(name, boxed, adjacent)| EnumDef {
        name: name.into(),
        rust_path: format!("fixture::{name}"),
        serde_tag: Some("kind".into()),
        serde_content: adjacent.then(|| "data".into()),
        variants: vec![
            EnumVariant {
                name: "Record".into(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("Payload".into()),
                    is_boxed: boxed,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Empty".into(),
                ..Default::default()
            },
            EnumVariant {
                name: "Named".into(),
                fields: vec![FieldDef {
                    name: "label".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    })
    .collect();
    enums.push(EnumDef {
        name: "RecordOnly".into(),
        rust_path: "fixture::RecordOnly".into(),
        serde_tag: Some("kind".into()),
        variants: vec![
            EnumVariant {
                name: "Empty".into(),
                ..Default::default()
            },
            EnumVariant {
                name: "Named".into(),
                fields: vec![FieldDef {
                    name: "label".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    let mut all_typed = enums[0].clone();
    all_typed.name = "AllTyped".into();
    all_typed.rust_path = "fixture::AllTyped".into();
    all_typed.variants.retain(|variant| variant.name == "Record");
    let mut other = all_typed.variants[0].clone();
    other.name = "Other".into();
    other.fields[0].is_boxed = true;
    all_typed.variants.push(other);
    enums.push(all_typed);
    let payload = TypeDef {
        name: "Payload".into(),
        rust_path: "fixture::Payload".into(),
        has_serde: true,
        is_clone: true,
        fields: vec![FieldDef {
            name: "label".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    (
        ApiSurface {
            crate_name: "fixture".into(),
            types: vec![payload],
            enums,
            ..Default::default()
        },
        config.resolve().unwrap().remove(0),
    )
}

// Compile the emitted boundary types and conversions without unrelated core-crate adapters. ~keep
fn runtime_source(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    let files = MagnusBackend.generate_bindings(api, config).unwrap();
    let lib = files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated Rust library");
    let parsed = syn::parse_file(&lib.content).expect("parse generated Rust");
    let attributes = parsed
        .attrs
        .iter()
        .map(|item| item.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut declarations = Vec::new();
    for item in parsed.items {
        let keep = match &item {
            syn::Item::Struct(item) => item.ident == "Payload",
            syn::Item::Enum(_) => true,
            syn::Item::Fn(item) => item.sig.ident == "json_to_ruby",
            syn::Item::Impl(item) => match &item.trait_ {
                Some((path, _)) => {
                    let name = &path.segments.last().unwrap().ident;
                    ["IntoValue", "TryConvert", "IntoValueFromNative", "TryConvertOwned"]
                        .iter()
                        .any(|s| name == s)
                }
                None => !item.self_ty.to_token_stream().to_string().contains("Payload"),
            },
            _ => false,
        };
        if keep {
            declarations.push(item.into_token_stream().to_string());
        }
    }
    assert_eq!(declarations.iter().filter(|s| s.contains("pub enum")).count(), 6);
    format!(
        "{attributes}\nuse magnus::{{prelude::*, Ruby, IntoValueFromNative, try_convert::TryConvertOwned}};\n{}\n{}",
        declarations.join("\n"),
        include_str!("tagged_enum_reconstruction_runtime.rs"),
    )
}

fn write_fixture(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) {
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), runtime_source(api, config)).unwrap();
    let manifest = format!(
        "[package]\nname='ruby-enum-reconstruction'\nversion='0.0.0'\nedition='2024'\n\
         [dependencies]\nmagnus={{version='{}',features=['embed']}}\n\
         serde={{version='1',features=['derive']}}\nserde_json='1'\n[profile.dev]\ndebug=0\n[workspace]\n",
        crate::core::template_versions::cargo::MAGNUS,
    );
    std::fs::write(root.join("Cargo.toml"), manifest).unwrap();
    std::fs::write(root.join("mise.toml"), "[tasks.build]\nrun = \"cargo build --quiet\"\n").unwrap();
    let public = MagnusBackend.generate_public_api(api, config).unwrap();
    let native = public
        .iter()
        .find(|file| file.path.ends_with("native.rb"))
        .expect("generated Ruby classes");
    let start = native
        .content
        .find("\nmodule Fixture\n")
        .expect("generated marker modules");
    std::fs::write(root.join("variants.rb"), &native.content[start..]).unwrap();
}

fn ruby_library_dir(ruby: &Path) -> std::path::PathBuf {
    let output = Command::new(ruby)
        .args(["-rrbconfig", "-e", "print RbConfig::CONFIG.fetch('libdir')"])
        .output()
        .expect("read the selected Ruby's shared library directory");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let directory = String::from_utf8(output.stdout).expect("Ruby library directory must be UTF-8");
    assert!(!directory.is_empty(), "Ruby reported an empty shared library directory");
    directory.into()
}

fn set_ruby_library_path(command: &mut Command, directory: &Path, inherited: Option<&std::ffi::OsStr>) {
    let paths = std::iter::once(directory.to_path_buf()).chain(inherited.into_iter().flat_map(std::env::split_paths));
    command.env(
        "LD_LIBRARY_PATH",
        std::env::join_paths(paths).expect("Ruby shared library paths must form a valid loader path"),
    );
}

#[test]
fn generated_tagged_enums_reconstruct_typed_ruby_variants_and_keep_json_fallbacks() {
    let Some(ruby) = toolchain::RUBY.open() else { return };
    let (api, config) = fixture();
    assert_eq!(api.enums.len(), 6);
    let root = tempfile::tempdir().unwrap();
    write_fixture(root.path(), &api, &config);
    let target = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ruby-tagged-runtime-cache");
    let rust_target = ruby_native_target(&ruby);
    let mut build = crate::test_support::mise_task::mise_run(root.path(), "build");
    build
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_JOBS", "2")
        .env("RUSTC_WRAPPER", "")
        .env("RUBY", &ruby);
    configure_build_target(&mut build, rust_target);
    let output = build.output().expect("compile generated Magnus conversions");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    for (scenario, expected) in [
        ("missing-classes", 5),
        ("reconstruction", 5),
        ("missing-variants", 5),
        ("fallback", 10),
    ] {
        let mut command = Command::new(
            artifact_directory(&target, rust_target)
                .join("ruby-enum-reconstruction")
                .with_extension(std::env::consts::EXE_EXTENSION),
        );
        command.args([scenario]).arg(root.path().join("variants.rb"));
        if cfg!(target_os = "linux") {
            set_ruby_library_path(
                &mut command,
                &ruby_library_dir(&ruby),
                std::env::var_os("LD_LIBRARY_PATH").as_deref(),
            );
        }
        let output = command
            .output()
            .expect("run generated Magnus conversions in embedded Ruby");
        assert!(
            output.status.success(),
            "{scenario}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("{expected} runtime cases passed")
        );
    }
}

#[test]
fn should_prepend_the_selected_ruby_library_without_losing_inherited_paths() {
    let library = std::path::PathBuf::from("selected Ruby/lib");
    let inherited = std::env::join_paths(["previous/lib", "another/lib"]).unwrap();
    let mut command = Command::new("fixture");
    set_ruby_library_path(&mut command, &library, Some(&inherited));
    let value = command
        .get_envs()
        .find(|(key, _)| *key == "LD_LIBRARY_PATH")
        .and_then(|(_, value)| value)
        .expect("the child must receive its Ruby library path");
    assert_eq!(
        std::env::split_paths(value).collect::<Vec<_>>(),
        [library, "previous/lib".into(), "another/lib".into()]
    );
}

#[test]
fn should_set_the_ruby_library_when_no_loader_path_is_inherited() {
    let library = std::path::PathBuf::from("selected Ruby/lib");
    let mut command = Command::new("fixture");
    set_ruby_library_path(&mut command, &library, None);
    let value = command
        .get_envs()
        .find(|(key, _)| *key == "LD_LIBRARY_PATH")
        .and_then(|(_, value)| value)
        .expect("the child must receive its Ruby library path");
    assert_eq!(std::env::split_paths(value).collect::<Vec<_>>(), [library]);
}
