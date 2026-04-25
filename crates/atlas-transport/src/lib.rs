//! Pluggable transport layer.
//!
//! Today the wire is plain TCP via tokio. The trait exists so RDMA can
//! slot in later behind the `rdma` feature without churning callers.
//!
//! `Transport` is intentionally minimal:
//! - `connect(addr)` → a bidirectional byte stream
//! - `bind(addr)` → an acceptor yielding inbound streams
//!
//! Framing (length-prefixed bincode) lives in `atlas-proto`. This crate
//! is *just* the moral equivalent of `TcpStream` / `TcpListener`.

use async_trait::async_trait;
use std::io;
use std::net::SocketAddr;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("address parse: {0}")]
    Addr(String),
    #[error("transport not available: {0}")]
    Unavailable(&'static str),
}

pub type Result<T> = std::result::Result<T, TransportError>;

/// One bidirectional, ordered byte stream.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Acceptor for inbound connections.
#[async_trait]
pub trait Acceptor: Send + Sync {
    /// Wait for the next inbound connection.
    async fn accept(&self) -> Result<(Box<dyn Stream>, SocketAddr)>;

    /// Address actually bound to (after resolving `:0`).
    fn local_addr(&self) -> Result<SocketAddr>;
}

/// Outbound connector + inbound binder.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&self, addr: &str) -> Result<Box<dyn Stream>>;
    async fn bind(&self, addr: &str) -> Result<Box<dyn Acceptor>>;
}

pub mod tcp {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};

    /// Plain TCP transport.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct TcpTransport;

    pub struct TcpAcceptor {
        inner: TcpListener,
    }

    #[async_trait]
    impl Acceptor for TcpAcceptor {
        async fn accept(&self) -> Result<(Box<dyn Stream>, SocketAddr)> {
            let (s, peer) = self.inner.accept().await?;
            // disable nagle for small RPC frames
            let _ = s.set_nodelay(true);
            Ok((Box::new(s), peer))
        }

        fn local_addr(&self) -> Result<SocketAddr> {
            Ok(self.inner.local_addr()?)
        }
    }

    #[async_trait]
    impl Transport for TcpTransport {
        async fn connect(&self, addr: &str) -> Result<Box<dyn Stream>> {
            let s = TcpStream::connect(addr).await?;
            let _ = s.set_nodelay(true);
            Ok(Box::new(s))
        }

        async fn bind(&self, addr: &str) -> Result<Box<dyn Acceptor>> {
            let l = TcpListener::bind(addr).await?;
            Ok(Box::new(TcpAcceptor { inner: l }))
        }
    }
}

#[cfg(feature = "rdma")]
pub mod rdma {
    //! RDMA stub. The real impl will use `ibverbs` / `rdma-core` once
    //! we have a driver-equipped CI box. Until then, every method
    //! returns `TransportError::Unavailable`.
    use super::*;

    #[derive(Debug, Default, Clone, Copy)]
    pub struct RdmaTransport;

    #[async_trait]
    impl Transport for RdmaTransport {
        async fn connect(&self, _addr: &str) -> Result<Box<dyn Stream>> {
            Err(TransportError::Unavailable(
                "rdma transport not yet implemented",
            ))
        }

        async fn bind(&self, _addr: &str) -> Result<Box<dyn Acceptor>> {
            Err(TransportError::Unavailable(
                "rdma transport not yet implemented",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tcp::TcpTransport;
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn tcp_roundtrip() {
        let t = TcpTransport;
        let acc = t.bind("127.0.0.1:0").await.unwrap();
        let addr = acc.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut s, _) = acc.accept().await.unwrap();
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).await.unwrap();
            s.write_all(b"PONG").await.unwrap();
            buf
        });

        let mut c = t.connect(&addr.to_string()).await.unwrap();
        c.write_all(b"PINGX").await.unwrap();
        let mut reply = [0u8; 4];
        c.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"PONG");
        assert_eq!(&server.await.unwrap(), b"PINGX");
    }
}
