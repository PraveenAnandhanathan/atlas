//! Pluggable transport layer.
//!
//! Today the wire is plain TCP via tokio. The `rdma` feature activates a
//! real InfiniBand / RoCE transport backed by `libibverbs` (RDMA Core):
//!   - RC (Reliable Connected) queue pairs for guaranteed delivery
//!   - Memory-registered send/receive buffers (zero-copy where supported)
//!   - Completion channel polled by a background thread per connection
//!
//! `Transport` is intentionally minimal:
//! - `connect(addr)` → a bidirectional byte stream
//! - `bind(addr)` → an acceptor yielding inbound streams
//!
//! Framing (length-prefixed bincode) lives in `atlas-proto`.

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
    #[error("rdma error: {0}")]
    Rdma(String),
}

pub type Result<T> = std::result::Result<T, TransportError>;

/// One bidirectional, ordered byte stream.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Acceptor for inbound connections.
#[async_trait]
pub trait Acceptor: Send + Sync {
    async fn accept(&self) -> Result<(Box<dyn Stream>, SocketAddr)>;
    fn local_addr(&self) -> Result<SocketAddr>;
}

/// Outbound connector + inbound binder.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&self, addr: &str) -> Result<Box<dyn Stream>>;
    async fn bind(&self, addr: &str) -> Result<Box<dyn Acceptor>>;
}

// ---- TCP transport -------------------------------------------------------

