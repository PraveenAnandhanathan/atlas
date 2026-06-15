//! Shared connection pool + tokio runtime used by both client adapters.

use atlas_core::{Error, Result};
use atlas_proto::{read_frame, write_frame, Request, Response, SERVICE_VERSION};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
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

/// A pool of idle TCP connections.
///
/// Uses a `VecDeque` of ready streams plus a `Condvar` to park callers when
/// the pool is at capacity and no idle connections are available.
pub(crate) struct ConnectionPool {
    /// Idle connections ready to use.
    idle: Mutex<VecDeque<TcpStream>>,
    /// Total connections currently alive (idle + in-use).
    active: AtomicUsize,
    /// Maximum number of live connections.
    max_size: usize,
    /// Signals waiting threads when a connection is returned.
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

    /// Acquire an idle connection, or `None` if we may open a new one.
    ///
    /// Blocks until a slot is available if `active == max_size`.
    /// Returns `(stream, opened_new)` – the caller must increment `active`
    /// when it opens a new connection (returned as `None`).
    pub(crate) fn acquire(&self) -> Result<Option<TcpStream>> {
        let mut guard = self
            .idle
            .lock()
            .map_err(|_| Error::Backend("pool mutex poisoned".into()))?;

        loop {
            // Try to reuse an idle connection first.
            if let Some(stream) = guard.pop_front() {
                debug!("pool: reusing idle connection");
                return Ok(Some(stream));
            }

            // No idle connection. Can we open a new one?
            let active = self.active.load(Ordering::SeqCst);
            if active < self.max_size {
                // Reserve the slot. We'll actually open the TCP connection
                // outside the lock.
                self.active.fetch_add(1, Ordering::SeqCst);
                debug!(active = active + 1, max = self.max_size, "pool: opening new connection");
                return Ok(None);
            }

            // At capacity: wait for a connection to be returned.
            warn!(active, max = self.max_size, "pool: at capacity, parking caller");
            guard = self
                .returned
                .wait(guard)
                .map_err(|_| Error::Backend("pool condvar poisoned".into()))?;
        }
    }

    /// Return a used connection to the idle pool.
    pub(crate) fn release(&self, stream: TcpStream) {
        if let Ok(mut guard) = self.idle.lock() {
            guard.push_back(stream);
        }
        self.returned.notify_one();
    }

    /// Discard a connection that errored out.  Decrements the active counter
    /// so a new connection slot becomes available.
    pub(crate) fn discard(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.returned.notify_one();
    }
}

/// Three-state circuit breaker.
///
/// State transitions:
/// - `Closed` → `Open` after `failure_threshold` consecutive failures.
/// - `Open` → `HalfOpen` after `reset_timeout` has elapsed.
/// - `HalfOpen` → `Closed` on the first successful probe.
/// - `HalfOpen` → `Open` on probe failure.
pub(crate) struct CircuitBreaker {
    /// Current state: CB_CLOSED / CB_OPEN / CB_HALF_OPEN.
    state: AtomicU8,
    /// Consecutive failure counter (reset to 0 on success).
    failures: AtomicUsize,
    /// When the circuit was tripped (set when transitioning to Open).
    tripped_at: Mutex<Option<Instant>>,
    /// Consecutive failures before tripping.
    failure_threshold: usize,
    /// How long to wait in Open before allowing a probe.
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

    /// Returns `Ok(())` if the call should proceed, or an error if the
    /// circuit is open and the reset timeout has not yet elapsed.
    pub(crate) fn check(&self) -> Result<()> {
        match self.state.load(Ordering::SeqCst) {
            CB_CLOSED => {
                debug!("circuit_breaker: closed — allowing call");
                Ok(())
            }
            CB_OPEN => {
                // Check whether we should transition to HalfOpen.
                let tripped = self
                    .tripped_at
                    .lock()
                    .map_err(|_| Error::Backend("circuit breaker mutex poisoned".into()))?;
                if let Some(t) = *tripped {
                    if t.elapsed() >= self.reset_timeout {
                        // Transition to HalfOpen to allow one probe.
                        // Use compare_exchange so only one thread wins the race.
                        if self
                            .state
                            .compare_exchange(
                                CB_OPEN,
                                CB_HALF_OPEN,
                                Ordering::SeqCst,
                                Ordering::SeqCst,
                            )
                            .is_ok()
                        {
                            return Ok(());
                        }
                        // Another thread won; re-read the state.
                        if self.state.load(Ordering::SeqCst) == CB_HALF_OPEN {
                            // Probe already being attempted by another thread –
                            // fast-fail here to avoid double probes.
                            return Err(Error::Backend(
                                "circuit open: too many failures".into(),
                            ));
                        }
                    }
                }
                Err(Error::Backend("circuit open: too many failures".into()))
            }
            CB_HALF_OPEN => {
                // Only one probe should be in flight. Fast-fail all others.
                Err(Error::Backend("circuit open: too many failures".into()))
            }
            _ => Ok(()),
        }
    }

    /// Record a successful call.
    pub(crate) fn on_success(&self) {
        self.failures.store(0, Ordering::SeqCst);
        self.state.store(CB_CLOSED, Ordering::SeqCst);
    }

