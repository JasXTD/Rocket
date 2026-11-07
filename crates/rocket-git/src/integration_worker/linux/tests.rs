use super::*;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};

const F_SETFD: c_int = 2;

/// The probe worker: `probe.rs` built with the worker's own `lockdown.rs` beside it.
fn fixture() -> &'static IntegrationWorker {
    static FIXTURE: OnceLock<IntegrationWorker> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("rocket-worker-fixture-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("probe.rs");
        fs::write(&source, include_str!("probe.rs")).unwrap();
        fs::write(root.join("lockdown.rs"), include_str!("lockdown.rs")).unwrap();
        let executable = root.join("rocket-lua-host");
        let result = Command::new("rustc")
            .args(["--edition=2024", "-C", "opt-level=1"])
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let digest = crate::advisory_tool::digest(
            &mut File::open(&executable).unwrap(),
            MAX_WORKER,
            &CancellationToken::new(),
        )
        .unwrap();
        IntegrationWorker::new(executable, &digest)
    })
}
fn limits() -> CommandLimits {
    CommandLimits {
        timeout: Duration::from_secs(5),
        stdout_bytes: 4096,
        stderr_bytes: 4096,
    }
}

#[test]
fn worker_hash_rejects_unbound_and_changed_files_before_start() {
    let original = fixture();
    let cancel = CancellationToken::new();
    for digest in ["", "00", &"0".repeat(64)] {
        let worker = IntegrationWorker::new(original.path.clone(), digest);
        let error = worker
            .run(b"echo\nshould-not-run".to_vec(), &limits(), &cancel)
            .err()
            .expect("must reject before launch");
        assert!(
            error.to_string().contains("worker_unbound")
                || error.to_string().contains("hash mismatch"),
            "{error}"
        );
    }
    let mutated = original.path.with_file_name("mutated");
    let mut bytes = fs::read(&original.path).unwrap();
    bytes.push(0);
    fs::write(&mutated, bytes).unwrap();
    let worker = IntegrationWorker::new(mutated.clone(), &original.sha256);
    assert!(
        worker
            .run(Vec::new(), &limits(), &cancel)
            .err()
            .expect("must reject modified worker")
            .to_string()
            .contains("hash mismatch")
    );
    fs::remove_file(mutated).unwrap();
}

#[test]
fn only_a_worker_file_that_only_its_owner_can_change_is_run() {
    let worker = fixture();
    let root = worker.path.parent().unwrap();
    let shared = root.join("shared-worker");
    fs::copy(&worker.path, &shared).unwrap();
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o775)).unwrap();
    let link = root.join("linked-worker");
    let _ = fs::remove_file(&link);
    std::os::unix::fs::symlink(&worker.path, &link).unwrap();
    for path in [&shared, &link] {
        let error = IntegrationWorker::new(path.clone(), &worker.sha256)
            .run(b"echo\nx".to_vec(), &limits(), &CancellationToken::new())
            .err()
            .expect("must refuse before launch");
        assert!(
            error.to_string().contains("only its owner can change"),
            "{error}"
        );
    }
    let relative = IntegrationWorker::new(PathBuf::from("rocket-lua-host"), &worker.sha256);
    assert!(
        relative
            .run(b"echo\nx".to_vec(), &limits(), &CancellationToken::new())
            .err()
            .expect("must refuse a relative path")
            .to_string()
            .contains("absolute")
    );
    fs::remove_file(shared).unwrap();
    fs::remove_file(link).unwrap();
}