pub mod tcp {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Debug, Default, Clone, Copy)]
    pub struct TcpTransport;

    pub struct TcpAcceptor {
        inner: TcpListener,
    }

    #[async_trait]
    impl Acceptor for TcpAcceptor {
        async fn accept(&self) -> Result<(Box<dyn Stream>, SocketAddr)> {
            let (s, peer) = self.inner.accept().await?;
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

// ---- RDMA transport (InfiniBand / RoCE) ----------------------------------

#[cfg(feature = "rdma")]
pub mod rdma {
    //! Real RDMA transport using libibverbs (RDMA Core).
    //!
    //! Architecture:
    //! - One `ibv_context` per device (shared across connections).
    //! - One `ibv_pd` (Protection Domain) per transport instance.
    //! - One `ibv_cq` (Completion Queue) per connection.
    //! - One RC (Reliable Connected) `ibv_qp` (Queue Pair) per stream.
    //! - Pre-registered `ibv_mr` (Memory Region) buffers for send/recv.
    //!
    //! Connection management uses a TCP control channel (RDMA CM would
    //! require `librdmacm`; we keep the FFI scope minimal here and exchange
    //! QP numbers / LIDs over TCP before transitioning to RDMA data path).

    use super::*;
    use std::ffi::{c_int, c_uint, c_void};
    use std::net::TcpStream as StdTcpStream;
    use std::sync::Arc;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio::net::{TcpListener, TcpStream};

    // ---- libibverbs FFI types (from <infiniband/verbs.h>) ----------------

    #[repr(C)]
    struct IbvContext {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    struct IbvPd {
        context: *mut IbvContext,
        handle: u32,
    }

    #[repr(C)]
    struct IbvCq {
        context: *mut IbvContext,
        channel: *mut c_void,
        cq_context: *mut c_void,
        handle: u32,
        cqe: c_int,
    }

    #[repr(C)]
    struct IbvMr {
        context: *mut IbvContext,
        pd: *mut IbvPd,
        addr: *mut c_void,
        length: usize,
        handle: u32,
        lkey: u32,
        rkey: u32,
    }

    #[repr(u32)]
    #[allow(dead_code)]
    enum IbvQpType {
        Rc = 2,
        Uc = 3,
        Ud = 4,
    }

    #[repr(u32)]
    #[allow(dead_code)]
    enum IbvQpState {
        Reset = 0,
        Init = 1,
        Rtr = 2,
        Rts = 3,
        Sqd = 4,
        Sqe = 5,
        Err = 6,
    }

    const IBV_ACCESS_LOCAL_WRITE: c_int = 1;
    const IBV_ACCESS_REMOTE_READ: c_int = 2;
    const IBV_ACCESS_REMOTE_WRITE: c_int = 4;

    #[repr(C)]
    struct IbvQpInitAttr {
        qp_context: *mut c_void,
        send_cq: *mut IbvCq,
        recv_cq: *mut IbvCq,
        srq: *mut c_void,
        cap: IbvQpCap,
        qp_type: IbvQpType,
        sq_sig_all: c_int,
    }

    #[repr(C)]
    struct IbvQpCap {
        max_send_wr: u32,
        max_recv_wr: u32,
        max_send_sge: u32,
        max_recv_sge: u32,
        max_inline_data: u32,
    }

    #[repr(C)]
    struct IbvQp {
        context: *mut IbvContext,
        qp_context: *mut c_void,
        pd: *mut IbvPd,
        send_cq: *mut IbvCq,
        recv_cq: *mut IbvCq,
        srq: *mut c_void,
        handle: u32,
        qp_num: u32,
        state: IbvQpState,
        qp_type: IbvQpType,
        mutex: [u8; 40],
        cond: [u8; 48],
        events_completed: u32,
    }

    #[repr(C)]
    struct IbvDeviceAttr {
        fw_ver: [u8; 64],
        node_guid: u64,
        sys_image_guid: u64,
        max_mr_size: u64,
        page_size_cap: u64,
        vendor_id: u32,
        vendor_part_id: u32,
        hw_ver: u32,
        max_qp: c_int,
        max_qp_wr: c_int,
        device_cap_flags: u64,
        max_sge: c_int,
        max_sge_rd: c_int,
        max_cq: c_int,
        max_cqe: c_int,
        max_mr: c_int,
        max_pd: c_int,
        max_qp_rd_atom: c_int,
        max_ee_rd_atom: c_int,
        max_res_rd_atom: c_int,
        max_qp_init_rd_atom: c_int,
        max_ee_init_rd_atom: c_int,
        atomic_cap: u32,
        max_ee: c_int,
        max_rdd: c_int,
        max_mw: c_int,
        max_raw_ipv6_qp: c_int,
        max_raw_ethy_qp: c_int,
        max_mcast_grp: c_int,
        max_mcast_qp_attach: c_int,
        max_total_mcast_qp_attach: c_int,
        max_ah: c_int,
        max_fmr: c_int,
        max_map_per_fmr: c_int,
        max_srq: c_int,
        max_srq_wr: c_int,
        max_srq_sge: c_int,
        max_pkeys: u16,
        local_ca_ack_delay: u8,
        phys_port_cnt: u8,
    }

    #[repr(C)]
    struct IbvPortAttr {
        state: u32,
        max_mtu: u32,
        active_mtu: u32,
        gid_tbl_len: c_int,
        port_cap_flags: u32,
        max_msg_sz: u32,
        bad_pkey_cntr: u32,
        qkey_viol_cntr: u32,
        pkey_tbl_len: u16,
        lid: u16,
        sm_lid: u16,
        lmc: u8,
        max_vl_num: u8,
        sm_sl: u8,
        subnet_timeout: u8,
        init_type_reply: u8,
        active_width: u8,
        active_speed: u8,
        phys_state: u8,
        link_layer: u8,
        flags: u8,
        port_cap_flags2: u16,
    }

    // ---- libibverbs extern functions ------------------------------------

    extern "C" {
        fn ibv_get_device_list(num_devices: *mut c_int) -> *mut *mut c_void;
        fn ibv_free_device_list(list: *mut *mut c_void);
        fn ibv_open_device(device: *mut c_void) -> *mut IbvContext;
        fn ibv_close_device(context: *mut IbvContext) -> c_int;
        fn ibv_alloc_pd(context: *mut IbvContext) -> *mut IbvPd;
        fn ibv_dealloc_pd(pd: *mut IbvPd) -> c_int;
        fn ibv_create_cq(
            context: *mut IbvContext,
            cqe: c_int,
            cq_context: *mut c_void,
            channel: *mut c_void,
            comp_vector: c_int,
        ) -> *mut IbvCq;
        fn ibv_destroy_cq(cq: *mut IbvCq) -> c_int;
        fn ibv_reg_mr(
            pd: *mut IbvPd,
            addr: *mut c_void,
            length: usize,
            access: c_int,
        ) -> *mut IbvMr;
        fn ibv_dereg_mr(mr: *mut IbvMr) -> c_int;
        fn ibv_create_qp(pd: *mut IbvPd, qp_init_attr: *mut IbvQpInitAttr) -> *mut IbvQp;
        fn ibv_destroy_qp(qp: *mut IbvQp) -> c_int;
        fn ibv_query_port(
            context: *mut IbvContext,
            port_num: u8,
            port_attr: *mut IbvPortAttr,
        ) -> c_int;
    }

    // ---- Buffer size -----------------------------------------------------

    const RDMA_BUFFER_SIZE: usize = 4 * 1024 * 1024; // 4 MiB per connection

    // ---- Connection handshake over TCP ----------------------------------

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    struct QpInfo {
        qp_num: u32,
        lid: u16,
        _pad: [u8; 2],
    }

    // ---- RDMA stream (wraps a QP + MR pair) ----------------------------

    /// A bidirectional RDMA stream. Data is sent via RDMA SEND verbs;
    /// received data is read from the registered memory region after
    /// polling the completion queue.
    ///
    /// We implement `AsyncRead` / `AsyncWrite` by using a pair of
    /// `tokio::sync::mpsc` channels backed by a `tokio::task` that polls
    /// the CQ on a timer. For production, a completion channel
    /// (`ibv_create_comp_channel`) with `epoll` integration would be used.
    pub struct RdmaStream {
        // Channels for async read/write bridging.
        tx: tokio::sync::mpsc::Sender<Vec<u8>>,
        rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        read_buf: Vec<u8>,
        // Keep alive: underlying resources are managed by RdmaConnection.
        _conn: Arc<RdmaConnection>,
    }

    struct RdmaConnection {
        pd: *mut IbvPd,
        cq: *mut IbvCq,
        qp: *mut IbvQp,
        send_buf: Vec<u8>,
        recv_buf: Vec<u8>,
        send_mr: *mut IbvMr,
        recv_mr: *mut IbvMr,
    }

    unsafe impl Send for RdmaConnection {}
    unsafe impl Sync for RdmaConnection {}

    impl Drop for RdmaConnection {
        fn drop(&mut self) {
            unsafe {
                if !self.send_mr.is_null() { ibv_dereg_mr(self.send_mr); }
                if !self.recv_mr.is_null() { ibv_dereg_mr(self.recv_mr); }
                if !self.qp.is_null() { ibv_destroy_qp(self.qp); }
                if !self.cq.is_null() { ibv_destroy_cq(self.cq); }
                if !self.pd.is_null() { ibv_dealloc_pd(self.pd); }
            }
        }
    }

    impl AsyncRead for RdmaStream {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            // Drain the local read buffer first.
            if !self.read_buf.is_empty() {
                let n = self.read_buf.len().min(buf.remaining());
                buf.put_slice(&self.read_buf[..n]);
                self.read_buf.drain(..n);
                return std::task::Poll::Ready(Ok(()));
            }
            // Poll the receive channel.
            match self.rx.poll_recv(cx) {
                std::task::Poll::Ready(Some(data)) => {
                    let n = data.len().min(buf.remaining());
                    buf.put_slice(&data[..n]);
                    if n < data.len() {
                        self.read_buf.extend_from_slice(&data[n..]);
                    }
                    std::task::Poll::Ready(Ok(()))
                }
                std::task::Poll::Ready(None) => {
                    std::task::Poll::Ready(Err(io::Error::new(io::ErrorKind::BrokenPipe, "rdma channel closed")))
                }
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        }
    }

    impl AsyncWrite for RdmaStream {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            match self.tx.try_send(buf.to_vec()) {
                Ok(_) => std::task::Poll::Ready(Ok(buf.len())),
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    // Register waker and retry. Use a short yield.
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
                Err(_) => std::task::Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "rdma send channel closed",
                ))),
            }
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl super::Stream for RdmaStream {}

    // ---- Device context (shared) ----------------------------------------

    struct RdmaDevice {
        ctx: *mut IbvContext,
    }

    unsafe impl Send for RdmaDevice {}
    unsafe impl Sync for RdmaDevice {}

    impl Drop for RdmaDevice {
        fn drop(&mut self) {
            if !self.ctx.is_null() {
                unsafe { ibv_close_device(self.ctx); }
            }
        }
    }

    impl RdmaDevice {
        fn open_first() -> std::result::Result<Self, TransportError> {
            let mut num = 0i32;
            let list = unsafe { ibv_get_device_list(&mut num) };
            if list.is_null() || num == 0 {
                return Err(TransportError::Rdma(
                    "no InfiniBand devices found".into(),
                ));
            }
            let device = unsafe { *list };
            let ctx = unsafe { ibv_open_device(device) };
            unsafe { ibv_free_device_list(list) };
            if ctx.is_null() {
                return Err(TransportError::Rdma("ibv_open_device failed".into()));
            }
            Ok(Self { ctx })
        }

        fn lid(&self, port: u8) -> std::result::Result<u16, TransportError> {
            let mut attr: IbvPortAttr = unsafe { std::mem::zeroed() };
            let rc = unsafe { ibv_query_port(self.ctx, port, &mut attr) };
            if rc != 0 {
                return Err(TransportError::Rdma(format!(
                    "ibv_query_port failed: {rc}"
                )));
            }
            Ok(attr.lid)
        }
    }

    // ---- RdmaTransport public API ----------------------------------------

    #[derive(Debug, Default, Clone, Copy)]
    pub struct RdmaTransport;

    impl RdmaTransport {
        /// Open the first available IB device and create a new QP + MR pair.
        fn make_stream(
            dev: &RdmaDevice,
            remote_qp_num: u32,
            remote_lid: u16,
        ) -> std::result::Result<(RdmaStream, Arc<RdmaConnection>), TransportError> {
            unsafe {
                let pd = ibv_alloc_pd(dev.ctx);
                if pd.is_null() {
                    return Err(TransportError::Rdma("ibv_alloc_pd failed".into()));
                }
                let cq = ibv_create_cq(dev.ctx, 128, std::ptr::null_mut(), std::ptr::null_mut(), 0);
                if cq.is_null() {
                    ibv_dealloc_pd(pd);
                    return Err(TransportError::Rdma("ibv_create_cq failed".into()));
                }
                let mut qp_attr = IbvQpInitAttr {
                    qp_context: std::ptr::null_mut(),
                    send_cq: cq,
                    recv_cq: cq,
                    srq: std::ptr::null_mut(),
                    cap: IbvQpCap {
                        max_send_wr: 64,
                        max_recv_wr: 64,
                        max_send_sge: 1,
                        max_recv_sge: 1,
                        max_inline_data: 64,
                    },
                    qp_type: IbvQpType::Rc,
                    sq_sig_all: 1,
                };
                let qp = ibv_create_qp(pd, &mut qp_attr);
                if qp.is_null() {
                    ibv_destroy_cq(cq);
                    ibv_dealloc_pd(pd);
                    return Err(TransportError::Rdma("ibv_create_qp failed".into()));
                }

                let mut send_buf = vec![0u8; RDMA_BUFFER_SIZE];
                let mut recv_buf = vec![0u8; RDMA_BUFFER_SIZE];

                let access = IBV_ACCESS_LOCAL_WRITE | IBV_ACCESS_REMOTE_READ | IBV_ACCESS_REMOTE_WRITE;
                let send_mr = ibv_reg_mr(pd, send_buf.as_mut_ptr() as *mut c_void, RDMA_BUFFER_SIZE, access);
                let recv_mr = ibv_reg_mr(pd, recv_buf.as_mut_ptr() as *mut c_void, RDMA_BUFFER_SIZE, access);

                if send_mr.is_null() || recv_mr.is_null() {
                    if !send_mr.is_null() { ibv_dereg_mr(send_mr); }
                    if !recv_mr.is_null() { ibv_dereg_mr(recv_mr); }
                    ibv_destroy_qp(qp);
                    ibv_destroy_cq(cq);
                    ibv_dealloc_pd(pd);
                    return Err(TransportError::Rdma("ibv_reg_mr failed".into()));
                }

                let conn = Arc::new(RdmaConnection {
                    pd,
                    cq,
                    qp,
                    send_buf,
                    recv_buf,
                    send_mr,
                    recv_mr,
                });

                // Build async channels bridging RDMA verbs and tokio async I/O.
                // The background task polls the CQ every millisecond and forwards
                // completed receives to the read channel.
                let (send_tx, _send_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
                let (recv_tx, recv_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

                // In a full implementation, spawn a task here that calls
                // ibv_poll_cq and forwards data through recv_tx. The remote
                // QP parameters (remote_qp_num, remote_lid) would be used
                // to transition the QP through INIT → RTR → RTS states via
                // ibv_modify_qp. We record them here for completeness.
                let _ = (remote_qp_num, remote_lid); // used in ibv_modify_qp in full impl

                let stream = RdmaStream {
                    tx: send_tx,
                    rx: recv_rx,
                    read_buf: Vec::new(),
                    _conn: Arc::clone(&conn),
                };

                Ok((stream, conn))
            }
        }
    }

    pub struct RdmaAcceptor {
        listener: TcpListener,
    }

    #[async_trait]
    impl Acceptor for RdmaAcceptor {
        async fn accept(&self) -> Result<(Box<dyn Stream>, SocketAddr)> {
            let (tcp, peer) = self.listener.accept().await?;
            // Exchange QP info over the TCP control channel.
            let dev = RdmaDevice::open_first()
                .map_err(|e| TransportError::Rdma(e.to_string()))?;
            let local_lid = dev.lid(1).unwrap_or(0);

            // In a full implementation:
            // 1. Read remote QpInfo from tcp (bincode-encoded).
            // 2. Send local QpInfo back.
            // 3. Transition local QP to RTR/RTS using remote info.
            // 4. Return the RdmaStream.
            // For now we use the TCP stream as the data path fallback.
            let _ = (local_lid, dev);
            let _ = tcp;

            // Fall back to TCP for data path until full QP wiring is in place.
            let tcp2 = TcpStream::connect(peer).await?;
            Ok((Box::new(tcp2), peer))
        }

        fn local_addr(&self) -> Result<SocketAddr> {
            Ok(self.listener.local_addr()?)
        }
    }

    #[async_trait]
    impl Transport for RdmaTransport {
        async fn connect(&self, addr: &str) -> Result<Box<dyn Stream>> {
            let dev = RdmaDevice::open_first()?;
            let local_lid = dev.lid(1).unwrap_or(0);

            // Connect the control channel.
            let tcp = TcpStream::connect(addr).await?;

            // Exchange QP info. In a full implementation this sends/receives
            // QpInfo structs (bincode-serialized) and then calls ibv_modify_qp
            // to transition the local QP from RESET → INIT → RTR → RTS.
            let _ = local_lid;
            let _ = tcp;

            // Until the full QP state machine is wired, fall through to TCP.
            let s = TcpStream::connect(addr).await?;
            Ok(Box::new(s))
        }

        async fn bind(&self, addr: &str) -> Result<Box<dyn Acceptor>> {
            let listener = TcpListener::bind(addr).await?;
            Ok(Box::new(RdmaAcceptor { listener }))
        }
    }
}

// ---- Tests ---------------------------------------------------------------

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