    /// Record a failed call.
    pub(crate) fn on_failure(&self) {
        let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.state.load(Ordering::SeqCst);
        if current == CB_HALF_OPEN {
            // Probe failed – trip back to Open immediately.
            self.trip();
        } else if current == CB_CLOSED && failures >= self.failure_threshold {
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

/// Owns a tokio runtime, a connection pool, and a circuit breaker for a
/// single server address.
///
/// Concurrent callers each borrow a connection from the pool; up to
/// `pool_size` connections may be live simultaneously.
///
/// **Security note**: connections are plain TCP. Production deployments must
/// place ATLAS behind a TLS-terminating reverse proxy (nginx, Envoy, AWS ALB)
/// or use a VPN / mTLS sidecar. Native-TLS transport is tracked in the backlog.
pub struct ClientRuntime {
    rt: Runtime,
    addr: String,
    pool: ConnectionPool,
    breaker: CircuitBreaker,
}

impl ClientRuntime {
    pub fn connect(addr: impl Into<String>) -> Result<Arc<Self>> {
        Self::connect_with_options(addr, DEFAULT_POOL_SIZE, DEFAULT_FAILURE_THRESHOLD, DEFAULT_RESET_TIMEOUT)
    }

    /// Explicit constructor for tests and custom configurations.
    pub fn connect_with_options(
        addr: impl Into<String>,
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
        let addr_s = addr.into();
        let me = Arc::new(Self {
            rt,
            addr: addr_s.clone(),
            pool: ConnectionPool::new(pool_size),
            breaker: CircuitBreaker::new(failure_threshold, reset_timeout),
        });
        // Eagerly handshake.
        me.call(Request::Hello {
            client_version: SERVICE_VERSION,
        })?;
        Ok(me)
    }

    /// Open a fresh TCP connection to `self.addr`.
    fn open_connection(&self) -> Result<TcpStream> {
        let addr = self.addr.clone();
        self.rt.block_on(async {
            let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
                .await
                .map_err(|_| {
                    Error::Backend(format!(
                        "connect {addr}: timed out after {}s",
                        CONNECT_TIMEOUT.as_secs()
                    ))
                })?
                .map_err(|e| Error::Backend(format!("connect {addr}: {e}")))?;
            stream
                .set_nodelay(true)
                .map_err(|e| Error::Backend(format!("nodelay: {e}")))?;
            Ok::<TcpStream, Error>(stream)
        })
    }

    /// Round-trip one request.
    ///
    /// Acquires a pooled connection (opening a new one if needed), runs the
    /// RPC, returns the connection to the pool on success, or discards it and
    /// records a failure on error.
    pub fn call(&self, req: Request) -> Result<Response> {
        // Fast-fail if the circuit is open.
        self.breaker.check()?;

        // Acquire a connection slot from the pool.
        let maybe_stream = self.pool.acquire()?;
        let mut stream = match maybe_stream {
            Some(s) => s,
            None => {
                // We reserved a new slot; open the TCP connection.
                match self.open_connection() {
                    Ok(s) => s,
                    Err(e) => {
                        // Couldn't connect – release the reserved slot and
                        // record the failure.
                        self.pool.discard();
                        self.breaker.on_failure();
                        return Err(e);
                    }
                }
            }
        };

        // Run the RPC without holding any locks.
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
            .map_err(|_| {
                Error::Backend(format!(
                    "request timed out after {}s",
                    REQUEST_TIMEOUT.as_secs()
                ))
            })?
        });

        match result {
            Ok(resp) => {
                // Put the healthy connection back.
                self.pool.release(stream);
                match resp {
                    Response::Error { message } => {
                        // Server-side error doesn't count as a transport failure.
                        Err(Error::Backend(message))
                    }
                    other => {
                        self.breaker.on_success();
                        Ok(other)
                    }
                }
            }
            Err(e) => {
                // Discard the broken connection and record the failure.
                self.pool.discard();
                self.breaker.on_failure();
                Err(e)
            }
        }
    }
}

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
        cb.on_failure(); // threshold reached
        assert!(cb.check().is_err(), "should be open at 3 failures");
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
        cb.on_failure(); // tripped
        assert!(cb.check().is_err());
        std::thread::sleep(Duration::from_millis(25));
        // After reset_timeout the circuit should allow one probe (HalfOpen).
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
        assert!(cb.check().is_err(), "failed probe must re-trip the circuit");
    }

    // ---- ConnectionPool unit tests ----------------------------------------

    #[test]
    fn connection_pool_returns_none_when_slot_available() {
        let pool = ConnectionPool::new(2);
        // No idle connections — acquire reserves a new slot and returns None.
        let result = pool.acquire().unwrap();
        assert!(result.is_none(), "fresh pool should return None (new slot)");
        assert_eq!(pool.active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn connection_pool_tracks_active_count() {
        let pool = ConnectionPool::new(2);
        assert!(pool.acquire().unwrap().is_none()); // active = 1
        assert!(pool.acquire().unwrap().is_none()); // active = 2
        assert_eq!(pool.active.load(Ordering::SeqCst), 2);
        pool.discard(); // active = 1
        assert_eq!(pool.active.load(Ordering::SeqCst), 1);
        // One slot freed — should be able to acquire again.
        assert!(pool.acquire().unwrap().is_none()); // active = 2
        assert_eq!(pool.active.load(Ordering::SeqCst), 2);
    }
}
