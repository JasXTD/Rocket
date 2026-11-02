//! The volume's own change journal, so a launch can skip reading every runtime directory.
//! NTFS writes a record for each change below the verified root before the call that made
//! it returns, whichever way the directory was opened, which a directory change report
//! doesn't. A record for a runtime directory, or anything but a clean answer, sends the
//! launch back to the full read, which alone decides whether the runtime is acceptable.
use std::{
    collections::BTreeSet,
    ffi::c_void,
    fs::File,
    mem::MaybeUninit,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::Path,
    ptr::{from_mut, from_ref, null, null_mut},
    time::{Duration, Instant},
};

// HANDLE is pointer-sized. These opaque values never become Rust references.
type Handle = *mut c_void;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        device: Handle,
        code: u32,
        input: *const c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetFileInformationByHandle(file: Handle, info: *mut FileInformation) -> i32;
    fn GetVolumeInformationByHandleW(
        file: Handle,
        name: *mut u16,
        name_size: u32,
        serial: *mut u32,
        component: *mut u32,
        flags: *mut u32,
        system: *mut u16,
        system_size: u32,
    ) -> i32;
}

const FSCTL_QUERY_USN_JOURNAL: u32 = 0x0009_00F4;
const FSCTL_READ_UNPRIVILEGED_USN_JOURNAL: u32 = 0x0009_03AB;
const DRIVE_FIXED: u32 = 3;
/// Past this much of other programs' activity, a full read is the cheaper way to catch up.
const CATCH_UP_BYTES: usize = 16 * 1024 * 1024;
/// Bounds anything the journal never shows, whatever hid it.
pub(super) const FULL_READ_EVERY: Duration = Duration::from_secs(2 * 60);

#[repr(C)]
struct FileInformation {
    attributes: u32,
    creation: [u32; 2],
    access: [u32; 2],
    write: [u32; 2],
    volume: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}
// USN_JOURNAL_DATA_V2.
#[repr(C)]
struct JournalData {
    id: u64,
    first: i64,
    next: i64,
    rest: [u64; 9],
}
// READ_USN_JOURNAL_DATA_V0, which is answered in USN_RECORD_V2.
#[repr(C)]
struct ReadRequest {
    start: i64,
    reasons: u32,
    only_on_close: u32,
    timeout: u64,
    bytes_to_wait_for: u64,
    journal: u64,
}
#[repr(C, align(8))]
struct Records([u8; 64 * 1024]);

/// Default has no journal to go on: every launch reads the directories.
#[derive(Default)]
pub(super) struct ChangeJournal(Option<Position>);

struct Position {
    // Any handle on the volume reads its journal.
    volume: File,
    journal: u64,
    next: i64,
    directories: BTreeSet<u64>,
    records: Box<Records>,
    catch_up_bytes: usize,
    changes: u64,
    read_through: u64,
    last_read: Instant,
}

impl ChangeJournal {
    /// Call before the first read of the runtime: this is where its records begin.
    pub(super) fn start(path: &Path, root: &File) -> Self {
        Self(
            fixed_ntfs(path, root)
                .then(|| Position::start(root))
                .flatten(),
        )
    }

    /// Records name directories by ID. Call before the first `stale`.
    pub(super) fn cover(&mut self, directories: &[File]) {
        let found: Option<BTreeSet<u64>> = directories.iter().map(reference).collect();
        match (found, &mut self.0) {
            (Some(found), Some(position)) => position.directories = found,
            _ => self.0 = None,
        }
    }

    /// None while nothing has changed since the last complete read. Otherwise read
    /// everything, then pass this value to `read`. A newer change stays outstanding.
    pub(super) fn stale(&mut self) -> Option<u64> {
        match &mut self.0 {
            Some(position) => position.stale(),
            None => Some(0),
        }
    }

    pub(super) fn read(&mut self, through: u64) {
        if let Some(position) = &mut self.0 {
            position.read_through = position.read_through.max(through);
            position.last_read = Instant::now();
        }
    }

    #[cfg(test)]
    pub(super) fn is_live(&self) -> bool {
        self.0.is_some()
    }

    #[cfg(test)]
    pub(super) fn age(&mut self, by: Duration) {
        let position = self.0.as_mut().unwrap();
        position.last_read = position.last_read.checked_sub(by).unwrap();
    }

    #[cfg(test)]
    pub(super) fn lose_position(&mut self) {
        self.0.as_mut().unwrap().journal ^= 1;
    }

    #[cfg(test)]
    pub(super) fn limit_catch_up(&mut self, bytes: usize) {
        self.0.as_mut().unwrap().catch_up_bytes = bytes;
    }
}

impl Position {
    fn start(root: &File) -> Option<Self> {
        let volume = root.try_clone().ok()?;
        let (journal, next) = locate(&volume)?;
        Some(Self {
            volume,
            journal,
            next,
            directories: BTreeSet::new(),
            records: Box::new(Records([0; 64 * 1024])),
            catch_up_bytes: CATCH_UP_BYTES,
            changes: 0,
            read_through: 0,
            last_read: Instant::now(),
        })
    }

