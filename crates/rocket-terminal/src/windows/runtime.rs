//! ABI from the reviewed Microsoft.Windows.Console.ConPTY package's conpty.h.
//! Load only the versioned runtime beside this executable, never from PATH/CWD.
use super::{Coord, Handle, wide};
use std::{ffi::c_void, io, path::Path, ptr};

type Create = unsafe extern "system" fn(Coord, Handle, Handle, u32, *mut Handle) -> i32;
type Resize = unsafe extern "system" fn(Handle, Coord) -> i32;
type Close = unsafe extern "system" fn(Handle);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(path: *const u16, file: Handle, flags: u32) -> Handle;
    fn FreeLibrary(module: Handle) -> i32;
    fn GetProcAddress(module: Handle, name: *const u8) -> *mut c_void;
}
struct Module(Handle);
impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: one successful load, released after the owned console closes.
        unsafe {
            FreeLibrary(self.0);
        }
    }
}
pub(super) struct Runtime {
    _module: Module,
    pub create: Create,
    pub resize: Resize,
    pub close: Close,
}
impl Runtime {
    pub fn load() -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        let directory = executable
            .parent()
            .ok_or_else(|| io::Error::other("Missing executable directory"))?
            .join(env!("ROCKET_CONPTY_DIRECTORY"));
        Self::from_directory(&directory).map_err(|error| io::Error::other(format!(
            "Bundled terminal runtime unavailable: {error}. Rebuild Rocket or restore {} beside the executable.",
            env!("ROCKET_CONPTY_DIRECTORY")
        )))
    }
    fn from_directory(directory: &Path) -> io::Result<Self> {
        if !directory.is_absolute() || !directory.join("OpenConsole.exe").is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "OpenConsole.exe is missing",
            ));
        }
        let path = wide(directory.join("conpty.dll").as_os_str())?;
        // SAFETY: absolute NUL-terminated path; resolve dependent DLLs only from
        // this package directory and System32 (LOAD_LIBRARY_SEARCH_* flags).
        let module = unsafe { LoadLibraryExW(path.as_ptr(), ptr::null_mut(), 0x100 | 0x800) };
        if module.is_null() {
            return Err(io::Error::last_os_error());
        }
        let module = Module(module);
        let address = |name: &'static [u8]| -> io::Result<*mut c_void> {
            // SAFETY: module stays loaded; every caller supplies a static C name.
            let address = unsafe { GetProcAddress(module.0, name.as_ptr()) };
            if address.is_null() {
                Err(io::Error::other("ConPTY export is missing"))
            } else {
                Ok(address)
            }
        };
        // SAFETY: exact named exports and system ABIs in the pinned native
        // header. Module ownership outlives every invocation of these pointers.
        let create = unsafe {
            std::mem::transmute::<*mut c_void, Create>(address(b"ConptyCreatePseudoConsole\0")?)
        };
        let resize = unsafe {
            std::mem::transmute::<*mut c_void, Resize>(address(b"ConptyResizePseudoConsole\0")?)
        };
        let close = unsafe {
            std::mem::transmute::<*mut c_void, Close>(address(b"ConptyClosePseudoConsole\0")?)
        };
        Ok(Self {
            _module: module,
            create,
            resize,
            close,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_runtime_is_an_error_without_path_or_system_fallback() {
        let missing = std::env::current_exe().unwrap().join("not-a-directory");
        assert!(Runtime::from_directory(&missing).is_err());
        assert!(Runtime::from_directory(Path::new(".")).is_err());
    }
}