#[test]
fn locked_down_worker_denies_files_network_children_signals_and_executable_memory() {
    let worker = fixture();
    let root = worker.path.parent().unwrap();
    let private = root.join("private-fixture");
    let denied = root.join("must-not-exist");
    fs::write(&private, b"local synthetic data").unwrap();
    assert_eq!(fs::read(&private).unwrap(), b"local synthetic data");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    // The target is reachable from the parent, so a denied child socket is meaningful.
    let baseline = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let _ = listener.accept().unwrap();
    drop(baseline);
    listener.set_nonblocking(true).unwrap();
    let input = format!(
        "permissions\n{}\n{}\n{}",
        private.display(),
        denied.display(),
        listener.local_addr().unwrap()
    );
    // Descriptors left inheritable here must not reach the worker. Few and brief: the
    // tests beside this one start processes too.
    let sentinel = File::open(&private).unwrap();
    let inheritable: Vec<_> = (0..32)
        .map(|_| {
            let file = sentinel.try_clone().unwrap();
            // SAFETY: clears close-on-exec on a synthetic descriptor this test owns.
            assert_ne!(unsafe { fcntl(file.as_raw_fd(), F_SETFD, 0) }, -1);
            file
        })
        .collect();
    let output = worker.run(input.into_bytes(), &limits(), &CancellationToken::new());
    drop(inheritable);
    let output = output.unwrap();
    assert!(
        output.status.success(),
        "status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "read=false;write=false;network=false;child=false;executable_memory=false;large_allocation=false;inherited=false;signal=false"
    );
    // EPERM: seccomp refused the socket before Landlock was asked.
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "network_error=1"
    );
    assert!(!denied.exists());
    assert_eq!(fs::read(&private).unwrap(), b"local synthetic data");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(sentinel);
    fs::remove_file(private).unwrap();
}

#[test]
fn isolated_worker_echo_limits_and_cancellation_reap_the_process() {
    let worker = fixture();
    let cancel = CancellationToken::new();
    let output = worker
        .run("echo\n資料 🚀".as_bytes().to_vec(), &limits(), &cancel)
        .unwrap();
    assert!(
        output.status.success(),
        "status={} stdout={}",
        output.status,
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(output.stdout, "資料 🚀".as_bytes());
    assert!(matches!(
        worker.run(b"flood".to_vec(), &limits(), &cancel),
        Err(Error::OutputLimit { .. })
    ));
    let short = CommandLimits {
        timeout: Duration::from_millis(100),
        ..limits()
    };
    assert!(matches!(
        worker.run(b"wait".to_vec(), &short, &cancel),
        Err(Error::TimedOut)
    ));
    let mut observed = None;
    let result = run_observed(worker, b"wait".to_vec(), &limits(), &cancel, |pid| {
        // SAFETY: signal 0 only checks that our live child exists.
        assert_eq!(unsafe { kill(pid as c_int, 0) }, 0);
        observed = Some(pid);
        cancel.cancel();
    });
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "{:?}",
        result.err()
    );
    // Ended and collected, not left behind as a zombie.
    let pid = observed.unwrap();
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn lock_down_refuses_a_process_with_other_threads() {
    // A second thread for certain while asking. If the refusal failed, this test process
    // would be locked down and the opens below would fail.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let other = std::thread::spawn(move || {
        let _ = held.recv();
    });
    let refused = lockdown::apply();
    let through_worker_api = IntegrationWorker::enter_isolation();
    drop(release);
    other.join().unwrap();
    assert!(
        refused.unwrap_err().to_string().contains("single-threaded"),
        "the lock-down must refuse first"
    );
    assert!(through_worker_api.is_err());
    assert!(File::open("/proc/self/status").is_ok());
    assert!(TcpListener::bind("127.0.0.1:0").is_ok());
}

