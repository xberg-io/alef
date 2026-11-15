//! A pipelined protocol-2 connection: many callers share one sidecar without a lock held across
//! their waits.
//!
//! A writer thread owns the child's stdin and writes whole frames in the order they are queued; a
//! reader thread owns its stdout and hands each response to the caller waiting on that request id
//! through a one-shot channel. At most `max_in_flight` requests are unanswered at once, counting
//! abandoned ones. A caller whose deadline passes abandons its id: the late response, if it ever
//! comes, is dropped. Any transport failure (EOF, a bad frame, a response for an id that is not
//! outstanding, a failed write) marks the connection dead and fails every waiting caller.

use super::frame::{
    BinaryFrame, BinaryFrameKind, decode_response_header, encode_binary_frame, encode_call_header, read_binary_frame,
};
use crate::ComponentError;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

type Reply = Result<(i32, Vec<u8>), ComponentError>;

/// How a failed call leaves the connection.
#[derive(Debug)]
pub(crate) enum CallError {
    /// The connection is still usable: an invalid request, or a timeout while the sidecar kept
    /// answering other requests.
    Recoverable(ComponentError),
    /// The connection is unusable and must be stopped: the sidecar died, broke the protocol, or
    /// answered nothing for a whole deadline.
    Fatal(ComponentError),
}

#[derive(Default)]
struct State {
    /// Unanswered requests by id; `None` marks one whose caller gave up waiting.
    pending: HashMap<u64, Option<mpsc::SyncSender<Reply>>>,
    abandoned: usize,
    dead: Option<String>,
    next_id: u64,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    slot_freed: Condvar,
    /// Response frames read so far, abandoned ones included: the sidecar's progress counter.
    responses: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn deliver(&self, frame: BinaryFrame) -> Result<(), String> {
        if frame.kind != BinaryFrameKind::Response {
            return Err(format!(
                "sidecar sent a {:?} frame; only responses are valid",
                frame.kind
            ));
        }
        let status = decode_response_header(&frame.header).map_err(|error| error.to_string())?;
        self.responses.fetch_add(1, Ordering::AcqRel);
        let mut state = self.lock();
        match state.pending.remove(&frame.id) {
            Some(Some(waiter)) => {
                // The buffered one-shot channel never blocks; a send error only means the caller
                // is gone, which drops the reply as intended.
                let _ = waiter.send(Ok((status, frame.payload)));
            }
            Some(None) => state.abandoned -= 1,
            None => {
                return Err(format!(
                    "sidecar answered request id {} which is not outstanding",
                    frame.id
                ));
            }
        }
        drop(state);
        self.slot_freed.notify_one();
        Ok(())
    }

    fn fail_all(&self, reason: String) {
        let mut state = self.lock();
        let reason = state.dead.get_or_insert(reason).clone();
        for waiter in state.pending.drain().filter_map(|(_, waiter)| waiter) {
            let _ = waiter.send(Err(connection_failed(&reason)));
        }
        state.abandoned = 0;
        drop(state);
        self.slot_freed.notify_all();
    }
}

pub(crate) struct BinaryConnection {
    shared: Arc<Shared>,
    writer: mpsc::Sender<Vec<u8>>,
    max_in_flight: usize,
}

impl BinaryConnection {
    /// Start the writer and reader threads over a child's stdin and stdout. Both threads end on
    /// their own: the writer once this connection is dropped or a write fails, the reader at the
    /// child's EOF (the child is killed when the connection is stopped).
    pub(crate) fn start<W, R>(mut stdin: W, mut stdout: R, max_in_flight: u32) -> Result<Self, ComponentError>
    where
        W: Write + Send + 'static,
        R: Read + Send + 'static,
    {
        let shared = Arc::new(Shared::default());
        let (writer, frames) = mpsc::channel::<Vec<u8>>();
        let writer_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("alef-sidecar-writer".into())
            .spawn(move || {
                while let Ok(frame) = frames.recv() {
                    if let Err(error) = stdin.write_all(&frame).and_then(|()| stdin.flush()) {
                        writer_shared.fail_all(format!("writing a request failed: {error}"));
                        break;
                    }
                }
            })?;
        let reader_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("alef-sidecar-reader".into())
            .spawn(move || {
                loop {
                    let delivered = read_binary_frame(&mut stdout)
                        .map_err(|error| error.to_string())
                        .and_then(|frame| reader_shared.deliver(frame));
                    if let Err(reason) = delivered {
                        reader_shared.fail_all(reason);
                        break;
                    }
                }
            })?;
        Ok(Self {
            shared,
            writer,
            max_in_flight: max_in_flight.max(1) as usize,
        })
    }