    fn stale(&mut self) -> Option<u64> {
        let quiet = if self.last_read.elapsed() < FULL_READ_EVERY {
            self.quiet()
        } else {
            // The read this leads to covers the backlog, so don't go through it.
            self.resume();
            false
        };
        // This counts skipped records too: a read begun earlier can't answer for them.
        if !quiet {
            self.changes += 1;
        }
        (!quiet || self.read_through != self.changes).then_some(self.changes)
    }

    /// Takes every record up to now. True only if the journal answered cleanly and none
    /// of them concerns the runtime.
    fn quiet(&mut self) -> bool {
        let mut taken = 0usize;
        loop {
            let request = ReadRequest {
                start: self.next,
                reasons: u32::MAX,
                only_on_close: 0,
                timeout: 0,
                bytes_to_wait_for: 0,
                journal: self.journal,
            };
            let mut returned = 0u32;
            // SAFETY: a live synchronous handle, and input and output buffers that
            // outlive the call at their stated sizes.
            let ok = unsafe {
                DeviceIoControl(
                    self.volume.as_raw_handle(),
                    FSCTL_READ_UNPRIVILEGED_USN_JOURNAL,
                    from_ref(&request).cast(),
                    size_of::<ReadRequest>() as u32,
                    self.records.0.as_mut_ptr().cast(),
                    size_of::<Records>() as u32,
                    &mut returned,
                    null_mut(),
                )
            };
            // An answer starts with the position after its records.
            let answer = self.records.0.get(..returned as usize).filter(|_| ok != 0);
            let Some((next, records)) = answer.and_then(|a| a.split_first_chunk::<8>()) else {
                // Wrapped, deleted or never readable: this position is no use now.
                self.resume();
                return false;
            };
            if records.is_empty() {
                return true;
            }
            let found = names_directory(records, &self.directories);
            self.next = i64::from_ne_bytes(*next);
            if found {
                return false;
            }
            taken += records.len();
            if taken > self.catch_up_bytes {
                self.resume();
                return false;
            }
        }
    }

    /// Moves on to the journal's end. The read that follows covers what came before.
    fn resume(&mut self) {
        if let Some((journal, next)) = locate(&self.volume) {
            self.journal = journal;
            self.next = next;
        }
    }
}

/// A record that can't be understood counts as one for the runtime.
pub(super) fn names_directory(mut records: &[u8], directories: &BTreeSet<u64>) -> bool {
    while !records.is_empty() {
        // USN_RECORD_V2 starts with its length, version, file and parent.
        let Some(header) = records.first_chunk::<24>() else {
            return true;
        };
        let length = u32::from_ne_bytes(header[0..4].try_into().unwrap()) as usize;
        let version = u16::from_ne_bytes(header[4..6].try_into().unwrap());
        let parent = u64::from_ne_bytes(header[16..24].try_into().unwrap());
        if version != 2 || !(24..=records.len()).contains(&length) {
            return true;
        }
        if directories.contains(&parent) {
            return true;
        }
        records = &records[length..];
    }
    false
}

fn locate(volume: &File) -> Option<(u64, i64)> {
    let mut data = JournalData {
        id: 0,
        first: 0,
        next: 0,
        rest: [0; 9],
    };
    let mut returned = 0u32;
    // SAFETY: a live synchronous handle, no input, and an output of the stated size.
    let ok = unsafe {
        DeviceIoControl(
            volume.as_raw_handle(),
            FSCTL_QUERY_USN_JOURNAL,
            null(),
            0,
            from_mut(&mut data).cast(),
            size_of::<JournalData>() as u32,
            &mut returned,
            null_mut(),
        )
    };
    (ok != 0).then_some((data.id, data.next))
}

/// The file reference number that journal records use for this directory.
pub(super) fn reference(directory: &File) -> Option<u64> {
    let mut info = MaybeUninit::<FileInformation>::uninit();
    // SAFETY: File owns a live handle; Win32 initializes the repr(C) structure on success.
    if unsafe { GetFileInformationByHandle(directory.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return None;
    }
    // SAFETY: the call above succeeded.
    let info = unsafe { info.assume_init() };
    Some(u64::from(info.index_high) << 32 | u64::from(info.index_low))
}

/// Only where the journal's behaviour was measured. Other volumes keep reading.
fn fixed_ntfs(path: &Path, root: &File) -> bool {
    let Some(drive) = path.ancestors().last() else {
        return false;
    };
    let drive: Vec<u16> = drive.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut system = [0u16; 16];
    // SAFETY: a terminated path, a live handle and a buffer of the stated length. The
    // other outputs are optional.
    let (kind, named) = unsafe {
        (
            GetDriveTypeW(drive.as_ptr()),
            GetVolumeInformationByHandleW(
                root.as_raw_handle(),
                null_mut(),
                0,
                null_mut(),
                null_mut(),
                null_mut(),
                system.as_mut_ptr(),
                system.len() as u32,
            ),
        )
    };
    named != 0 && local_ntfs(kind, &system)
}

pub(super) fn local_ntfs(drive: u32, system: &[u16]) -> bool {
    drive == DRIVE_FIXED && String::from_utf16_lossy(system).trim_end_matches('\0') == "NTFS"
}
