//! One privilege boundary for Rocket's hosts and child-process launchers.
//! Kept std-only: the independently compiled terminal helper includes this same
//! source, and its build script watches it. No environment or release override.
use std::io;

pub(crate) fn check() -> io::Result<()> {
    #[cfg(windows)]
    {
        #[cfg(test)]
        if let Some(state) = TEST_STATE.with(std::cell::Cell::get) {
            return permit(state);
        }
        permit(windows::current().map_err(|_| ()))
    }
    #[cfg(unix)]
    {
        permit_user(unix::current())
    }
    #[cfg(not(any(windows, unix)))]
    Ok(())
}

/// Refuse root as Windows refuses an elevated token: a real or effective user ID of 0
/// (sudo, a root shell or a set-user-ID launch).
#[cfg(unix)]
fn permit_user((real, effective): (u32, u32)) -> io::Result<()> {
    if real == 0 || effective == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Rocket cannot run as root. Close Rocket and reopen it as your own user, without sudo, so commands, terminals and helpers cannot inherit root rights.",
        ));
    }
    Ok(())
}

#[cfg(unix)]
mod unix {
    unsafe extern "C" {
        fn getuid() -> u32;
        fn geteuid() -> u32;
    }
    /// The real and effective user IDs (uid_t is 32-bit on Linux and macOS).
    pub(super) fn current() -> (u32, u32) {
        // SAFETY: both take no arguments, always succeed and only return a value.
        unsafe { (getuid(), geteuid()) }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
struct State {
    elevated: bool,
    integrity: u32,
}

#[cfg(windows)]
fn permit(state: Result<State, ()>) -> io::Result<()> {
    let state = state.map_err(|()| io::Error::new(io::ErrorKind::PermissionDenied,
        "Rocket could not verify Windows process privileges. Close Rocket and reopen it normally."))?;
    if state.elevated || state.integrity >= 0x3000 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Rocket cannot run as administrator. Close Rocket and reopen it without Run as administrator so commands, terminals and helpers cannot inherit elevated rights.",
        ));
    }
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::State;
    use std::{ffi::c_void, io, mem, ptr};
    type Handle = *mut c_void;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            class: u32,
            output: Handle,
            length: u32,
            needed: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
    }
    struct Token(Handle);
    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: this is the unique handle returned by OpenProcessToken.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn invalid() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Windows token information",
        )
    }
    pub(super) fn current() -> io::Result<State> {
        let mut token = ptr::null_mut();
        // SAFETY: valid pseudo-process handle and writable output; TOKEN_QUERY only.
        if unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Token(token);
        let mut elevated = 0u32;
        let mut needed = 0;
        // TokenElevation (20) returns one DWORD, TOKEN_ELEVATION.TokenIsElevated.
        if unsafe {
            GetTokenInformation(
                token.0,
                20,
                (&mut elevated as *mut u32).cast(),
                4,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if needed != 4 || elevated > 1 {
            return Err(invalid());
        }
        // TokenIntegrityLevel (25) returns TOKEN_MANDATORY_LABEL plus its SID.
        if unsafe { GetTokenInformation(token.0, 25, ptr::null_mut(), 0, &mut needed) } != 0
            || io::Error::last_os_error().raw_os_error() != Some(122)
            || !(12..=4096).contains(&needed)
        {
            return Err(invalid());
        }
        let mut buffer = vec![0usize; (needed as usize).div_ceil(mem::size_of::<usize>())];
        let mut returned = 0;
        // SAFETY: aligned owned storage is at least needed bytes long and stays
        // alive while the returned in-buffer SID pointer is inspected below.
        if unsafe {
            GetTokenInformation(
                token.0,
                25,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut returned,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let integrity = integrity(&buffer, returned as usize).ok_or_else(invalid)?;
        Ok(State {
            elevated: elevated != 0,
            integrity,
        })
    }
    pub(super) fn integrity(buffer: &[usize], length: usize) -> Option<u32> {
        if length > mem::size_of_val(buffer) || length < 2 * mem::size_of::<usize>() {
            return None;
        }
        let offset = buffer.first()?.checked_sub(buffer.as_ptr() as usize)?;
        if offset < 2 * mem::size_of::<usize>() || offset.checked_add(12)? > length {
            return None;
        }
        // SAFETY: length is within initialized aligned storage. We convert the
        // SID pointer to a checked offset instead of dereferencing an unchecked pointer.
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), length) };
        let sid = &bytes[offset..offset + 12];
        // Integrity labels use S-1-16-<level>, with exactly one subauthority.
        if sid[..8] != [1, 1, 0, 0, 0, 0, 0, 16] {
            return None;
        }
        Some(u32::from_le_bytes(sid[8..12].try_into().ok()?))
    }
}