/// Runs a classic BPF program over one system call, as the kernel runs a seccomp filter.
fn verdict(program: &[lockdown::Instruction], arch: u32, number: u32, arguments: [u64; 6]) -> u32 {
    let mut data = [0u8; 64];
    data[..4].copy_from_slice(&number.to_le_bytes());
    data[4..8].copy_from_slice(&arch.to_le_bytes());
    for (index, value) in arguments.iter().enumerate() {
        data[16 + 8 * index..24 + 8 * index].copy_from_slice(&value.to_le_bytes());
    }
    let (mut accumulator, mut next) = (0u32, 0usize);
    loop {
        let instruction = program[next];
        next += 1;
        let jump = |taken: bool| {
            usize::from(if taken {
                instruction.jt
            } else {
                instruction.jf
            })
        };
        match instruction.code {
            0x20 => {
                let at = instruction.k as usize;
                accumulator = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
            }
            0x15 => next += jump(accumulator == instruction.k),
            0x25 => next += jump(accumulator > instruction.k),
            0x45 => next += jump(accumulator & instruction.k != 0),
            0x06 => return instruction.k,
            code => panic!("unexpected BPF instruction {code:#x}"),
        }
    }
}

#[test]
fn seccomp_filter_leaves_only_the_workers_own_calls() {
    let pid = 4242;
    let program = lockdown::filter(pid);
    assert!(program.len() < 256);
    let call = |number: u32, arguments: [u64; 6]| {
        verdict(&program, lockdown::AUDIT_ARCH_X86_64, number, arguments)
    };
    let (allow, deny) = (lockdown::ALLOW, lockdown::DENY);
    let none = [0; 6];
    // Another architecture, or an x32 call, ends the worker.
    const AUDIT_ARCH_I386: u32 = 0x4000_0003;
    assert_eq!(verdict(&program, AUDIT_ARCH_I386, 0, none), lockdown::KILL);
    assert_eq!(call(lockdown::X32_SYSCALL_BIT | 1, none), lockdown::KILL);
    // Independent policy, deliberately NOT derived from lockdown::ALLOWED. Adding an
    // allowed syscall in production must fail here until its policy is reviewed too.
    const UNCONDITIONAL: &[u32] = &[
        3, 11, 12, 13, 14, 15, 24, 25, 28, 39, 60, 96, 131, 186, 201, 219, 231, 318,
    ];
    for number in 0..1024 {
        let (arguments, expected) = match number {
            n if UNCONDITIONAL.contains(&n) => ([7; 6], allow),
            0 | 19 => (none, allow),
            1 | 20 => ([1, 0, 0, 0, 0, 0], allow),
            9 => ([0, 4096, 3, 0x22, u64::MAX, 0], allow),
            10 => ([0, 4096, 3, 0, 0, 0], allow),
            202 => ([0, 128, 0, 0, 0, 0], allow),
            228 => (none, allow),
            234 => ([pid.into(), pid.into(), 6, 0, 0, 0], allow),
            _ => (none, deny),
        };
        assert_eq!(call(number, arguments), expected, "call {number}");
        if expected == deny {
            assert_eq!(call(number, [u64::MAX; 6]), deny, "call {number}");
        }
        assert_eq!(
            call(lockdown::X32_SYSCALL_BIT | number, arguments),
            lockdown::KILL,
            "x32 call {number}"
        );
    }
    for &number in UNCONDITIONAL {
        assert_eq!(call(number, [7; 6]), allow, "call {number}");
    }
    // read and readv: standard input only. The kernel reads only an int's low 32 bits.
    for number in [0, 19] {
        assert_eq!(call(number, none), allow);
        assert_eq!(call(number, [1 << 32, 0, 0, 0, 0, 0]), allow);
        for fd in [1, 2, 3] {
            assert_eq!(
                call(number, [fd, 0, 0, 0, 0, 0]),
                deny,
                "call {number} fd {fd}"
            );
        }
    }
    // write and writev: standard output and error only.
    for number in [1, 20] {
        for fd in [1, 2] {
            assert_eq!(call(number, [fd, 0, 0, 0, 0, 0]), allow);
        }
        for fd in [0, 3, 1024] {
            assert_eq!(
                call(number, [fd, 0, 0, 0, 0, 0]),
                deny,
                "call {number} fd {fd}"
            );
        }
    }
    // mmap and mprotect: never executable memory.
    for number in [9, 10] {
        assert_eq!(call(number, [0, 4096, 3, 0x22, u64::MAX, 0]), allow);
        for protection in [4, 5, 7] {
            assert_eq!(call(number, [0, 4096, protection, 0, 0, 0]), deny);
        }
    }
    // tgkill: to itself only.
    assert_eq!(call(234, [pid.into(), pid.into(), 6, 0, 0, 0]), allow);
    assert_eq!(call(234, [1, 1, 9, 0, 0, 0]), deny);
    // Files, sockets, new processes and programs, signals to others, tracing, timers,
    // io_uring, BPF, prctl, ioctl and further sandbox changes all fail with EPERM.
    for number in [
        2, 257, 437, 41, 42, 56, 57, 58, 59, 322, 435, 62, 200, 101, 310, 319, 425, 321, 157, 16,
        445, 317, 110, 35, 230,
    ] {
        assert_eq!(call(number, none), deny, "call {number}");
    }
}

