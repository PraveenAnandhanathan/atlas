//! Shared connection pool + tokio runtime used by both client adapters.

use atlas_core::{Error, Result};
use atlas_proto::{read_frame, write_frame, Request, Response, SERVICE_VERSION};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::runtime::Runtime;

/// How long to wait for the TCP handshake to complete.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for a response frame after the request is sent.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Owns a tokio runtime and a single multiplexed connection to the
/// storage server.
///
/// Every blocking call serializes on the connection mutex; concurrent
/// clients hold their own [`ClientRuntime`].
pub struct ClientRuntime {
    rt: Runtime,
    addr: String,
    conn: Mutex<Option<TcpStream>>,
}

impl ClientRuntime {
    pub fn connect(addr: impl Into<String>) -> Result<Arc<Self>> {
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
            conn: Mutex::new(None),
        });
        // Eagerly handshake.
        me.call(Request::Hello {
            client_version: SERVICE_VERSION,
        })?;
        Ok(me)
    }

    /// Round-trip one request. Reconnects on connection drop.
    pub fn call(&self, req: Request) -> Result<Response> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| Error::Backend("conn mutex poisoned".into()))?;
        if guard.is_none() {
            let addr = self.addr.clone();
            let stream = self
                .rt
                .block_on(async {
                    tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
                        .await
                        .map_err(|_| Error::Backend(format!(
                            "connect {addr}: timed out after {}s",
                            CONNECT_TIMEOUT.as_secs()
                        )))?
                        .map_err(|e| Error::Backend(format!("connect {addr}: {e}")))
                })?;
            stream
                .set_nodelay(true)
                .map_err(|e| Error::Backend(format!("nodelay: {e}")))?;
            *guard = Some(stream);
        }
        // Take the stream out so we can `block_on` without holding the
        // mutex across an await. We put it back on success — dropped on
        // error so the next call reconnects.
        let mut stream = guard.take().unwrap();
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
                *guard = Some(stream);
                match resp {
                    Response::Error { message } => Err(Error::Backend(message)),
                    other => Ok(other),
                }
            }
            Err(e) => {
                // Drop the stream so the next call reconnects.
                Err(e)
            }
        }
    }
}
