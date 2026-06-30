//! Shared connection pool + tokio runtime used by both client adapters.
//!
//! Supports both plain TCP and TLS connections.  Use [`ClientRuntime::connect`]
//! for plain TCP (requires a TLS-terminating proxy / VPN at the infrastructure
//! layer) and [`ClientRuntime::connect_tls`] for native TLS with the system root
//! CA store.

use atlas_core::{Error, Result};
use atlas_proto::{read_frame, write_frame, Request, Response, SERVICE_VERSION};
use rustls::pki_types::ServerName;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio_rustls::client::TlsStream;
use tracing::{debug, warn};

/// How long to wait for the TCP handshake to complete.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for a response frame after the request is sent.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum connections in the pool.
const DEFAULT_POOL_SIZE: usize = 4;
/// Default number of consecutive failures before tripping the circuit.
const DEFAULT_FAILURE_THRESHOLD: usize = 5;
/// Default time the circuit stays Open before transitioning to HalfOpen.
const DEFAULT_RESET_TIMEOUT: Duration = Duration::from_secs(30);

// --- Circuit breaker state constants (stored as u8 atomics) ---
const CB_CLOSED: u8 = 0;
const CB_OPEN: u8 = 1;
const CB_HALF_OPEN: u8 = 2;

// ---------------------------------------------------------------------------
// ConnStream — unified plain/TLS stream abstraction
// ---------------------------------------------------------------------------

/// A connected stream: either a plain TCP socket or a TLS-encrypted one.
///
/// Both variants are `Unpin` (`TcpStream: Unpin`, `Box<T>: Unpin`), so
/// `ConnStream` is `Unpin` and can be used with `Pin::new()` in
/// `AsyncRead`/`AsyncWrite` implementations.
pub enum ConnStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for ConnStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ConnStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ConnStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ConnStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ConnStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ConnStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

// ---------------------------------------------------------------------------
// TLS configuration
// ---------------------------------------------------------------------------

/// TLS configuration for [`ClientRuntime::connect_tls`].
///
/// Uses the system's trusted CA certificates (via `webpki-roots`) to verify
/// the server certificate.  Certificate verification is always enabled; to
/// disable it for testing use a local CA and supply its cert instead.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// SNI hostname sent to the server (the hostname part of the address,
    /// e.g. `"atlas.example.com"`, not `"atlas.example.com:7645"`).
    pub server_name: String,
}

// ---------------------------------------------------------------------------
// ConnectionPool
// ---------------------------------------------------------------------------

/// A pool of idle connections (plain or TLS).
///
/// Uses a `VecDeque` plus a `Condvar` to park callers when the pool is at
/// capacity and no idle connections are available.
pub(crate) struct ConnectionPool {
    idle: Mutex<VecDeque<ConnStream>>,
    pub(crate) active: AtomicUsize,
    max_size: usize,
    returned: Condvar,
}

impl ConnectionPool {
    pub(crate) fn new(max_size: usize) -> Self {
        Self {
            idle: Mutex::new(VecDeque::new()),
            active: AtomicUsize::new(0),
            max_size,
            returned: Condvar::new(),
        }
    }

    /// Acquire an idle connection, or `None` when a new one should be opened.
    ///
    /// Blocks the calling thread until a slot is available when
    /// `active == max_size`.
    pub(crate) fn acquire(&self) -> Result<Option<ConnStream>> {
        let mut guard = self
            .idle
            .lock()
            .map_err(|_| Error::Backend("pool mutex poisoned".into()))?;

        loop {
            if let Some(stream) = guard.pop_front() {
                debug!("pool: reusing idle connection");
                return Ok(Some(stream));
            }

            let active = self.active.load(Ordering::SeqCst);
            if active < self.max_size {
                self.active.fetch_add(1, Ordering::SeqCst);
                debug!(active = active + 1, max = self.max_size, "pool: opening new connection");
                return Ok(None);
            }

            warn!(active, max = self.max_size, "pool: at capacity, parking caller");
            guard = self
                .returned
                .wait(guard)
                .map_err(|_| Error::Backend("pool condvar poisoned".into()))?;
        }
    }

