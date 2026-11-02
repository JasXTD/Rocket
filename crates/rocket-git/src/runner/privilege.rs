//! Git error adapter for the shared host/launcher privilege boundary.
pub(super) fn check() -> crate::Result<()> {
    crate::ensure_unprivileged_process()
}

#[cfg(all(windows, test))]
mod tests {
    use crate::runner::{ProcessTimings, run_process_tracked};
    use crate::{CancellationToken, CommandLimits, Error};
    use std::{
        process::Command,
        sync::atomic::{AtomicBool, Ordering},
    };
    #[test]
    fn elevated_unknown_and_high_integrity_tokens_refuse_before_process_start() {
        for state in [
            Err(()),
            Ok((true, 0x2000)),
            Ok((false, 0x3000)),
            Ok((true, 0x4000)),
        ] {
            crate::process_privilege::with_test_token(state, || {
                let started = AtomicBool::new(false);
                let mut timings = ProcessTimings::default();
                let result = run_process_tracked(
                    Command::new("rocket-must-not-start.exe"),
                    vec![],
                    &CommandLimits::default(),
                    &CancellationToken::new(),
                    None,
                    &mut timings,
                    Some(&started),
                    None,
                );
                assert!(matches!(result, Err(Error::InvalidInput(_))));
                assert!(!started.load(Ordering::Acquire));
                assert!(timings.prepare.is_none() && timings.spawn.is_none());
                let error = crate::process_tree::ProcessTree::prepare(&mut Command::new(
                    "rocket-must-not-start.exe",
                ))
                .err()
                .expect("direct launch preparation must refuse");
                assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
                let cancel = CancellationToken::new();
                cancel.cancel();
                assert!(matches!(
                    run_process_tracked(
                        Command::new("rocket-must-not-start.exe"),
                        vec![],
                        &CommandLimits::default(),
                        &cancel,
                        None,
                        &mut timings,
                        Some(&started),
                        None
                    ),
                    Err(Error::Cancelled)
                ));
            });
        }
    }
}
