//! An opt-in test relay. Bytes are opaque: TLS and SSH still terminate at GitHub.
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    stop: AtomicBool,
    failed: AtomicBool,
    active: AtomicUsize,
    connections: AtomicUsize,
    received: AtomicU64,
    sockets: Mutex<Vec<TcpStream>>,
    closures: Mutex<Vec<String>>,
}

pub struct Relay {
    pub port: u16,
    state: Arc<State>,
    listener: Option<JoinHandle<()>>,
}

impl Relay {
    pub fn start(addresses: Vec<SocketAddr>, https: bool) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let state = Arc::new(State::default());
        let shared = state.clone();
        let handle = thread::spawn(move || {
            let mut workers = Vec::new();
            while !shared.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        // Bounded resources, even if another local process connects.
                        if shared.connections.fetch_add(1, Ordering::SeqCst) >= 8 {
                            shared.failed.store(true, Ordering::SeqCst);
                            break;
                        }
                        shared.active.fetch_add(1, Ordering::SeqCst);
                        let state = shared.clone();
                        let addresses = addresses.clone();
                        workers.push(thread::spawn(move || {
                            let closing = client.try_clone().ok();
                            if serve(client, &addresses, https, &state).is_err()
                                && !state.stop.load(Ordering::SeqCst)
                            {
                                state.failed.store(true, Ordering::SeqCst);
                            }
                            if let Some(socket) = closing {
                                let _ = socket.shutdown(Shutdown::Both);
                            }
                            state.active.fetch_sub(1, Ordering::SeqCst);
                        }));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => {
                        shared.failed.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
            for worker in workers {
                if worker.join().is_err() {
                    shared.failed.store(true, Ordering::SeqCst);
                }
            }
        });
        Ok(Self {
            port,
            state,
            listener: Some(handle),
        })
    }

    pub fn cut(&self) {
        self.state.stop.store(true, Ordering::SeqCst);
        for socket in self.state.sockets.lock().unwrap().iter() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    pub fn active(&self) -> usize {
        self.state.active.load(Ordering::SeqCst)
    }
    pub fn connections(&self) -> usize {
        self.state.connections.load(Ordering::SeqCst)
    }
    pub fn received(&self) -> u64 {
        self.state.received.load(Ordering::SeqCst)
    }
    pub fn failed(&self) -> bool {
        self.state.failed.load(Ordering::SeqCst)
    }
    pub fn closures(&self) -> Vec<String> {
        self.state.closures.lock().unwrap().clone()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.cut();
        if let Some(handle) = self.listener.take() {
            let _ = handle.join();
        }
    }
}

fn register(socket: &TcpStream, state: &State) -> io::Result<()> {
    // Windows accepted sockets inherit the listener's nonblocking mode. The
    // pumps use blocking writes with bounded timeouts; write_all must not treat
    // ordinary backpressure (WSAEWOULDBLOCK) as a dropped connection.
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut sockets = state.sockets.lock().unwrap();
    // Serialize registration with cut: a socket registered after cut must close.
    if state.stop.load(Ordering::SeqCst) {
        socket.shutdown(Shutdown::Both)?;
    }
    sockets.push(socket.try_clone()?);
    Ok(())
}

fn connect_header(client: &mut TcpStream, state: &State) -> io::Result<()> {
    let start = Instant::now();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192
            || start.elapsed() > Duration::from_secs(5)
            || state.stop.load(Ordering::SeqCst)
        {
            return Err(io::Error::other("bounded CONNECT header rejected"));
        }
        let mut byte = [0];
        match client.read(&mut byte) {
            Ok(0) => return Err(io::Error::other("incomplete CONNECT header")),
            Ok(_) => header.push(byte[0]),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e),
        }
    }
    let first = header.split(|&b| b == b'\n').next().unwrap_or_default();
    if first != b"CONNECT github.com:443 HTTP/1.1\r"
        && first != b"CONNECT github.com:443 HTTP/1.0\r"
    {
        return Err(io::Error::other("only GitHub HTTPS CONNECT is allowed"));
    }
    Ok(())
}

