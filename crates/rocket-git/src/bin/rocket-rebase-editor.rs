#![forbid(unsafe_code)]
fn main() {
    if let Err(error) = rocket_git::ensure_unprivileged_process() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    if rocket_git::run_rebase_editor().is_err() {
        // Fixed output only: the plan, messages and repository paths stay private.
        eprintln!(
            "Rocket rebase editor refused an invalid or changed plan. Review the current rebase state."
        );
        std::process::exit(1);
    }
}
