//! Sockets, and the two rules the shipped C++ learned the hard way.
//!
//! **A read has three outcomes, not two.** `Ok`, `Timeout`, `Closed`. Collapsing
//! a timeout into "closed" caused the worst failure in the old core: the control
//! loop spun at full speed on a 1 s timeout, leaked 39 CLOSE_WAIT sockets and
//! pegged a CPU core, while every log line said the phone was fine.
//!
//! **A write retries on timeout.** A send timeout means the peer is slow, not
//! gone. Returning an error there leaves a half-written frame on the wire and
//! permanently corrupts the peer's framing, because the length prefix has
//! already been sent.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The outcome of a read attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadResult {
    /// `n` bytes were read into the buffer.
    Ok(usize),
    /// The socket is still alive but had nothing to give within the timeout.
    /// **Not** an error, and **not** a disconnection.
    Timeout,
    /// The peer closed, or the socket failed unrecoverably.
    Closed,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("port {port} is already in use. Another PhoneCam instance may be running.")]
    PortInUse { port: u16 },
    #[error("could not bind {address}: {source}")]
    Bind {
        address: String,
        #[source]
        source: std::io::Error,
    },
    #[error("socket error: {0}")]
    Io(#[from] std::io::Error),
}

/// A TCP connection with the timeout semantics the protocol needs.
pub struct Connection {
    stream: TcpStream,
    /// Set when the peer is known to be gone, so a closed connection is not
    /// re-polled forever.
    closed: AtomicBool,
}

impl Connection {
    pub fn new(stream: TcpStream) -> std::io::Result<Self> {
        stream.set_nodelay(true)?;
        Ok(Self { stream, closed: AtomicBool::new(false) })
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.stream.peer_addr().ok()
    }

