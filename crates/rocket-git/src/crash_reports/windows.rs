use super::*;
use std::{ffi::c_void, ptr, sync::atomic::AtomicPtr};
#[repr(C)]
struct ModuleInfo {
    base: *mut c_void,
    size: u32,
    entry: *mut c_void,
}
#[derive(Clone, Copy, Default)]
struct Range {
    base: usize,
    size: usize,
}
pub(super) struct Modules([Range; 6]);
#[repr(C)]
struct ExceptionRecord {
    code: u32,
    flags: u32,
    record: *const ExceptionRecord,
    address: *const c_void,
    count: u32,
    information: [usize; 15],
}
#[repr(C)]
struct ExceptionPointers {
    record: *const ExceptionRecord,
    context: *const c_void,
}
type Filter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;
static PREVIOUS: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetCurrentThreadId() -> u32;
    fn K32GetModuleInformation(
        process: *mut c_void,
        module: *mut c_void,
        info: *mut ModuleInfo,
        size: u32,
    ) -> i32;
    fn RtlCaptureStackBackTrace(
        skip: u32,
        count: u32,
        frames: *mut *mut c_void,
        hash: *mut u32,
    ) -> u16;
    fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
}
impl Modules {
    pub(super) fn new() -> Self {
        let mut ranges = [Range::default(); 6];
        for (i, name) in [
            "",
            "ntdll.dll",
            "kernel32.dll",
            "kernelbase.dll",
            "ucrtbase.dll",
            "opengl32.dll",
        ]
        .iter()
        .enumerate()
        {
            let wide: Vec<_> = name.encode_utf16().chain([0]).collect();
            let mut info = ModuleInfo {
                base: ptr::null_mut(),
                size: 0,
                entry: ptr::null_mut(),
            };
            // SAFETY: fixed trusted names, current process pseudo-handle, writable SDK structure.
            unsafe {
                let module = GetModuleHandleW(if i == 0 { ptr::null() } else { wide.as_ptr() });
                if !module.is_null()
                    && K32GetModuleInformation(
                        GetCurrentProcess(),
                        module,
                        &mut info,
                        std::mem::size_of::<ModuleInfo>() as u32,
                    ) != 0
                {
                    ranges[i] = Range {
                        base: info.base as usize,
                        size: info.size as usize,
                    };
                }
            }
        }
        Self(ranges)
    }
    fn frame(&self, address: usize) -> Frame {
        for (i, range) in self.0.iter().enumerate() {
            if address >= range.base && address - range.base < range.size {
                return Frame {
                    module: (i + 1) as u8,
                    offset: (address - range.base) as u64,
                };
            }
        }
        Frame::default()
    }
    pub(super) fn stack(&self) -> ([Frame; MAX_FRAMES], usize) {
        let mut addresses = [ptr::null_mut(); MAX_FRAMES];
        // SAFETY: caller-owned output array is exactly count entries; no symbol loader.
        let count = unsafe {
            RtlCaptureStackBackTrace(
                2,
                MAX_FRAMES as u32,
                addresses.as_mut_ptr(),
                ptr::null_mut(),
            )
        } as usize;
        let mut frames = [Frame::default(); MAX_FRAMES];
        for (i, address) in addresses.iter().take(count.min(MAX_FRAMES)).enumerate() {
            frames[i] = self.frame(*address as usize);
        }
        (frames, count.min(MAX_FRAMES))
    }
}
pub(super) fn thread_id() -> u32 {
    unsafe { GetCurrentThreadId() }
}
unsafe extern "system" fn unhandled(pointers: *mut ExceptionPointers) -> i32 {
    // SAFETY: Windows supplies exception pointers for this synchronous callback.
    // Read only the documented code/instruction fields, never fault data or memory.
    if let Some(runtime) = RUNTIME.get()
        && !pointers.is_null()
    {
        let record = unsafe { (*pointers).record };
        if !record.is_null() {
            let record = unsafe { &*record };
            runtime.record(
                Kind::WindowsException,
                None,
                record.code,
                runtime.modules.frame(record.address as usize),
            );
        }
    }
    let previous = PREVIOUS.load(Ordering::Acquire);
    if !previous.is_null() {
        // SAFETY: initialized only from SetUnhandledExceptionFilter's function pointer.
        let callback: Filter = unsafe { std::mem::transmute(previous) };
        unsafe { callback(pointers) }
    } else {
        0
    } // Continue normal Windows failure handling; never resume a faulting process.
}
pub(super) fn install() {
    // SAFETY: callback and its process-wide immutable state live until process exit.
    let previous = unsafe { SetUnhandledExceptionFilter(Some(unhandled)) };
    if let Some(previous) = previous {
        PREVIOUS.store(previous as *mut c_void, Ordering::Release);
    }
}
