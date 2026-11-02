use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Component,
};
pub(super) struct SecretBytes(pub Vec<u8>);
impl Drop for SecretBytes {
    fn drop(&mut self) {
        for b in &mut self.0 {
            unsafe { std::ptr::write_volatile(b, 0) }
        }
    }
}
pub(super) fn safe_path(p: &Path) -> Result<()> {
    let text = p.to_str().ok_or(fail(SshFailure::UnsafePath))?;
    if !p.is_absolute()
        || text
            .chars()
            .any(|c| c.is_control() || "\"'%$\x60".contains(c))
        || p.components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(fail(SshFailure::UnsafePath));
    }
    #[cfg(windows)]
    if !matches!(p.components().next(),Some(Component::Prefix(v)) if matches!(v.kind(),std::path::Prefix::Disk(_)|std::path::Prefix::VerbatimDisk(_)))
    {
        return Err(fail(SshFailure::UnsafePath));
    }
    Ok(())
}
pub(super) fn pin(p: &Path) -> Result<Vec<File>> {
    safe_path(p)?;
    let disk: PathBuf = p.components().take(2).collect();
    local_files::pin_directories(&disk, p).map_err(|_| fail(SshFailure::UnsafePath))
}
pub(super) fn locked(p: &Path) -> Result<File> {
    safe_path(p)?;
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(1).custom_flags(0x00200000);
    }
    let f = o.open(p).map_err(|e| Error::io("open SSH file", e))?;
    let m = f.metadata().map_err(|e| Error::io("inspect SSH file", e))?;
    if !m.is_file() || local_files::redirected(&m) {
        return Err(fail(SshFailure::UnsafePath));
    }
    Ok(f)
}
pub(super) fn read(p: &Path, limit: usize) -> Result<Vec<u8>> {
    let _parents = pin(p.parent().ok_or(fail(SshFailure::UnsafePath))?)?;
    let f = locked(p)?;
    let mut b = Vec::new();
    f.take(limit as u64 + 1)
        .read_to_end(&mut b)
        .map_err(|e| Error::io("read SSH file", e))?;
    if b.len() > limit {
        return Err(fail(SshFailure::KeyInvalid));
    }
    Ok(b)
}
pub(super) fn create_file(p: &Path, b: &[u8]) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .and_then(|mut f| {
            f.write_all(b)?;
            f.sync_all()
        })
        .map_err(|e| Error::io("create SSH profile file", e))
}
pub(super) fn ensure_private_parent(p: &Path) -> Result<()> {
    safe_path(p)?;
    match fs::symlink_metadata(p) {
        Ok(_) => {
            pin(p)?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = p.parent().ok_or(fail(SshFailure::UnsafePath))?;
            ensure_private_parent(parent)?;
            let _guard = pin(parent)?;
            create_private_dir(p)
        }
        Err(e) => Err(Error::io("inspect SSH directory", e)),
    }
}
#[cfg(windows)]
pub(super) fn create_private_dir(p: &Path) -> Result<()> {
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
    let io = || {
        Error::io(
            "create protected SSH directory",
            std::io::Error::last_os_error(),
        )
    };
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
    let name: Vec<_> = p.as_os_str().encode_wide().chain(Some(0)).collect();
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
#[cfg(not(windows))]
pub(super) fn create_private_dir(_: &Path) -> Result<()> {
    Err(fail(SshFailure::Unavailable))
}
pub(super) fn public_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".pub");
    PathBuf::from(s)
}
fn base64(s: &[u8]) -> Result<SecretBytes> {
    let mut out = Vec::new();
    let mut value = 0u32;
    let mut bits = 0;
    let mut padded = false;
    for c in s.iter().copied().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            padded = true;
            continue;
        }
        if padded {
            return Err(fail(SshFailure::KeyInvalid));
        }
        let n = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(fail(SshFailure::KeyInvalid)),
        };
        value = (value << 6) | u32::from(n);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((value >> bits) as u8)
        }
    }
    if bits >= 6 {
        return Err(fail(SshFailure::KeyInvalid));
    }
    Ok(SecretBytes(out))
}
fn string<'a>(b: &mut &'a [u8]) -> Result<&'a [u8]> {
    if b.len() < 4 {
        return Err(fail(SshFailure::KeyInvalid));
    }
    let n = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
    if n > b.len() - 4 {
        return Err(fail(SshFailure::KeyInvalid));
    }
    let result = &b[4..4 + n];
    *b = &b[4 + n..];
    Ok(result)
}
pub(super) fn key_pair(p: &Path) -> Result<(String, bool)> {
    safe_path(p)?;
    let bytes = SecretBytes(read(p, 1024 * 1024)?);
    // Locate the exact footer; support CRLF and LF without exposing the payload.
    let footer = b"-----END OPENSSH PRIVATE KEY-----";
    let body = bytes
        .0
        .strip_prefix(b"-----BEGIN OPENSSH PRIVATE KEY-----")
        .and_then(|b| {
            b.windows(footer.len())
                .position(|w| w == footer)
                .map(|n| &b[..n])
        })
        .ok_or(fail(SshFailure::KeyInvalid))?;
    let decoded = base64(body)?;
    let mut b = decoded
        .0
        .strip_prefix(b"openssh-key-v1\0")
        .ok_or(fail(SshFailure::KeyInvalid))?;
    let cipher = string(&mut b)?;
    let _kdf = string(&mut b)?;
    let _options = string(&mut b)?;
    if b.len() < 4 || b[..4] != [0, 0, 0, 1] {
        return Err(fail(SshFailure::KeyInvalid));
    }
    b = &b[4..];
    let public = string(&mut b)?;
    let pub_bytes = read(&public_path(p), 16384)?;
    let text = std::str::from_utf8(&pub_bytes).map_err(|_| fail(SshFailure::KeyInvalid))?;
    if text.lines().count() != 1 {
        return Err(fail(SshFailure::KeyInvalid));
    }
    let mut pieces = text.split_whitespace();
    let kind = pieces.next().ok_or(fail(SshFailure::KeyInvalid))?;
    if !matches!(
        kind,
        "ssh-ed25519"
            | "ssh-rsa"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
    ) {
        return Err(fail(SshFailure::KeyInvalid));
    }
    let data = pieces.next().ok_or(fail(SshFailure::KeyInvalid))?;
    let parsed = base64(data.as_bytes())?;
    if parsed.0 != public {
        return Err(fail(SshFailure::KeyInvalid));
    }
    let mut pub_slice = public;
    if string(&mut pub_slice)? != kind.as_bytes() {
        return Err(fail(SshFailure::KeyInvalid));
    }
    Ok((format!("{kind} {data}"), cipher != b"none"))
}