// Test-only, thread-local probe replacement. Never compiled into shipped hosts.
#[cfg(all(windows, test))]
thread_local! {
    static TEST_STATE: std::cell::Cell<Option<Result<State, ()>>> = const { std::cell::Cell::new(None) };
}
#[cfg(all(windows, test))]
pub(crate) fn with_test_token<T>(state: Result<(bool, u32), ()>, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Result<State, ()>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_STATE.set(self.0);
        }
    }
    let _restore = Restore(
        TEST_STATE.replace(Some(state.map(|(elevated, integrity)| State {
            elevated,
            integrity,
        }))),
    );
    run()
}
#[cfg(all(windows, test))]
mod tests {
    use super::*;
    #[test]
    fn elevated_high_and_unknown_tokens_fail_closed() {
        for state in [
            Err(()),
            Ok((true, 0x2000)),
            Ok((false, 0x3000)),
            Ok((true, 0x4000)),
        ] {
            with_test_token(state, || {
                assert_eq!(check().unwrap_err().kind(), io::ErrorKind::PermissionDenied)
            });
        }
    }
    #[test]
    fn normal_filtered_and_low_integrity_tokens_remain_allowed() {
        for integrity in [0, 0x1000, 0x2000, 0x2100] {
            permit(Ok(State {
                elevated: false,
                integrity,
            }))
            .unwrap();
        }
        let actual = windows::current().expect("query the actual Windows token");
        println!(
            "Current process: elevated={}, integrity={:#x}",
            actual.elevated, actual.integrity
        );
        assert_eq!(
            check().is_ok(),
            !actual.elevated && actual.integrity < 0x3000
        );
    }

    #[test]
    fn integrity_sid_offsets_and_shape_are_checked() {
        let mut buffer = vec![0usize; 8];
        let offset = 2 * std::mem::size_of::<usize>();
        buffer[0] = buffer.as_ptr() as usize + offset;
        // SAFETY: the initialized buffer covers the complete synthetic label.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                buffer.as_mut_ptr().cast::<u8>(),
                8 * std::mem::size_of::<usize>(),
            )
        };
        bytes[offset..offset + 12].copy_from_slice(&[1, 1, 0, 0, 0, 0, 0, 16, 0, 0x30, 0, 0]);
        assert_eq!(windows::integrity(&buffer, offset + 12), Some(0x3000));
        assert_eq!(windows::integrity(&buffer, offset + 11), None);
        assert_eq!(windows::integrity(&buffer, usize::MAX), None);
        buffer[0] = 0;
        assert_eq!(windows::integrity(&buffer, offset + 12), None);
    }
}
#[cfg(all(unix, test))]
mod unix_tests {
    use super::*;
    #[test]
    fn root_real_or_effective_users_fail_closed() {
        for ids in [(0, 0), (0, 1000), (1000, 0)] {
            assert_eq!(
                permit_user(ids).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        permit_user((1000, 1000)).unwrap();
        let (real, effective) = unix::current();
        println!("Current process: uid={real}, euid={effective}");
        assert_eq!(check().is_ok(), real != 0 && effective != 0);
    }
}
