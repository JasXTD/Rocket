//! Read-only Darwin ACL verification for private storage. No existing ACL is edited.
use std::{
    ffi::{c_int, c_uint, c_void},
    fs, io,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
};

// Apple Libc include/sys/acl.h and posix1e/acl_{file,entry}.c. Darwin's iterator
// returns 0 for an entry and -1/EINVAL at the end (unlike POSIX/Linux libacl).
unsafe extern "C" {
    fn acl_get_fd_np(fd: c_int, kind: c_uint) -> *mut c_void;
    fn acl_valid(acl: *mut c_void) -> c_int;
    fn acl_get_entry(acl: *mut c_void, which: c_int, entry: *mut *mut c_void) -> c_int;
    fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_uint) -> c_int;
    fn acl_free(acl: *mut c_void) -> c_int;
}
struct Acl(*mut c_void);
impl Drop for Acl {
    fn drop(&mut self) {
        // SAFETY: this guard owns the allocation returned by acl_get_fd_np.
        unsafe {
            acl_free(self.0);
        }
    }
}

pub(super) fn check(file: &fs::File, path: &Path, expected: &fs::Metadata) -> io::Result<()> {
    let stable = |now: &fs::Metadata| {
        now.dev() == expected.dev()
            && now.ino() == expected.ino()
            && now.uid() == expected.uid()
            && now.mode() == expected.mode()
            && now.ctime() == expected.ctime()
            && now.ctime_nsec() == expected.ctime_nsec()
    };
    if !stable(&file.metadata()?) {
        return Err(changed());
    }
    // SAFETY: live open descriptor; ACL_TYPE_EXTENDED is 0x100 on Darwin.
    let raw = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
    let result = if raw.is_null() {
        let error = io::Error::last_os_error();
        // Darwin Libc's descriptor-bound getter reports ENOENT for an absent
        // FILESEC_ACL property. The descriptor, unlike a pathname, cannot vanish
        // during lookup. Recheck its metadata below; other failures stay errors.
        if error.raw_os_error() == Some(2) {
            Ok(())
        } else {
            Err(error)
        }
    } else {
        inspect(&Acl(raw), path)
    };
    if !stable(&file.metadata()?) {
        return Err(changed());
    }
    result
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Storage permissions changed during verification; retry",
    )
}

fn inspect(acl: &Acl, path: &Path) -> io::Result<()> {
    // SAFETY: the allocated ACL remains owned for this call and all iteration.
    if unsafe { acl_valid(acl.0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    for index in 0..=128 {
        let mut entry = std::ptr::null_mut();
        // SAFETY: validated live ACL, FIRST=0/NEXT=-1, initialized out pointer.
        if unsafe { acl_get_entry(acl.0, if index == 0 { 0 } else { -1 }, &mut entry) } != 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(22) {
                Ok(())
            } else {
                Err(error)
            };
        }
        if entry.is_null() {
            return Err(changed());
        }
        let mut tag = 0;
        // SAFETY: entry belongs to the still-live ACL; tag is writable.
        if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if tag != 2 {
            // ACL_EXTENDED_DENY; reject allow and unknown entry kinds.
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "Rocket private storage refuses ACL access grants on {}. Choose a private folder; existing permissions were preserved.",
                    path.display()
                ),
            ));
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Storage ACL exceeds the Darwin entry limit",
    ))
}
