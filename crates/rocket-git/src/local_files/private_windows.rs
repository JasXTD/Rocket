//! Atomically create a directory with an inheritable, protected DACL for the user, SYSTEM and administrators.
pub(crate) fn create(path: &std::path::Path) -> std::io::Result<()> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};
    type Handle = *mut c_void;
    #[repr(C)]
    struct Sa {
        len: u32,
        descriptor: Handle,
        inherit: i32,
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(p: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(t: Handle, class: u32, out: Handle, n: u32, needed: *mut u32)
        -> i32;
        fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            s: *const u16,
            revision: u32,
            out: *mut Handle,
            len: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(h: Handle) -> i32;
        fn LocalFree(h: Handle) -> Handle;
        fn CreateDirectoryW(name: *const u16, sa: *const Sa) -> i32;
    }
    struct Owned(Handle, bool);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                if self.1 {
                    LocalFree(self.0);
                } else {
                    CloseHandle(self.0);
                }
            }
        }
    }
    let io = std::io::Error::last_os_error;
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut token) } == 0 {
        return Err(io());
    }
    let token = Owned(token, false);
    let mut n = 0;
    unsafe {
        GetTokenInformation(token.0, 1, ptr::null_mut(), 0, &mut n);
    }
    if n == 0 || n > 65536 {
        return Err(io());
    }
    let mut b = vec![0usize; (n as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token.0, 1, b.as_mut_ptr().cast(), n, &mut n) } == 0 {
        return Err(io());
    }
    // TOKEN_USER starts with a SID pointer. Aligned storage remains live below.
    let sid = unsafe { *(b.as_ptr().cast::<Handle>()) };
    let mut s = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut s) } == 0 {
        return Err(io());
    }
    let _sid = Owned(s.cast(), true);
    let mut len = 0;
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    let user = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(s, len) });
    let sddl: Vec<_> = format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io());
    }
    let _descriptor = Owned(descriptor, true);
    let name: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let sa = Sa {
        len: std::mem::size_of::<Sa>() as u32,
        descriptor,
        inherit: 0,
    };
    if unsafe { CreateDirectoryW(name.as_ptr(), &sa) } == 0 {
        return Err(io());
    }
    Ok(())
}
