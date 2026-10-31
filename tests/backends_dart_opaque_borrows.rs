use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use quote::ToTokens;

#[path = "support/mise_task.rs"]
mod mise_task;

const SOURCE: &str = r#"
pub struct Counter { value: u32 }
pub fn create_counter() -> Counter { Counter { value: 0 } }
pub fn shared(value: &Counter) -> u32 { value.value }
pub fn exclusive(value: &mut Counter) { value.value += 1; }
pub fn consume(value: Counter) -> u32 { value.value }
"#;

fn generated() -> Vec<syn::Item> {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(&source, SOURCE).unwrap();
    let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 4);
    let config: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=[\"dart\"]\n[[crates]]\nname=\"sample\"\nsources=[\"src/lib.rs\"]\n")
            .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let source = &files
        .iter()
        .find(|f| f.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    syn::parse_file(source).unwrap().items
}

#[test]
fn opaque_function_parameter_ownership_matches_core_source() {
    let items = generated();
    for (name, expected) in [
        ("shared", "& Counter"),
        ("exclusive", "& mut Counter"),
        ("consume", "Counter"),
    ] {
        let function = items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(f) if f.sig.ident == name => Some(f),
                _ => None,
            })
            .unwrap();
        let syn::FnArg::Typed(parameter) = function.sig.inputs.first().unwrap() else {
            panic!("typed parameter")
        };
        assert_eq!(parameter.ty.to_token_stream().to_string(), expected);
    }
}

#[test]
fn actual_opaque_function_borrows_reuse_one_nonclone_handle_and_moves_stay_owned() {
    use alef::snippets::validators::run_command_streams;
    let directory = tempfile::tempdir().unwrap();
    let items: Vec<_> = generated()
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Fn(f)
                if ["create_counter", "shared", "exclusive", "consume"]
                    .iter()
                    .any(|name| f.sig.ident == *name) =>
            {
                Some(syn::Item::Fn(f))
            }
            syn::Item::Struct(mut s) if s.ident == "Counter" => {
                s.attrs.retain(|a| !a.path().is_ident("frb"));
                Some(syn::Item::Struct(s))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        items.len(),
        5,
        "compile the actual four functions and emitted opaque carrier"
    );
    let declarations = quote::quote!(#(#items)*);
    let prefix = format!("mod sample {{ {SOURCE} }}\n{declarations}\n");
    let positive = format!(
        "{prefix}fn main() {{ let mut value = create_counter(); assert_eq!(shared(&value), 0); exclusive(&mut value); assert_eq!(shared(&value), 1); assert_eq!(shared(&value), 1); assert_eq!(consume(value), 1); }}"
    );
    std::fs::write(directory.path().join("probe.rs"), positive).unwrap();
    std::fs::write(
        directory.path().join("rust-toolchain.toml"),
        include_str!("../rust-toolchain.toml"),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("mise.toml"),
        format!(
            "[tasks.compile]\nrun='rustc --edition=2024 probe.rs -o probe{}'\n[tasks.execute]\nrun='./probe{}'\n",
            std::env::consts::EXE_SUFFIX,
            std::env::consts::EXE_SUFFIX
        ),
    )
    .unwrap();
    for task in ["compile", "execute"] {
        let mut command = mise_task::mise_run(directory.path(), task);
        let output = run_command_streams(&mut command, 60).unwrap();
        assert!(output.success, "{task}: {}\n{}", output.stdout, output.stderr);
    }
    std::fs::write(
        directory.path().join("probe.rs"),
        format!("{prefix}fn main() {{ let value = create_counter(); consume(value); shared(&value); }}"),
    )
    .unwrap();
    let mut command = mise_task::mise_run(directory.path(), "compile");
    let output = run_command_streams(&mut command, 60).unwrap();
    assert!(
        !output.success && output.stderr.contains("E0382"),
        "owned handle must still move: {}\n{}",
        output.stdout,
        output.stderr
    );
}

#[test]
fn container_opaque_borrows_fail_before_owned_carriers_are_emitted() {
    for ty in ["&Option<Counter>", "Option<&Counter>", "Vec<&Counter>"] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("lib.rs");
        std::fs::write(
            &source,
            format!("pub struct Counter {{ value: u32 }} pub fn borrow(value: {ty}) {{ let _ = value; }}"),
        )
        .unwrap();
        let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
        assert_eq!(api.functions.len(), 1);
        assert!(api.functions[0].params[0].original_type.is_some());
        let config: NewAlefConfig = toml::from_str(
            "[workspace]\nlanguages=[\"dart\"]\n[[crates]]\nname=\"sample\"\nsources=[\"src/lib.rs\"]\n",
        )
        .unwrap();
        let config = config.resolve().unwrap().remove(0);
        let error = DartBackend.generate_bindings(&api, &config).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("opaque borrow") || message.contains("interior reference"),
            "{ty}: {error:#}"
        );
    }
}
