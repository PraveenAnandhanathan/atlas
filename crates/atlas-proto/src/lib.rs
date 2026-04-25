//! ATLAS wire protocol — request/response messages plus a length-prefixed
//! framing codec for TCP.
//!
//! Phase 2 ships a pure-Rust bincode-over-TCP RPC layer that exposes the
//! same `ChunkStore` and `MetaStore` traits used by `atlas-fs`. The wire
//! format is:
//!
//! ```text
//! [u32 LE length][bincode payload]
//! ```
//!
//! A swap to tonic+protobuf is mechanical: replace [`frame::read_frame`] /
//! [`frame::write_frame`] with the gRPC codec; the request enums map 1:1
//! to RPC methods.

pub mod frame;
pub mod messages;

pub use frame::{read_frame, write_frame, FrameError};
pub use messages::{
    BatchOp, ChunkRequest, ChunkResponse, MetaRequest, MetaResponse, ReplicateRequest,
    ReplicateResponse, Request, Response, SERVICE_VERSION,
};
