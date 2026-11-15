use super::*;
use crate::stdio::MAX_FRAME_SIZE;
use crate::stdio::frame::{decode_call_header, encode_response_header, write_binary_frame};
use std::io::PipeWriter;
use std::thread::JoinHandle;
use std::time::Instant;

/// The sidecar end of an in-memory connection: requests the host wrote, and the pipe the test
/// answers on (taken to simulate the sidecar closing its stdout).
struct FakeSidecar {
    requests: mpsc::Receiver<BinaryFrame>,
    responses: Mutex<Option<PipeWriter>>,
}

impl FakeSidecar {
    fn next(&self) -> BinaryFrame {
        self.requests.recv_timeout(Duration::from_secs(5)).expect("a request")
    }

    fn respond(&self, id: u64, payload: &[u8]) {
        let mut out = self.responses.lock().unwrap();
        let out = out.as_mut().expect("responses still open");
        write_binary_frame(out, BinaryFrameKind::Response, id, &encode_response_header(0), payload).unwrap();
    }

    fn echo(&self, request: &BinaryFrame) {
        self.respond(request.id, &request.payload);
    }

    fn close(&self) {
        self.responses.lock().unwrap().take();
    }
}

fn connect(max_in_flight: u32) -> (Arc<BinaryConnection>, FakeSidecar) {
    let (request_reader, request_writer) = std::io::pipe().unwrap();
    let (response_reader, response_writer) = std::io::pipe().unwrap();
    let connection = BinaryConnection::start(request_writer, response_reader, max_in_flight).unwrap();
    let (requests, received) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = request_reader;
        while let Ok(frame) = read_binary_frame(&mut reader) {
            if requests.send(frame).is_err() {
                break;
            }
        }
    });
    let sidecar = FakeSidecar {
        requests: received,
        responses: Mutex::new(Some(response_writer)),
    };
    (Arc::new(connection), sidecar)
}

type CallResult = Result<(i32, Vec<u8>), CallError>;

fn spawn_call(connection: &Arc<BinaryConnection>, payload: &[u8], timeout: Duration) -> JoinHandle<CallResult> {
    let connection = Arc::clone(connection);
    let payload = payload.to_vec();
    std::thread::spawn(move || connection.call("engine", "echo", &payload, timeout))
}

fn ok_payload(result: CallResult) -> Vec<u8> {
    match result {
        Ok((0, payload)) => payload,
        other => panic!("expected a successful response, got {other:?}"),
    }
}

fn fatal_code(result: CallResult) -> &'static str {
    match result {
        Err(CallError::Fatal(error)) => error.code(),
        other => panic!("expected a fatal error, got {other:?}"),
    }
}

#[test]
fn should_send_the_contract_and_method_in_the_call_header() {
    let (connection, sidecar) = connect(1);
    let call = spawn_call(&connection, b"\0raw\xff", Duration::from_secs(5));
    let request = sidecar.next();
    assert_eq!(request.kind, BinaryFrameKind::Call);
    assert_eq!(request.id, 1);
    assert_eq!(decode_call_header(&request.header).unwrap(), ("engine", "echo"));
    assert_eq!(request.payload, b"\0raw\xff");
    sidecar.echo(&request);
    assert_eq!(ok_payload(call.join().unwrap()), b"\0raw\xff");
}

#[test]
fn should_dispatch_out_of_order_responses_to_their_callers() {
    let (connection, sidecar) = connect(4);
    let first = spawn_call(&connection, b"first", Duration::from_secs(5));
    let first_request = sidecar.next();
    let second = spawn_call(&connection, b"second", Duration::from_secs(5));
    let second_request = sidecar.next();
    assert_ne!(first_request.id, second_request.id);
    sidecar.echo(&second_request);
    assert_eq!(ok_payload(second.join().unwrap()), b"second");
    sidecar.echo(&first_request);
    assert_eq!(ok_payload(first.join().unwrap()), b"first");
}