    pub(crate) fn release(&self, stream: ConnStream) {
        if let Ok(mut guard) = self.idle.lock() {
            guard.push_back(stream);
        }
        self.returned.notify_one();
    }

    pub(crate) fn discard(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.returned.notify_one();
    }
}

// ---------------------------------------------------------------------------
// CircuitBreaker
// ---------------------------------------------------------------------------

/// Three-state circuit breaker.
///
/// - `Closed` → `Open` after `failure_threshold` consecutive failures.
/// - `Open` → `HalfOpen` after `reset_timeout` has elapsed.
/// - `HalfOpen` → `Closed` on the first successful probe.
/// - `HalfOpen` → `Open` on probe failure.
pub(crate) struct CircuitBreaker {
    state: AtomicU8,
    pub(crate) failures: AtomicUsize,
    tripped_at: Mutex<Option<Instant>>,
    failure_threshold: usize,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    pub(crate) fn new(failure_threshold: usize, reset_timeout: Duration) -> Self {
        Self {
            state: AtomicU8::new(CB_CLOSED),
            failures: AtomicUsize::new(0),
            tripped_at: Mutex::new(None),
            failure_threshold,
            reset_timeout,
        }
    }

    pub(crate) fn check(&self) -> Result<()> {
        match self.state.load(Ordering::SeqCst) {
            CB_CLOSED => {
                debug!("circuit_breaker: closed — allowing call");
                Ok(())
            }
            CB_OPEN => {
                let tripped = self
                    .tripped_at
                    .lock()
                    .map_err(|_| Error::Backend("circuit breaker mutex poisoned".into()))?;
                if let Some(t) = *tripped {
                    if t.elapsed() >= self.reset_timeout {
                        if self
                            .state
                            .compare_exchange(CB_OPEN, CB_HALF_OPEN, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                        {
                            debug!("circuit_breaker: transitioning to HALF_OPEN for probe");
                            return Ok(());
                        }
                        if self.state.load(Ordering::SeqCst) == CB_HALF_OPEN {
                            return Err(Error::Backend("circuit open: too many failures".into()));
                        }
                    }
                }
                Err(Error::Backend("circuit open: too many failures".into()))
            }
            CB_HALF_OPEN => Err(Error::Backend("circuit open: too many failures".into())),
            _ => Ok(()),
        }
    }

    pub(crate) fn on_success(&self) {
        self.failures.store(0, Ordering::SeqCst);
        self.state.store(CB_CLOSED, Ordering::SeqCst);
    }

    pub(crate) fn on_failure(&self) {
        let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.state.load(Ordering::SeqCst);
        // A failure in HALF_OPEN trips immediately; in CLOSED it trips only
        // once the failure count crosses the threshold.
        if current == CB_HALF_OPEN
            || (current == CB_CLOSED && failures >= self.failure_threshold)
        {
            self.trip();
        }
    }

    pub(crate) fn trip(&self) {
        warn!(failures = self.failures.load(Ordering::SeqCst), "circuit_breaker: tripping to OPEN");
        self.state.store(CB_OPEN, Ordering::SeqCst);
        if let Ok(mut t) = self.tripped_at.lock() {
            *t = Some(Instant::now());
        }
    }
}

// ---------------------------------------------------------------------------
// ClientRuntime
// ---------------------------------------------------------------------------

/// Owns a tokio runtime, a connection pool, and a circuit breaker for a
/// single server address.
///
/// Concurrent callers each borrow a connection from the pool; up to
/// `pool_size` connections may be live simultaneously.
///
/// ## Transport security
///
/// Use [`ClientRuntime::connect_tls`] for native TLS (certificate verified
/// against the system root CA store).  [`ClientRuntime::connect`] uses plain
/// TCP and requires a TLS-terminating reverse proxy (nginx, Envoy, AWS ALB)
/// or VPN / mTLS sidecar in front of the server.
pub struct ClientRuntime {
    rt: Runtime,
    addr: String,
    pool: ConnectionPool,
    breaker: CircuitBreaker,
    /// Present when TLS is enabled.
    tls: Option<TlsConfig>,
}

impl ClientRuntime {
    /// Connect over plain TCP (infrastructure-layer TLS required in production).
    pub fn connect(addr: impl Into<String>) -> Result<Arc<Self>> {
        Self::connect_with_options(
            addr,
            DEFAULT_POOL_SIZE,
            DEFAULT_FAILURE_THRESHOLD,
            DEFAULT_RESET_TIMEOUT,
        )
    }

