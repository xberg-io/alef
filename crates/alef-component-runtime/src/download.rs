//! Download tuning for [`ArtifactCache`](crate::ArtifactCache): timeouts, retry policy,
//! progress reporting, cancellation, and transient-error classification.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Default time allowed to establish a connection to a component's download URL.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default time allowed for an entire component download, connection included.
pub const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);
/// Default number of attempts (the first try included) for transient download failures.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;
/// Default delay before the first retry; doubled for every further retry.
pub const DEFAULT_RETRY_BACKOFF: Duration = Duration::from_millis(500);

/// Callback invoked as archive bytes stream in: `(downloaded, total)`, where `total` is the
/// archive size pinned by the lock entry.
pub type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// How a component archive is downloaded.
///
/// `Default` reproduces the historical behaviour: 10 s connect timeout, 300 s total timeout,
/// no progress callback, no cancellation, and up to 3 attempts on transient failures.
/// Build one with the `with_*` methods:
///
/// ```
/// use alef_component_runtime::DownloadOptions;
/// use std::time::Duration;
///
/// let options = DownloadOptions::default()
///     .with_total_timeout(Duration::from_secs(900))
///     .with_progress(|downloaded, total| eprintln!("{downloaded}/{total}"));
/// ```
///
/// Retries apply to connect errors, timeouts, interrupted body reads and HTTP 408/429/5xx.
/// HTTP 4xx responses and integrity failures (size, digest, signature) are never retried.
/// `progress` and `cancel` also apply to `file://` sources and
/// [`ArtifactCache::install_from_reader`](crate::ArtifactCache::install_from_reader).
#[derive(Clone)]
#[non_exhaustive]
pub struct DownloadOptions {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub progress: Option<ProgressCallback>,
    /// Set to `true` from any thread to abort; checked between chunks and between retries.
    /// An aborted install returns [`ComponentError::Cancelled`](crate::ComponentError::Cancelled).
    pub cancel: Option<Arc<AtomicBool>>,
    /// Total attempts including the first; values below 1 are treated as 1.
    pub max_attempts: u32,
    /// Delay before the first retry, doubled each time.
    pub retry_backoff: Duration,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            total_timeout: DEFAULT_TOTAL_TIMEOUT,
            progress: None,
            cancel: None,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            retry_backoff: DEFAULT_RETRY_BACKOFF,
        }
    }
}

impl fmt::Debug for DownloadOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloadOptions")
            .field("connect_timeout", &self.connect_timeout)
            .field("total_timeout", &self.total_timeout)
            .field("progress", &self.progress.as_ref().map(|_| "<callback>"))
            .field("cancel", &self.cancel)
            .field("max_attempts", &self.max_attempts)
            .field("retry_backoff", &self.retry_backoff)
            .finish()
    }
}

impl DownloadOptions {
    #[must_use]
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_total_timeout(mut self, timeout: Duration) -> Self {
        self.total_timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_progress(mut self, progress: impl Fn(u64, u64) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(progress));
        self
    }

    #[must_use]
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    #[must_use]
    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts;
        self
    }

    #[must_use]
    pub fn with_retry_backoff(mut self, backoff: Duration) -> Self {
        self.retry_backoff = backoff;
        self
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    pub(crate) fn report(&self, downloaded: u64, total: u64) {
        if let Some(progress) = &self.progress {
            progress(downloaded, total);
        }
    }

    /// Delay to wait after failed attempt number `attempt` (1-based).
    pub(crate) fn backoff_delay(&self, attempt: u32) -> Duration {
        self.retry_backoff
            .saturating_mul(1_u32 << attempt.saturating_sub(1).min(16))
    }
}

/// Whether an HTTP status is worth retrying.
pub(crate) fn is_transient_status(status: u16) -> bool {
    status >= 500 || status == 408 || status == 429
}

/// Whether a request-phase failure is worth retrying.
pub(crate) fn is_transient_request_error(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(status) => is_transient_status(*status),
        ureq::Error::Io(_)
        | ureq::Error::Timeout(_)
        | ureq::Error::ConnectionFailed
        | ureq::Error::HostNotFound
        | ureq::Error::BodyStalled => true,
        _ => false,
    }
}

/// Wraps a body reader and remembers whether the underlying read failed, so a broken
/// connection (retryable) can be told apart from a local write or integrity failure.
pub(crate) struct TrackedReader<R> {
    inner: R,
    pub(crate) read_failed: bool,
}

impl<R> TrackedReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            read_failed: false,
        }
    }
}

impl<R: std::io::Read> std::io::Read for TrackedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf).inspect_err(|_| self.read_failed = true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_historical_timeouts() {
        let options = DownloadOptions::default();
        assert_eq!(options.connect_timeout, Duration::from_secs(10));
        assert_eq!(options.total_timeout, Duration::from_secs(300));
        assert_eq!(options.max_attempts, 3);
    }

    #[test]
    fn only_server_and_throttle_statuses_are_transient() {
        for status in [500, 502, 503, 504, 408, 429] {
            assert!(is_transient_status(status), "{status}");
        }
        for status in [400, 401, 403, 404, 410] {
            assert!(!is_transient_status(status), "{status}");
        }
    }

    #[test]
    fn request_errors_are_classified() {
        assert!(is_transient_request_error(&ureq::Error::StatusCode(503)));
        assert!(!is_transient_request_error(&ureq::Error::StatusCode(404)));
        assert!(is_transient_request_error(&ureq::Error::ConnectionFailed));
        assert!(is_transient_request_error(&ureq::Error::Io(std::io::Error::other(
            "reset"
        ))));
        assert!(!is_transient_request_error(&ureq::Error::TooManyRedirects));
    }

    #[test]
    fn backoff_doubles_per_attempt() {
        let options = DownloadOptions::default().with_retry_backoff(Duration::from_millis(100));
        assert_eq!(options.backoff_delay(1), Duration::from_millis(100));
        assert_eq!(options.backoff_delay(2), Duration::from_millis(200));
        assert_eq!(options.backoff_delay(3), Duration::from_millis(400));
    }

    #[test]
    fn tracked_reader_records_read_failures() {
        struct Failing;
        impl std::io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let mut reader = TrackedReader::new(Failing);
        assert!(std::io::Read::read(&mut reader, &mut [0; 4]).is_err());
        assert!(reader.read_failed);
    }
}
