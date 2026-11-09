//! Shared test-only roots. Unix fixtures need native permissions and link semantics
//! even when WSL compiles source from a Windows mount.
use std::path::PathBuf;

pub fn base() -> PathBuf {
    let base = std::env::var_os("ROCKET_TEST_REPOSITORIES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(unix)]
            {
                std::env::temp_dir()
                    .join(format!("rocket-test-repositories-{}", std::process::id()))
            }
            #[cfg(not(unix))]
            {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .ancestors()
                    .nth(2)
                    .expect("crate is inside the workspace")
                    .join("work/test-repositories")
            }
        });
    assert!(base.is_absolute(), "test repository root must be absolute");
    std::fs::create_dir_all(&base).expect("create test repository root");
    #[cfg(unix)]
    {
        // In particular, resolve macOS's /var and /tmp aliases before the private
        // storage guard checks each component. Do not relax that production guard.
        base.canonicalize().expect("resolve test repository root")
    }
    #[cfg(not(unix))]
    {
        // Git config environment values do not accept Windows verbatim prefixes.
        base
    }
}