    /// Connect over plain TCP with explicit pool / breaker parameters.
    pub fn connect_with_options(
        addr: impl Into<String>,
        pool_size: usize,
        failure_threshold: usize,
        reset_timeout: Duration,
    ) -> Result<Arc<Self>> {
        Self::build(addr, None, pool_size, failure_threshold, reset_timeout)
    }

    /// Connect with native TLS — server certificate is verified against the
    /// system root CA store (via `webpki-roots`).
    ///
    /// `tls.server_name` is the SNI hostname, e.g. `"atlas.example.com"`.
    pub fn connect_tls(addr: impl Into<String>, tls: TlsConfig) -> Result<Arc<Self>> {
        Self::connect_tls_with_options(
            addr,
            tls,
            DEFAULT_POOL_SIZE,
            DEFAULT_FAILURE_THRESHOLD,
            DEFAULT_RESET_TIMEOUT,
        )
    }

    /// Connect with native TLS and explicit pool / breaker parameters.
    pub fn connect_tls_with_options(
        addr: impl Into<String>,
        tls: TlsConfig,
        pool_size: usize,
        failure_threshold: usize,
        reset_timeout: Duration,
    ) -> Result<Arc<Self>> {
        Self::build(addr, Some(tls), pool_size, failure_threshold, reset_timeout)
    }

