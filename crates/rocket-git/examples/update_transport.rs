//! Loopback-only focused update transport probe; never installs downloaded code.
use rocket_git::{
    CancellationToken,
    app_update::{UpdateCheck, UpdateClient, UpdateEndpoint},
};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3);
    let port: u16 = args[1].parse().unwrap();
    let directory = std::path::PathBuf::from(&args[2]);
    let cancel = CancellationToken::new();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let (done, cancel, stop) = (done.clone(), cancel.clone(), directory.join("cancel"));
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Acquire) {
                if stop.exists() {
                    cancel.cancel();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
    };
    let result = (|| {
        let client = UpdateClient::new(UpdateEndpoint::Loopback(port), None)?;
        let release = match client.check("0.1.0", &directory, &cancel)? {
            UpdateCheck::Available(release) => release,
            _ => {
                return Err(rocket_git::Error::InvalidInput(
                    "Expected a fixture update".into(),
                ));
            }
        };
        let archive = client.download(&release, &directory, &cancel)?;
        client.extract(&archive, &directory.join("bundle"), &cancel)
    })();
    done.store(true, std::sync::atomic::Ordering::Release);
    watcher.join().unwrap();
    match result {
        Ok(()) => println!("PASS downloaded and extracted verified fixture"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