#[test]
fn should_never_exceed_the_declared_in_flight_limit() {
    let (connection, sidecar) = connect(2);
    let calls: Vec<_> = (0..5_u8)
        .map(|value| spawn_call(&connection, &[value], Duration::from_secs(10)))
        .collect();
    let mut answered = 0;
    let mut batches = Vec::new();
    while answered < 5 {
        std::thread::sleep(Duration::from_millis(150));
        let batch: Vec<_> = sidecar.requests.try_iter().collect();
        assert!(batch.len() <= 2, "{} requests were unanswered at once", batch.len());
        batches.push(batch.len());
        for request in &batch {
            sidecar.echo(request);
        }
        answered += batch.len();
    }
    assert_eq!(batches[0], 2, "both slots are used: {batches:?}");
    let mut payloads: Vec<_> = calls.into_iter().map(|call| ok_payload(call.join().unwrap())).collect();
    payloads.sort();
    assert_eq!(payloads, [[0], [1], [2], [3], [4]]);
}

#[test]
fn should_drop_a_late_response_after_a_recoverable_timeout() {
    let (connection, sidecar) = connect(4);
    let slow = spawn_call(&connection, b"slow", Duration::from_secs(1));
    let slow_request = sidecar.next();
    let fast = spawn_call(&connection, b"fast", Duration::from_secs(5));
    sidecar.echo(&sidecar.next());
    assert_eq!(ok_payload(fast.join().unwrap()), b"fast");

    match slow.join().unwrap() {
        Err(CallError::Recoverable(error)) => assert_eq!(error.code(), "sidecar_timeout"),
        other => panic!("a sidecar that kept answering must not be stopped: {other:?}"),
    }
    sidecar.echo(&slow_request);
    let next = spawn_call(&connection, b"next", Duration::from_secs(5));
    sidecar.echo(&sidecar.next());
    assert_eq!(
        ok_payload(next.join().unwrap()),
        b"next",
        "the late response is not delivered"
    );
    assert!(!connection.is_dead());
}

#[test]
fn should_treat_a_timeout_without_any_response_as_fatal() {
    let (connection, sidecar) = connect(4);
    let start = Instant::now();
    let call = spawn_call(&connection, b"stuck", Duration::from_millis(200));
    let _request = sidecar.next();
    assert_eq!(fatal_code(call.join().unwrap()), "sidecar_timeout");
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn should_fail_every_waiter_when_the_sidecar_closes_its_stdout() {
    let (connection, sidecar) = connect(4);
    let calls: Vec<_> = (0..3_u8)
        .map(|value| spawn_call(&connection, &[value], Duration::from_secs(10)))
        .collect();
    for _ in 0..3 {
        sidecar.next();
    }
    let start = Instant::now();
    sidecar.close();
    for call in calls {
        assert_eq!(fatal_code(call.join().unwrap()), "sidecar_protocol");
    }
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "waiters fail at once, not at their deadline"
    );
    assert!(connection.is_dead());
    assert_eq!(
        fatal_code(connection.call("engine", "echo", b"", Duration::from_secs(5))),
        "sidecar_protocol"
    );
}

#[test]
fn should_kill_the_connection_on_a_response_for_an_unknown_id() {
    let (connection, sidecar) = connect(4);
    let call = spawn_call(&connection, b"x", Duration::from_secs(10));
    let request = sidecar.next();
    sidecar.respond(request.id + 100, b"stray");
    match call.join().unwrap() {
        Err(CallError::Fatal(error)) => assert!(error.to_string().contains("not outstanding"), "{error}"),
        other => panic!("expected a fatal error, got {other:?}"),
    }
    assert!(connection.is_dead());
}

#[test]
fn should_reject_an_oversized_request_without_taking_a_slot() {
    let (connection, sidecar) = connect(1);
    let oversized = vec![0; MAX_FRAME_SIZE + 1];
    match connection.call("engine", "echo", &oversized, Duration::from_secs(5)) {
        Err(CallError::Recoverable(error)) => assert_eq!(error.code(), "sidecar_protocol"),
        other => panic!("expected a recoverable error, got {other:?}"),
    }
    let call = spawn_call(&connection, b"after", Duration::from_secs(5));
    sidecar.echo(&sidecar.next());
    assert_eq!(ok_payload(call.join().unwrap()), b"after");
}
