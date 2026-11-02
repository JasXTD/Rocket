//! Developer smoke probe: source locations, locals and backtraces under Cargo's dev profile.
#[inline(never)]
fn checkpoint(value: usize) {
    let path = rocket_git::RepoPath::new("debug/文件.txt".as_bytes().to_vec()).unwrap();
    std::hint::black_box((&path, value));
    assert!(
        std::hint::black_box(cfg!(debug_assertions)),
        "Use the development profile for this probe"
    );
    println!(
        "{}\n{}",
        path.display_lossy(),
        std::backtrace::Backtrace::force_capture()
    );
}
fn main() {
    checkpoint(42);
}
