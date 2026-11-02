//! Exercise the real curl transport against a loopback service which applies a
//! mutation before returning a redirect. No account or external endpoint is used.
use rocket_git::{CancellationToken, Error, GitClient, IntegrationConnection};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

struct Service {
    connection: IntegrationConnection,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Service {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("accept fixture request: {e}"),
                };
                // Windows inherits the listener's nonblocking mode. Request I/O
                // must wait for bytes, bounded by the timeouts below.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    assert!(head.len() < 16384);
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    head.push(byte[0]);
                }
                let head = String::from_utf8(head).unwrap();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(length < 4096);
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                // Recording the completed request models applying the mutation
                // before the redirect response leaves the service.
                let request = head.lines().next().unwrap().to_owned();
                let status = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .trim_start_matches('/')
                    .parse::<u16>()
                    .unwrap_or(200);
                seen.lock().unwrap().push(request);
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\nLocation: http://127.0.0.1:{port}/followed\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        Self {
            connection: IntegrationConnection::loopback_fixture(port, "Bearer synthetic").unwrap(),
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn error_code(result: rocket_git::Result<rocket_git::IntegrationHttpResponse>) -> String {
    match result {
        Err(Error::InvalidInput(message)) => message.split(':').next().unwrap().into(),
        Err(error) => panic!("unexpected transport error: {error}"),
        Ok(_) => panic!("redirect was accepted"),
    }
}

#[test]
fn redirected_writes_have_unknown_outcomes_and_are_never_followed() {
    let service = Service::new();
    let client = GitClient::discover().unwrap();
    let mut count = 0;
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        for status in [300, 301, 302, 303, 304, 307, 308, 399] {
            let result = client.integration_http(
                &service.connection,
                method,
                &format!("/{status}"),
                Some(r#"{"text":"synthetic mutation"}"#),
                true,
                &CancellationToken::new(),
            );
            count += 1;
            assert_eq!(
                service.requests.lock().unwrap().len(),
                count,
                "redirect followed"
            );
            assert_eq!(
                error_code(result),
                "integration_outcome_unknown",
                "{method} {status}"
            );
        }
    }
}

#[test]
fn read_redirects_remain_refused_without_write_uncertainty() {
    let service = Service::new();
    let client = GitClient::discover().unwrap();
    for (index, status) in [301, 302, 303, 307, 308].into_iter().enumerate() {
        let result = client.integration_http(
            &service.connection,
            "GET",
            &format!("/{status}"),
            None,
            false,
            &CancellationToken::new(),
        );
        assert_eq!(error_code(result), "integration_redirect");
        assert_eq!(
            service.requests.lock().unwrap().len(),
            index + 1,
            "redirect followed"
        );
    }
}
