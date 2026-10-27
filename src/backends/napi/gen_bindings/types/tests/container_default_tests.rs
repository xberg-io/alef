use crate::backends::napi::type_map::NapiMapper;
use crate::codegen::conversions::ConversionConfig;
use crate::core::ir::{ApiSurface, FieldDef, PrimitiveType, TypeDef, TypeRef};
use std::path::Path;
use std::process::{Command, Output};

fn fixture_type(name: &str, fields: &[(&str, TypeRef)], container_default: bool) -> TypeDef {
    let mut typ = TypeDef {
        name: name.into(),
        rust_path: format!("sample_core::{name}"),
        has_default: true,
        serde_container_default: container_default,
        fields: fields
            .iter()
            .map(|(name, ty)| FieldDef {
                name: (*name).into(),
                ty: ty.clone(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    typ.version.legacy_constructor_fields = fields.iter().map(|(name, _)| (*name).into()).collect();
    typ
}

fn fixture() -> ApiSurface {
    ApiSurface {
        types: vec![
            fixture_type(
                "BrowserOptions",
                &[("mode", TypeRef::String), ("timeout", TypeRef::Duration)],
                true,
            ),
            fixture_type(
                "JobOptions",
                &[
                    ("max_depth", TypeRef::Primitive(PrimitiveType::U32)),
                    ("browser", TypeRef::Named("BrowserOptions".into())),
                ],
                true,
            ),
            fixture_type(
                "Record",
                &[
                    ("content", TypeRef::String),
                    ("row", TypeRef::Primitive(PrimitiveType::U32)),
                ],
                false,
            ),
        ],
        ..Default::default()
    }
}

fn declarations(api: &ApiSurface) -> String {
    super::super::super::errors::gen_dts(
        api,
        "Js",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "sample_core",
        None,
    )
}

#[test]
fn container_default_keeps_legacy_config_fields_optional() {
    let api = fixture();
    for typ in &api.types {
        for field in &typ.fields {
            assert_eq!(
                super::napi_field_is_optional(field, typ),
                typ.serde_container_default,
                "{}.{}",
                typ.name,
                field.name
            );
        }
    }
    let dts = declarations(&api);
    for expected in [
        "readonly mode?:",
        "readonly timeout?:",
        "readonly maxDepth?:",
        "readonly browser?:",
        "readonly content:",
        "readonly row:",
    ] {
        assert!(dts.contains(expected), "missing {expected}: {dts}");
    }
}

fn generated_source(api: &ApiSurface) -> String {
    let config = ConversionConfig {
        type_name_prefix: "Js",
        optionalize_defaults: true,
        optionalize_bare_field_defaults: true,
        option_duration_on_defaults: true,
        cast_large_ints_to_i64: true,
        cast_f32_to_f64: true,
        ..Default::default()
    };
    let mut source = "use napi_derive::napi;\n".to_string();
    for typ in &api.types {
        source.push_str(&super::super::gen_struct(
            typ,
            &NapiMapper::new("Js".into()),
            "Js",
            false,
            &Default::default(),
            &[],
            &[],
            "sample_core",
            &Default::default(),
            None,
            &[],
        ));
        source.push_str(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
            typ,
            "sample_core",
            &config,
        ));
        source.push_str(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
            typ,
            "sample_core",
            &Default::default(),
            &config,
        ));
    }
    source.push_str(include_str!("container_default_fixture.rs"));
    source
}

fn run(dir: &Path, program: &str, args: &[&str]) -> Output {
    Command::new(program)
        .current_dir(dir)
        .env_remove("CARGO_TARGET_DIR")
        .args(args)
        .output()
        .expect("run native fixture tool")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires Cargo NAPI dependencies, Node, TypeScript and Task"]
fn container_default_sparse_typescript_and_native_node() {
    let dir = tempfile::tempdir().expect("owned native fixture");
    let dir = dir.path();
    std::fs::create_dir(dir.join("src")).expect("fixture sources");
    std::fs::write(dir.join("src/lib.rs"), generated_source(&fixture())).expect("generated library");
    std::fs::write(dir.join("Cargo.toml"), include_str!("container_default_manifest.toml")).expect("native manifest");
    std::fs::write(dir.join("build.rs"), "fn main() { napi_build::setup(); }\n").expect("build script");
    std::fs::write(
        dir.join("Taskfile.yml"),
        "version: '3'\ntasks:\n  build:\n    cmds:\n      - cargo build --offline\n",
    )
    .expect("build task");
    let dts = declarations(&fixture())
        + "\nexport function roundTrip(value: JobOptions): JobOptions;\nexport function requiredRecord(value: Record): Record;\n";
    std::fs::write(dir.join("index.d.ts"), dts).expect("declarations");
    std::fs::write(dir.join("check.ts"), include_str!("container_default_probe.ts")).expect("TypeScript probe");
    let typescript = run(dir, "tsc", &["--strict", "--noEmit", "--skipLibCheck", "check.ts"]);
    let built = run(dir, "task", &["build"]);
    assert_success(&built);
    let library = format!(
        "target/debug/{}sparse_probe{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    std::fs::copy(dir.join(library), dir.join("index.node")).expect("native module");
    std::fs::write(dir.join("check.cjs"), include_str!("container_default_probe.cjs")).expect("Node probe");
    let node = run(dir, "node", &["check.cjs"]);
    assert_success(&node);
    assert_success(&typescript);
}