#[test]
fn seccomp_filter_restricts_mapping_futex_and_clock_arguments() {
    let program = lockdown::filter(4242);
    let call = |number, args| verdict(&program, lockdown::AUDIT_ARCH_X86_64, number, args);
    // Private anonymous mappings only; MAP_SHARED_VALIDATE also contains MAP_SHARED.
    for flags in [0, 1, 2, 3, 0x20, 0x21, 0x23, 0x122, 0x123] {
        assert_eq!(
            call(9, [0, 4096, 3, flags, u64::MAX, 0]),
            lockdown::DENY,
            "flags {flags:#x}"
        );
    }
    for flags in [0x22, 0x4022, 0x20022] {
        assert_eq!(call(9, [0, 4096, 3, flags, u64::MAX, 0]), lockdown::ALLOW);
    }
    // Only process-private wait, wake and timed wait-bitset; no shared, PI, requeue
    // or wake-op variants. Upper bits of int arguments are ignored by Linux too.
    for op in 0..1024 {
        let expected = if matches!(op, 128 | 129 | 137) {
            lockdown::ALLOW
        } else {
            lockdown::DENY
        };
        for high in [0, 0xffff_ffff_0000_0000] {
            assert_eq!(
                call(202, [0, op | high, 0, 0, 0, 0]),
                expected,
                "futex op {op}"
            );
        }
    }
    for clock in (0..32).chain([u32::MAX, u32::MAX - 5, 0x8000_0000]) {
        let expected = if clock <= 7 {
            lockdown::ALLOW
        } else {
            lockdown::DENY
        };
        for high in [0, 0xffff_ffff_0000_0000] {
            assert_eq!(
                call(228, [u64::from(clock) | high, 0, 0, 0, 0, 0]),
                expected,
                "clock {clock}"
            );
        }
    }
}

#[test]
fn native_memory_and_syscall_guards_reject_bypasses() {
    for (input, expected) in [
        ("memory", "shared=1;growsdown=1;reserve=12;remap=12"),
        (
            "calls",
            "shared_futex=1;requeue=1;pi=1;private_wait=11;private_wake=0;foreign_clock=1;monotonic=0",
        ),
    ] {
        let output = fixture()
            .run(
                input.as_bytes().to_vec(),
                &limits(),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
    }
}

// Fully feed a real child and wait for its post-lockdown READY. This rules out death
// from EOF or startup failure. A failed assertion drops Process and reaps the child.
fn waiting_probe() -> Process {
    let mut file = protected_worker(&fixture().path).unwrap();
    let sealed = sealed_copy(&mut file).unwrap();
    let (process, mut pipes) = spawn(&sealed).unwrap();
    pipes.stdin.write_all(b"wait").unwrap();
    drop(pipes.stdin);
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut ready = String::new();
        let result = BufReader::new(pipes.stdout)
            .read_line(&mut ready)
            .map(|_| ready);
        let _ = send.send(result);
    });
    assert_eq!(
        receive
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap(),
        "READY\n"
    );
    reader.join().unwrap();
    process
}

