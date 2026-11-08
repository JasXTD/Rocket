//! Current-user protection: DPAPI, Linux Secret Service, or macOS Keychain.
#[cfg(not(target_os = "macos"))]
use crate::Error;
use crate::Result;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
mod macos_policy;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod sealed;
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
const LIMIT: usize = 512 * 1024;
#[cfg(windows)]
fn refused() -> Error {
    Error::InvalidInput("Hosting credentials could not be protected or decrypted for this Windows user. Existing data was preserved.".into())
}
#[cfg(target_os = "linux")]
pub fn protect_hosting_secret(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    // The same bounds as DPAPI's: nothing empty, nothing over 512 KiB (plus the seal).
    if bytes.is_empty() || bytes.len() > LIMIT + if encrypt { 0 } else { 64 } {
        return Err(Error::InvalidInput(
            "Hosting credentials could not be protected or decrypted: the data is empty or too large. Existing data was preserved.".into(),
        ));
    }
    linux::protect(bytes, encrypt)
}
#[cfg(target_os = "macos")]
pub fn protect_hosting_secret(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    macos::protect(bytes, encrypt)
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn protect_hosting_secret(_: &[u8], _: bool) -> Result<Vec<u8>> {
    Err(Error::Unsupported(
        "Remembering credentials currently requires Windows, Linux or macOS.".into(),
    ))
}
#[cfg(windows)]
pub fn protect_hosting_secret(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use std::ffi::c_void;
    #[repr(C)]
    struct Blob {
        len: u32,
        data: *mut u8,
    }
    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }
    if bytes.is_empty() || bytes.len() > LIMIT {
        return Err(refused());
    }
    let input = Blob {
        len: bytes.len() as u32,
        data: bytes.as_ptr() as *mut u8,
    };
    let mut output = Blob {
        len: 0,
        data: std::ptr::null_mut(),
    };
    // Current user only: never CRYPTPROTECT_LOCAL_MACHINE. No prompt from a worker.
    // Input is borrowed read-only per the Windows contract; output is LocalFree-owned.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                1,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                1,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(refused());
    }
    if output.data.is_null() || output.len as usize > LIMIT {
        if !output.data.is_null() {
            unsafe {
                LocalFree(output.data.cast());
            }
        }
        return Err(refused());
    }
    let result = unsafe { std::slice::from_raw_parts(output.data, output.len as usize) }.to_vec();
    // Clear the native plaintext allocation before returning it to Windows.
    if !encrypt {
        for i in 0..output.len as usize {
            unsafe {
                std::ptr::write_volatile(output.data.add(i), 0);
            }
        }
    }
    unsafe {
        LocalFree(output.data.cast());
    }
    Ok(result)
}
