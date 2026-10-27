mod sample_core {
    pub struct BrowserOptions {
        pub mode: String,
        pub timeout: std::time::Duration,
    }

    impl Default for BrowserOptions {
        fn default() -> Self {
            Self {
                mode: "auto".into(),
                timeout: std::time::Duration::from_millis(1500),
            }
        }
    }

    pub struct JobOptions {
        pub max_depth: u32,
        pub browser: BrowserOptions,
    }

    impl Default for JobOptions {
        fn default() -> Self {
            Self {
                max_depth: 7,
                browser: BrowserOptions::default(),
            }
        }
    }

    #[derive(Default)]
    pub struct Record {
        pub content: String,
        pub row: u32,
    }
}

#[napi]
pub fn round_trip(value: JsJobOptions) -> JsJobOptions {
    let core: sample_core::JobOptions = value.into();
    core.into()
}

#[napi]
pub fn required_record(value: JsRecord) -> JsRecord {
    let core: sample_core::Record = value.into();
    core.into()
}