#[test]
fn native_worker_has_bounded_address_stack_locked_memory_and_signal_quota() {
    let process = waiting_probe();
    let limits = fs::read_to_string(format!("/proc/{}/limits", process.child.id())).unwrap();
    for (name, maximum) in [
        ("Max address space", 128 * 1024 * 1024),
        ("Max data size", 128 * 1024 * 1024),
        ("Max stack size", 2 * 1024 * 1024),
        ("Max locked memory", 0),
        ("Max pending signals", 32),
    ] {
        let row = limits.lines().find_map(|s| s.strip_prefix(name)).unwrap();
        let values: Vec<_> = row.split_whitespace().take(2).collect();
        assert_eq!(values.len(), 2, "{name}");
        for value in values {
            assert!(
                value.parse::<u64>().is_ok_and(|n| n <= maximum),
                "{name}: {row}"
            );
        }
    }
    let status = fs::read_to_string(format!("/proc/{}/status", process.child.id())).unwrap();
    for name in ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"] {
        let value = status.lines().find_map(|s| s.strip_prefix(name)).unwrap();
        assert_eq!(u64::from_str_radix(value.trim(), 16).unwrap(), 0, "{name}");
    }
}

#[test]
fn native_worker_drops_capabilities_before_exec_without_changing_the_parent() {
    let capabilities = || -> Vec<String> {
        fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .filter(|line| {
                ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            })
            .map(str::to_owned)
            .collect()
    };
    let before = capabilities();
    assert_eq!(before.len(), 4);
    let output = fixture()
        .run(
            b"capabilities".to_vec(),
            &limits(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"inherited_capabilities=0\n");
    assert_eq!(
        capabilities(),
        before,
        "only the child may lose capabilities"
    );
}

#[test]
fn landlock_rulesets_name_only_what_each_abi_knows() {
    let expect = |abi, files, network, scoped, size| {
        assert_eq!(
            lockdown::ruleset(abi),
            (
                lockdown::Ruleset {
                    handled_access_fs: files,
                    handled_access_net: network,
                    scoped,
                },
                size
            ),
            "ABI {abi}"
        );
    };
    expect(1, 0x1fff, 0, 0, 8);
    expect(2, 0x3fff, 0, 0, 8);
    expect(3, 0x7fff, 0, 0, 8);
    expect(4, 0x7fff, 3, 0, 16);
    expect(5, 0xffff, 3, 0, 16);
    expect(6, 0xffff, 3, 3, 24);
    expect(7, 0xffff, 3, 3, 24);
    expect(8, 0xffff, 3, 3, 24);
}

#[test]
#[ignore = "invoked only by the parent-exit regression with an owned fixture"]
fn parent_exit_helper() {
    let process = waiting_probe();
    println!("WORKER:{}", process.child.id());
    std::io::stdout().flush().unwrap();
    // The child is now spinning, independent of closed pipes, with seconds of CPU
    // time left. Exit deliberately bypasses Process::drop and its normal cleanup.
    std::process::exit(0);
}

#[test]
fn abrupt_parent_exit_kills_the_worker() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "integration_worker::linux::tests::parent_exit_helper",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let pid: u32 = text
        .lines()
        .find_map(|line| line.strip_prefix("WORKER:"))
        .expect("worker started")
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while running(pid) {
        if Instant::now() >= deadline {
            // Clean up our owned orphan even when a regression removes PDEATHSIG.
            unsafe { kill(pid as c_int, SIGKILL) };
            panic!("orphan worker survived its parent for over one second");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether `pid` is a live process. A zombie waiting to be collected has ended.
fn running(pid: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let state = stat.rsplit(')').next().map(str::trim_start);
    !matches!(state.and_then(|s| s.chars().next()), Some('Z' | 'X'))
}
