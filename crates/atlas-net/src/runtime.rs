//! Shared connection pool + tokio runtime used by both client adapters.

use atlas_core::{Error, Result};
use atlas_proto::{read_frame, write_frame, Request, Response, SERVICE_VERSION};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;

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
struct ConnectionPool {
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
    fn new(max_size: usize) -> Self {
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
    fn acquire(&self) -> Result<Option<TcpStream>> {
        let mut guard = self
            .idle
            .lock()
            .map_err(|_| Error::Backend("pool mutex poisoned".into()))?;

        loop {
            // Try to reuse an idle connection first.
            if let Some(stream) = guard.pop_front() {
                return Ok(Some(stream));
            }

            // No idle connection. Can we open a new one?
            let active = self.active.load(Ordering::SeqCst);
            if active < self.max_size {
                // Reserve the slot. We'll actually open the TCP connection
                // outside the lock.
                self.active.fetch_add(1, Ordering::SeqCst);
                return Ok(None);
            }

            // At capacity: wait for a connection to be returned.
            guard = self
                .returned
                .wait(guard)
                .map_err(|_| Error::Backend("pool condvar poisoned".into()))?;
        }
    }

    /// Return a used connection to the idle pool.
    fn release(&self, stream: TcpStream) {
        if let Ok(mut guard) = self.idle.lock() {
            guard.push_back(stream);
        }
        self.returned.notify_one();
    }

    /// Discard a connection that errored out.  Decrements the active counter
    /// so a new connection slot becomes available.
    fn discard(&self) {
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
struct CircuitBreaker {
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
    fn new(failure_threshold: usize, reset_timeout: Duration) -> Self {
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
    fn check(&self) -> Result<()> {
        match self.state.load(Ordering::SeqCst) {
            CB_CLOSED => Ok(()),
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
    fn on_success(&self) {
        self.failures.store(0, Ordering::SeqCst);
        self.state.store(CB_CLOSED, Ordering::SeqCst);
    }

    /// Record a failed call.
    fn on_failure(&self) {
        let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        let current = self.state.load(Ordering::SeqCst);
        if current == CB_HALF_OPEN {
            // Probe failed – trip back to Open immediately.
            self.trip();
        } else if current == CB_CLOSED && failures >= self.failure_threshold {
            self.trip();
        }
    }

    fn trip(&self) {
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
