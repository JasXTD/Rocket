// First-party disposable executable used only by updater transaction tests.
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    #[cfg(windows)]
    if let Some(pair) = args.windows(2).find(|a| a[0] == "--hold-directory") {
        use std::os::windows::fs::OpenOptionsExt;
        let _handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x02000000)
            .open(&pair[1])
            .unwrap();
        println!("ready");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        return;
    }
    if let Some(pair) = args.windows(2).find(|a| a[0] == "--rocket-update-result") {
        let job = std::path::PathBuf::from(&pair[1]);
        let receipt =
            std::fs::read_to_string(job.join("result.json")).expect("receipt precedes restart");
        std::fs::write(
            job.join("restarted"),
            format!("{}\n{}", env!("ROCKET_UPDATE_PROBE_VERSION"), receipt),
        )
        .unwrap();
    }
}
