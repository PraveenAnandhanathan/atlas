//! Shared connection pool + tokio runtime used by both client adapters.

use atlas_core::{Error, Result};
use atlas_proto::{read_frame, write_frame, Request, Response, SERVICE_VERSION};
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;

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
            let stream = self
                .rt
                .block_on(TcpStream::connect(&self.addr))
                .map_err(|e| Error::Backend(format!("connect {}: {e}", self.addr)))?;
            stream
                .set_nodelay(true)
                .map_err(|e| Error::Backend(format!("nodelay: {e}")))?;
            *guard = Some(stream);
        }
        // Take the stream out so we can `block_on` without holding the
        // mutex across an await. We put it back when we're done — even
        // on error, since the failure may be transient.
        let mut stream = guard.take().unwrap();
        let result = self.rt.block_on(async {
            write_frame(&mut stream, &req)
                .await
                .map_err(|e| Error::Backend(format!("write: {e}")))?;
            let resp: Response = read_frame(&mut stream)
                .await
                .map_err(|e| Error::Backend(format!("read: {e}")))?;
            Ok::<Response, Error>(resp)
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