    /// Whether a transport failure has already ended this connection.
    pub(crate) fn is_dead(&self) -> bool {
        self.shared.lock().dead.is_some()
    }

    /// Send one call and wait at most `timeout` for its `(status, payload)` response.
    pub(crate) fn call(
        &self,
        contract: &str,
        method: &str,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<(i32, Vec<u8>), CallError> {
        let header = encode_call_header(contract, method).map_err(CallError::Recoverable)?;
        // Encoded with a placeholder id before a slot is taken, so a request the frame limits reject
        // never occupies one; the real id is patched in below.
        let mut frame =
            encode_binary_frame(BinaryFrameKind::Call, 1, &header, payload).map_err(CallError::Recoverable)?;
        let (reply_sender, reply) = mpsc::sync_channel(1);
        let (id, progress) = self.reserve(reply_sender, timeout)?;
        frame[4..12].copy_from_slice(&id.to_le_bytes());
        if self.writer.send(frame).is_err() {
            self.shared.lock().pending.remove(&id);
            return Err(CallError::Fatal(self.dead_error()));
        }
        match reply.recv_timeout(timeout) {
            Ok(result) => result.map_err(CallError::Fatal),
            Err(mpsc::RecvTimeoutError::Timeout) => self.abandon(id, progress, &reply),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(CallError::Fatal(ComponentError::SidecarStopped)),
        }
    }

    /// Wait for a free in-flight slot and register the request under a fresh id. Returns the id and
    /// the progress counter at registration.
    fn reserve(&self, waiter: mpsc::SyncSender<Reply>, timeout: Duration) -> Result<(u64, u64), CallError> {
        let mut state = self.shared.lock();
        loop {
            if let Some(reason) = &state.dead {
                return Err(CallError::Fatal(connection_failed(reason)));
            }
            if state.pending.len() < self.max_in_flight {
                break;
            }
            if state.abandoned >= self.max_in_flight {
                // Every slot is held by a request nobody waits for any more and the sidecar has not
                // answered: nothing can ever free a slot.
                return Err(CallError::Fatal(ComponentError::SidecarTimeout));
            }
            state = self
                .shared
                .slot_freed
                .wait_timeout(state, timeout)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        state.next_id += 1;
        let id = state.next_id;
        state.pending.insert(id, Some(waiter));
        Ok((id, self.shared.responses.load(Ordering::Acquire)))
    }

    /// Give up on request `id` after its deadline. The late response, if any, is dropped. The
    /// timeout is fatal when the sidecar answered nothing at all since this request was registered,
    /// or when every slot is now held by an abandoned request.
    fn abandon(&self, id: u64, progress: u64, reply: &mpsc::Receiver<Reply>) -> Result<(i32, Vec<u8>), CallError> {
        let mut state = self.shared.lock();
        match state.pending.get_mut(&id) {
            Some(waiter) if waiter.is_some() => {
                *waiter = None;
                state.abandoned += 1;
            }
            _ => {
                // Answered, or failed, between the deadline and taking the lock.
                drop(state);
                return match reply.try_recv() {
                    Ok(result) => result.map_err(CallError::Fatal),
                    Err(_) => Err(CallError::Fatal(ComponentError::SidecarStopped)),
                };
            }
        }
        let stalled = self.shared.responses.load(Ordering::Acquire) == progress;
        if stalled || state.abandoned >= self.max_in_flight {
            Err(CallError::Fatal(ComponentError::SidecarTimeout))
        } else {
            Err(CallError::Recoverable(ComponentError::SidecarTimeout))
        }
    }

    fn dead_error(&self) -> ComponentError {
        match &self.shared.lock().dead {
            Some(reason) => connection_failed(reason),
            None => ComponentError::SidecarStopped,
        }
    }
}

fn connection_failed(reason: &str) -> ComponentError {
    ComponentError::SidecarProtocol(format!("sidecar connection failed: {reason}"))
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod tests;