    /// True when the peer is on this machine, which is the only source trusted
    /// for auth mode 0.
    pub fn is_loopback(&self) -> bool {
        match self.peer_addr() {
            Some(addr) => addr.ip().is_loopback(),
            None => false,
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.stream.set_read_timeout(timeout)
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.stream.set_write_timeout(timeout)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// One read attempt, mapping the OS error classes to the three outcomes.
    pub fn read_some(&self, buf: &mut [u8]) -> ReadResult {
        if self.is_closed() {
            return ReadResult::Closed;
        }
        let mut stream = &self.stream;
        match stream.read(buf) {
            Ok(0) => {
                self.closed.store(true, Ordering::Relaxed);
                ReadResult::Closed
            }
            Ok(n) => ReadResult::Ok(n),
            Err(e) => match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => ReadResult::Timeout,
                std::io::ErrorKind::Interrupted => ReadResult::Timeout,
                _ => {
                    self.closed.store(true, Ordering::Relaxed);
                    ReadResult::Closed
                }
            },
        }
    }

    /// Fills `buf` completely, or reports why it could not.
    ///
    /// A timeout part-way through is `Timeout`, and the caller must decide
    /// whether to retry â€” it must never treat a partial fill as a frame.
    pub fn read_exact(&self, buf: &mut [u8]) -> ReadResult {
        let mut filled = 0usize;
        while filled < buf.len() {
            match self.read_some(&mut buf[filled..]) {
                ReadResult::Ok(0) => return ReadResult::Closed,
                ReadResult::Ok(n) => filled += n,
                ReadResult::Timeout => {
                    if filled == 0 {
                        return ReadResult::Timeout;
                    }
                    // Partially read: the peer stopped mid-frame. Treating this
                    // as a frame would desynchronise the stream permanently.
                    return ReadResult::Timeout;
                }
                ReadResult::Closed => return ReadResult::Closed,
            }
        }
        ReadResult::Ok(filled)
    }

    /// Writes a whole frame, retrying on a send timeout.
    ///
    /// Returns false only when the peer is actually gone.
    pub fn write_all(&self, mut data: &[u8]) -> bool {
        if self.is_closed() {
            return false;
        }
        let mut stream = &self.stream;
        while !data.is_empty() {
            match stream.write(data) {
                Ok(0) => return false,
                Ok(n) => data = &data[n..],
                Err(e) => match e.kind() {
                    // A slow peer is not a gone peer. Retrying is what keeps the
                    // frame intact.
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => continue,
                    std::io::ErrorKind::Interrupted => continue,
                    _ => {
                        self.closed.store(true, Ordering::Relaxed);
                        return false;
                    }
                },
            }
        }
        let _ = stream.flush();
        true
    }

    /// Reports whether data is already waiting, without consuming it.
    ///
    /// Used to drain a backlog: if a newer frame is already in the socket buffer,
    /// the one in hand is stale, and decoding it in order would make the picture
    /// fall further behind.
    pub fn peek(&self, buf: &mut [u8]) -> ReadResult {
        if self.is_closed() {
            return ReadResult::Closed;
        }
        // A one-byte peek is enough to know something is there.
        if buf.is_empty() {
            return ReadResult::Timeout;
        }
        let mut one = [0u8; 1];
        match (&self.stream).peek(&mut one) {
            Ok(0) => {
                self.closed.store(true, Ordering::Relaxed);
                ReadResult::Closed
            }
            Ok(n) => ReadResult::Ok(n),
            Err(e) => match e.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => ReadResult::Timeout,
                std::io::ErrorKind::Interrupted => ReadResult::Timeout,
                _ => {
                    self.closed.store(true, Ordering::Relaxed);
                    ReadResult::Closed
                }
            },
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("peer", &self.peer_addr())
            .field("closed", &self.is_closed())
            .finish()
    }
}

/// A TCP listener that can be stopped from another thread.
pub struct Listener {
    inner: TcpListener,
    running: Arc<AtomicBool>,
    port: u16,
}

impl Listener {
    pub fn bind(port: u16) -> Result<Self, TransportError> {
        let inner = TcpListener::bind(("0.0.0.0", port)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                TransportError::PortInUse { port }
            } else {
                TransportError::Bind { address: format!("0.0.0.0:{port}"), source: e }
            }
        })?;
        inner.set_nonblocking(false)?;
        // The bound port, not the requested one: binding 0 asks the OS to choose,
        // and reporting 0 back would leave the caller advertising a port nothing
        // is listening on.
        let port = inner.local_addr().map(|addr| addr.port()).unwrap_or(port);
        Ok(Self { inner, running: Arc::new(AtomicBool::new(true)), port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn running_flag(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    /// Blocks until a connection arrives or the listener is stopped.
    pub fn accept(&self) -> Option<Connection> {
        if !self.running.load(Ordering::Relaxed) {
            return None;
        }
        match self.inner.accept() {
            Ok((stream, _addr)) => Connection::new(stream).ok(),
            Err(_) => None,
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// A UDP socket for discovery.
///
/// One socket on `0.0.0.0:48720` receives on every interface, and a probe is
/// answered **unicast to the sender's own port** â€” a phone that binds its
/// listener to 48720 and sends from an ephemeral socket will never see a reply
/// addressed to the discovery port.
pub struct DiscoverySocket {
    socket: UdpSocket,
}

impl DiscoverySocket {
    pub fn bind(port: u16) -> Result<Self, TransportError> {
        let socket = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                TransportError::PortInUse { port }
            } else {
                TransportError::Bind { address: format!("0.0.0.0:{port}"), source: e }
            }
        })?;
        socket.set_read_timeout(Some(Duration::from_millis(500)))?;
        socket.set_broadcast(true)?;
        Ok(Self { socket })
    }

    /// Receives one datagram. `None` means the timeout elapsed, not a failure.
    pub fn recv(&self) -> Option<(Vec<u8>, SocketAddr)> {
        let mut buf = vec![0u8; 4096];
        match self.socket.recv_from(&mut buf) {
            Ok((n, addr)) => {
                buf.truncate(n);
                Some((buf, addr))
            }
            Err(_) => None,
        }
    }

    /// Sends to a specific peer â€” the unicast probe reply.
    pub fn send_to(&self, data: &[u8], addr: SocketAddr) -> bool {
        self.socket.send_to(data, addr).is_ok()
    }

    /// Broadcasts to the limited broadcast address and to each interface's
    /// directed broadcast, so a phone on a different subnet of a multi-homed PC
    /// still hears the beacon.
    pub fn broadcast(&self, data: &[u8], port: u16, directed: &[std::net::Ipv4Addr]) -> usize {
        let mut sent = 0;
        let limited = SocketAddr::from(([255, 255, 255, 255], port));
        if self.socket.send_to(data, limited).is_ok() {
            sent += 1;
        }
        for addr in directed {
            let target = SocketAddr::from((*addr, port));
            if self.socket.send_to(data, target).is_ok() {
                sent += 1;
            }
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_timeout_is_not_a_disconnection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (server_stream, _) = listener.accept().unwrap();

        let conn = Connection::new(server_stream).unwrap();
        conn.set_read_timeout(Some(Duration::from_millis(60))).unwrap();

        // Nothing was written: this must be Timeout, and the connection must
        // still be usable. Treating it as Closed is the documented bug.
        let mut buf = [0u8; 8];
        assert_eq!(conn.read_some(&mut buf), ReadResult::Timeout);
        assert!(!conn.is_closed(), "a timeout must not mark the peer gone");

        // And it still works afterwards.
        let mut writer = &client;
        writer.write_all(b"hello").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        match conn.read_some(&mut buf) {
            ReadResult::Ok(n) => assert_eq!(&buf[..n], b"hello"),
            other => panic!("expected the data after a timeout, got {other:?}"),
        }
    }

    #[test]
    fn a_closed_peer_reads_as_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let conn = Connection::new(server_stream).unwrap();
        conn.set_read_timeout(Some(Duration::from_millis(100))).unwrap();

        drop(client);
        std::thread::sleep(Duration::from_millis(50));
        let mut buf = [0u8; 8];
        assert_eq!(conn.read_some(&mut buf), ReadResult::Closed);
        assert!(conn.is_closed());
    }

    #[test]
    fn read_exact_fills_the_whole_buffer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let conn = Connection::new(server_stream).unwrap();
        conn.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

        client.write_all(b"0123456789").unwrap();
        let mut buf = [0u8; 10];
        assert_eq!(conn.read_exact(&mut buf), ReadResult::Ok(10));
        assert_eq!(&buf, b"0123456789");
    }

    #[test]
    fn read_exact_reports_a_partial_frame_as_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let conn = Connection::new(server_stream).unwrap();
        conn.set_read_timeout(Some(Duration::from_millis(80))).unwrap();

        // Half a frame arrives, then the peer goes quiet.
        client.write_all(b"01234").unwrap();
        let mut buf = [0u8; 10];
        assert_eq!(
            conn.read_exact(&mut buf),
            ReadResult::Timeout,
            "a partial read must never be reported as a complete frame"
        );
        assert!(!conn.is_closed());
    }

    #[test]
    fn write_then_read_round_trips() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let conn = Connection::new(server_stream).unwrap();
        conn.set_read_timeout(Some(Duration::from_millis(500))).unwrap();

        assert!(conn.write_all(b"ping"));
        let mut reader = client;
        reader.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
    }

    #[test]
    fn loopback_peer_is_recognised() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let conn = Connection::new(server_stream).unwrap();
        assert!(conn.is_loopback());
    }

    #[test]
    fn a_second_bind_on_the_same_port_is_named_clearly() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.inner.local_addr().unwrap().port();
        match Listener::bind(port) {
            Err(TransportError::PortInUse { .. }) => {}
            Err(other) => panic!("expected PortInUse, got {other:?}"),
            Ok(_) => panic!("expected PortInUse, but the second bind succeeded"),
        }
    }

    #[test]
    fn discovery_socket_round_trips_a_datagram() {
        let a = DiscoverySocket::bind(0).unwrap();
        let b = DiscoverySocket::bind(0).unwrap();
        // The socket is bound to 0.0.0.0, which is not a valid destination, so
        // the port is paired with a real loopback address here.
        let b_port = b.socket.local_addr().unwrap().port();
        let b_addr = SocketAddr::from(([127, 0, 0, 1], b_port));
        assert!(a.send_to(b"beacon", b_addr));
        let (data, _from) = b.recv().expect("datagram should arrive");
        assert_eq!(data, b"beacon");
    }
}



