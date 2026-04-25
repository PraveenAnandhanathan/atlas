//! TCP listener + per-connection request loop.

use crate::handlers::{handle_chunk, handle_meta};
use atlas_chunk::{ChunkStore, LocalChunkStore};
use atlas_meta::{MetaStore, SledStore};
use atlas_proto::{read_frame, write_frame, FrameError, Request, Response, SERVICE_VERSION};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub chunks_dir: PathBuf,
    pub meta_dir: PathBuf,
}

/// Start serving forever. Returns when the listener fails to accept a
/// new connection (only on hard OS errors). Each connection is handled
/// on its own tokio task.
pub async fn serve(cfg: ServerConfig) -> anyhow::Result<()> {
    let chunks: Arc<dyn ChunkStore> = Arc::new(LocalChunkStore::open(&cfg.chunks_dir)?);
    let meta: Arc<dyn MetaStore> = Arc::new(SledStore::open(&cfg.meta_dir)?);
    let listener = TcpListener::bind(&cfg.bind).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!(%local_addr, "atlas-storage listening");
    loop {
        let (stream, peer) = listener.accept().await?;
        let chunks = chunks.clone();
        let meta = meta.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, chunks, meta).await {
                tracing::warn!(%peer, error = %e, "connection ended");
            }
        });
    }
}

/// Like [`serve`], but binds, returns the bound `SocketAddr`, and runs the
/// accept loop on the current task. Useful in tests where the caller wants
/// to know the ephemeral port.
pub async fn serve_with_addr(
    cfg: ServerConfig,
) -> anyhow::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let chunks: Arc<dyn ChunkStore> = Arc::new(LocalChunkStore::open(&cfg.chunks_dir)?);
    let meta: Arc<dyn MetaStore> = Arc::new(SledStore::open(&cfg.meta_dir)?);
    let listener = TcpListener::bind(&cfg.bind).await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => break,
            };
            let chunks = chunks.clone();
            let meta = meta.clone();
            tokio::spawn(async move {
                let _ = handle_connection(stream, chunks, meta).await;
            });
        }
    });
    Ok((addr, handle))
}

async fn handle_connection(
    mut stream: TcpStream,
    chunks: Arc<dyn ChunkStore>,
    meta: Arc<dyn MetaStore>,
) -> anyhow::Result<()> {
    stream.set_nodelay(true)?;
    loop {
        let req: Request = match read_frame(&mut stream).await {
            Ok(r) => r,
            Err(FrameError::Closed) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let resp = dispatch(&chunks, &meta, req);
        write_frame(&mut stream, &resp).await?;
    }
}

fn dispatch(chunks: &Arc<dyn ChunkStore>, meta: &Arc<dyn MetaStore>, req: Request) -> Response {
    match req {
        Request::Hello { client_version } => {
            if client_version != SERVICE_VERSION {
                return Response::Error {
                    message: format!(
                        "wire version mismatch: client={client_version} server={SERVICE_VERSION}"
                    ),
                };
            }
            Response::Hello {
                server_version: SERVICE_VERSION,
            }
        }
        Request::Chunk(c) => match handle_chunk(chunks.as_ref(), *c) {
            Ok(r) => Response::Chunk(Box::new(r)),
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
        Request::Meta(m) => match handle_meta(meta.as_ref(), *m) {
            Ok(r) => Response::Meta(Box::new(r)),
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
        Request::Replicate(_) => Response::Error {
            message: "replication not handled by this server (use atlas-replicate)".into(),
        },
        Request::Goodbye => Response::Hello {
            server_version: SERVICE_VERSION,
        },
    }
}