fn serve(
    mut client: TcpStream,
    addresses: &[SocketAddr],
    https: bool,
    state: &Arc<State>,
) -> io::Result<()> {
    register(&client, state)?;
    if https {
        connect_header(&mut client, state)?;
    }
    let mut upstream = None;
    for address in addresses.iter().take(4) {
        if state.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        if let Ok(socket) = TcpStream::connect_timeout(address, Duration::from_secs(3)) {
            upstream = Some(socket);
            break;
        }
    }
    let upstream = upstream.ok_or_else(|| io::Error::other("relay could not connect"))?;
    register(&upstream, state)?;
    if https {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    }
    let client_back = client.try_clone()?;
    let upstream_back = upstream.try_clone()?;
    let shared = state.clone();
    let upload = thread::spawn(move || pump(client, upstream, &shared, false));
    pump(upstream_back, client_back, state, true);
    upload
        .join()
        .map_err(|_| io::Error::other("relay worker panicked"))?;
    Ok(())
}

fn pump(mut from: TcpStream, mut to: TcpStream, state: &State, downstream: bool) {
    let mut buffer = [0; 32 * 1024];
    let direction = if downstream { "download" } else { "upload" };
    let mut ending = "cut".to_string();
    while !state.stop.load(Ordering::SeqCst) {
        match from.read(&mut buffer) {
            Ok(0) => {
                ending = "eof".into();
                break;
            }
            Ok(count) => {
                if let Err(error) = to.write_all(&buffer[..count]) {
                    ending = format!(
                        "write:{:?}:{}",
                        error.kind(),
                        error.raw_os_error().unwrap_or(0)
                    );
                    break;
                }
                if downstream {
                    state.received.fetch_add(count as u64, Ordering::SeqCst);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                ending = format!(
                    "read:{:?}:{}",
                    error.kind(),
                    error.raw_os_error().unwrap_or(0)
                );
                break;
            }
        }
    }
    state
        .closures
        .lock()
        .unwrap()
        .push(format!("{direction}:{ending}"));
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_transfer_survives_receiver_backpressure() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let relay = Relay::start(vec![server.local_addr().unwrap()], false).unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", relay.port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (mut upstream, _) = server.accept().unwrap();
        upstream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let writer = thread::spawn(move || {
            let payload = vec![0xa5; 16 * 1024 * 1024];
            upstream.write_all(&payload).unwrap();
            // Keep the send side alive until the receiver confirms completion.
            upstream
        });
        // Let kernel buffers fill before this intentionally slow consumer reads.
        thread::sleep(Duration::from_millis(100));
        let mut data = vec![0; 16 * 1024 * 1024];
        client.read_exact(&mut data).unwrap();
        assert!(data.iter().all(|&byte| byte == 0xa5));
        let upstream = writer.join().unwrap();
        assert!(!relay.failed());
        drop(client);
        drop(upstream);
    }

    #[test]
    fn forwards_opaque_bytes_and_cuts_only_its_connection() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let relay = Relay::start(vec![server.local_addr().unwrap()], false).unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", relay.port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let (mut upstream, _) = server.accept().unwrap();
        upstream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let payload = [0, 255, 13, 10, 27, 128];
        client.write_all(&payload).unwrap();
        let mut observed = [0; 6];
        upstream.read_exact(&mut observed).unwrap();
        assert_eq!(observed, payload);
        upstream.write_all(&payload).unwrap();
        client.read_exact(&mut observed).unwrap();
        assert_eq!(observed, payload);
        relay.cut();
        assert_eq!(client.read(&mut observed).unwrap(), 0);
        drop(relay);
        // The unrelated listener survives relay cleanup.
        assert!(TcpStream::connect(server.local_addr().unwrap()).is_ok());
    }

    #[test]
    fn connect_proxy_preserves_tls_bytes_and_rejects_other_hosts() {
        for allowed in [true, false] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let relay = Relay::start(vec![server.local_addr().unwrap()], true).unwrap();
            let mut client = TcpStream::connect(("127.0.0.1", relay.port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let host = if allowed {
                "github.com"
            } else {
                "example.invalid"
            };
            write!(
                client,
                "CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n"
            )
            .unwrap();
            if allowed {
                let (mut upstream, _) = server.accept().unwrap();
                let mut response = [0; 39];
                client.read_exact(&mut response).unwrap();
                assert_eq!(&response, b"HTTP/1.1 200 Connection Established\r\n\r\n");
                client.write_all(b"opaque tls bytes").unwrap();
                let mut data = [0; 16];
                upstream.read_exact(&mut data).unwrap();
                assert_eq!(&data, b"opaque tls bytes");
            } else {
                assert_eq!(client.read(&mut [0]).unwrap(), 0);
                server.set_nonblocking(true).unwrap();
                assert_eq!(
                    server.accept().unwrap_err().kind(),
                    io::ErrorKind::WouldBlock
                );
            }
        }
    }
}
