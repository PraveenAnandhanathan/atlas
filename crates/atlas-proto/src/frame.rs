//! Length-prefixed bincode frame codec.

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Per-frame length cap: 64 MiB.
///
/// Chunks are at most 4 MiB (ADR-0002), so this leaves plenty of headroom
/// for batched manifests and bulk operations.
pub const MAX_FRAME_SIZE: u32 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame too large: {0} > {1}")]
    TooLarge(u32, u32),
    #[error("decode: {0}")]
    Decode(String),
    #[error("encode: {0}")]
    Encode(String),
    #[error("connection closed")]
    Closed,
}

pub type FrameResult<T> = std::result::Result<T, FrameError>;

/// Read one length-prefixed bincode frame.
pub async fn read_frame<R, T>(reader: &mut R) -> FrameResult<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_bytes = [0u8; 4];
    match reader.read_exact(&mut len_bytes).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(FrameError::Closed),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge(len, MAX_FRAME_SIZE));
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).await?;
    bincode::deserialize(&buf).map_err(|e| FrameError::Decode(e.to_string()))
}

/// Write one length-prefixed bincode frame.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> FrameResult<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = bincode::serialize(value).map_err(|e| FrameError::Encode(e.to_string()))?;
    if bytes.len() > MAX_FRAME_SIZE as usize {
        return Err(FrameError::TooLarge(bytes.len() as u32, MAX_FRAME_SIZE));
    }
    let len = (bytes.len() as u32).to_le_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Msg {
        kind: String,
        payload: Vec<u8>,
    }

    #[tokio::test]
    async fn roundtrip_in_memory() {
        let msg = Msg {
            kind: "hello".into(),
            payload: vec![1, 2, 3, 4],
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &msg).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let back: Msg = read_frame(&mut cursor).await.unwrap();
        assert_eq!(msg, back);
    }

    #[tokio::test]
    async fn empty_input_signals_close() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let r: FrameResult<Msg> = read_frame(&mut cursor).await;
        assert!(matches!(r, Err(FrameError::Closed)));
    }
}
