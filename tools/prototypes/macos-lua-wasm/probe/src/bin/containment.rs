//! Fixed, bounded native adversarial fixtures. Never shipped or called by Lua.
#[cfg(unix)]
fn main() {
    use rocket_lua_wasm_probe::guard;
    use std::{io::Write, time::Duration};
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.len() >= 3);
    let _guard = match args[0].as_str() {
        "guarded" => match guard::start(args[1].parse().unwrap()) {
            Ok(g) => Some(g),
            Err(code) => {
                eprintln!("{{\"event\":\"guard_refused\",\"code\":\"{code}\"}}");
                std::process::exit(guard::SETUP);
            }
        },
        "control" => None,
        _ => panic!("invalid fixture"),
    };
    let operation = args[2].as_str();
    let result = match operation {
        "read" => serde_json::json!({"success": std::fs::read(&args[3]).is_ok()}),
        "write" => {
            serde_json::json!({"success": std::fs::OpenOptions::new().write(true).create_new(true).open(&args[3]).and_then(|mut f| f.write_all(b"synthetic")).is_ok()})
        }
        "tcp" => {
            let address: std::net::SocketAddr = format!("127.0.0.1:{}", args[3]).parse().unwrap();
            serde_json::json!({"success": std::net::TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok()})
        }
        "unix" => {
            serde_json::json!({"success": std::os::unix::net::UnixStream::connect(&args[3]).is_ok()})
        }
        "fd" => {
            let fd: i32 = args[3].parse().unwrap();
            let mut byte = [0u8];
            let read = unsafe { libc::pread(fd, byte.as_mut_ptr().cast(), 1, 0) };
            serde_json::json!({"success": read == 1, "closed": unsafe {libc::fcntl(fd, libc::F_GETFD)} == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EBADF)})
        }
        "fork" => {
            let child = unsafe { libc::fork() };
            if child == 0 {
                unsafe { libc::_exit(0) };
            }
            if child > 0 {
                let mut status = 0;
                assert_eq!(unsafe { libc::waitpid(child, &mut status, 0) }, child);
            }
            serde_json::json!({"success": child > 0})
        }
        "wait" => {
            println!("{{\"ready\":true}}");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(8)); // Guard must end it first.
            serde_json::json!({"unexpected_completion":true})
        }
        "private" | "shared" => {
            println!("{{\"ready\":true}}");
            let mut chunks = Vec::new();
            // At most 256 MiB even if the guard regresses, never an OOM attack.
            for n in 1..=64 {
                let size = 4 * 1024 * 1024;
                let mode = if operation == "shared" {
                    libc::MAP_SHARED
                } else {
                    libc::MAP_PRIVATE
                };
                let p = unsafe {
                    libc::mmap(
                        std::ptr::null_mut(),
                        size,
                        libc::PROT_READ | libc::PROT_WRITE,
                        libc::MAP_ANON | mode,
                        -1,
                        0,
                    )
                };
                assert_ne!(p, libc::MAP_FAILED);
                chunks.push(p);
                for offset in (0..size).step_by(4096) {
                    unsafe {
                        p.cast::<u8>().add(offset).write_volatile(0x5a);
                    }
                }
                println!(
                    "{{\"allocated_bytes\":{},\"sample_bytes\":{}}}",
                    n * size,
                    guard::sample_memory().unwrap()
                );
                std::io::stdout().flush().unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            for p in chunks {
                assert_eq!(unsafe { libc::munmap(p, 4 * 1024 * 1024) }, 0);
            }
            serde_json::json!({"unexpected_completion":true})
        }
        _ => panic!("unknown fixture"),
    };
    println!("{result}");
}

#[cfg(not(unix))]
fn main() {
    panic!("Unix-only disposable prototype test");
}
