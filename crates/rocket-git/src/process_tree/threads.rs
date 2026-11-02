//! Locate a suspended child's initial thread without scanning every system thread.
//! Windows 8.1+ Process Snapshotting is part of the OS; Rocket requires Windows 10.
use std::{
    ffi::c_void,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Child,
    ptr,
};

type Handle = *mut c_void;

// PSS_THREAD_ENTRY from processsnapshot.h. FILETIME is two DWORDs (alignment 4),
// not a u64; using u64 would move subsequent fields on some target architectures.
#[repr(C)]
#[derive(Default)]
struct ThreadEntry {
    exit_status: u32,
    teb: Handle,
    process_id: u32,
    thread_id: u32,
    affinity: usize,
    priority: i32,
    base_priority: i32,
    syscall_argument: Handle,
    syscall_number: u16,
    create_time: [u32; 2],
    exit_time: [u32; 2],
    kernel_time: [u32; 2],
    user_time: [u32; 2],
    start_address: Handle,
    capture_time: [u32; 2],
    flags: u32,
    suspend_count: u16,
    context_size: u16,
    context: Handle,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn PssCaptureSnapshot(process: Handle, flags: u32, context: u32, snapshot: *mut Handle) -> u32;
    fn PssFreeSnapshot(process: Handle, snapshot: Handle) -> u32;
    fn PssWalkMarkerCreate(allocator: *const c_void, marker: *mut Handle) -> u32;
    fn PssWalkMarkerFree(marker: Handle) -> u32;
    fn PssWalkSnapshot(
        snapshot: Handle,
        class: u32,
        marker: Handle,
        buffer: *mut c_void,
        length: u32,
    ) -> u32;
    fn GetCurrentProcess() -> Handle;
    fn OpenThread(access: u32, inherit: i32, id: u32) -> Handle;
    fn ResumeThread(thread: Handle) -> u32;
}

struct Snapshot(Handle);
impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: this snapshot was created in our process (not duplicated into
        // the child). It must be freed with our process handle, exactly once.
        unsafe {
            PssFreeSnapshot(GetCurrentProcess(), self.0);
        }
    }
}
struct Marker(Handle);
impl Drop for Marker {
    fn drop(&mut self) {
        // SAFETY: unique live walk marker; the default allocator was used.
        unsafe {
            PssWalkMarkerFree(self.0);
        }
    }
}

fn checked(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

pub(super) fn resume_primary_thread(child: &Child) -> io::Result<()> {
    // The child is still suspended and already assigned to its job. Capture only
    // its thread IDs: no address-space clone, memory, handles or CPU contexts.
    let mut snapshot = ptr::null_mut();
    // SAFETY: child owns the live process handle; output points to writable storage.
    checked(unsafe { PssCaptureSnapshot(child.as_raw_handle(), 0x80, 0, &mut snapshot) })?;
    let snapshot = Snapshot(snapshot);
    let mut marker = ptr::null_mut();
    // SAFETY: null selects the default allocator and marker is writable.
    checked(unsafe { PssWalkMarkerCreate(ptr::null(), &mut marker) })?;
    let marker = Marker(marker);
    loop {
        let mut entry = ThreadEntry::default();
        // SAFETY: both snapshot handles are live and the buffer has the SDK layout
        // and exact size for PSS_WALK_THREADS (3).
        let code = unsafe {
            PssWalkSnapshot(
                snapshot.0,
                3,
                marker.0,
                (&mut entry as *mut ThreadEntry).cast(),
                size_of::<ThreadEntry>() as u32,
            )
        };
        if code == 259 {
            // ERROR_NO_MORE_ITEMS
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "suspended Git thread not found",
            ));
        }
        checked(code)?;
        if entry.process_id != child.id() || entry.thread_id == 0 {
            continue;
        }
        // SAFETY: the suspended child's initial thread cannot exit/reuse its ID
        // before being resumed. Only THREAD_SUSPEND_RESUME access is requested.
        let handle = unsafe { OpenThread(2, 0, entry.thread_id) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenThread returned a new, uniquely owned handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        // SAFETY: the owned handle has suspend/resume rights.
        if unsafe { ResumeThread(handle.as_raw_handle()) } == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        return Ok(());
    }
}
