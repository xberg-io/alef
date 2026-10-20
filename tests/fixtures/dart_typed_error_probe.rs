mod sample {
    #[derive(Debug)]
    pub enum RequestError {
        Rejected { message: String },
    }
    impl std::fmt::Display for RequestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "request rejected")
        }
    }
    pub fn request(_: u16) -> Result<(), RequestError> {
        Err(RequestError::Rejected {
            message: "blocked".into(),
        })
    }
}

fn main() {
    match request(1) {
        Err(AlefBridgeError(AlefBridgeErrorWire::RequestError {
            error: RequestError::Rejected { message },
        })) => assert_eq!(message, "blocked"),
        _ => panic!("core error variant and payload must survive"),
    }
    match request(-1) {
        Err(AlefBridgeError(AlefBridgeErrorWire::InvalidInput { message })) => {
            assert!(message.contains("must be between 0 and u16::MAX"))
        }
        _ => panic!("invalid input must retain its own error"),
    }
}
