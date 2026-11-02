//! Bounded, loopback-only Streamable HTTP in JSON response mode (MCP 2025-11-25).
//! One persistent UI, no transport session IDs, no server push or automatic replay.
use crate::protocol::{self, Server, Shared};
use rocket_mcp_control::Config;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_HEADER: usize = 8192;
const MAX_CONNECTIONS: usize = 16;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}
struct Response {
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
}
impl Response {
    fn text(status: u16, message: &str) -> Self {
        Self {
            status,
            body: message.as_bytes().to_vec(),
            content_type: "text/plain; charset=utf-8",
        }
    }
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&value).unwrap(),
            content_type: "application/json",
        }
    }
}
struct Pending {
    original: Value,
    output: mpsc::SyncSender<Value>,
}
struct Transport {
    config: Config,
    bearer: String,
    shared: Arc<Shared>,
    server: Mutex<Server>,
    pending: Mutex<HashMap<u64, Pending>>,
    next: AtomicU64,
    requests: AtomicU64,
    rejected: AtomicU64,
}
impl Transport {
    fn handle(&self, request: HttpRequest) -> Response {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let reject = |code, message| {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            Response::text(code, message)
        };
        if request.headers.get("host") != Some(&self.config.address().to_string()) {
            return reject(403, "Loopback Host required");
        }
        if request
            .headers
            .get("origin")
            .is_some_and(|origin| origin != &format!("http://{}", self.config.address()))
        {
            return reject(403, "Origin rejected");
        }
        if !constant_equal(
            request
                .headers
                .get("authorization")
                .map_or(&[], |s| s.as_bytes()),
            self.bearer.as_bytes(),
        ) {
            return reject(401, "Local access token required");
        }
        if request.path == "/health" && request.method == "GET" {
            return Response::text(
                200,
                &format!(
                    "rocket-mcp\n{}\n{}\n{}\n",
                    env!("CARGO_PKG_VERSION"),
                    std::process::id(),
                    self.shared.window.load(Ordering::Acquire)
                ),
            );
        }
        if request.path == "/shutdown" && request.method == "POST" {
            let _server = self.server.lock().unwrap();
            let _transition = self.shared.transition.lock().unwrap();
            if !matches!(self.shared.window.load(Ordering::Acquire), 0 | 3)
                || !self.pending.lock().unwrap().is_empty()
            {
                return reject(409, "Close the MCP test window before stopping the service");
            }
            self.shared.shutdown();
            return Response::text(202, "Stopping");
        }
        if request.path != "/mcp" {
            return reject(404, "Unknown endpoint");
        }
        if request.method != "POST" {
            return reject(
                405,
                "Use POST; this service does not provide SSE or transport sessions",
            );
        }
        if request.headers.contains_key("mcp-session-id") {
            return reject(400, "This service does not use transport session IDs");
        }
        if request
            .headers
            .get("content-type")
            .is_none_or(|s| s.split(';').next().unwrap_or("").trim() != "application/json")
        {
            return reject(415, "Use application/json");
        }
        let accept = request.headers.get("accept").map_or("", String::as_str);
        if !accept.split(',').any(|s| s.trim() == "application/json")
            || !accept.split(',').any(|s| s.trim() == "text/event-stream")
        {
            return reject(406, "Accept application/json and text/event-stream");
        }
        let Ok(mut value) = serde_json::from_slice::<Value>(&request.body) else {
            return reject(400, "Invalid JSON");
        };
        if !value.is_object() || value["jsonrpc"] != "2.0" || !value["method"].is_string() {
            return reject(400, "A single JSON-RPC request or notification is required");
        }
        let method = value["method"].as_str().unwrap().to_owned();
        let version = request
            .headers
            .get("mcp-protocol-version")
            .map(String::as_str);
        if (method != "initialize" && version != Some(protocol::VERSION))
            || version.is_some_and(|v| v != protocol::VERSION)
        {
            return reject(400, "Unsupported or missing MCP-Protocol-Version");
        }
        let Some(original) = value.get("id").cloned() else {
            if method == "notifications/cancelled" {
                let key = &value["params"]["requestId"];
                let found = self
                    .pending
                    .lock()
                    .unwrap()
                    .iter()
                    .find_map(|(id, p)| (p.original == *key).then_some(*id));
                if let Some(id) = found {
                    value["params"]["requestId"] = json!(id);
                    self.server
                        .lock()
                        .unwrap()
                        .handle(&serde_json::to_vec(&value).unwrap());
                    // Release the HTTP waiter too; cancelled native input has no
                    // response ticket. Already delivered operations are not undone.
                    if let Some(pending) = self.pending.lock().unwrap().remove(&id) {
                        let _ = pending.output.try_send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32800,"message":"Request cancelled; inspect state for any input already delivered"}}));
                    }
                }
            }
            return Response::text(202, "");
        };
        if !(original.as_i64().is_some() || original.as_str().is_some_and(|s| s.len() <= 128)) {
            return reject(400, "Invalid request ID");
        }
        if method == "initialize" {
            let params = &value["params"];
            return Response::json(
                if params["protocolVersion"].is_string()
                    && params["capabilities"].is_object()
                    && params["clientInfo"].is_object()
                {
                    json!({"jsonrpc":"2.0","id":original,"result":protocol::initialize_result()})
                } else {
                    json!({"jsonrpc":"2.0","id":original,"error":{"code":-32602,"message":"Invalid initialization"}})
                },
            );
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (output, response) = mpsc::sync_channel(1);
        {
            let mut pending = self.pending.lock().unwrap();
            // One controller per endpoint. Never route an ambiguous cancellation
            // to another HTTP caller that happens to use the same request ID.
            if pending.len() >= 16 || pending.values().any(|p| p.original == original) {
                return reject(409, "Request ID already active or request limit reached");
            }
            pending.insert(
                id,
                Pending {
                    original: original.clone(),
                    output,
                },
            );
        }
        value["id"] = json!(id);
        {
            let mut server = self.server.lock().unwrap();
            if self.shared.stop.load(Ordering::Acquire) {
                self.pending.lock().unwrap().remove(&id);
                return reject(503, "Service is stopping");
            }
            server.handle(&serde_json::to_vec(&value).unwrap());
        }
        match response.recv_timeout(Duration::from_secs(24)) {
            Ok(mut reply) => {
                reply["id"] = original;
                Response::json(reply)
            }
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                self.server.lock().unwrap().handle(&serde_json::to_vec(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}})).unwrap());
                reject(
                    504,
                    "Result unavailable. Inspect state before retrying; writes are never replayed automatically",
                )
            }
        }
    }
}
fn constant_equal(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    for (i, expected) in b.iter().enumerate() {
        difference |= usize::from(a.get(i).copied().unwrap_or(0) ^ expected);
    }
    difference == 0
}
fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, Response> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if bytes.len() >= MAX_HEADER {
            return Err(Response::text(431, "Headers too large"));
        }
        read_some(stream, &mut bytes, deadline, MAX_HEADER)?;
    };
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| Response::text(400, "Invalid headers"))?;
    let mut lines = header.split("\r\n");
    let first: Vec<_> = lines.next().unwrap_or("").split(' ').collect();
    if first.len() != 3 || first[2] != "HTTP/1.1" {
        return Err(Response::text(400, "Use HTTP/1.1"));
    }
    let (method, path) = (first[0].to_string(), first[1].to_string());
    let mut headers = HashMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| Response::text(400, "Malformed header"))?;
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.bytes().any(|b| b.is_ascii_control() && b != b'\t')
            || headers
                .insert(key.to_ascii_lowercase(), value.trim().to_string())
                .is_some()
        {
            return Err(Response::text(400, "Invalid or duplicate header"));
        }
    }
    if headers.contains_key("transfer-encoding") || headers.contains_key("expect") {
        return Err(Response::text(
            400,
            "Transfer encoding and Expect are unsupported",
        ));
    }
    let size = if let Some(size) = headers.get("content-length") {
        if size.is_empty() || !size.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Response::text(400, "Invalid content length"));
        }
        size.parse::<usize>()
            .map_err(|_| Response::text(413, "Body too large"))?
    } else if method == "POST" {
        return Err(Response::text(411, "Content-Length required"));
    } else {
        0
    };
    if size > protocol::MAX_LINE {
        return Err(Response::text(413, "Body too large"));
    }
    let end = header_end + size;
    while bytes.len() < end {
        read_some(stream, &mut bytes, deadline, end)?;
    }
    if bytes.len() != end {
        return Err(Response::text(400, "Pipelined or excess input rejected"));
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}
fn read_some(
    stream: &mut TcpStream,
    bytes: &mut Vec<u8>,
    deadline: Instant,
    limit: usize,
) -> Result<(), Response> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Response::text(408, "Request timeout"));
    }
    stream
        .set_read_timeout(Some(left))
        .map_err(|_| Response::text(400, "Socket unavailable"))?;
    let mut buffer = [0; 4096];
    let count = buffer.len().min(limit.saturating_sub(bytes.len()));
    let n = stream
        .read(&mut buffer[..count])
        .map_err(|_| Response::text(408, "Request timeout"))?;
    if n == 0 {
        return Err(Response::text(400, "Incomplete request"));
    }
    bytes.extend_from_slice(&buffer[..n]);
    Ok(())
}
fn send(stream: &mut TcpStream, response: Response) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let reason = match response.status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n{}\r\n",
        response.status,
        response.content_type,
        response.body.len(),
        if response.status == 405 {
            "Allow: POST\r\n"
        } else {
            ""
        }
    );
    write_before(stream, header.as_bytes(), deadline)?;
    write_before(stream, &response.body, deadline)
}
fn write_before(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Response deadline elapsed",
            ));
        }
        stream.set_write_timeout(Some(remaining))?;
        let count = stream.write(bytes)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "Response connection closed",
            ));
        }
        bytes = &bytes[count..];
    }
    Ok(())
}
pub fn spawn(
    config: Config,
    token: String,
    mut server: Server,
    responses: mpsc::Receiver<Value>,
) -> io::Result<thread::JoinHandle<()>> {
    config.validate()?;
    let listener = TcpListener::bind(config.address())?;
    listener.set_nonblocking(true)?;
    server.http_ready();
    let transport = Arc::new(Transport {
        config,
        bearer: format!("Bearer {token}"),
        shared: server.shared.clone(),
        server: Mutex::new(server),
        pending: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
        requests: AtomicU64::new(0),
        rejected: AtomicU64::new(0),
    });
    let route = transport.clone();
    thread::Builder::new()
        .name("mcp-http".into())
        .spawn(move || {
            eprintln!("Rocket MCP HTTP started; loopback only; token authentication enabled");
            let dispatch = thread::spawn(move || {
                while !route.shared.stop.load(Ordering::Acquire) {
                    match responses.recv_timeout(Duration::from_millis(100)) {
                        Ok(value) => {
                            if let Some(id) = value["id"].as_u64()
                                && let Some(p) = route.pending.lock().unwrap().remove(&id)
                            {
                                let _ = p.output.try_send(value);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
            });
            let count = Arc::new(AtomicUsize::new(0));
            let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
            while !transport.shared.stop.load(Ordering::Acquire) {
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].is_finished() {
                        if workers.swap_remove(index).join().is_err() {
                            eprintln!("Rocket MCP HTTP worker failed; stopping service");
                            transport.shared.shutdown();
                        }
                    } else {
                        index += 1;
                    }
                }
                if transport.shared.stop.load(Ordering::Acquire) {
                    break;
                }
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        // Windows accept inherits the listener's nonblocking mode.
                        // Workers use blocking I/O with explicit deadlines; otherwise
                        // a client whose headers arrive after accept gets reset.
                        if stream.set_nonblocking(false).is_err() {
                            eprintln!("Rocket MCP HTTP socket setup failed");
                            continue;
                        }
                        if !peer.ip().is_loopback()
                            || count.load(Ordering::Acquire) >= MAX_CONNECTIONS
                        {
                            let _ =
                                send(&mut stream, Response::text(503, "Connection limit reached"));
                            continue;
                        }
                        let service = transport.clone();
                        let active = count.clone();
                        active.fetch_add(1, Ordering::AcqRel);
                        workers.push(thread::spawn(move || {
                            let response = match read_request(&mut stream) {
                                Ok(request) => service.handle(request),
                                Err(error) => error,
                            };
                            let _ = send(&mut stream, response);
                            active.fetch_sub(1, Ordering::AcqRel);
                        }));
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => {
                        transport.shared.shutdown();
                        break;
                    }
                }
            }
            drop(listener);
            for worker in workers {
                if worker.join().is_err() {
                    eprintln!("Rocket MCP HTTP worker failed during shutdown");
                }
            }
            if dispatch.join().is_err() {
                eprintln!("Rocket MCP HTTP response dispatcher failed");
            }
            eprintln!(
                "Rocket MCP HTTP stopped; requests={}; rejected={}",
                transport.requests.load(Ordering::Relaxed),
                transport.rejected.load(Ordering::Relaxed)
            );
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU8};
    struct Fixture {
        config: Config,
        shared: Arc<Shared>,
        thread: Option<thread::JoinHandle<()>>,
        requests: mpsc::Receiver<protocol::Request>,
    }
    impl Fixture {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let config = Config {
                port,
                root: std::env::temp_dir(),
            };
            let (output, responses) = mpsc::sync_channel(32);
            let shared = Arc::new(Shared {
                stop: AtomicBool::new(false),
                window: AtomicU8::new(0),
                transition: Mutex::new(()),
                output,
                pending: Mutex::new(HashMap::new()),
                context: Mutex::new(None),
                diagnostics: Mutex::new(None),
            });
            let (queue, requests) = mpsc::sync_channel(16);
            let (start, _) = mpsc::sync_channel(1);
            let server = Server::new(
                shared.clone(),
                queue,
                start,
                crate::policy::Policy::new(&config.root).unwrap(),
            );
            let thread = Some(spawn(config.clone(), "ab".repeat(32), server, responses).unwrap());
            Self {
                config,
                shared,
                thread,
                requests,
            }
        }
        fn raw(&self, data: &str) -> String {
            let mut socket = TcpStream::connect(self.config.address()).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(8)))
                .unwrap();
            socket.write_all(data.as_bytes()).unwrap();
            let mut out = String::new();
            socket.read_to_string(&mut out).unwrap();
            out
        }
        fn rpc(&self, body: &str, extra: &str) -> String {
            self.raw(&format!("POST /mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {}\r\nContent-Length: {}\r\n{extra}\r\n{body}",self.config.address(),"ab".repeat(32),protocol::VERSION,body.len()))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.shared.shutdown();
            self.thread.take().unwrap().join().unwrap();
        }
    }
    #[test]
    fn authenticated_http_reconnects_without_sessions_and_exposes_real_protocol() {
        let f = Fixture::new();
        let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture"}}}"#;
        for _ in 0..2 {
            let reply = f.rpc(initialize, "");
            assert!(reply.starts_with("HTTP/1.1 200"));
            assert!(!reply.to_lowercase().contains("mcp-session-id"));
            assert!(reply.contains(env!("CARGO_PKG_VERSION")));
        }
        assert!(
            f.rpc(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                ""
            )
            .starts_with("HTTP/1.1 202")
        );
        let list = f.rpc(r#"{"jsonrpc":"2.0","id":"list","method":"tools/list"}"#, "");
        assert!(list.contains("rocket_resize"));
        assert!(list.contains("\"id\":\"list\""));
        f.shared.window.store(3, Ordering::Release);
        let state=f.rpc(r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"rocket_state","arguments":{}}}"#,"");
        assert!(state.contains("closed"));
    }
    #[test]
    fn http_rejects_missing_auth_host_origin_and_ambiguous_framing() {
        let f = Fixture::new();
        let addr = f.config.address();
        let auth = "ab".repeat(32);
        for (request, code) in [
            (format!("GET /health HTTP/1.1\r\nHost: {addr}\r\n\r\n"), 401),
            (
                format!(
                    "GET /health HTTP/1.1\r\nHost: evil.invalid\r\nAuthorization: Bearer {auth}\r\n\r\n"
                ),
                403,
            ),
            (
                format!(
                    "GET /health HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {auth}\r\nOrigin: https://evil.invalid\r\n\r\n"
                ),
                403,
            ),
            (
                format!(
                    "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n"
                ),
                400,
            ),
            (
                format!("POST /mcp HTTP/1.1\r\nHost: {addr}\r\nTransfer-Encoding: chunked\r\n\r\n"),
                400,
            ),
            (
                format!("POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 65537\r\n\r\n"),
                413,
            ),
            (
                format!(
                    "GET /mcp HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {auth}\r\n\r\n"
                ),
                405,
            ),
        ] {
            let response = f.raw(&request);
            assert!(
                response.starts_with(&format!("HTTP/1.1 {code}")),
                "{response}"
            );
            assert!(!response.contains(&auth));
        }
        assert!(f.rpc("[]", "").starts_with("HTTP/1.1 400"));
        assert!(
            f.rpc(
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
                "Mcp-Session-Id: stale\r\n"
            )
            .starts_with("HTTP/1.1 400")
        );
    }
    #[test]
    fn shutdown_requires_closed_window_and_valid_local_authentication() {
        let f = Fixture::new();
        f.shared.window.store(2, Ordering::Release);
        let request = format!(
            "POST /shutdown HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Length: 0\r\n\r\n",
            f.config.address(),
            "ab".repeat(32)
        );
        assert!(f.raw(&request).starts_with("HTTP/1.1 409"));
        assert!(!f.shared.stop.load(Ordering::Acquire));
        f.shared.window.store(3, Ordering::Release);
        assert!(f.raw(&request).starts_with("HTTP/1.1 202"));
        assert!(f.shared.stop.load(Ordering::Acquire));
    }
    #[test]
    fn accepted_socket_waits_for_request_bytes_before_responding() {
        let f = Fixture::new();
        let mut socket = TcpStream::connect(f.config.address()).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // The TCP connection can arrive before the client writes its HTTP headers.
        thread::sleep(Duration::from_millis(150));
        write!(socket, "GET /health HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n", f.config.address(), "ab".repeat(32)).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }
    #[test]
    fn cancellation_routes_original_ids_and_releases_the_http_request() {
        let f = Fixture::new();
        f.shared.window.store(2, Ordering::Release);
        let address = f.config.address();
        let body = r#"{"jsonrpc":"2.0","id":"cancel-me","method":"tools/call","params":{"name":"rocket_wait","arguments":{}}}"#;
        let original = thread::spawn(move || {
            let mut socket = TcpStream::connect(address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(8)))
                .unwrap();
            write!(socket,"POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {}\r\nContent-Length: {}\r\n\r\n{body}", "ab".repeat(32), protocol::VERSION, body.len()).unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            response
        });
        let queued = f.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(f.rpc(body, "").starts_with("HTTP/1.1 409"));
        let response = f.rpc(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"cancel-me"}}"#, "");
        assert!(response.starts_with("HTTP/1.1 202"));
        assert!(queued.ticket.inactive());
        let response = original.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        let value: Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(value["id"], "cancel-me");
        assert_eq!(value["error"]["code"], -32800);
        assert!(f.requests.try_recv().is_err());
    }
}
