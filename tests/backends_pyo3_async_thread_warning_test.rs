use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn generate(sendable: bool, asynchronous: bool, parameter: Option<bool>) -> (String, String) {
    generate_kind(sendable, asynchronous, parameter, false)
}

fn generate_kind(sendable: bool, asynchronous: bool, parameter: Option<bool>, trait_type: bool) -> (String, String) {
    let option = if sendable {
        "send_sync_types=['LocalHandle']\n"
    } else {
        ""
    };
    let config: NewAlefConfig = toml::from_str(&format!(
        "[workspace]\nlanguages=['python']\n[[crates]]\nname='sample'\nsources=[]\n[crates.python]\n{option}"
    ))
    .unwrap();
    let mut api = ApiSurface {
        types: vec![TypeDef {
            name: "LocalHandle".into(),
            rust_path: "sample::LocalHandle".into(),
            is_opaque: true,
            is_trait: trait_type,
            methods: vec![MethodDef {
                name: "fetch".into(),
                receiver: Some(ReceiverKind::Ref),
                is_async: asynchronous,
                return_type: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    if let Some(free_function) = parameter {
        let params = vec![ParamDef {
            name: "handle".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("LocalHandle".into()))),
            ..Default::default()
        }];
        if free_function {
            api.types[0].methods.clear();
            api.functions.push(FunctionDef {
                name: "fetch".into(),
                rust_path: "sample::fetch".into(),
                params,
                is_async: asynchronous,
                return_type: TypeRef::String,
                ..Default::default()
            });
        } else {
            api.types[0].methods[0].is_static = true;
            api.types[0].methods[0].receiver = None;
            api.types[0].methods[0].params = params;
        }
    }
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let files = tracing::subscriber::with_default(subscriber, || {
        Pyo3Backend
            .generate_bindings(&api, &config.resolve().unwrap().remove(0))
            .unwrap()
    });
    let warning = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    (
        warning,
        files.into_iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content,
    )
}

#[test]
fn async_unsendable_receiver_warns_without_changing_sendability() {
    let (warning, native) = generate(false, true, None);
    assert!(
        warning.contains("LocalHandle") && warning.contains("send_sync_types"),
        "{warning}"
    );
    assert!(native.contains("unsendable"));
}

#[test]
fn sendable_and_synchronous_handles_do_not_warn() {
    assert_eq!(generate(true, true, None).0, "");
    assert_eq!(generate(false, false, None).0, "");
}

#[test]
fn nested_opaque_async_parameters_warn_for_free_and_static_functions() {
    for free_function in [false, true] {
        let warning = generate(false, true, Some(free_function)).0;
        assert!(
            warning.contains("LocalHandle") && warning.contains("send_sync_types"),
            "{warning}"
        );
        assert_eq!(generate(true, true, Some(free_function)).0, "");
    }
}

#[test]
fn trait_bridge_markers_do_not_report_unsendable_handle_advice() {
    assert_eq!(generate_kind(false, true, None, true).0, "");
}