    fn build(
        addr: impl Into<String>,
        tls: Option<TlsConfig>,
        pool_size: usize,
        failure_threshold: usize,
        reset_timeout: Duration,
    ) -> Result<Arc<Self>> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| Error::Backend(format!("tokio runtime: {e}")))?;
        let me = Arc::new(Self {
            rt,
            addr: addr.into(),
            pool: ConnectionPool::new(pool_size),
            breaker: CircuitBreaker::new(failure_threshold, reset_timeout),
            tls,
        });
        me.call(Request::Hello { client_version: SERVICE_VERSION })?;
        Ok(me)
    }

    /// Open a fresh connection, doing a TLS handshake when configured.
    fn open_connection(&self) -> Result<ConnStream> {
        let addr = self.addr.clone();
        self.rt.block_on(async {
            let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
                .await
                .map_err(|_| Error::Backend(format!(
                    "connect {addr}: timed out after {}s", CONNECT_TIMEOUT.as_secs()
                )))?
                .map_err(|e| Error::Backend(format!("connect {addr}: {e}")))?;
            tcp.set_nodelay(true)
                .map_err(|e| Error::Backend(format!("nodelay: {e}")))?;

            if let Some(tls_cfg) = &self.tls {
                let mut roots = rustls::RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let client_cfg = rustls::ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                let connector = tokio_rustls::TlsConnector::from(Arc::new(client_cfg));
                let server_name: ServerName<'static> =
                    ServerName::try_from(tls_cfg.server_name.clone())
                        .map_err(|_| Error::Backend(format!(
                            "invalid TLS server name '{}'", tls_cfg.server_name
                        )))?;
                let tls_stream = connector
                    .connect(server_name, tcp)
                    .await
                    .map_err(|e| Error::Backend(format!("TLS handshake {addr}: {e}")))?;
                debug!(addr = %addr, "TLS handshake complete");
                Ok(ConnStream::Tls(Box::new(tls_stream)))
            } else {
                Ok(ConnStream::Plain(tcp))
            }
        })
    }

    /// Send one request and receive one response, using a pooled connection.
    pub fn call(&self, req: Request) -> Result<Response> {
        self.breaker.check()?;

        let maybe_stream = self.pool.acquire()?;
        let mut stream = match maybe_stream {
            Some(s) => s,
            None => match self.open_connection() {
                Ok(s) => s,
                Err(e) => {
                    self.pool.discard();
                    self.breaker.on_failure();
                    return Err(e);
                }
            },
        };

        let result = self.rt.block_on(async {
            tokio::time::timeout(REQUEST_TIMEOUT, async {
                write_frame(&mut stream, &req)
                    .await
                    .map_err(|e| Error::Backend(format!("write: {e}")))?;
                let resp: Response = read_frame(&mut stream)
                    .await
                    .map_err(|e| Error::Backend(format!("read: {e}")))?;
                Ok::<Response, Error>(resp)
            })
            .await
            .map_err(|_| Error::Backend(format!(
                "request timed out after {}s", REQUEST_TIMEOUT.as_secs()
            )))?
        });

        match result {
            Ok(resp) => {
                self.pool.release(stream);
                match resp {
                    Response::Error { message } => Err(Error::Backend(message)),
                    other => {
                        self.breaker.on_success();
                        Ok(other)
                    }
                }
            }
            Err(e) => {
                self.pool.discard();
                self.breaker.on_failure();
                Err(e)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- CircuitBreaker unit tests ----------------------------------------

    #[test]
    fn circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert!(cb.check().is_ok());
    }

    #[test]
    fn circuit_breaker_trips_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.on_failure();
        cb.on_failure();
        assert!(cb.check().is_ok(), "should still be closed at 2 failures");
        cb.on_failure();
        assert!(cb.check().is_err(), "should be open at threshold");
    }

    #[test]
    fn circuit_breaker_closes_on_success() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.on_failure();
        cb.on_failure();
        cb.on_failure();
        assert!(cb.check().is_err());
        cb.on_success();
        assert!(cb.check().is_ok(), "success must close the circuit");
    }

    #[test]
    fn circuit_breaker_transitions_to_half_open_after_timeout() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(10));
        cb.on_failure();
        cb.on_failure();
        assert!(cb.check().is_err());
        std::thread::sleep(Duration::from_millis(25));
        assert!(cb.check().is_ok(), "should transition to HalfOpen after timeout");
    }

    #[test]
    fn circuit_breaker_half_open_failure_trips_again() {
        let cb = CircuitBreaker::new(2, Duration::from_millis(10));
        cb.on_failure();
        cb.on_failure();
        std::thread::sleep(Duration::from_millis(25));
        cb.check().ok(); // transitions to HalfOpen
        cb.on_failure(); // probe fails → back to Open
        assert!(cb.check().is_err(), "failed probe must re-trip");
    }

    // ---- ConnectionPool unit tests ----------------------------------------

    #[test]
    fn connection_pool_returns_none_when_slot_available() {
        let pool = ConnectionPool::new(2);
        let result = pool.acquire().unwrap();
        assert!(result.is_none(), "fresh pool should return None (new slot)");
        assert_eq!(pool.active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn connection_pool_tracks_active_count() {
        let pool = ConnectionPool::new(2);
        assert!(pool.acquire().unwrap().is_none());
        assert!(pool.acquire().unwrap().is_none());
        assert_eq!(pool.active.load(Ordering::SeqCst), 2);
        pool.discard();
        assert_eq!(pool.active.load(Ordering::SeqCst), 1);
        assert!(pool.acquire().unwrap().is_none());
        assert_eq!(pool.active.load(Ordering::SeqCst), 2);
    }

    // ---- TlsConfig construction -------------------------------------------

    #[test]
    fn tls_config_stores_server_name() {
        let cfg = TlsConfig { server_name: "atlas.example.com".into() };
        assert_eq!(cfg.server_name, "atlas.example.com");
    }
}
